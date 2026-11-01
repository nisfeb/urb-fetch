//! Go's `log` package as the gateway uses it: each line on stderr,
//! after the local date and time, `2006/01/02 15:04:05 `.

use std::ffi::c_int;
use std::io::Write;

#[repr(C)]
struct Tm {
    sec: c_int,
    min: c_int,
    hour: c_int,
    mday: c_int,
    mon: c_int,
    year: c_int,
    wday: c_int,
    yday: c_int,
    isdst: c_int,
    gmtoff: std::ffi::c_long,
    zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
}

/// `log.Printf`'s prefix: the local time, as `2006/01/02 15:04:05 `
pub fn stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    if unsafe { localtime_r(&now, &mut tm) }.is_null() {
        return String::new();
    }
    format!(
        "{:04}/{:02}/{:02} {:02}:{:02}:{:02} ",
        tm.year + 1900,
        tm.mon + 1,
        tm.mday,
        tm.hour,
        tm.min,
        tm.sec
    )
}

/// `log.Print(line)`: one line on stderr, written whole
pub fn line(text: &str) {
    let mut s = stamp();
    s.push_str(text);
    if !s.ends_with('\n') {
        s.push('\n');
    }
    let _ = std::io::stderr().write_all(s.as_bytes());
}

/// `log.Fatal(text)`: the line, then exit 1
pub fn fatal(text: &str) -> ! {
    line(text);
    std::process::exit(1)
}
