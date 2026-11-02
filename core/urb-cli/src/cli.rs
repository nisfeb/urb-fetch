//! The commands, each a port of its Node command line, line for line.

use crate::args::{self, Kind, Parsed};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use urb::core::{ForumCall, FurumCall, Kind as Ek, Output};
use urb::host::Host;
use urb::host::net::{Log, NetIo, Relay, Tap, Traced, Transport, Udp, libmesa_log, now_ms, random};
use urb::host::store::{FileStore, Given, node_error, node_read_error, node_rename_error};
use urb::js::{self, Js, Json, Object};
use urb::msg::{Id, Target};
use urb::{Failure, Reply, Request};

type H = Host<NetIo>;
type R<T> = Result<T, Failure>;

/// node.mjs's log: console.error with the time, mm:ss.sss
fn stamp() -> String {
    let ms = now_ms();
    format!(
        "{:02}:{:02}.{:03}",
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

fn logger() -> Log {
    Arc::new(|line: &str| eprintln!("{} {line}", stamp()))
}

/// `((Date.now() - t) / 1000).toFixed(1)`
fn secs(t: u64) -> String {
    format!("{:.1}", (now_ms().saturating_sub(t)) as f64 / 1000.0)
}

/// an error as console.error prints it, first line only: `Error: …`
fn printed(f: &Failure) -> String {
    format!("{:?}: {}", f.kind, f.message)
}

/// `e.message || e`
fn message(f: &Failure) -> String {
    if f.message.is_empty() {
        printed(f)
    } else {
        f.message.clone()
    }
}

const COMMON: &[(&str, Kind)] = &[
    ("state", Kind::Str),
    ("relay", Kind::Str),
    ("trace", Kind::Bool),
    ("moon", Kind::Str),
    ("new-moon", Kind::Bool),
    ("fake", Kind::Str),
];

fn options(more: &[(&'static str, Kind)]) -> Vec<(&'static str, Kind)> {
    COMMON.iter().copied().chain(more.iter().copied()).collect()
}

/// parseArgs, its throw printed as Node prints an uncaught one (the
/// error's line), exit 1
fn parse(a: &[String], more: &[(&'static str, Kind)], positionals: bool) -> Result<Parsed, i32> {
    args::parse(a, &options(more), positionals).map_err(|e| {
        eprintln!("TypeError [{}]: {}", e.code, e.message);
        1
    })
}

pub fn main(a: Vec<String>) -> i32 {
    let (cmd, rest) = match a.split_first() {
        Some((c, r)) => (c.as_str(), r),
        None => ("", &[][..]),
    };
    match cmd {
        "read" => read(rest),
        "serve" => serve(rest),
        "furum" => furum(rest),
        "forum" => forum(rest),
        _ => {
            eprintln!("usage: urb read|serve|furum|forum [options] ... (docs/rust-core.md, M7)");
            2
        }
    }
}

// ------------------------------------------------------------ node.mjs

/// what node.mjs's connect() gives: the engine, our ship, its folder,
/// and what its outputs tell the command
struct Cli {
    host: H,
    our: String,
    dir: PathBuf,
}

/// node.mjs's connect(): the engine, as the options say
fn connect(opt: &Parsed, log: &Log, tap: Option<Tap>) -> R<Cli> {
    let base = match opt.str("state") {
        Some(s) => PathBuf::from(s),
        None => {
            let st = std::env::var_os("XDG_STATE_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
                });
            st.join("urb-fetch-wasm")
        }
    };
    let mut dir = base;
    let mut given = None;
    let fake = opt.str("fake");
    let relay = opt.str("relay");
    if let Some(f) = &fake {
        if relay.is_some() {
            return Err(Failure::error(
                "a fake network is on this machine: no relay",
            ));
        }
        let ship =
            urb::engine::ship_parse(f).ok_or_else(|| Failure::urb(format!("not a ship: {f}")))?;
        given = Some(Given {
            keyfile: urb::engine::fake_keyfile(ship),
            fresh: true,
        });
        dir = dir.join(format!("fake-{}", f.strip_prefix('~').unwrap_or(f)));
    } else if let Some(m) = opt.str("moon") {
        let path = Path::new(&m);
        let keyfile = std::fs::read(path)
            .map_err(|e| Failure::error(node_read_error(&e, path)))
            .map(|b| String::from_utf8_lossy(&b).trim().to_string())?;
        let info = urb::engine::keyfile_ship(&keyfile);
        let Some(((lo, hi), rift, life)) = info.filter(|((lo, hi), ..)| *hi == 0 && *lo >= 1 << 32)
        else {
            return Err(Failure::error(format!("{m} holds no moon's key")));
        };
        let ship = urb::engine::ship_text((lo, hi));
        log(&format!(
            "moon {ship}, life {life}, rift {}",
            rift.map_or("not in the keyfile".into(), |r| r.to_string())
        ));
        given = Some(Given {
            keyfile,
            fresh: opt.bool("new-moon"),
        });
        dir = dir.join(&ship[1..]);
    }
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| Failure::error(node_error(&e, "mkdir", &dir)))?;
    }
    let t = now_ms();
    let transport: Box<dyn Transport> = match &relay {
        Some(r) => Box::new(Relay::new(r)),
        None => Box::new(Udp::new(&dir, fake.is_some(), log.clone()).map_err(Failure::error)?),
    };
    let transport: Box<dyn Transport> = if opt.bool("trace") {
        Box::new(Traced {
            inner: transport,
            log: log.clone(),
        })
    } else {
        transport
    };
    // libmesa reads MESA_TRACE from the environment: --trace sets it,
    // and nothing else does, as node.mjs's env does
    unsafe {
        if opt.bool("trace") {
            std::env::set_var("MESA_TRACE", "1");
        } else {
            std::env::remove_var("MESA_TRACE");
        }
    }
    libmesa_log(log.clone());
    let mut io = NetIo::new(
        transport,
        Box::new(FileStore::new(&dir, given)),
        log.clone(),
    );
    io.tap = tap;
    let mut host = Host::new(io, random);
    let our = host.open(fake.is_some())?;
    log(&format!(
        "as {our}, ready in {}s{}",
        secs(t),
        match (&relay, &fake) {
            (Some(r), _) => format!(" through {r}"),
            (None, Some(_)) => " on the fake network".into(),
            _ => String::new(),
        }
    ));
    Ok(Cli { host, our, dir })
}

impl Cli {
    fn call(&mut self, req: Request) -> R<Reply> {
        self.host.call(req)
    }

    fn js(&mut self, req: Request) -> R<Js> {
        self.call(req).map(js_of)
    }

    /// `await new Promise(r => setTimeout(r, ms))`, the engine running
    fn sleep(&mut self, ms: u64) {
        self.host.run_for(ms, |_| false);
    }

    fn close(&mut self) {
        let _ = self.host.close();
    }
}

/// a result as the JS value the call gave
fn js_of(r: Reply) -> Js {
    match r {
        Reply::Js(j) => j,
        Reply::True => Js::Bool(true),
        Reply::Open { our } => Js::Str(our),
        Reply::Poke(p) => {
            let mut o = Object::default();
            o.set("ok".into(), Js::Bool(p.ok));
            if let Some(e) = p.error {
                o.set("error".into(), Js::Str(e));
            }
            if p.later {
                o.set("later".into(), Js::Bool(true));
            }
            Js::Obj(o)
        }
        Reply::Peek(urb::core::Peeked::Null) => Js::Null,
        Reply::Peek(urb::core::Peeked::Text(s)) => Js::Str(s),
        Reply::Peek(urb::core::Peeked::True) => Js::Bool(true),
        Reply::Peek(urb::core::Peeked::Files(_)) => Js::Arr(Vec::new()),
        Reply::Json(j) => json_js(&j),
        Reply::Read(p) => Js::Str(p.body),
    }
}

fn json_js(j: &Json) -> Js {
    match j {
        Json::Null => Js::Null,
        Json::Bool(b) => Js::Bool(*b),
        Json::Num(n) => Js::Num(*n),
        Json::Str(s) => Js::Str(s.clone()),
        Json::Arr(a) => Js::Arr(a.iter().map(json_js).collect()),
        Json::Obj(o) => {
            let mut out = Object::default();
            for (k, v) in o.entries() {
                out.set(k.to_string(), json_js(v));
            }
            Js::Obj(out)
        }
    }
}

// ------------------------------------------------------------ JS values

/// x[key], or None for undefined
fn get<'a>(x: &'a Js, key: &str) -> Option<&'a Js> {
    match x {
        Js::Obj(o) => o.get(key),
        _ => None,
    }
}

/// x[key] where reading undefined's or null's key throws, as the JS
/// does
fn at<'a>(x: Option<&'a Js>, key: &str) -> R<Option<&'a Js>> {
    match x {
        None => Err(Failure::type_error(format!(
            "Cannot read properties of undefined (reading '{key}')"
        ))),
        Some(Js::Null) => Err(Failure::type_error(format!(
            "Cannot read properties of null (reading '{key}')"
        ))),
        Some(x) => Ok(get(x, key)),
    }
}

