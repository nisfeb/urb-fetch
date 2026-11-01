//! the files of a directory's %snap, out of its silo: their paths and
//! their text never past OUT_MAX
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::{ball_of, cord};

fuzz_target!(|data: &[u8]| {
    if let Some(n) = urb_fuzz::nouns(data, 2) {
        let r = ball_of(n[0].borrow(), n[1].borrow(), |files| {
            files
                .into_iter()
                .map(|(p, f)| (p, cord(f.noun())))
                .collect::<Vec<_>>()
        });
        urb_fuzz::hostile(&r);
        if let Ok(Some(files)) = r {
            let mut len = 0;
            for (p, f) in &files {
                urb_fuzz::hostile(f);
                len += p.iter().map(String::len).sum::<usize>();
                len += f.as_ref().map_or(0, |t| t.as_ref().map_or(0, String::len));
            }
            urb_fuzz::bounded(len);
        }
    }
    urb_fuzz::done();
});
