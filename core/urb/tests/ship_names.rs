//! urb-render's ship names, held to libmesa's: `urb_render::ship::parses`
//! must read a name exactly when `mesa_ship_parse` does, as the Go
//! gateway's `mesa.ParseShip` calls it.

use urb::engine::ship_parse;
use urb_render::ship::parses;

const SIS: &str = "dozmarbinwansamlitsighidfidlissogdirwacsabwissibrigsoldopmodfoglidhopdardorlorhodfolrintogsilmir";
const DEX: &str = "zodnecbudwessevpersutletfulpensytdurwepserwylsunrypsyxdyrnuphebpeglupdepdysputlughecryttyvsydnex";

/// a small deterministic generator, so a failure repeats
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn ship_names_as_libmesa_reads_them() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let syl = |t: &str, i: usize| t[i * 3..i * 3 + 3].to_string();
    let mut names: Vec<String> = vec![
        "~zod".into(),
        "~dozzod".into(),
        "~doznec".into(),
        "~marzod".into(),
        "~abc".into(),
        "".into(),
        "~".into(),
        "zod".into(),
        "~~zod".into(),
        "~zod-".into(),
        "~-zod".into(),
        "~sampel-palnet".into(),
        "~sampel--palnet".into(),
        "~dozzod-dozzod".into(),
        "~dozzod-marzod".into(),
    ];
    for _ in 0..20_000 {
        let groups = 1 + rng.pick(9);
        let mut s = String::from(if rng.pick(8) == 0 { "" } else { "~" });
        for g in 0..groups {
            if g > 0 {
                s += ["-", "--", "", "-"][rng.pick(4)];
            }
            //  mostly the low syllables, so leading zero chunks come up
            let wide = if rng.pick(2) == 0 { 2 } else { 32 };
            let (p, q) = (rng.pick(wide), rng.pick(32));
            if groups == 1 && rng.pick(3) == 0 {
                s += &syl(DEX, q);
            } else {
                s += &syl(SIS, p);
                s += &syl(DEX, q);
            }
        }
        if rng.pick(20) == 0 {
            s.insert(rng.pick(s.len() + 1), ['x', '-', 'Z', '~'][rng.pick(4)]);
        }
        names.push(s);
    }
    let mut yes = 0;
    for n in &names {
        let mesa = ship_parse(&if n.starts_with('~') {
            n.clone()
        } else {
            format!("~{n}")
        })
        .is_some();
        assert_eq!(parses(n.as_bytes()), mesa, "{n:?}");
        yes += usize::from(mesa);
    }
    assert!(
        yes > 1000 && yes < names.len() - 1000,
        "{yes} of {}",
        names.len()
    );
}
