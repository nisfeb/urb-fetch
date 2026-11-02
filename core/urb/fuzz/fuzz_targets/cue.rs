//! +cue of any bytes: a noun or none. A noun within the caps jams and
//! cues back to itself. Only a measured one: libmesa's mesa_eq (and so
//! jam's dedup) walks a noun as a tree, so two equal copies of a shared
//! noun vast as a tree take as long as the tree is big, which is why
//! every noun from another ship is measured before anything walks it, in
//! the JS and here, and why only a Measured jams.
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::Noun;
use urb::codec::measure;

fuzz_target!(|data: &[u8]| {
    if let Some(n) = Noun::cue(data)
        && let Ok(m) = measure(n.borrow())
    {
        let back = Noun::cue(&m.jam()).expect("a jam cues");
        assert!(back == n);
    }
    urb_fuzz::done();
});
