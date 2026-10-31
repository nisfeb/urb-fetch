//! The bindings link and call into libmesa, natively and under WASI.
//! bindgen's own layout tests check every struct against the headers.

use mesa_sys::*;

#[test]
fn none_is_the_headers_value() {
    // MESA_NONE is ((mesa_noun)UINT64_MAX) in noun.h
    assert_eq!(MESA_NONE, u64::MAX);
}

#[test]
fn jam_and_cue_round_trip() {
    unsafe {
        let cord = c"urb-fetch";
        let n = mesa_cell(mesa_atom_u64(42), mesa_atom_cord(cord.as_ptr()));
        let mut len = 0usize;
        let buf = mesa_jam_bytes(n, &mut len);
        assert!(!buf.is_null() && len > 0);
        let back = mesa_cue_bytes(len, buf);
        mesa_free(buf.cast());
        assert_ne!(back, MESA_NONE);
        assert!(mesa_eq(n, back));
        assert_eq!(mesa_mug(n), mesa_mug(back));
        let mut v = 0u64;
        assert!(mesa_atom_get_u64(mesa_head(back), &mut v));
        assert_eq!(v, 42);
        mesa_unref(back);
        mesa_unref(n);
    }
}

#[test]
fn jam_of_a_known_noun() {
    // (jam [1 2]) is 4.657 (0x1231) in vere
    unsafe {
        let n = mesa_cell(mesa_atom_u64(1), mesa_atom_u64(2));
        let j = mesa_jam(n);
        let mut v = 0u64;
        assert!(mesa_atom_get_u64(j, &mut v));
        assert_eq!(v, 0x1231);
        mesa_unref(j);
        mesa_unref(n);
    }
}

#[test]
fn a_bad_cue_is_none() {
    unsafe {
        let junk = [0u8; 0];
        assert_eq!(mesa_cue_bytes(0, junk.as_ptr()), MESA_NONE);
    }
}
