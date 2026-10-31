//! an error noun's lines: tang measures it, so it never fails
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::tang;

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        let _ = tang(n[0].borrow());
    }
    urb_fuzz::done();
});
