//! an url, as deUrb resolves it: any text gives a resolved url or one of
//! deUrb's errors, never a panic
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::url::{Mount, de_urb};

fuzz_target!(|data: &[u8]| {
    let url = String::from_utf8_lossy(data);
    match de_urb(&url) {
        Ok(u) => {
            assert!(u.ship.starts_with('~'));
            if let Mount::Forum { post: Some(p) } = &u.mount {
                assert!(u.rel.len() == 1 && !p.le().is_empty() || p.is_zero());
            }
        }
        Err(e) => {
            let m = e.message;
            assert!(
                m == "not an urb:// url"
                    || m.starts_with("no ship in ")
                    || m.starts_with("bad path in ")
                    || m.ends_with("/ is not a mount"),
                "{m}"
            );
        }
    }
});
