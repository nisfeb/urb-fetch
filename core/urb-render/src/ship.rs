//! Ship names, as each renderer checks them before it links to one.
//!
//! - [`is_ship`] is the extension's `isShip` (lib/ship.js), which
//!   checks the name's shape only: `~abc` passes.
//! - [`parses`] is the gateway's `mesa.ParseShip`, libmesa's
//!   `u3_ship_of_string`: real syllables, in their canonical rendering
//!   only, so `~abc`, `~dozzod` and `~doznec` fail.
//!
//! The two disagree (docs/rust-core.md, "M5b's findings"), and each
//! renderer keeps its own.

const SIS: &[u8; 768] = b"dozmarbinwansamlitsighidfidlissogdirwacsabwissib\
rigsoldopmodfoglidhopdardorlorhodfolrintogsilmir\
holpaslacrovlivdalsatlibtabhanticpidtorbolfosdot\
losdilforpilramtirwintadbicdifrocwidbisdasmidlop\
rilnardapmolsanlocnovsitnidtipsicropwitnatpanmin\
ritpodmottamtolsavposnapnopsomfinfonbanmorworsip\
ronnorbotwicsocwatdolmagpicdavbidbaltimtasmallig\
sivtagpadsaldivdactansidfabtarmonranniswolmispal\
lasdismaprabtobrollatlonnodnavfignomnibpagsopral\
bilhaddocridmocpacravripfaltodtiltinhapmicfanpat\
taclabmogsimsonpinlomrictapfirhasbosbatpochactid\
havsaplindibhosdabbitbarracparloddosbortochilmac\
tomdigfilfasmithobharmighinradmashalraglagfadtop\
mophabnilnosmilfopfamdatnoldinhatnacrisfotribhoc\
nimlarfitwalrapsarnalmoslandondanladdovrivbacpol\
laptalpitnambonrostonfodponsovnocsorlavmatmipfip";

const DEX: &[u8; 768] = b"zodnecbudwessevpersutletfulpensytdurwepserwylsun\
rypsyxdyrnuphebpeglupdepdysputlughecryttyvsydnex\
lunmeplutseppesdelsulpedtemledtulmetwenbynhexfeb\
pyldulhetmevruttylwydtepbesdexsefwycburderneppur\
rysrebdennutsubpetrulsynregtydsupsemwynrecmegnet\
secmulnymtevwebsummutnyxrextebfushepbenmuswyxsym\
selrucdecwexsyrwetdylmynmesdetbetbeltuxtugmyrpel\
syptermebsetdutdegtexsurfeltudnuxruxrenwytnubmed\
lytdusnebrumtynseglyxpunresredfunrevrefmectedrus\
bexlebduxrynnumpyxrygryxfeptyrtustyclegnemfermer\
tenlusnussyltecmexpubrymtucfyllepdebbermughuttun\
bylsudpemdevlurdefbusbeprunmelpexdytbyttyplevmyl\
wedducfurfexnulluclennerlexrupnedlecrydlydfenwel\
nydhusrelrudneshesfetdesretdunlernyrsebhulryllud\
remlysfynwerrycsugnysnyllyndyndemluxfedsedbecmun\
lyrtesmudnytbyrsenwegfyrmurtelreptegpecnelnevfes";

fn syllable(table: &[u8; 768], s: &[u8]) -> Option<u128> {
    table.chunks(3).position(|t| t == s).map(|i| i as u128)
}

/// the extension's `isShip` (lib/ship.js): `~` and groups of a-z
/// joined by `-`; a galaxy or star is one group of 3 or 6 letters, and
/// a planet, moon or comet 2, 4 or 8 groups of 6
pub fn is_ship(s: &str) -> bool {
    let Some(name) = s.strip_prefix('~') else {
        return false;
    };
    let groups: Vec<&str> = name.split('-').collect();
    if groups
        .iter()
        .any(|g| g.is_empty() || !g.bytes().all(|b| b.is_ascii_lowercase()))
    {
        return false;
    }
    match groups.len() {
        1 => matches!(groups[0].len(), 3 | 6),
        2 | 4 | 8 => groups.iter().all(|g| g.len() == 6),
        _ => false,
    }
}

/// the gateway's `mesa.ParseShip(text)`: whether libmesa's
/// `u3_ship_of_string` reads it. A `~` is added when it has none. C
/// reads the name up to its first NUL. A name parses when its
/// syllables are real and it is written as `u3_ship_to_string` writes
/// the ship it names: `+fein` and `+fynd` are inverses, so that is
/// the chunks' own rendering.
pub fn parses(text: &[u8]) -> bool {
    let text = &text[..text.iter().position(|&b| b == 0).unwrap_or(text.len())];
    let s = text.strip_prefix(b"~").unwrap_or(text);
    if s.len() == 3 {
        return syllable(DEX, s).is_some();
    }
    let (mut v, mut n, mut i) = (0u128, 0, 0);
    while i < s.len() {
        if n >= 8 || s.len() - i < 6 {
            return false;
        }
        let (Some(p), Some(q)) = (syllable(SIS, &s[i..i + 3]), syllable(DEX, &s[i + 3..i + 6]))
        else {
            return false;
        };
        v = (v << 16) | (p << 8) | q;
        n += 1;
        i += 6;
        while i < s.len() && s[i] == b'-' {
            i += 1;
        }
    }
    n > 0 && render(v).as_bytes() == [b"~", s].concat()
}

/// `u3_ship_to_string` of a ship already `+fein`ed: the galaxy's
/// syllable, or its 16-bit chunks, most significant first, joined by
/// `-`, with `--` between a comet's halves
fn render(v: u128) -> String {
    let syl = |t: &[u8; 768], i: u128| {
        let i = i as usize * 3;
        String::from_utf8_lossy(&t[i..i + 3]).into_owned()
    };
    if v < 0x100 {
        return format!("~{}", syl(DEX, v));
    }
    let chunks = (128 - v.leading_zeros()).div_ceil(16);
    let mut out = String::from("~");
    for imp in (0..chunks).rev() {
        let c = (v >> (16 * imp)) & 0xffff;
        out += &syl(SIS, c >> 8);
        out += &syl(DEX, c & 0xff);
        if imp > 0 {
            if imp % 4 == 0 {
                out.push('-');
            }
            out.push('-');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in [
            "~zod",
            "zod",
            "~marzod",
            "~sampel-palnet",
            "~doznec-dozzod-dozzod-marzod",
            "~dasres-ragnep-lislyt-ribpyl--mosnyx-bisdem-nidful-marzod",
        ] {
            assert!(parses(ok.as_bytes()), "{ok}");
        }
        for bad in [
            "~abc",
            "~dozzod",
            "~doznec",
            "~",
            "",
            "~~zod",
            "~sampel--palnet",
            "~sampelpalnet",
            "~sampel-palne",
            "~ZOD",
            "~dozzod-dozzod-dozzod-marzod",
        ] {
            assert!(!parses(bad.as_bytes()), "{bad}");
        }
        assert!(parses(b"~zod\0junk"));
        assert!(is_ship("~abc") && is_ship("~dozzod") && is_ship("~abcdef-abcdef"));
        assert!(!is_ship("~ab") && !is_ship("~abc-abcdef") && !is_ship("zod") && !is_ship("~zod-"));
    }
}
