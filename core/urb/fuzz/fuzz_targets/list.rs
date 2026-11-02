//! list, walked whole
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::items;

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        urb_fuzz::hostile(&items(n[0].borrow()));
    }
    urb_fuzz::done();
});
