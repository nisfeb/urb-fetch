//! Go's `flag` package, as `flag.Parse()` reads the Go gateway's flags:
//! `-name value`, `--name value`, `-name=value`, a bool alone or as
//! `-name=false`, parsing stopping at the first argument that is no
//! flag or after `--`. Its errors and its usage are Go's words.

/// what a flag holds
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Str,
    Bool,
}

/// one flag: its name, kind, default and usage line
pub struct Flag {
    pub name: &'static str,
    pub kind: Kind,
    pub default: String,
    pub usage: &'static str,
}

/// the flags as given
#[derive(Debug, Default)]
pub struct Parsed {
    pub values: Vec<(&'static str, String)>,
}

impl Parsed {
    pub fn str(&self, name: &str, flags: &[Flag]) -> String {
        self.values
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| {
                flags
                    .iter()
                    .find(|f| f.name == name)
                    .map(|f| f.default.clone())
                    .unwrap_or_default()
            })
    }

    pub fn bool(&self, name: &str, flags: &[Flag]) -> bool {
        self.str(name, flags) == "true"
    }
}

/// why parsing stopped: the message (none for -h) and the exit code
pub struct Stop {
    pub message: Option<String>,
    pub code: i32,
}

/// strconv.ParseBool
fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Some(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

/// `flag.Parse()` of args (without the program's name)
pub fn parse(args: &[String], flags: &[Flag]) -> Result<Parsed, Stop> {
    let mut out = Parsed::default();
    let mut i = 0;
    let fail = |m: String| Stop {
        message: Some(m),
        code: 2,
    };
    while i < args.len() {
        let a = &args[i];
        if a.len() < 2 || !a.starts_with('-') {
            break;
        }
        let mut name = &a[1..];
        if let Some(n) = name.strip_prefix('-') {
            if n.is_empty() {
                break; // "--" ends the flags
            }
            name = n;
        }
        if name.is_empty() || name.starts_with('-') || name.starts_with('=') {
            return Err(fail(format!("bad flag syntax: {a}")));
        }
        i += 1;
        let (name, value) = match name.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (name, None),
        };
        let Some(f) = flags.iter().find(|f| f.name == name) else {
            if name == "help" || name == "h" {
                return Err(Stop {
                    message: None,
                    code: 0,
                });
            }
            return Err(fail(format!("flag provided but not defined: -{name}")));
        };
        let v = match (f.kind, value) {
            (Kind::Bool, Some(v)) => match parse_bool(&v) {
                Some(b) => b.to_string(),
                None => {
                    return Err(fail(format!(
                        "invalid boolean value {v:?} for -{name}: parse error"
                    )));
                }
            },
            (Kind::Bool, None) => "true".into(),
            (Kind::Str, Some(v)) => v,
            (Kind::Str, None) => match args.get(i) {
                Some(v) => {
                    i += 1;
                    v.clone()
                }
                None => return Err(fail(format!("flag needs an argument: -{name}"))),
            },
        };
        out.values.push((f.name, v));
    }
    Ok(out)
}

/// `flag.PrintDefaults()`'s text, after `Usage of <prog>:`
pub fn usage(prog: &str, flags: &[Flag]) -> String {
    let mut fs: Vec<&Flag> = flags.iter().collect();
    fs.sort_by_key(|f| f.name);
    let mut out = format!("Usage of {prog}:\n");
    for f in fs {
        out.push_str(&format!("  -{}", f.name));
        if f.kind == Kind::Str {
            out.push_str(" string");
        }
        out.push_str("\n    \t");
        out.push_str(&f.usage.replace('\n', "\n    \t"));
        if f.kind == Kind::Str && !f.default.is_empty() {
            out.push_str(&format!(" (default {:?})", f.default));
        }
        out.push('\n');
    }
    out
}
