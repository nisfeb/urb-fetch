//! $json, and Number() and TextDecoder on the input as text
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::{codec::json, js};

fuzz_target!(|data: &[u8]| {
    let _ = js::number(&js::utf8(data));
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        urb_fuzz::hostile(&json(n[0].borrow()));
    }
    urb_fuzz::done();
});
