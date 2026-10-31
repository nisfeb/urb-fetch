//! Numbers and dates as Hoon writes and keeps them (mesa.mjs's uv, da).

use crate::Int;

/// A number as @uv text, as `(scot %uv n)` writes it: 0v, then base-32
/// digits in dotted groups of five from the right.
pub fn uv(n: &Int) -> String {
    const DIGITS: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";
    let b = n.le();
    let bits = b.len() * 8;
    let mut d: Vec<u8> = (0..bits.div_ceil(5))
        .map(|i| {
            let mut v = 0usize;
            for k in 0..5 {
                let at = i * 5 + k;
                if at < bits && b[at / 8] >> (at % 8) & 1 == 1 {
                    v |= 1 << k;
                }
            }
            DIGITS[v]
        })
        .collect();
    while d.len() > 1 && d.last() == Some(&b'0') {
        d.pop();
    }
    if d.is_empty() {
        d.push(b'0');
    }
    d.reverse();
    let head = match d.len() % 5 {
        0 => 5,
        h => h,
    };
    let mut out = String::from("0v");
    out.push_str(std::str::from_utf8(&d[..head]).unwrap_or_default());
    for g in d[head..].chunks(5) {
        out.push('.');
        out.push_str(std::str::from_utf8(g).unwrap_or_default());
    }
    out
}

/// Milliseconds since the epoch as a @da. The JS takes any number; every
/// caller passes a whole number of milliseconds since 1970.
pub fn da(ms: u64) -> u128 {
    let secs = u128::from(ms / 1000) + 0x8000000cce9e0d80;
    (secs << 64) | ((u128::from(ms % 1000) << 64) / 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv_as_scot_writes_it() {
        assert_eq!(uv(&Int::default()), "0v0");
        assert_eq!(uv(&31u64.into()), "0vv");
        assert_eq!(uv(&32u64.into()), "0v10");
        assert_eq!(uv(&0xdeadbeefu64.into()), "0v3f.arfnf");
        assert_eq!(uv(&u128::MAX.into()), "0v7.vvvvv.vvvvv.vvvvv.vvvvv.vvvvv");
    }

    #[test]
    fn da_of_the_epoch() {
        assert_eq!(da(0), 170141184475152167957503069145530368000);
        assert_eq!(da(1761600000123), 170141184507647952320018764793342242848);
    }
}
