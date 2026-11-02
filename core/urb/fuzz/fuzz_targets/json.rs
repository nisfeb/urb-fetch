//! $json, and Number() and TextDecoder on the input as text; a $json
//! never reads past OUT_MAX
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::{codec::json, js};

fuzz_target!(|data: &[u8]| {
    let _ = js::number(&js::utf8(data));
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        let r = json(n[0].borrow());
        urb_fuzz::hostile(&r);
        if let Ok(j) = &r {
            urb_fuzz::bounded(urb_fuzz::json_len(j));
        }
    }
    urb_fuzz::done();
});
