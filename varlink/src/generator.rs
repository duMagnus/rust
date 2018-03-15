extern crate varlink_parser;

use std::env;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::io::Error as IOError;
use std::error::Error;
use std::io::{Read, Write};
use std::result::Result;
use std::fmt;
use std::collections::HashMap;
use std::iter::FromIterator;
use std::io;
use std::fs::File;

use varlink_parser::{Interface, VStructOrEnum, VType, VTypeExt, Varlink};

type EnumHash<'a> = HashMap<String, Vec<String>>;

trait ToRust {
    fn to_rust(&self, parent: &str, enumhash: &mut EnumHash) -> Result<String, ToRustError>;
}

#[derive(Debug)]
pub enum ToRustError {
    IoError(IOError),
}

impl Error for ToRustError {
    fn description(&self) -> &str {
        match *self {
            ToRustError::IoError(_) => "an I/O error occurred",
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &ToRustError::IoError(ref err) => Some(&*err as &Error),
        }
    }
}

impl From<IOError> for ToRustError {
    fn from(err: IOError) -> ToRustError {
        ToRustError::IoError(err)
    }
}

impl fmt::Display for ToRustError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())?;
        Ok(())
    }
}

impl<'a> ToRust for VType<'a> {
    fn to_rust(&self, parent: &str, enumhash: &mut EnumHash) -> Result<String, ToRustError> {
        match *self {
            VType::Bool(_) => Ok("bool".into()),
            VType::Int(_) => Ok("i64".into()),
            VType::Float(_) => Ok("f64".into()),
            VType::VString(_) => Ok("String".into()),
            VType::VData(_) => Ok("String".into()),
            VType::VTypename(v) => Ok(v.into()),
            VType::VEnum(ref v) => {
                enumhash.insert(
                    parent.into(),
                    Vec::from_iter(v.elts.iter().map(|s| String::from(*s))),
                );
                Ok(format!("{}", parent).into())
            }
            VType::VStruct(_) => Ok(format!("{}", parent).into()),
        }
    }
}

impl<'a> ToRust for VTypeExt<'a> {
    fn to_rust(&self, parent: &str, enumhash: &mut EnumHash) -> Result<String, ToRustError> {
        let v = self.vtype.to_rust(parent, enumhash)?;

        if self.isarray {
            Ok(format!("Vec<{}>", v).into())
        } else {
            Ok(v.into())
        }
    }
}

fn camel_case_to_underscore(s: &str) -> String {
    let mut out = String::from("");
    for g in s.chars() {
        match g {
            c @ 'A'...'Z' => {
                out += "_";
                out += c.to_lowercase().to_string().as_ref();
            }
            c => out += c.to_string().as_ref(),
        }
    }
    out
}

trait InterfaceToRust {
    fn to_rust(&self, description: &String) -> Result<String, ToRustError>;
}

