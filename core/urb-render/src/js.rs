//! JavaScript's reading of text, where a rule turns on it: its
//! whitespace (what `\s` matches and `trim()` trims), its line
//! terminators (what `.` refuses), and lengths in UTF-16 code units.

use crate::cost;

/// JavaScript's whitespace: `\s`, and what `trim()` trims
pub fn is_space(c: char) -> bool {
    let named = matches!(
        c,
        '\t' | '\n'
            | '\u{b}'
            | '\u{c}'
            | '\r'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2028}'
            | '\u{2029}'
    ) || matches!(c, '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}');
    named || ('\u{2000}'..='\u{200a}').contains(&c)
}

/// a line terminator, which `.` in a regex does not match
pub fn is_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `s.trim()`
pub fn trim(s: &str) -> &str {
    trim_end(trim_start(s))
}

/// `s.trimStart()`
pub fn trim_start(s: &str) -> &str {
    let t = s.trim_start_matches(is_space);
    cost::step(s.len() - t.len() + 1);
    t
}

/// `s.trimEnd()`
pub fn trim_end(s: &str) -> &str {
    let t = s.trim_end_matches(is_space);
    cost::step(s.len() - t.len() + 1);
    t
}

/// `s.length`: UTF-16 code units
pub fn units(s: &str) -> usize {
    cost::step(s.len());
    s.chars().map(char::len_utf16).sum()
}
