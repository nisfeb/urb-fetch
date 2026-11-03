//! The public gateway's limits (docs/rust-core.md, public mode): who a
//! request is from, a token bucket per client, and per publisher a cap
//! on reads in flight and a rate, so a crawler can make the gateway
//! neither answer it without end nor hammer one ship. Time is handed
//! in, so the tests run on a fake clock.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A token bucket: `rate` tokens a minute, at most `burst` kept.
#[derive(Clone, Copy, Debug)]
pub struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn new(burst: u32, now: Instant) -> Bucket {
        Bucket {
            tokens: f64::from(burst),
            at: now,
        }
    }

    /// refilled to now
    fn fill(&mut self, rate: u32, burst: u32, now: Instant) {
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + dt * f64::from(rate) / 60.0).min(f64::from(burst));
        self.at = now;
    }

    /// take a token, or how long until one is there
    fn take(&mut self, rate: u32, burst: u32, now: Instant) -> Result<(), Duration> {
        self.fill(rate, burst, now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return Ok(());
        }
        if rate == 0 {
            return Err(Duration::from_secs(60));
        }
        let need = (1.0 - self.tokens) * 60.0 / f64::from(rate);
        Err(Duration::from_secs_f64(need))
    }

    fn full(&self, burst: u32) -> bool {
        self.tokens >= f64::from(burst)
    }
}

/// whole seconds to wait, at least one, for `Retry-After`
pub fn retry_secs(d: Duration) -> u64 {
    (d.as_secs() + u64::from(d.subsec_nanos() > 0)).max(1)
}

/// The client a request is from: the TCP peer, or, when the peer is
/// loopback and the gateway trusts its proxy, the last entry of the
/// last `X-Forwarded-For` (the one the proxy itself added). A header
/// from any other peer is ignored, so nobody can pose as another.
pub fn client(peer: Option<IpAddr>, forwarded: &[&[u8]], trust_proxy: bool) -> Option<IpAddr> {
    let peer = peer?;
    if !(trust_proxy && peer.is_loopback()) {
        return Some(peer);
    }
    let last = forwarded
        .last()
        .and_then(|v| v.split(|&b| b == b',').next_back());
    match last.and_then(|v| std::str::from_utf8(v).ok()) {
        Some(v) => v.trim().parse().ok().or(Some(peer)),
        None => Some(peer),
    }
}

/// the key a client's bucket is kept under: an IPv4 address, or an IPv6
/// address's /64, since one host is given a whole /64
pub fn client_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return IpAddr::V4(v4);
            }
            let s = v6.segments();
            IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
    }
}

/// the limits as configured
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// requests a minute per client
    pub rate: u32,
    /// requests a client may make at once before the rate applies
    pub burst: u32,
    /// reads in flight per publisher
    pub ship_cold: u32,
    /// reads a minute per publisher
    pub ship_rate: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            rate: 60,
            burst: 20,
            ship_cold: 4,
            ship_rate: 120,
        }
    }
}

/// buckets kept before idle (full) ones are dropped
const KEEP: usize = 10_000;

/// The limits' state.
pub struct Limits {
    pub cfg: Config,
    clients: Mutex<HashMap<IpAddr, Bucket>>,
    ships: Mutex<HashMap<String, (u32, Bucket)>>,
}

/// a read in flight on a publisher, given back when dropped
pub struct Cold<'a> {
    limits: &'a Limits,
    ship: String,
}

impl Drop for Cold<'_> {
    fn drop(&mut self) {
        let mut ships = self.limits.ships.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((n, _)) = ships.get_mut(&self.ship) {
            *n = n.saturating_sub(1);
        }
    }
}

impl Limits {
    pub fn new(cfg: Config) -> Limits {
        Limits {
            cfg,
            clients: Mutex::new(HashMap::new()),
            ships: Mutex::new(HashMap::new()),
        }
    }

    /// a request from ip: allowed, or how long to wait
    pub fn request(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let (rate, burst) = (self.cfg.rate, self.cfg.burst);
        let mut cs = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if cs.len() >= KEEP {
            cs.retain(|_, b| {
                b.fill(rate, burst, now);
                !b.full(burst)
            });
            // a flood from more clients than that: memory before fairness
            if cs.len() >= KEEP * 10 {
                cs.clear();
            }
        }
        cs.entry(client_key(ip))
            .or_insert_with(|| Bucket::new(burst, now))
            .take(rate, burst, now)
    }