impl<'a> InterfaceToRust for Interface<'a> {
    fn to_rust(&self, description: &String) -> Result<String, ToRustError> {
        let mut out: String = "".to_owned();
        let mut enumhash = EnumHash::new();

        for t in self.typedefs.values() {
            out += "#[derive(Serialize, Deserialize, Debug)]\n";
            match t.elt {
                VStructOrEnum::VStruct(ref v) => {
                    out += format!("pub struct {} {{\n", t.name).as_ref();
                    for e in &v.elts {
                        out += format!(
                            "    pub {}: Option<{}>,\n",
                            e.name,
                            e.vtype
                                .to_rust(format!("{}_{}", t.name, e.name).as_ref(), &mut enumhash)?
                        ).as_ref();
                    }
                }
                VStructOrEnum::VEnum(ref v) => {
                    out += format!("pub enum {} {{\n", t.name).as_ref();
                    let mut iter = v.elts.iter();
                    if let Some(fst) = iter.next() {
                        out += format!("    {}", fst).as_ref();
                        for elt in iter {
                            out += format!(",\n    {}", elt).as_ref();
                        }
                    }
                    out += "\n";
                }
            }
            out += "}\n\n";
        }

        for t in self.methods.values() {
            if t.output.elts.len() > 0 {
                out += "#[allow(non_camel_case_types)]\n#[derive(Serialize, Deserialize, Debug)]\n";
                out += format!("struct _{}Reply {{\n", t.name).as_ref();
                for e in &t.output.elts {
                    out += format!(
                        "    {}: Option<{}>,\n",
                        e.name,
                        e.vtype.to_rust(self.name, &mut enumhash)?
                    ).as_ref();
                }
                out += "}\n";
                out += format!("impl varlink::VarlinkReply for _{}Reply {{}}\n\n", t.name).as_ref();
            }

            if t.input.elts.len() > 0 {
                out += "#[allow(non_camel_case_types)]\n#[derive(Serialize, Deserialize, Debug)]\n";
                out += format!("struct _{}Args {{\n", t.name).as_ref();
                for e in &t.input.elts {
                    out += format!(
                        "    {}: Option<{}>,\n",
                        e.name,
                        e.vtype.to_rust(self.name, &mut enumhash)?
                    ).as_ref();
                }
                out += "}\n\n";
            }
        }

        for t in self.errors.values() {
            if t.parm.elts.len() > 0 {
                out += "#[allow(non_camel_case_types)]\n#[derive(Serialize, Deserialize, Debug)]\n";
                out += format!("struct _{}Args {{\n", t.name).as_ref();
                for e in &t.parm.elts {
                    out += format!(
                        "    {}: Option<{}>,\n",
                        e.name,
                        e.vtype.to_rust(self.name, &mut enumhash)?
                    ).as_ref();
                }
                out += "}\n\n";
            }
        }

        out += "pub trait _CallErr: varlink::CallTrait {\n";
        if self.errors.len() > 0 {
            for t in self.errors.values() {
                let mut inparms: String = "".to_owned();
                let mut innames: String = "".to_owned();
                if t.parm.elts.len() > 0 {
                    for e in &t.parm.elts {
                        inparms += format!(
                            ", {}: Option<{}>",
                            e.name,
                            e.vtype.to_rust(self.name, &mut enumhash)?
                        ).as_ref();
                        innames += format!("{}, ", e.name).as_ref();
                        innames.pop();
                        innames.pop();
                    }
                }

                out += format!(
                    r#"    fn reply{}(&mut self{}) -> io::Result<()> {{
        self.reply_struct(varlink::Reply::error(
            "{}.{}".into(),
"#,
                    camel_case_to_underscore(t.name),
                    inparms,
                    self.name,
                    t.name,
                ).as_ref();

                out += format!(
                    "            Some(serde_json::to_value(_{}Args {{ {} }}).unwrap()),",
                    t.name, innames
                ).as_ref();

                out += r#"
        ))
    }
"#;
            }
        }
        out += "}\nimpl<'a> _CallErr for varlink::Call<'a> {}\n\n";

        for (name, v) in &enumhash {
            out += format!("pub enum {} {{\n", name).as_ref();
            let mut iter = v.iter();
            if let Some(fst) = iter.next() {
                out += format!("    {}", fst).as_ref();
                for elt in iter {
                    out += format!(",\n    {}", elt).as_ref();
                }
            }
            out += "\n}\n\n";
        }

        for t in self.methods.values() {
            let mut inparms: String = "".to_owned();
            let mut innames: String = "".to_owned();
            if t.output.elts.len() > 0 {
                for e in &t.output.elts {
                    inparms += format!(
                        ", {}: Option<{}>",
                        e.name,
                        e.vtype.to_rust(self.name, &mut enumhash)?
                    ).as_ref();
                    innames += format!("{}, ", e.name).as_ref();
                    innames.pop();
                    innames.pop();
                }
            }
            let mut c = t.name.chars();
            let fname = match c.next() {
                None => String::from(t.name),
                Some(f) => f.to_lowercase().chain(c).collect(),
            };

            out += format!("pub trait _Call{}: _CallErr {{\n", t.name).as_ref();
            out += format!("    fn reply(&mut self{}) -> io::Result<()> {{\n", inparms).as_ref();
            out += format!(
                "        self.reply_struct(_{}Reply {{ {} }}.into())\n",
                t.name, innames
            ).as_ref();
            out += format!(
                "    }}\n}}\nimpl<'a> _Call{} for varlink::Call<'a> {{}}\n\n",
                t.name
            ).as_ref();
        }

