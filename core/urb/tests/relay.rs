//! The native host's relay transport (src/host/net.rs) against the Go
//! relay itself, `urb-fetch -relay -relay-fake`: a datagram to a galaxy
//! and one to an address go out of the relay's UDP port to where the
//! fake network has them, and a datagram back to that port comes in
//! through the WebSocket with its address. A relay nobody runs fails
//! the start with the shim's words, and so, after 10 s, does one that
//! accepts and never answers the upgrade.
//!
//! It builds the relay with `go build` (or takes URB_FETCH, a built
//! one). Without go it skips, unless REQUIRE_ALL is set.

use std::net::{Ipv4Addr, UdpSocket};
use std::process::{Child, Command};
use std::sync::mpsc::channel;
use std::time::Duration;
use urb::core::Lane;
use urb::host::Event;
use urb::host::net::{Relay, Transport};

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn relay_binary() -> Option<String> {
    if let Ok(b) = std::env::var("URB_FETCH") {
        return Some(b);
    }
    let out = std::env::temp_dir().join(format!("urb-relay-test-{}", std::process::id()));
    let repo = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    match Command::new("go")
        .args(["build", "-o"])
        .arg(&out)
        .arg(".")
        .current_dir(repo)
        .status()
    {
        Ok(s) if s.success() => Some(out.display().to_string()),
        r => {
            assert!(
                std::env::var_os("REQUIRE_ALL").is_none(),
                "REQUIRE_ALL: no relay to test against ({r:?})"
            );
            None
        }
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_relay_nobody_runs_fails_the_start() {
    let mut r = Relay::new("ws://127.0.0.1:1");
    let (tx, _rx) = channel();
    assert_eq!(
        r.start("~zod", tx).unwrap_err(),
        "can't reach the relay at ws://127.0.0.1:1"
    );
}

#[test]
fn a_relay_that_never_answers_fails_the_start() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", l.local_addr().unwrap());
    let held = std::thread::spawn(move || l.accept());
    let mut r = Relay::new(&url);
    let (tx, _rx) = channel();
    let t = std::time::Instant::now();
    assert_eq!(
        r.start("~zod", tx).unwrap_err(),
        format!("can't reach the relay at {url}")
    );
    let took = t.elapsed();
    assert!(
        took >= Duration::from_secs(9) && took < Duration::from_secs(15),
        "{took:?}"
    );
    drop(held);
}

#[test]
fn datagrams_go_through_the_go_relay_both_ways() {
    let Some(bin) = relay_binary() else {
        eprintln!("skipped: no go to build the relay");
        return;
    };
    let port = free_port();
    let _relay = Killed(
        Command::new(&bin)
            .args(["-relay", &format!("127.0.0.1:{port}"), "-relay-fake"])
            .spawn()
            .expect("the relay runs"),
    );
    // galaxy 250 on the fake network is 127.0.0.1:31587
    let g = 250u8;
    let fake = UdpSocket::bind((Ipv4Addr::LOCALHOST, 31337 + u16::from(g))).unwrap();
    fake.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut r = Relay::new(&format!("ws://127.0.0.1:{port}"));
    let (tx, rx) = channel();
    let mut started = Err(String::new());
    for _ in 0..50 {
        started = r.start("~natdul-molnec-dozzod-dozhus", tx.clone());
        if started.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    started.expect("the relay answers");
    // the relay sends nothing until it has the ship's name: give it a
    // moment, then resend until a datagram arrives
    let mut buf = [0u8; 2048];
    let mut from = None;
    for _ in 0..20 {
        r.send(Lane::Galaxy(g), b"to the galaxy");
        fake.set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        if let Ok((n, f)) = fake.recv_from(&mut buf) {
            assert_eq!(&buf[..n], b"to the galaxy");
            from = Some(f);
            break;
        }
    }
    let from = from.expect("the galaxy hears the relay");
    // by address too
    r.send(
        Lane::Ip(u32::from(Ipv4Addr::LOCALHOST), 31337 + u16::from(g)),
        b"to the address",
    );
    fake.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let (n, _) = fake.recv_from(&mut buf).expect("by address");
    assert_eq!(&buf[..n], b"to the address");
    // and back, through the WebSocket, with where it came from
    fake.send_to(b"an answer", from).unwrap();
    match rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the answer comes in")
    {
        Event::Packet { ip, port, bytes } => {
            assert_eq!(bytes, b"an answer");
            assert_eq!(ip, u32::from(Ipv4Addr::LOCALHOST));
            assert_eq!(port, 31337 + u16::from(g));
        }
        e => panic!("{e:?}"),
    }
    r.close();
}
