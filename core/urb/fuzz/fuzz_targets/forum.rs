//! What furum.mjs and forum.mjs read out of a host's grubs: each of
//! furum's buckets and notes, and lattice's board grubs, read into a
//! board (two grubs, each at a place the input picks) that is then
//! assembled, its thread found and its JS made, and its refusal notes.
//! Whatever the nouns, no decoder panics, every error is a hostile-noun
//! error or one of forum.mjs's own, every noun is let go, and nothing
//! given back is past OUT_MAX.
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::measure;
use urb::forum::{board, furum};
use urb::{Int, Noun};
use urb_fuzz::{board_error, bounded, js_len};

const PAX: &[&str] = &[
    "pub/card",
    "pub/roles",
    "pub/conf",
    "content/pins",
    "content/sidebar",
    "content/posts/b0",
    "content/threads/b0",
    "content/votes/b0",
    "content/other/b0",
    "pub/other",
];

fn one(n: &Noun, pick: u8, id: f64) {
    let Ok(m) = measure(n.borrow()) else { return };
    match pick % 6 {
        0 => board_error(&furum::next_post(m)),
        1 => {
            let r = furum::posts(m);
            board_error(&r);
            if let Ok(mut p) = r {
                furum::sort_posts(&mut p);
                bounded(p.iter().map(|p| js_len(&p.js())).sum());
            }
        }
        2 => {
            let r = furum::comments(m, id);
            board_error(&r);
            if let Ok(Some(mut c)) = r {
                furum::sort_comments(&mut c);
                bounded(c.iter().map(|c| js_len(&c.js())).sum());
            }
        }
        3 => {
            let r = furum::votes(m, id);
            board_error(&r);
            if let Ok(Some((up, down))) = r {
                bounded(js_len(&furum::votes_js(&up, &down)));
            }
        }
        4 => board_error(&furum::note(furum::MSG, m)),
        _ => {
            let to = (board::BASES[0], "forum.sig");
            board_error(&board::note("~zod", "~zod", to, board::NOTE, m));
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let [.., a, b, c] = data else { return };
    if let Some(n) = urb_fuzz::nouns(data, 2) {
        if c % 2 == 0 {
            one(&n[0], a / 2, f64::from(*b));
        } else {
            let mut acc = board::Acc::new("~zod");
            let mut ok = true;
            for (k, x) in [(a, &n[0]), (b, &n[1])] {
                let Ok(m) = measure(x.borrow()) else { continue };
                let pax: Vec<String> = PAX[*k as usize % PAX.len()]
                    .split('/')
                    .map(String::from)
                    .collect();
                let r = acc.read(&pax, m);
                board_error(&r);
                ok &= r.is_ok();
            }
            if ok {
                let r = acc.assemble();
                board_error(&r);
                if let Ok(bd) = r {
                    bounded(js_len(&bd.js()));
                    let t = bd.thread("~zod", "b", &Int::from(u64::from(*c / 2)));
                    board_error(&t);
                    if let Ok(t) = t {
                        bounded(js_len(&t));
                    }
                }
            }
        }
    }
    urb_fuzz::done();
});