        out += "pub trait VarlinkInterface {\n";
        for t in self.methods.values() {
            let mut inparms: String = "".to_owned();
            if t.input.elts.len() > 0 {
                for e in &t.input.elts {
                    inparms += format!(
                        ", {}: Option<{}>",
                        e.name,
                        e.vtype.to_rust(self.name, &mut enumhash)?
                    ).as_ref();
                }
            }
            let mut c = t.name.chars();
            let fname = match c.next() {
                None => String::from(t.name),
                Some(f) => f.to_lowercase().chain(c).collect(),
            };

            out += format!(
                "    fn {}(&self, &mut _Call{}{}) -> io::Result<()>;\n",
                fname, t.name, inparms
            ).as_ref();
        }
        out += r#"    fn call_upgraded(&self, &mut varlink::Call) -> io::Result<()> {
        Ok(())
    }
}
"#;

        out += format!(
            r####"
pub struct _InterfaceProxy {{
    inner: Box<VarlinkInterface + Send + Sync>,
}}

pub fn new(inner: Box<VarlinkInterface + Send + Sync>) -> _InterfaceProxy {{
    _InterfaceProxy {{ inner }}
}}

impl varlink::Interface for _InterfaceProxy {{
    fn get_description(&self) -> &'static str {{
        r#"
{}
"#
    }}

    fn get_name(&self) -> &'static str {{
        "{}"
    }}

"####,
            description, self.name
        ).as_ref();

        out += r#"    fn call_upgraded(&self, call: &mut varlink::Call) -> io::Result<()> {
        self.inner.call_upgraded(call)
    }

    fn call(&self, call: &mut varlink::Call) -> io::Result<()> {
        let req = call.request.unwrap();
        let method = req.method.clone();
        match method.as_ref() {
"#;

        for t in self.methods.values() {
            let mut inparms: String = "".to_owned();
            if t.input.elts.len() > 0 {
                let ref e = t.input.elts[0];
                inparms += format!("args.{}", e.name).as_ref();
                for e in &t.input.elts[1..] {
                    inparms += format!(", args.{}, ", e.name).as_ref();
                }
            }
            let mut c = t.name.chars();
            let fname = match c.next() {
                None => String::from(t.name),
                Some(f) => f.to_lowercase().chain(c).collect(),
            };

            out += format!("            \"{}.{}\" => {{", self.name, t.name).as_ref();
            if t.input.elts.len() > 0 {
                out +=
                    format!(
                        concat!("\n                if let Some(args) = req.parameters.clone() {{\n",
"                    let args: _{}Args = serde_json::from_value(args)?;\n",
"                    return self.inner.{}(call as &mut _Call{}, {});\n",
"                }} else {{\n",
"                    return call.reply_invalid_parameter(None);\n",
"                }}\n",
"            }}\n"),
                        t.name,
                        fname, t.name,
                        inparms
                    ).as_ref();
            } else {
                out += format!(
                    "\n                return self.inner.{}(call as &mut _Call{});\n            }}\n",
                    fname, t.name
                ).as_ref();
            }
        }
        out += concat!(
            "\n",
            "            m => {\n",
            "                let method: String = m.clone().into();\n",
            "                return call.reply_method_not_found(Some(method));\n",
            "            }\n",
            "        }\n",
            "    }\n",
            "}"
        );

        Ok(out)
    }
}

pub fn generate(mut reader: Box<Read>, mut writer: Box<Write>) -> io::Result<()> {
    let mut buffer = String::new();

    reader.read_to_string(&mut buffer)?;

    let vr = Varlink::from_string(&buffer);

    if let Err(e) = vr {
        eprintln!("{}", e);
        exit(1);
    }

    match vr.unwrap().interface.to_rust(&buffer) {
        Ok(out) => {
            writeln!(
                writer,
                r#"// This file is automatically generated by the varlink rust generator
use std::io;

use varlink;
use serde_json;

{}"#,
                out
            )?;
        }
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    }

    Ok(())
}

/// Errors are emitted to stderr and terminate the process.
pub fn cargo_build<T: AsRef<Path> + ?Sized>(input_path: &T) {
    let mut stderr = io::stderr();
    let input_path = input_path.as_ref();

    let reader: Box<Read>;

    let out_dir: PathBuf = env::var_os("OUT_DIR").unwrap().into();
    let rust_path = out_dir
        .join(input_path.file_name().unwrap())
        .with_extension("rs");

    let writer: Box<Write> = Box::new(File::create(&rust_path).unwrap());

    match File::open(input_path) {
        Ok(r) => reader = Box::new(r),
        Err(e) => {
            writeln!(
                stderr,
                "Could not read varlink input file `{}`: {}",
                input_path.display(),
                e
            ).unwrap();
            exit(1);
        }
    }
    if let Err(e) = generate(reader, writer) {
        writeln!(
            stderr,
            "Could not generate rust code from varlink file `{}`: {}",
            input_path.display(),
            e
        ).unwrap();
        exit(1);
    }

    println!("cargo:rerun-if-changed={}", input_path.display());
}