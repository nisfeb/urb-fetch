//! What the JS does for free that the port has to spell out: how a
//! TextDecoder reads bytes, how Number() reads text, and the order a JS
//! object keeps its keys in.

use std::collections::HashMap;

/// Bytes as `new TextDecoder().decode()` reads them: UTF-8, each maximal
/// invalid subpart one U+FFFD (as Rust's lossy decoding does too), and a
/// leading byte-order mark dropped.
pub fn utf8(bytes: &[u8]) -> String {
    let b = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    String::from_utf8_lossy(b).into_owned()
}

/// JS's StrWhiteSpaceChar: WhiteSpace and LineTerminator (not Rust's
/// White_Space, which has U+0085 and lacks U+FEFF)
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `Number(text)`: JS's StringToNumber. Blank is 0; anything that isn't
/// a StringNumericLiteral is NaN.
pub fn number(text: &str) -> f64 {
    let t = text.trim_matches(js_space);
    if t.is_empty() {
        return 0.0;
    }
    let b = t.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            return radix_number(&t[2..], radix);
        }
    }
    let (sign, rest) = match b[0] {
        b'-' => (-1.0, &t[1..]),
        b'+' => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    if rest == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !decimal_literal(rest) {
        return f64::NAN;
    }
    // Rust's parse is correctly rounded, as JS's is
    rest.parse::<f64>().map_or(f64::NAN, |v| sign * v)
}

/// StrUnsignedDecimalLiteral, less Infinity: digits, an optional point
/// and fraction (at least one digit in all), and an optional exponent
fn decimal_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let at = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - at
    };
    let mut n = digits(&mut i);
    if i < b.len() && b[i] == b'.' {
        i += 1;
        n += digits(&mut i);
    }
    if n == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

/// a 0x, 0o or 0b literal's digits, rounded to the nearest double, ties
/// to even, as JS rounds them
fn radix_number(digits: &str, radix: u32) -> f64 {
    // folded as they come: the first 53 significant bits, the next (half)
    // and whether any after it is set (sticky), so a long literal costs no
    // memory
    let per = radix.trailing_zeros();
    let (mut len, mut mant, mut half, mut sticky) = (0usize, 0u64, false, false);
    for c in digits.chars() {
        let Some(d) = c.to_digit(radix) else {
            return f64::NAN;
        };
        for k in (0..per).rev() {
            let x = d >> k & 1 == 1;
            match len {
                0 if !x => continue,
                0..53 => mant = mant << 1 | u64::from(x),
                53 => half = x,
                _ => sticky |= x,
            }
            len += 1;
        }
    }
    if len <= 53 {
        // exact: mant is under 2^53
        return mant as f64;
    }
    if half && (sticky || mant & 1 == 1) {
        mant += 1;
    }
    let exp = i32::try_from(len - 53).unwrap_or(i32::MAX);
    if exp > 1023 {
        return f64::INFINITY;
    }
    // mant is at most 2^53, so this is exact until it overflows
    mant as f64 * 2f64.powi(exp)
}

/// A number as JS's Number.prototype.toString writes it: the shortest
/// digits that read back as it, in JS's choice of plain or exponent form.
pub fn number_text(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x < 0.0 {
        return format!("-{}", number_text(-x));
    }
    if x.is_infinite() {
        return "Infinity".into();
    }
    // Rust's {:e} gives the shortest round-trip digits too
    let e = format!("{x:e}");
    let (mant, exp) = e.split_once('e').expect("{:e} has an e");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i64;
    let n = exp.parse::<i64>().expect("an exponent") + 1;
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign}{}", e.abs())
        } else {
            format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs())
        }
    }
}

/// A JS value as mesa.mjs's json() returns it.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Object),
}

/// A JS object made by Object.fromEntries: a key set twice keeps its
/// first place and its last value, and the keys come out as JS orders
/// them, array indices first, in numeric order, then the rest in the
/// order they were first set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object {
    keys: Vec<(String, Json)>,
    at: HashMap<String, usize>,
}

