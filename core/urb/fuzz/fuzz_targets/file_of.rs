//! the file a %snap names, out of its silo
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::{cord, file_of};

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 2) {
        urb_fuzz::hostile(&file_of(n[0].borrow(), n[1].borrow(), cord));
    }
    urb_fuzz::done();
});