    /// a read that reaches ship, through the engine: a slot while it
    /// runs, or how long to wait. Cached answers never ask.
    pub fn cold(&self, ship: &str, now: Instant) -> Result<Cold<'_>, Duration> {
        let (rate, cap) = (self.cfg.ship_rate, self.cfg.ship_cold);
        let mut ships = self.ships.lock().unwrap_or_else(|e| e.into_inner());
        if ships.len() >= KEEP {
            ships.retain(|_, (n, b)| {
                b.fill(rate, rate, now);
                *n > 0 || !b.full(rate)
            });
            if ships.len() >= KEEP * 10 {
                ships.clear();
            }
        }
        let (n, b) = ships
            .entry(ship.to_string())
            .or_insert_with(|| (0, Bucket::new(rate, now)));
        if *n >= cap {
            return Err(Duration::from_secs(5));
        }
        b.take(rate, rate, now)?;
        *n += 1;
        Ok(Cold {
            limits: self,
            ship: ship.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_bucket_lets_the_burst_through_then_refills_at_the_rate() {
        let l = Limits::new(Config::default());
        let t = Instant::now();
        let a = ip("203.0.113.7");
        for _ in 0..20 {
            assert!(l.request(a, t).is_ok());
        }
        let wait = l.request(a, t).unwrap_err();
        assert_eq!(retry_secs(wait), 1, "60 a minute is one a second");
        // another client is not held up
        assert!(l.request(ip("203.0.113.8"), t).is_ok());
        // a second later, one more; then none
        let t1 = t + Duration::from_secs(1);
        assert!(l.request(a, t1).is_ok());
        assert!(l.request(a, t1).is_err());
        // a minute later the burst is back, and no more
        let t2 = t1 + Duration::from_secs(60);
        for _ in 0..20 {
            assert!(l.request(a, t2).is_ok());
        }
        assert!(l.request(a, t2).is_err());
    }

    #[test]
    fn an_ipv6_host_is_one_client_per_64() {
        let l = Limits::new(Config {
            burst: 2,
            ..Config::default()
        });
        let t = Instant::now();
        assert!(l.request(ip("2001:db8:1:2::1"), t).is_ok());
        assert!(l.request(ip("2001:db8:1:2::ffff"), t).is_ok());
        assert!(l.request(ip("2001:db8:1:2:aaaa::3"), t).is_err());
        assert!(l.request(ip("2001:db8:1:3::1"), t).is_ok());
        assert_eq!(client_key(ip("::ffff:192.0.2.1")), ip("192.0.2.1"));
    }

    #[test]
    fn the_forwarded_address_is_trusted_only_from_a_loopback_proxy() {
        let xff: &[&[u8]] = &[b"198.51.100.1, 192.0.2.9"];
        let lo = Some(ip("127.0.0.1"));
        let far = Some(ip("192.0.2.200"));
        assert_eq!(client(lo, xff, true), Some(ip("192.0.2.9")));
        // a peer that is not loopback can't name another client
        assert_eq!(client(far, xff, true), far);
        // nor can anyone, without -trust-proxy
        assert_eq!(client(lo, xff, false), lo);
        // the last header's last entry is the proxy's own
        let two: &[&[u8]] = &[b"10.0.0.1", b"192.0.2.10"];
        assert_eq!(client(lo, two, true), Some(ip("192.0.2.10")));
        // garbage, or no header, is the peer
        assert_eq!(client(lo, &[b"nope"], true), lo);
        assert_eq!(client(lo, &[], true), lo);
        assert_eq!(
            client(Some(ip("::1")), &[b"2001:db8::5"], true),
            Some(ip("2001:db8::5"))
        );
    }

    #[test]
    fn a_publisher_has_a_cap_in_flight_and_a_rate() {
        let l = Limits::new(Config {
            ship_cold: 2,
            ship_rate: 3,
            ..Config::default()
        });
        let t = Instant::now();
        let a = l.cold("~zod", t).unwrap();
        let b = l.cold("~zod", t).unwrap();
        assert_eq!(l.cold("~zod", t).err(), Some(Duration::from_secs(5)));
        // another ship is not held up
        let _c = l.cold("~nec", t).unwrap();
        drop(a);
        // a slot is free; the third read this minute takes the last token
        let d = l.cold("~zod", t).unwrap();
        drop((b, d));
        let wait = l.cold("~zod", t).err().expect("over the rate");
        assert_eq!(retry_secs(wait), 20, "3 a minute is one in 20 s");
        assert!(l.cold("~zod", t + Duration::from_secs(20)).is_ok());
    }

    #[test]
    fn retry_after_is_whole_seconds_and_at_least_one() {
        assert_eq!(retry_secs(Duration::from_millis(1)), 1);
        assert_eq!(retry_secs(Duration::ZERO), 1);
        assert_eq!(retry_secs(Duration::from_millis(1500)), 2);
        assert_eq!(retry_secs(Duration::from_secs(5)), 5);
    }
}
