//! an error noun's lines: tang measures it, so it never fails, and it
//! never prints past OUT_MAX
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::tang;

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        urb_fuzz::bounded(tang(n[0].borrow()).len());
    }
    urb_fuzz::done();
});
