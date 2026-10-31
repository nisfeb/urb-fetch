//! the files of a directory's %snap, out of its silo
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::{ball_of, cord};

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 2) {
        let r = ball_of(n[0].borrow(), n[1].borrow(), |files| {
            files
                .into_iter()
                .map(|(p, f)| (p, cord(f)))
                .collect::<Vec<_>>()
        });
        urb_fuzz::hostile(&r);
        if let Ok(Some(files)) = r {
            for (_, f) in &files {
                urb_fuzz::hostile(f);
            }
        }
    }
    urb_fuzz::done();
});
