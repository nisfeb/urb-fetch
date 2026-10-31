//! +cue of any bytes: a noun or none. A noun within the cell cap jams
//! and cues back to itself. Only a measured one: libmesa's mesa_eq (and
//! so jam's dedup) walks a noun as a tree, so two equal copies of a
//! shared noun vast as a tree take as long as the tree is big, which is
//! why every noun from another ship is measured before anything walks
//! it, in the JS and here.
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::Noun;
use urb::codec::{MAX_CELLS, measure};

fuzz_target!(|data: &[u8]| {
    if let Some(n) = Noun::cue(data)
        && measure(n.borrow(), MAX_CELLS).is_ok()
    {
        let back = Noun::cue(&n.borrow().jam()).expect("a jam cues");
        assert!(back == n);
    }
    urb_fuzz::done();
});
