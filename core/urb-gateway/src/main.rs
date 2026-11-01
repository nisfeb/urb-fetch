//! `urb-gateway`, the Go gateway (`urb-fetch`) in Rust (docs/rust-core.md,
//! M8): a local gateway to lattice pages on the Urbit network, with no
//! Urbit ship. It runs as a comet (or your moon) and serves
//! urb://~ship/page at http://ship.urb.localhost:8475/page, with the Go
//! gateway's flags, routes, headers, errors and state folder.
//!
//! The relay (`-relay`) stays in the Go binary, as the plan has it.

#[cfg(not(target_family = "wasm"))]
mod run {
    use std::collections::HashMap;
    use std::net::{TcpListener, ToSocketAddrs};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use urb::host::net::{Log, NetIo, libmesa_log, random};
    use urb::host::{Event, Host};
    use urb::{Reply, Request};
    use urb_gateway::flags::{self, Flag, Kind};
    use urb_gateway::routes::{Fail, Gateway, Page, fail_of};
    use urb_gateway::state::{self, GatewayUdp, go_path_error};
    use urb_gateway::{golog, http};

    static STOP: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_int(_: i32) {
        STOP.store(true, Ordering::Relaxed);
    }

    unsafe extern "C" {
        fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
    }

    const SIGINT: i32 = 2;

    fn default_state() -> String {
        match std::env::var("XDG_STATE_HOME") {
            Ok(d) if !d.is_empty() => format!("{d}/urb-fetch"),
            _ => format!(
                "{}/.local/state/urb-fetch",
                std::env::var("HOME").unwrap_or_default()
            ),
        }
    }

    fn the_flags() -> Vec<Flag> {
        let f = |name, kind, default: &str, usage| Flag {
            name,
            kind,
            default: default.to_string(),
            usage,
        };
        vec![
            f(
                "listen",
                Kind::Str,
                "localhost:8475",
                "address to serve on (keep it on loopback); localhost means both 127.0.0.1 and ::1",
            ),
            f(
                "state",
                Kind::Str,
                &default_state(),
                "where the identity and its network state live",
            ),
            f(
                "moon",
                Kind::Str,
                "",
                "run as this moon: the keyfile its parent's |moon printed",
            ),
            f(
                "new-moon",
                Kind::Bool,
                "",
                "the moon has never run anywhere: start its state empty",
            ),
            f(
                "relay",
                Kind::Str,
                "",
                "instead of the gateway, run the relay for libmesa in a browser (wasm/mesa.mjs) on this address, e.g. localhost:8476",
            ),
            f(
                "relay-origins",
                Kind::Str,
                "chrome-extension://*",
                "comma-separated Origin patterns the relay accepts (clients that send none, such as Node, are always accepted)",
            ),
            f(
                "relay-fake",
                Kind::Bool,
                "",
                "for tests: relay to a fake network (urbit -F) on this machine, galaxy g at 127.0.0.1:31337+g",
            ),
            f(
                "fake",
                Kind::Str,
                "",
                "for tests: run the gateway as this ship on a fake network (urbit -F) on this machine",
            ),
        ]
    }

    /// `listenAll(addr)`: the listeners and the port, on both loopbacks
    /// for localhost
    fn listen_all(addr: &str) -> (Vec<TcpListener>, String) {
        let Some(host) = http::split_host_port(addr.as_bytes()) else {
            let why = if addr.matches(':').count() > 1 && !addr.starts_with('[') {
                "too many colons in address"
            } else {
                "missing port in address"
            };
            golog::fatal(&format!("{addr}: address {addr}: {why}"));
        };
        let host = String::from_utf8_lossy(host).into_owned();
        let port = addr[addr.rfind(':').expect("a port") + 1..].to_string();
        let join = |h: &str| {
            if h.contains(':') {
                format!("[{h}]:{port}")
            } else {
                format!("{h}:{port}")
            }
        };
        let addrs = if host == "localhost" {
            vec![join("127.0.0.1"), join("::1")]
        } else {
            vec![addr.to_string()]
        };
        let mut lns = Vec::new();
        for a in addrs {
            let got = a
                .to_socket_addrs()
                .map_err(|e| e.to_string())
                .and_then(|mut s| s.next().ok_or_else(|| "no address".to_string()))
                .and_then(|s| {
                    TcpListener::bind(s).map_err(|e| {
                        let mut why = e.to_string();
                        if let Some(i) = why.find(" (os error") {
                            why.truncate(i);
                        }
                        format!("listen tcp {a}: bind: {}", why.to_lowercase())
                    })
                });
            match got {
                Ok(l) => lns.push(l),
                Err(e) if !lns.is_empty() => golog::line(&format!("not serving on {a}: {e}")),
                Err(e) => golog::fatal(&e),
            }
        }
        (lns, port)
    }

    /// The exit code once the host's thread has ended on its own: every
    /// read would fail, so the gateway ends, as the Go gateway ends when
    /// its node does, and whatever runs it starts it again. A panic
    /// (printed already) is Go's exit code for one, 2.
    pub fn ended(host: thread::JoinHandle<()>) -> i32 {
        match host.join() {
            Err(_) => 2,
            Ok(()) => {
                golog::line("node: the host stopped");
                0
            }
        }
    }

    /// what the HTTP threads ask of the host's thread
    enum Job {
        Read(String, Sender<Result<Page, Fail>>),
        Close(Sender<()>),
    }

