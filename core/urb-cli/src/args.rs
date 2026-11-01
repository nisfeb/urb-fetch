//! Node's util.parseArgs, strict, as the Node command lines call it:
//! `--name value` and `--name=value`, booleans, options given more than
//! once, positionals, `--`, and its errors, word for word.

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Str,
    Bool,
    /// a string option that may be given again and again
    Many,
}

/// what was given: each option's values (a boolean's is empty), and the
/// positionals
#[derive(Default, Debug)]
pub struct Parsed {
    pub given: HashMap<String, Vec<String>>,
    pub positionals: Vec<String>,
}

impl Parsed {
    pub fn str(&self, name: &str) -> Option<String> {
        self.given.get(name).and_then(|v| v.last().cloned())
    }

    pub fn bool(&self, name: &str) -> bool {
        self.given.contains_key(name)
    }

    pub fn many(&self, name: &str) -> Vec<String> {
        self.given.get(name).cloned().unwrap_or_default()
    }
}

/// an error parseArgs throws: its code and message
pub struct ArgError {
    pub code: &'static str,
    pub message: String,
}

fn unknown(a: &str) -> ArgError {
    ArgError {
        code: "ERR_PARSE_ARGS_UNKNOWN_OPTION",
        message: format!(
            "Unknown option '{a}'. To specify a positional argument starting with a '-', \
             place it at the end of the command after '--', as in '-- \"{a}\""
        ),
    }
}

fn invalid(message: String) -> ArgError {
    ArgError {
        code: "ERR_PARSE_ARGS_INVALID_OPTION_VALUE",
        message,
    }
}

pub fn parse(
    args: &[String],
    options: &[(&str, Kind)],
    positionals: bool,
) -> Result<Parsed, ArgError> {
    let kind = |n: &str| options.iter().find(|o| o.0 == n).map(|o| o.1);
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            for rest in &args[i..] {
                if !positionals {
                    return Err(unexpected(rest));
                }
                p.positionals.push(rest.clone());
            }
            break;
        }
        if let Some(long) = a.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let Some(k) = kind(name) else {
                return Err(unknown(&format!("--{name}")));
            };
            match k {
                Kind::Bool => {
                    if inline.is_some() {
                        return Err(invalid(format!(
                            "Option '--{name}' does not take an argument"
                        )));
                    }
                    p.given.entry(name.into()).or_default();
                }
                Kind::Str | Kind::Many => {
                    let v = match inline {
                        Some(v) => v,
                        None => match args.get(i) {
                            None => {
                                return Err(invalid(format!(
                                    "Option '--{name} <value>' argument missing"
                                )));
                            }
                            Some(v) if v.starts_with('-') => {
                                return Err(invalid(format!(
                                    "Option '--{name}' argument is ambiguous.\nDid you forget to \
                                     specify the option argument for '--{name}'?\nTo specify an \
                                     option argument starting with a dash use '--{name}=-XYZ'."
                                )));
                            }
                            Some(v) => {
                                i += 1;
                                v.clone()
                            }
                        },
                    };
                    let e = p.given.entry(name.into()).or_default();
                    if k == Kind::Str {
                        e.clear();
                    }
                    e.push(v);
                }
            }
            continue;
        }
        if a.len() > 1 && a.starts_with('-') {
            return Err(unknown(&a[..2]));
        }
        if !positionals {
            return Err(unexpected(a));
        }
        p.positionals.push(a.clone());
    }
    Ok(p)
}

fn unexpected(a: &str) -> ArgError {
    ArgError {
        code: "ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL",
        message: format!(
            "Unexpected argument '{a}'. This command does not take positional arguments"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Parsed, String> {
        let a: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        parse(
            &a,
            &[
                ("state", Kind::Str),
                ("trace", Kind::Bool),
                ("put", Kind::Many),
            ],
            true,
        )
        .map_err(|e| format!("{} {}", e.code, e.message))
    }

    #[test]
    fn as_parse_args() {
        let x = p(&["--state", "d", "u", "--trace", "--put=a=b", "--put", "c=d"]).unwrap();
        assert_eq!(x.str("state").as_deref(), Some("d"));
        assert!(x.bool("trace"));
        assert_eq!(x.many("put"), vec!["a=b", "c=d"]);
        assert_eq!(x.positionals, vec!["u"]);
        assert_eq!(p(&["--", "--x"]).unwrap().positionals, vec!["--x"]);
        assert!(
            p(&["--x"])
                .unwrap_err()
                .starts_with("ERR_PARSE_ARGS_UNKNOWN_OPTION Unknown option '--x'")
        );
        assert_eq!(
            p(&["--state"]).unwrap_err(),
            "ERR_PARSE_ARGS_INVALID_OPTION_VALUE Option '--state <value>' argument missing"
        );
        assert!(p(&["--state", "-x"]).unwrap_err().contains("is ambiguous"));
        assert!(
            p(&["--trace=1"])
                .unwrap_err()
                .contains("does not take an argument")
        );
        assert!(p(&["-x"]).unwrap_err().contains("Unknown option '-x'"));
    }
}
