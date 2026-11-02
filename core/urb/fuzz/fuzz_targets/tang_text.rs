//! a tang's lines, from a noun measured first from the same budget, as
//! on_poke measures the %pack it comes in; and one tank, deep. Neither
//! prints past OUT_MAX.
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::{Budget, measure_in, tang_text, tank_text};

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        let t = n[0].borrow();
        let mut b = Budget::new();
        if let Ok(m) = measure_in(t, &mut b) {
            let r = tang_text(m, &mut b);
            urb_fuzz::hostile(&r);
            urb_fuzz::bounded(r.map_or(0, |s| s.len()));
            let r = tank_text(t, &mut Budget::new(), 0);
            urb_fuzz::hostile(&r);
            urb_fuzz::bounded(r.map_or(0, |s| s.len()));
        }
    }
    urb_fuzz::done();
});
