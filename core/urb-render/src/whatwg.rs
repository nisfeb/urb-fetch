//! The part of a WHATWG URL parser the extension's `linkTarget` uses:
//! `new URL(href, "http://x" + basePath).pathname`, for an href that
//! `linkTarget` has already let through, which has no scheme, no
//! colon, no control character or space, and does not start with two
//! slashes.

use crate::cost;

/// the path percent-encode set: C0 controls, space, `"` `#` `<` `>`
/// `?` `^` `` ` `` `{` `}`, and anything past `~`
fn escaped(c: char) -> bool {
    !(' '..='~').contains(&c)
        || matches!(c, ' ' | '"' | '#' | '<' | '>' | '?' | '^' | '`' | '{' | '}')
}

fn single_dot(s: &str) -> bool {
    s == "." || s.eq_ignore_ascii_case("%2e")
}

fn double_dot(s: &str) -> bool {
    s == ".."
        || s.eq_ignore_ascii_case(".%2e")
        || s.eq_ignore_ascii_case("%2e.")
        || s.eq_ignore_ascii_case("%2e%2e")
}

/// the path state of a special URL, from the start of `input` to its
/// end or its first `?` or `#`, onto `path`
fn path_state(path: &mut Vec<String>, input: &str) {
    let mut buf = String::new();
    let mut chars = input.chars();
    loop {
        let c = chars.next();
        cost::step(1);
        match c {
            None | Some('/' | '\\' | '?' | '#') => {
                let slash = matches!(c, Some('/' | '\\'));
                if double_dot(&buf) {
                    path.pop();
                    if !slash {
                        path.push(String::new());
                    }
                } else if single_dot(&buf) {
                    if !slash {
                        path.push(String::new());
                    }
                } else {
                    path.push(std::mem::take(&mut buf));
                }
                buf.clear();
                if !slash {
                    return;
                }
            }
            Some(c) if escaped(c) => {
                let mut b = [0; 4];
                for x in c.encode_utf8(&mut b).bytes() {
                    buf.push_str(&format!("%{x:02X}"));
                }
            }
            Some(c) => buf.push(c),
        }
    }
}

/// `new URL(href, "http://x" + base).pathname`, base a path that
/// starts with `/` and has no `?` or `#`
pub(crate) fn pathname(base: &str, href: &str) -> String {
    //  the base as the parser reads it: C0 controls and spaces off its
    //  end, tabs and newlines out of it, then the path after the host
    let base: String = base
        .trim_end_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    cost::step(base.len());
    let mut base_path = Vec::new();
    path_state(&mut base_path, base.strip_prefix('/').unwrap_or(&base));
    let mut path;
    match href.chars().next() {
        Some('/' | '\\') => {
            path = Vec::new();
            path_state(&mut path, &href[1..]);
        }
        Some('?' | '#') => path = base_path,
        _ => {
            path = base_path;
            path.pop();
            path_state(&mut path, href);
        }
    }
    let mut out = String::new();
    for s in &path {
        out.push('/');
        out.push_str(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::pathname;

    #[test]
    fn paths() {
        assert_eq!(pathname("/a/b", "c"), "/a/c");
        assert_eq!(pathname("/a/b", ".."), "/");
        assert_eq!(pathname("/a/b", "../.."), "/");
        assert_eq!(pathname("/a/b", "./"), "/a/");
        assert_eq!(pathname("/a/b", "."), "/a/");
        assert_eq!(pathname("/a/b", "?q"), "/a/b");
        assert_eq!(pathname("/a/b", "#f"), "/a/b");
        assert_eq!(pathname("/a/b", "/x/%2E%2e/y"), "/y");
        assert_eq!(pathname("/a/b", "\\x\\y"), "/x/y");
        assert_eq!(pathname("/a/b", "é^{}"), "/a/%C3%A9%5E%7B%7D");
        assert_eq!(pathname("/a/b c \t", "x"), "/a/x");
        assert_eq!(pathname("/a/b", "x/..?y"), "/a/");
    }
}
