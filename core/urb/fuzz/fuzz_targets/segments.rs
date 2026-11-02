//! a path's segments, its text, and one knot
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::{knot, path_of, segments};

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        let p = n[0].borrow();
        urb_fuzz::hostile(&segments(p));
        urb_fuzz::hostile(&path_of(p));
        urb_fuzz::hostile(&knot(p));
    }
    urb_fuzz::done();
});
