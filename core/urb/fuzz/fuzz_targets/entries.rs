//! a map's or set's entries
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::entries;

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        urb_fuzz::hostile(&entries(n[0].borrow()));
    }
    urb_fuzz::done();
});
