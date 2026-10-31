//! a ship's name, from its atom
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::ship;

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        urb_fuzz::hostile(&ship(n[0].borrow()));
    }
    urb_fuzz::done();
});
