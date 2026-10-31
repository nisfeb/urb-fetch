//! a tang's lines, from a noun measured first, as on_poke measures the
//! %pack it comes in; and one tank, deep
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::{MAX_CELLS, measure, tang_text, tank_text};

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        let t = n[0].borrow();
        if measure(t, MAX_CELLS).is_ok() {
            urb_fuzz::hostile(&tang_text(t));
            urb_fuzz::hostile(&tank_text(t, 0));
        }
    }
    urb_fuzz::done();
});