    pub fn main() -> i32 {
        let args: Vec<String> = std::env::args().collect();
        let prog = args.first().cloned().unwrap_or_default();
        let fl = the_flags();
        let opt = match flags::parse(&args[1..], &fl) {
            Ok(o) => o,
            Err(stop) => {
                if let Some(m) = stop.message {
                    eprintln!("{m}");
                }
                eprint!("{}", flags::usage(&prog, &fl));
                return stop.code;
            }
        };
        let (fake, moon) = (opt.str("fake", &fl), opt.str("moon", &fl));
        if let Err(e) = state::one_identity(&fake, &moon) {
            golog::fatal(&e);
        }
        let state = PathBuf::from(opt.str("state", &fl));
        {
            use std::os::unix::fs::DirBuilderExt;
            if let Err(e) = std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&state)
            {
                golog::fatal(&go_path_error("mkdir", &state, &e));
            }
        }
        if !opt.str("relay", &fl).is_empty() {
            eprintln!("urb-gateway: the relay stays in the Go binary: run urb-fetch -relay");
            return 2;
        }
        let (lns, port) = listen_all(&opt.str("listen", &fl));

        // the host lives on a thread of its own: the HTTP threads hand
        // it their reads and wake it
        let log: Log = Arc::new(|l: &str| golog::line(l));
        libmesa_log(log.clone());
        // libmesa reads MESA_TRACE from the environment; the Go binding
        // traces when it is set too, so it is left as it is
        let (jobs_tx, jobs_rx) = channel::<Job>();
        let (open_tx, open_rx) = channel::<Result<(String, Sender<Event>), String>>();
        let new_moon = opt.bool("new-moon", &fl);
        let st = state.clone();
        let hlog = log.clone();
        let host_thread = thread::spawn(move || {
            let found = match state::find(&st, &moon, new_moon, &fake) {
                Ok(f) => f,
                Err(e) => {
                    let _ = open_tx.send(Err(e));
                    return;
                }
            };
            let udp = GatewayUdp {
                fake: !fake.is_empty(),
                port: found.port,
                log: hlog.clone(),
                udp: None,
            };
            let io = NetIo::new(Box::new(udp), Box::new(found.store), hlog.clone());
            let waker = io.waker();
            let mut host = Host::new(io, random);
            let our = match host.open(!fake.is_empty()) {
                Ok(o) => o,
                Err(f) => {
                    let _ = open_tx.send(Err(f.message));
                    return;
                }
            };
            let _ = open_tx.send(Ok((our, waker)));
            let mut pending: HashMap<u64, Sender<Result<Page, Fail>>> = HashMap::new();
            loop {
                while let Ok(job) = jobs_rx.try_recv() {
                    match job {
                        Job::Read(url, tx) => {
                            let id = host.submit(Request::Read(url));
                            pending.insert(id, tx);
                        }
                        Job::Close(done) => {
                            let _ = host.close();
                            let _ = done.send(());
                            return;
                        }
                    }
                }
                let ready: Vec<u64> = pending.keys().copied().collect();
                for id in ready {
                    if let Some(r) = host.take(id) {
                        let tx = pending.remove(&id).expect("pending");
                        let _ = tx.send(match r {
                            Ok(Reply::Read(p)) => Ok(Page {
                                key: p.key,
                                mark: p.mark.unwrap_or_else(|| "gmi".into()),
                                body: p.body.into_bytes(),
                                how: p.how.into(),
                            }),
                            Ok(_) => Err((502, b"the read gave no page".to_vec())),
                            Err(f) => Err(fail_of(&f)),
                        });
                    }
                }
                host.turn();
            }
        });
        let (our, waker) = match open_rx.recv() {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => golog::fatal(&e),
            Err(_) => golog::fatal("the host stopped"),
        };
        golog::line(&format!("running as {our}"));
        unsafe { signal(SIGINT, on_int) };

        let jobs = Mutex::new(jobs_tx.clone());
        let fetch = move |url: &str| -> Result<Page, Fail> {
            let (tx, rx) = channel();
            let sent = jobs
                .lock()
                .map(|j| j.send(Job::Read(url.to_string(), tx)).is_ok())
                .unwrap_or(false);
            let _ = waker.send(Event::Wake);
            match sent.then(|| rx.recv().ok()).flatten() {
                Some(r) => r,
                None => Err((502, b"the gateway is closing".to_vec())),
            }
        };
        let fetch: &'static (dyn Fn(&str) -> Result<Page, Fail> + Sync) =
            Box::leak(Box::new(fetch));
        let glog: &'static (dyn Fn(&str) + Sync) = &|l: &str| golog::line(l);
        let gw: &'static Gateway<'static> = Box::leak(Box::new(Gateway {
            port: port.clone(),
            ship: our,
            fetch,
            log: glog,
        }));
        let handle: &'static (dyn Fn(&http::Request) -> http::Response + Sync) =
            Box::leak(Box::new(|r: &http::Request| gw.handle(r)));
        for ln in lns {
            thread::spawn(move || {
                for c in ln.incoming().flatten() {
                    thread::spawn(move || http::serve_conn(c, handle));
                }
            });
        }
        golog::line(&format!("serving http://urb.localhost:{port}/"));
        while !STOP.load(Ordering::Relaxed) {
            if host_thread.is_finished() {
                return ended(host_thread);
            }
            thread::sleep(Duration::from_millis(100));
        }
        let (done_tx, done_rx) = channel();
        if jobs_tx.send(Job::Close(done_tx)).is_ok() {
            let _ = done_rx.recv_timeout(Duration::from_secs(10));
        }
        drop(host_thread);
        0
    }
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    std::process::exit(run::main());
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    #[test]
    fn a_host_that_panics_ends_the_gateway_with_gos_code() {
        let h = std::thread::spawn(|| panic!("the host broke"));
        while !h.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(super::run::ended(h), 2);
        assert_eq!(super::run::ended(std::thread::spawn(|| {})), 0);
    }
}

#[cfg(target_family = "wasm")]
fn main() {
    eprintln!("urb-gateway: the gateway does not run as wasm");
    std::process::exit(1);
}