/// `${x}` of a JS value (None is undefined)
fn s(x: Option<&Js>) -> String {
    match x {
        None => "undefined".into(),
        Some(Js::Null) => "null".into(),
        Some(Js::Bool(b)) => b.to_string(),
        Some(Js::Num(n)) => js::number_text(*n),
        Some(Js::Big(i)) => i.decimal(),
        Some(Js::Str(t)) => t.clone(),
        Some(Js::Date(Some(ms))) => js::iso(*ms),
        Some(Js::Date(None)) => "Invalid Date".into(),
        Some(Js::Arr(a)) => a
            .iter()
            .map(|v| match v {
                Js::Null => String::new(),
                v => s(Some(v)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Js::Obj(_)) => "[object Object]".into(),
    }
}

fn truthy(x: Option<&Js>) -> bool {
    match x {
        None | Some(Js::Null) => false,
        Some(Js::Bool(b)) => *b,
        Some(Js::Num(n)) => *n != 0.0 && !n.is_nan(),
        Some(Js::Big(i)) => i.decimal() != "0",
        Some(Js::Str(t)) => !t.is_empty(),
        _ => true,
    }
}

/// `${x.length}`, of an array or a string (in UTF-16 units)
fn len(x: Option<&Js>) -> R<String> {
    match x {
        Some(Js::Arr(a)) => Ok(a.len().to_string()),
        Some(Js::Str(t)) => Ok(t.encode_utf16().count().to_string()),
        other => at(other, "length").map(|_| "undefined".into()),
    }
}

fn arr(x: Option<&Js>) -> &[Js] {
    match x {
        Some(Js::Arr(a)) => a,
        _ => &[],
    }
}

/// d.toISOString().slice(0, 16)
fn iso16(d: Option<&Js>) -> R<String> {
    match d {
        Some(Js::Date(Some(ms))) => Ok(js::iso(*ms).chars().take(16).collect()),
        Some(Js::Date(None)) => Err(Failure::range_error("Invalid time value")),
        other => Err(Failure::type_error(format!(
            "{} is not a function",
            match other {
                None => "undefined.toISOString".to_string(),
                Some(_) => "d.toISOString".to_string(),
            }
        ))),
    }
}

fn str_eq(x: Option<&Js>, t: &str) -> bool {
    matches!(x, Some(Js::Str(v)) if v == t)
}

fn includes(x: Option<&Js>, t: &str) -> bool {
    arr(x).iter().any(|v| matches!(v, Js::Str(s) if s == t))
}

// ------------------------------------------------------------ host.mjs

/// host.mjs: read one page; the body to stdout, what it was to stderr
fn read(a: &[String]) -> i32 {
    let opt = match parse(a, &[("tries", Kind::Str)], true) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let log = logger();
    let Some(url) = opt.positionals.first().cloned() else {
        log(
            "usage: node host.mjs [--state DIR] [--relay URL | --fake ~SHIP] [--moon KEYFILE [--new-moon]] [--tries N] [--trace] urb://~ship/page",
        );
        return 2;
    };
    let tries = js::number(&opt.str("tries").unwrap_or_else(|| "1".into()));
    let mut code = 0;
    let mut cli = None;
    let r = (|| -> R<()> {
        let c = cli.insert(connect(&opt, &log, None)?);
        let t = now_ms();
        let mut i = 1u64;
        let got = loop {
            match c.call(Request::Read(url.clone())) {
                Ok(Reply::Read(p)) => break p,
                Ok(r) => return Err(Failure::error(format!("read gave {r:?}"))),
                Err(e) => {
                    // i >= Number(opt.tries), NaN never
                    if e.kind != Ek::UrbError || (i as f64) >= tries {
                        return Err(e);
                    }
                    log(&format!("try {i}: {}", e.message));
                }
            }
            i += 1;
        };
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = out.write_all(got.body.as_bytes());
        let _ = out.flush();
        let how = match got.mark.as_deref() {
            Some(m) if !m.is_empty() => format!(
                " %{m}, renders as {}",
                urb_render::render_as(Some(m), &got.body)
            ),
            _ => String::new(),
        };
        log(&format!(
            "ok {url} -> {}{how}, {} bytes, by {}, {}s",
            got.key,
            got.body.len(),
            got.how,
            secs(t)
        ));
        Ok(())
    })();
    if let Err(e) = r {
        log(&if e.kind == Ek::UrbError {
            format!("FAIL {}", e.message)
        } else {
            printed(&e)
        });
        code = 1;
    }
    if let Some(c) = &mut cli {
        c.close();
    }
    code
}

// ------------------------------------------------------------ pub-cli.mjs

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_int(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}

unsafe extern "C" {
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}

const SIGINT: i32 = 2;

/// pub.mjs's markOf: a page's mark by its file's extension, in any case
fn mark_of(file: &str) -> &'static str {
    let ext = file
        .rfind('.')
        .map(|i| &file[i + 1..])
        .filter(|e| !e.is_empty() && !e.contains('/'));
    match ext.map(str::to_lowercase).as_deref() {
        Some("md") => "md",
        Some("html" | "htm") => "html",
        Some("txt") => "text",
        Some("js") => "js",
        Some("css") => "css",
        _ => "gmi",
    }
}

/// write a file through a `.new` and a rename, mode 0600, failing in
/// writeFileSync's and renameSync's words
fn save_file(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let new = PathBuf::from(format!("{}.new", path.display()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&new)
        .map_err(|e| node_error(&e, "open", &new))?;
    f.write_all(data)
        .map_err(|e| node_error(&e, "write", &new))?;
    drop(f);
    std::fs::rename(&new, path).map_err(|e| node_rename_error(&e, &new, path))
}

/// pub-cli.mjs: publish pages and serve them until Ctrl-C
fn serve(a: &[String]) -> i32 {
    let opt = match parse(a, &[("put", Kind::Many), ("remove", Kind::Many)], false) {
        Ok(o) => o,
        Err(c) => return c,
    };
    // where publishing's saves go, once the folder is known
    let file: Rc<RefCell<Option<PathBuf>>> = Rc::default();
    let f2 = file.clone();
    // a save that fails: writeFileSync throws inside put or remove, which
    // pub-cli catches (exit 1); while serving, it is uncaught (exit 1).
    // Either way nothing after the throw runs, so what the core logs
    // after a failed save, until it is reported, is not printed.
    let failed: Arc<Mutex<Option<String>>> = Arc::default();
    let failed2 = failed.clone();
    let quiet = failed.clone();
    let stamped = logger();
    let log: Log = Arc::new(move |line: &str| {
        if quiet.lock().unwrap().is_none() {
            stamped(line)
        }
    });
    let tap: Tap = Box::new(move |o| {
        if let Output::Event { name, value } = o
            && name == "pub.save"
            && let Some(p) = &*f2.borrow()
            && let Err(e) = save_file(p, js::stringify(value).as_bytes())
        {
            failed2.lock().unwrap().get_or_insert(e);
        }
    });
    let save_failed = || match failed.lock().unwrap().take() {
        Some(e) => Err(Failure::error(e)),
        None => Ok(()),
    };
    let mut cli = None;
    let r = (|| -> R<()> {
        let c = cli.insert(connect(&opt, &log, Some(tap))?);
        c.call(Request::Watch(true))?;
        let path = c.dir.join("pages.json");
        let saved = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .map(|v| from_serde(&v));
        let (pages, keepers) = match &saved {
            Some(Json::Obj(o)) => (
                match o.get("pages") {
                    Some(Json::Obj(p)) => Some(p.clone()),
                    _ => None,
                },
                o.get("keepers").cloned(),
            ),
            _ => (None, None),
        };
        *file.borrow_mut() = Some(path);
        c.call(Request::Pub { pages, keepers })?;
        save_failed()?;
        for p in opt.many("put") {
            let at = p.find('=').filter(|&i| i > 0);
            let rec = match at {
                Some(i) => {
                    let f = &p[i + 1..];
                    let body = std::fs::read(f)
                        .map_err(|e| Failure::error(node_read_error(&e, Path::new(f))))?;
                    c.call(Request::PubPut {
                        name: p[..i].to_string(),
                        body: String::from_utf8_lossy(&body).into_owned(),
                        mark: Some(mark_of(f).into()),
                    })?
                }
                None => Reply::Json(Json::Bool(false)),
            };
            save_failed()?;
            if matches!(rec, Reply::Json(Json::Null | Json::Bool(false))) {
                return Err(Failure::error(format!(
                    "--put {p}: NAME=FILE, NAME as lattice names pages (notes/intro)"
                )));
            }
        }
        for name in opt.many("remove") {
            let gone = c.call(Request::PubRemove(name.clone()))?;
            save_failed()?;
            if !matches!(gone, Reply::Json(Json::Bool(true))) {
                log(&format!("--remove {name}: no such page"));
            }
        }
        let Reply::Json(Json::Obj(pages)) = c.call(Request::PubPages)? else {
            return Ok(());
        };
        for (key, p) in pages.entries() {
            let p = json_js(p);
            let body = get(&p, "body");
            if matches!(body, Some(Js::Null)) {
                continue;
            }
            let n = match body {
                Some(Js::Str(t)) => t.encode_utf16().count().to_string(),
                None => {
                    return Err(Failure::type_error(
                        "Cannot read properties of undefined (reading 'length')",
                    ));
                }
                _ => "undefined".into(),
            };
            let mark = match get(&p, "mark") {
                None | Some(Js::Null) => "gmi".to_string(),
                m => s(m),
            };
            log(&format!(
                "urb://{}{key}  %{mark} rev {}, {n} characters",
                c.our,
                s(get(&p, "rev"))
            ));
        }
        Ok(())
    })();
    if let Err(e) = r {
        log(&message(&e));
        if let Some(c) = &mut cli {
            c.close();
        }
        return 1;
    }
    let mut c = cli.expect("connected");
    log("serving; Ctrl-C stops");
    unsafe { signal(SIGINT, on_int) };
    while !STOP.load(Ordering::Relaxed) {
        c.host.turn();
        if let Err(e) = save_failed() {
            // uncaught in Node: its error's line, as main prints one
            eprintln!("Error: {}", e.message);
            c.close();
            return 1;
        }
    }
    c.close();
    0
}

/// a JSON.parse value
fn from_serde(v: &serde_json::Value) -> Json {
    use serde_json::Value as V;
    match v {
        V::Null => Json::Null,
        V::Bool(b) => Json::Bool(*b),
        V::Number(n) => Json::Num(n.as_f64().unwrap_or(f64::NAN)),
        V::String(t) => Json::Str(t.clone()),
        V::Array(a) => Json::Arr(a.iter().map(from_serde).collect()),
        V::Object(o) => {
            let mut out = Object::default();
            for (k, v) in o {
                out.set(k.clone(), from_serde(v));
            }
            Json::Obj(out)
        }
    }
}

// ------------------------------------------------------------ furum-cli.mjs

/// a refusal heard, shared with the tap that hears it
type Refused = Rc<RefCell<Option<String>>>;

/// poll f until it gives something truthy, for up to a minute (30 tries
/// 2 s apart), or a refusal is heard: furum's throws, forum's gives up
fn until(
    c: &mut Cli,
    refused: &Refused,
    throws: bool,
    mut f: impl FnMut(&mut Cli) -> R<Option<Js>>,
) -> R<Option<Js>> {
    for _ in 0..30 {
        if let Some(r) = refused.borrow().clone() {
            if throws {
                return Err(Failure::error(format!("refused: {r}")));
            }
            return Ok(None);
        }
        if let Some(v) = f(c)? {
            return Ok(Some(v));
        }
        c.sleep(2000);
    }
    Ok(None)
}

/// what a poke-like call gave: {ok, error}
fn sent(x: &Js) -> (bool, String) {
    (truthy(get(x, "ok")), s(get(x, "error")))
}

/// furum-cli.mjs: a furum board on another ship
fn furum(a: &[String]) -> i32 {
    let more = [
        ("post", Kind::Str),
        ("body", Kind::Str),
        ("url", Kind::Str),
        ("thread", Kind::Str),
        ("comment", Kind::Str),
        ("upvote", Kind::Str),
    ];
    let opt = match parse(a, &more, true) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let log = logger();
    let (Some(host), Some(name)) = (opt.positionals.first(), opt.positionals.get(1)) else {
        log(
            "usage: node furum-cli.mjs [--fake ~SHIP | --relay URL] [--moon KEYFILE] [--trace] ~host board [--post TITLE [--body TEXT] [--url URL]]",
        );
        return 2;
    };
    let (host, name) = (host.clone(), name.clone());
    let refused: Refused = Rc::default();
    let (r2, l2) = (refused.clone(), log.clone());
    let tap: Tap = Box::new(move |o| {
        if let Output::Note { name, value } = o
            && name == "furum.note"
        {
            l2(&format!(
                "note from {}: {}: {}",
                s(get(value, "from")),
                s(get(value, "title")),
                s(get(value, "body"))
            ));
            *r2.borrow_mut() = Some(s(get(value, "body")));
        }
    });
    let fc = |call: FurumCall| Request::Furum {
        host: host.clone(),
        call,
    };
    let show = |p: &Js| -> R<String> {
        let mut out = format!(
            "#{} {}  ({}, {})",
            s(get(p, "id")),
            s(get(p, "title")),
            s(get(p, "author")),
            iso16(get(p, "created"))?
        );
        if truthy(get(p, "url")) {
            out += &format!("\n    {}", s(get(p, "url")));
        }
        if truthy(get(p, "body")) {
            out += &format!("\n    {}", s(get(p, "body")));
        }
        Ok(out)
    };
    let show_thread = |c: &mut Cli, id: f64| -> R<()> {
        let a = c.host.submit(fc(FurumCall::Comments(name.clone(), id)));
        let b = c.host.submit(fc(FurumCall::Votes(name.clone(), id)));
        let cs = c.host.wait_for(a).map(js_of);
        let vs = c.host.wait_for(b).map(js_of);
        let (cs, vs) = (cs?, vs?);
        let up = at(Some(&vs), "up")?;
        let nup = len(up)?;
        println!(
            "#{}: {nup} up{}, {} down, {} comments",
            js::number_text(id),
            // vs.up.length truthy
            if nup != "0" && nup != "undefined" {
                format!(
                    " ({})",
                    arr(up)
                        .iter()
                        .map(|v| s(Some(v)))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            } else {
                String::new()
            },
            len(at(Some(&vs), "down")?)?,
            len(Some(&cs))?
        );
        for cm in arr(Some(&cs)) {
            let parent = get(cm, "parent");
            println!(
                "  c{}{} {}: {}",
                s(get(cm, "id")),
                if matches!(parent, Some(Js::Null)) {
                    String::new()
                } else {
                    format!(" on c{}", s(parent))
                },
                s(get(cm, "author")),
                s(get(cm, "body"))
            );
        }
        Ok(())
    };
    let mut code = 0;
    let mut cli = None;
    let r = (|| -> R<()> {
        let c = cli.insert(connect(&opt, &log, Some(tap))?);
        c.call(fc(FurumCall::OnNote))?;
        let mut t = now_ms();
        let before = c.js(fc(FurumCall::Posts(name.clone())))?;
        log(&format!(
            "{} posts on {host}/{name}, read in {}s",
            len(Some(&before))?,
            secs(t)
        ));
        let truthy_opt = |k: &str| opt.str(k).filter(|v| !v.is_empty());
        if let Some(th) = truthy_opt("thread") {
            show_thread(c, js::number(&th))?;
        } else if let Some(idt) = truthy_opt("comment").or_else(|| truthy_opt("upvote")) {
            let id = js::number(&idt);
            let commenting = truthy_opt("comment").is_some();
            t = now_ms();
            let x = if commenting {
                c.js(fc(FurumCall::NewComment {
                    name: name.clone(),
                    post: id,
                    body: truthy_opt("body").unwrap_or_default(),
                    parent: None,
                }))?
            } else {
                c.js(fc(FurumCall::Upvote(name.clone(), id)))?
            };
            let (ok, error) = sent(&x);
            log(&format!(
                "{}: {} ({}s)",
                if commenting { "commented" } else { "voted" },
                if ok {
                    "the inbox took it".to_string()
                } else {
                    format!("refused: {error}")
                },
                secs(t)
            ));
            if !ok {
                return Err(Failure::error(error));
            }
            let (our, body) = (c.our.clone(), opt.str("body"));
            let there = until(c, &refused, true, |c| {
                if commenting {
                    let cs = c.js(fc(FurumCall::Comments(name.clone(), id)))?;
                    if matches!(cs, Js::Null) {
                        return Err(Failure::type_error(
                            "Cannot read properties of null (reading 'find')",
                        ));
                    }
                    Ok(arr(Some(&cs))
                        .iter()
                        .find(|cm| {
                            str_eq(get(cm, "author"), &our)
                                && body.as_deref().is_some_and(|b| str_eq(get(cm, "body"), b))
                        })
                        .cloned())
                } else {
                    let vs = c.js(fc(FurumCall::Votes(name.clone(), id)))?;
                    let up = at(Some(&vs), "up")?;
                    Ok(includes(up, &our).then_some(Js::Bool(true)))
                }
            })?;
            if there.is_none() {
                return Err(Failure::error("it never appeared on the board"));
            }
            log(&format!("on the board after {}s", secs(t)));
            show_thread(c, id)?;
        } else if let Some(title) = truthy_opt("post") {
            t = now_ms();
            let x = c.js(fc(FurumCall::NewPost {
                name: name.clone(),
                title: title.clone(),
                url: opt.str("url"),
                body: opt.str("body"),
            }))?;
            let (ok, error) = sent(&x);
            log(&format!(
                "posted: {} ({}s)",
                if ok {
                    "the inbox took it".to_string()
                } else {
                    format!("refused: {error}")
                },
                secs(t)
            ));
            if !ok {
                return Err(Failure::error(error));
            }
            let our = c.our.clone();
            let mine = until(c, &refused, true, |c| {
                let ps = c.js(fc(FurumCall::Posts(name.clone())))?;
                Ok(arr(Some(&ps))
                    .iter()
                    .find(|p| {
                        str_eq(get(p, "title"), &title)
                            && str_eq(get(p, "author"), &our)
                            && !arr(Some(&before))
                                .iter()
                                .any(|b| get(b, "id") == get(p, "id"))
                    })
                    .cloned())
            })?;
            let Some(mine) = mine else {
                return Err(Failure::error("the post never appeared on the board"));
            };
            log(&format!("on the board after {}s", secs(t)));
            println!("{}", show(&mine)?);
        } else {
            for p in arr(Some(&before)) {
                println!("{}", show(p)?);
            }
        }
        Ok(())
    })();
    if let Err(e) = r {
        log(&message(&e));
        code = 1;
    }
    if let Some(c) = &mut cli {
        c.close();
    }
    code
}

// ------------------------------------------------------------ forum-cli.mjs

/// forum-cli.mjs: a lattice forum board on another ship
fn forum(a: &[String]) -> i32 {
    let more = [
        ("post", Kind::Str),
        ("body", Kind::Str),
        ("url", Kind::Str),
        ("comment", Kind::Str),
        ("upvote", Kind::Bool),
    ];
    let opt = match parse(a, &more, true) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let log = logger();
    let usage = || {
        log(
            "usage: node forum-cli.mjs [--fake ~SHIP | --relay URL] [--moon KEYFILE] [--trace] urb://~host/forum/board[/ID] [--post TITLE [--body TEXT] [--url URL] | --comment TEXT | --upvote]",
        );
        2
    };
    let commenting = opt.str("comment").filter(|v| !v.is_empty());
    let upvoting = opt.bool("upvote");
    // deUrb throws out of the script: Node prints it and exits 1
    let (host, name, id) = match opt.positionals.first() {
        None => return usage(),
        Some(url) => match urb::url::de_urb(url) {
            Err(e) => {
                eprintln!("UrbError: {}", e.message);
                return 1;
            }
            Ok(u) => match u.mount {
                urb::url::Mount::Forum { post } => {
                    (u.ship, u.rel.first().cloned().unwrap_or_default(), post)
                }
                _ => return usage(),
            },
        },
    };
    if (commenting.is_some() || upvoting) && id.is_none() {
        return usage();
    }
    let refused: Refused = Rc::default();
    let (r2, l2, board) = (refused.clone(), log.clone(), name.clone());
    let tap: Tap = Box::new(move |o| {
        if let Output::Note { name, value } = o
            && name == "forum.note"
        {
            l2(&format!(
                "note from {} on {}: {}",
                s(get(value, "host")),
                s(get(value, "board")),
                s(get(value, "reason"))
            ));
            if str_eq(get(value, "board"), &board) {
                *r2.borrow_mut() = Some(s(get(value, "reason")));
            }
        }
    });
    let fc = |call: ForumCall| Request::Forum {
        host: host.clone(),
        call,
    };
    let votes = |x: &Js| -> R<String> {
        Ok(format!(
            "{} up, {} down",
            len(at(Some(x), "up")?)?,
            len(at(Some(x), "down")?)?
        ))
    };
    let when = |d: Option<&Js>| -> R<String> {
        if truthy(d) {
            iso16(d)
        } else {
            Ok("no date".into())
        }
    };
    let show = |p: &Js| -> R<String> {
        let mut out = format!(
            "#{} {}  ({}, {}, {}, {} comments)",
            s(get(p, "id")),
            s(get(p, "title")),
            s(get(p, "author")),
            when(get(p, "created"))?,
            votes(p)?,
            s(get(p, "comments"))
        );
        if truthy(get(p, "url")) {
            out += &format!("\n    {}", s(get(p, "url")));
        }
        if truthy(get(p, "body")) {
            out += &format!("\n    {}", s(get(p, "body")));
        }
        Ok(out)
    };
    let show_thread = |t: &Js| -> R<()> {
        let post = at(Some(t), "post")?;
        println!("{}", show(post.unwrap_or(&Js::Null))?);
        for cm in arr(get(t, "comments")) {
            let parent = get(cm, "parent");
            println!(
                "  c{}{} {}: {} ({})",
                s(get(cm, "id")),
                if matches!(parent, Some(Js::Null)) {
                    String::new()
                } else {
                    format!(" on c{}", s(parent))
                },
                s(get(cm, "author")),
                s(get(cm, "body")),
                votes(cm)?
            );
        }
        Ok(())
    };
    let pid = |i: &urb::Int| Id::Big(false, i.clone());
    let mut code = 0;
    let mut cli = None;
    let r = (|| -> R<()> {
        let c = cli.insert(connect(&opt, &log, Some(tap))?);
        c.call(fc(ForumCall::OnNote))?;
        let mut t = now_ms();
        let before = c.js(fc(ForumCall::Board(name.clone())))?;
        let info = at(Some(&before), "info")?;
        at(info, "title")?;
        log(&format!(
            "{} ({host}/{name}, {}): {} posts, read in {}s",
            s(info.and_then(|i| get(i, "title"))),
            if truthy(info.and_then(|i| get(i, "public"))) {
                "public"
            } else {
                "private"
            },
            len(at(Some(&before), "posts")?)?,
            secs(t)
        ));
        let our = c.our.clone();
        // done(): say what the host said, then wait for it on the board
        let done = |c: &mut Cli,
                    x: Js,
                    what: &str,
                    t: u64,
                    there: &mut dyn FnMut(&mut Cli) -> R<Option<Js>>|
         -> R<Js> {
            let (ok, error) = sent(&x);
            log(&format!(
                "{what}: {} ({}s)",
                if ok {
                    "/forum.sig took it".to_string()
                } else {
                    format!("not taken: {error}")
                },
                secs(t)
            ));
            if !ok {
                return Err(Failure::error(error));
            }
            let v = until(c, &refused, false, there)?;
            if let Some(r) = refused.borrow().clone() {
                return Err(Failure::error(format!("refused: {r}")));
            }
            let Some(v) = v else {
                return Err(Failure::error("it never appeared on the board"));
            };
            log(&format!("on the board after {}s", secs(t)));
            Ok(v)
        };
        if let Some(title) = opt.str("post").filter(|v| !v.is_empty()) {
            t = now_ms();
            let x = c.js(fc(ForumCall::NewPost {
                name: name.clone(),
                title: Some(title.clone()),
                url: opt.str("url"),
                body: opt.str("body"),
            }))?;
            let v = done(c, x, "posted", t, &mut |c| {
                let b = c.js(fc(ForumCall::Board(name.clone())))?;
                Ok(arr(get(&b, "posts"))
                    .iter()
                    .find(|p| {
                        str_eq(get(p, "title"), &title)
                            && str_eq(get(p, "author"), &our)
                            && !arr(get(&before, "posts"))
                                .iter()
                                .any(|b| get(b, "id") == get(p, "id"))
                    })
                    .cloned())
            })?;
            println!("{}", show(&v)?);
        } else if commenting.is_some() || upvoting {
            let id = id.clone().expect("checked above");
            t = now_ms();
            let x = match &commenting {
                Some(text) => c.js(fc(ForumCall::NewComment {
                    name: name.clone(),
                    post: pid(&id),
                    body: Some(text.clone()),
                    parent: None,
                }))?,
                None => c.js(fc(ForumCall::Upvote(
                    name.clone(),
                    Target {
                        post: pid(&id),
                        comment: None,
                    },
                )))?,
            };
            let what = if commenting.is_some() {
                "commented"
            } else {
                "voted"
            };
            done(c, x, what, t, &mut |c| {
                let th = c.js(fc(ForumCall::Thread(name.clone(), pid(&id))))?;
                let yes = match &commenting {
                    Some(text) => arr(get(&th, "comments"))
                        .iter()
                        .any(|cm| str_eq(get(cm, "author"), &our) && str_eq(get(cm, "body"), text)),
                    None => includes(at(at(Some(&th), "post")?, "up")?, &our),
                };
                Ok(yes.then_some(th))
            })?;
            let th = c.js(fc(ForumCall::Thread(name.clone(), pid(&id))))?;
            show_thread(&th)?;
        } else if let Some(id) = &id {
            let th = c.js(fc(ForumCall::Thread(name.clone(), pid(id))))?;
            show_thread(&th)?;
        } else {
            for p in arr(get(&before, "posts")) {
                println!("{}", show(p)?);
            }
        }
        Ok(())
    })();
    if let Err(e) = r {
        log(&message(&e));
        code = 1;
    }
    if let Some(c) = &mut cli {
        c.close();
    }
    code
}
