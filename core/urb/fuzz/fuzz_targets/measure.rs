//! measure, at its cap and at a small one
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::{MAX_CELLS, measure};

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        urb_fuzz::hostile(&measure(n[0].borrow(), MAX_CELLS));
        urb_fuzz::hostile(&measure(n[0].borrow(), 16));
    }
    urb_fuzz::done();
});
