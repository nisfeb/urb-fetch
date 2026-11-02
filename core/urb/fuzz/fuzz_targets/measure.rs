//! measure, at its caps of cells and bytes
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::measure;

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        urb_fuzz::hostile(&measure(n[0].borrow()));
    }
    urb_fuzz::done();
});