impl Object {
    pub fn set(&mut self, key: String, value: Json) {
        match self.at.get(&key) {
            Some(&i) => self.keys[i].1 = value,
            None => {
                self.at.insert(key.clone(), self.keys.len());
                self.keys.push((key, value));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        self.at.get(key).map(|&i| &self.keys[i].1)
    }

    /// the entries, in the order Object.entries gives them
    pub fn entries(&self) -> Vec<(&str, &Json)> {
        let mut idx: Vec<(u32, &str, &Json)> = Vec::new();
        let mut rest = Vec::new();
        for (k, v) in &self.keys {
            match array_index(k) {
                Some(n) => idx.push((n, k, v)),
                None => rest.push((k.as_str(), v)),
            }
        }
        idx.sort_by_key(|e| e.0);
        idx.into_iter()
            .map(|(_, k, v)| (k, v))
            .chain(rest)
            .collect()
    }
}

/// a key that is an array index: the canonical text of 0 to 2^32 - 2
fn array_index(k: &str) -> Option<u32> {
    let b = k.as_bytes();
    if b.is_empty()
        || b.len() > 10
        || !b.iter().all(u8::is_ascii_digit)
        || (b[0] == b'0' && b.len() > 1)
    {
        return None;
    }
    k.parse::<u64>()
        .ok()
        .filter(|&n| n < u64::from(u32::MAX))
        .map(|n| n as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_as_js_writes_them() {
        for (x, t) in [
            (1.0, "1"),
            (-1.0, "-1"),
            (1.5, "1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (2f64.powi(53), "9007199254740992"),
            (f64::NAN, "NaN"),
            (-f64::INFINITY, "-Infinity"),
            (-0.0, "0"),
        ] {
            assert_eq!(number_text(x), t, "{x}");
        }
    }

    #[test]
    fn numbers_as_js_reads_them() {
        let nan = |s: &str| number(s).is_nan();
        assert_eq!(number(""), 0.0);
        assert_eq!(number(" \n\u{feff}12\u{3000}"), 12.0);
        assert!(nan("\u{85}12"));
        assert_eq!(number("1500"), 1500.0);
        assert_eq!(number("-1.5e3"), -1500.0);
        assert_eq!(number(".5"), 0.5);
        assert_eq!(number("5."), 5.0);
        assert!(nan("."));
        assert!(nan("1e"));
        assert!(nan("1_000"));
        assert!(nan("inf"));
        assert!(nan("NaN"));
        assert!(nan("infinity"));
        assert_eq!(number("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(number("0x10"), 16.0);
        assert_eq!(number("0B101"), 5.0);
        assert_eq!(number("0o17"), 15.0);
        assert!(nan("-0x10"));
        assert!(nan("0x"));
        assert!(nan("0xg"));
        assert_eq!(number("0x20000000000001"), 9007199254740992.0);
        assert_eq!(number("0x20000000000003"), 9007199254740996.0);
        assert_eq!(number("0x20000000000002"), 9007199254740994.0);
        assert_eq!(number(&format!("0x1{}", "0".repeat(256))), f64::INFINITY);
        assert_eq!(number("1e400"), f64::INFINITY);
        assert!(number("-0").is_sign_negative());
    }

    #[test]
    fn objects_keep_js_order() {
        let mut o = Object::default();
        for (k, v) in [
            ("b", 1.0),
            ("2", 2.0),
            ("a", 3.0),
            ("1", 4.0),
            ("b", 5.0),
            ("01", 6.0),
            ("4294967295", 7.0),
        ] {
            o.set(k.into(), Json::Num(v));
        }
        let keys: Vec<&str> = o.entries().iter().map(|e| e.0).collect();
        assert_eq!(keys, ["1", "2", "b", "a", "01", "4294967295"]);
        assert_eq!(o.get("b"), Some(&Json::Num(5.0)));
    }

    #[test]
    fn text_as_a_text_decoder_reads_it() {
        assert_eq!(utf8(b"\xef\xbb\xbfo"), "o");
        assert_eq!(utf8(b"\xef\xbb\xbf\xef\xbb\xbfo"), "\u{feff}o");
        assert_eq!(utf8(b"a\xffb"), "a\u{fffd}b");
        assert_eq!(utf8(b"\xe2\x82"), "\u{fffd}");
        assert_eq!(utf8(b"\xf0\x80\x80"), "\u{fffd}\u{fffd}\u{fffd}");
    }
}
