//! The host side of "connect anywhere" for one PRS stream socket, shared by every desktop host
//! (docs/remote-protocol.md): phone candidates and the punching window, an optional TURN
//! allocation on the same socket, which peers arrive through it, and the relay packet-rate gate.
//! Std only: no platform API, so Windows, Linux and macOS hosts use the same code.

use crate::turn::{self, Turn};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Packet rate through a TURN relay. Relays drop bursts above their packet-rate limit
/// (Cloudflare: 5-10 kpps per allocation), which keyframes hit.
pub const RELAY_PPS: u64 = 3_000;
/// Phone candidates kept per stream.
pub const MAX_CANDS: usize = 16;

#[derive(Default)]
pub struct Link {
    cands: Mutex<Vec<SocketAddr>>,
    punch_until: Mutex<Option<Instant>>,
    /// Our own TURN allocation, when NATs need relay-to-relay.
    turn: Mutex<Option<Arc<Turn>>>,
    /// Peers whose packets arrived through `turn`: replies go back the same way.
    via_relay: Mutex<HashSet<SocketAddr>>,
    gate: Mutex<Option<Instant>>,
}

impl Link {
    pub fn new() -> Link {
        Link::default()
    }

    /// Adds phone candidates and (re)starts the punching window of `window`.
    pub fn punch(&self, add: impl IntoIterator<Item = SocketAddr>, window: Duration) {
        let mut c = self.cands.lock().unwrap();
        for a in add {
            if c.len() < MAX_CANDS && !c.contains(&a) {
                c.push(a);
            }
        }
        if !c.is_empty() {
            *self.punch_until.lock().unwrap() = Some(Instant::now() + window);
        }
    }

    /// Still looking for the phone: no authenticated PROBE yet (`peer_known` false) and inside the window.
    pub fn punching(&self, peer_known: bool) -> bool {
        !peer_known && self.punch_until.lock().unwrap().is_some_and(|t| Instant::now() < t)
    }

    pub fn cands(&self) -> Vec<SocketAddr> {
        self.cands.lock().unwrap().clone()
    }

    pub fn turn(&self) -> Option<Arc<Turn>> {
        self.turn.lock().unwrap().clone()
    }

    pub fn is_relayed(&self, peer: &SocketAddr) -> bool {
        self.via_relay.lock().unwrap().contains(peer)
    }

    /// Records that `peer` reached us through the relay; true the first time.
    pub fn mark_relayed(&self, peer: SocketAddr) -> bool {
        self.via_relay.lock().unwrap().insert(peer)
    }

    /// Sends a sealed packet to `to`: wrapped for the relay when that peer came in through it.
    pub fn transmit(&self, sock: &UdpSocket, to: SocketAddr, pkt: &[u8]) -> bool {
        if let (Some(t), SocketAddr::V4(v4)) = (self.turn(), to) {
            if self.is_relayed(&to) {
                return sock.send_to(&t.wrap(v4, pkt), t.server).is_ok();
            }
        }
        sock.send_to(pkt, to).is_ok()
    }

    /// The relay copy of a punching packet: for public candidates not (yet) reached through it.
    pub fn transmit_punch_copy(&self, sock: &UdpSocket, to: SocketAddr, pkt: &[u8]) {
        if let (Some(t), SocketAddr::V4(v4)) = (self.turn(), to) {
            if turn::is_public(&v4) && !self.is_relayed(&to) {
                let _ = sock.send_to(&t.wrap(v4, pkt), t.server);
            }
        }
    }

    /// Classifies one received datagram. None: a TURN answer, already delivered to the waiting
    /// request. Some((peer, None)): a direct packet from `from`. Some((peer, Some(data))): relayed
    /// data from `peer`.
    pub fn unwrap(&self, from: SocketAddr, pkt: &[u8]) -> Option<(SocketAddr, Option<Vec<u8>>)> {
        match self.turn() {
            Some(t) if from == t.server => {
                let (peer, data) = t.on_packet(pkt)?;
                Some((SocketAddr::V4(peer), Some(data)))
            }
            _ => Some((from, None)),
        }
    }

    /// Allocates our own relay (blocking, ~100 ms), permits the phone's candidates and keeps the
    /// allocation alive until `stopped()`. Returns the relayed address to tell the phone.
    /// `sock` is the stream socket; its receive loop must hand the server's answers to
    /// [`Link::unwrap`] while this runs.
    pub fn start_turn(
        self: &Arc<Self>,
        sock: &UdpSocket,
        stopped: impl Fn() -> bool + Send + 'static,
        server: SocketAddr,
        user: String,
        pass: String,
        diagnostics: Option<fn(&str)>,
    ) -> Option<SocketAddrV4> {
        if self.turn().is_some() {
            return None;
        }
        let mut t = Turn::new(server, user, pass);
        if let Some(d) = diagnostics {
            t = t.with_diagnostics(d);
        }
        let t = Arc::new(t);
        *self.turn.lock().unwrap() = Some(t.clone()); // the receive loop now routes the server's answers to it
        let Some(relayed) = t.allocate(sock) else {
            *self.turn.lock().unwrap() = None;
            if let Some(d) = diagnostics {
                d("turn: allocation failed");
            }
            return None;
        };
        t.permit(sock, &v4(&self.cands()));
        if let Some(d) = diagnostics {
            d(&format!("turn: relaying at {relayed}"));
        }
        let (link, sock) = (self.clone(), sock.try_clone().ok()?);
        std::thread::spawn(move || loop {
            for _ in 0..240 {
                std::thread::sleep(Duration::from_secs(1));
                if stopped() {
                    t.refresh(&sock, 0); // free the allocation now
                    return;
                }
            }
            t.refresh(&sock, 600);
            t.permit(&sock, &v4(&link.cands()));
        });
        Some(relayed)
    }

    /// Permits one new phone candidate on our relay, if we have one (off the calling thread).
    pub fn permit_in_background(&self, sock: &UdpSocket, a: SocketAddr) {
        if let (Some(t), SocketAddr::V4(v4)) = (self.turn(), a) {
            if let Ok(sock) = sock.try_clone() {
                std::thread::spawn(move || t.permit(&sock, &[v4]));
            }
        }
    }

    /// Blocks until this packet's slot when sending through the relay at [`RELAY_PPS`].
    /// Does nothing for direct peers.
    pub fn throttle(&self, peer: &SocketAddr) {
        if !self.is_relayed(peer) {
            return;
        }
        let now = Instant::now();
        let slot = {
            let mut g = self.gate.lock().unwrap();
            let slot = g.map_or(now, |n| n.max(now));
            *g = Some(slot + Duration::from_micros(1_000_000 / RELAY_PPS));
            slot
        };
        let wait = slot.saturating_duration_since(now);
        if wait > Duration::from_micros(500) {
            std::thread::sleep(wait);
        }
    }
}

fn v4(c: &[SocketAddr]) -> Vec<SocketAddrV4> {
    c.iter().filter_map(|a| if let SocketAddr::V4(v) = a { Some(*v) } else { None }).collect()
}

/// Sends `items` of one frame through a relay spread over `items.len() / RELAY_PPS` seconds.
pub fn send_relay_paced<T>(items: &[T], mut send: impl FnMut(&T)) {
    let t0 = Instant::now();
    for (i, item) in items.iter().enumerate() {
        let due = t0 + Duration::from_micros(i as u64 * 1_000_000 / RELAY_PPS);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            if wait > Duration::from_micros(500) {
                std::thread::sleep(wait);
            }
        }
        send(item);
    }
}

/// `stream.ready` candidates in the order every host advertises them: LAN addresses, the
/// router-mapped public one (only when the stream socket is on the `mapped_port` the router maps),
/// then the STUN-reflexive one. Duplicates removed.
pub fn host_cands(lan: &[String], port: u16, mapped_port: u16, public: Option<IpAddr>, reflexive: Option<SocketAddr>) -> Vec<String> {
    let mut v: Vec<String> = lan.iter().map(|ip| format!("{ip}:{port}")).collect();
    v.extend(public.filter(|_| port == mapped_port).map(|ip| SocketAddr::new(ip, port).to_string()));
    v.extend(reflexive.map(|a| a.to_string()));
    let mut seen = HashSet::new();
    v.retain(|c| seen.insert(c.clone()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeServer;
    use crate::turn::{xor_addr, Msg};

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn candidates_in_order_without_duplicates() {
        let lan = ["192.168.0.126".to_string(), "10.0.0.2".to_string()];
        let pub_ip = Some(IpAddr::from([203, 0, 113, 7]));
        let refl = Some(sa("203.0.113.7:48998"));
        assert_eq!(host_cands(&lan, 48998, 48998, pub_ip, refl), ["192.168.0.126:48998", "10.0.0.2:48998", "203.0.113.7:48998"]);
        let refl = Some(sa("198.51.100.4:6100"));
        assert_eq!(host_cands(&lan[..1], 50000, 48998, pub_ip, refl), ["192.168.0.126:50000", "198.51.100.4:6100"], "the router maps only one port");
        assert_eq!(host_cands(&[], 5, 5, None, None), Vec::<String>::new());
    }

    #[test]
    fn punch_window_candidates_and_cap() {
        let l = Link::new();
        assert!(!l.punching(false), "nothing to punch yet");
        l.punch([], Duration::from_secs(1));
        assert!(!l.punching(false), "no candidates, no window");
        l.punch([sa("203.0.113.9:1000"), sa("203.0.113.9:1000")], Duration::from_millis(80));
        assert_eq!(l.cands().len(), 1, "deduplicated");
        assert!(l.punching(false));
        assert!(!l.punching(true), "stops at the first authenticated PROBE");
        std::thread::sleep(Duration::from_millis(120));
        assert!(!l.punching(false), "window over");
        l.punch((1..40).map(|p| sa(&format!("203.0.113.9:{p}"))), Duration::from_secs(1));
        assert_eq!(l.cands().len(), MAX_CANDS);
        assert!(l.punching(false), "a new candidate restarts the window");
    }

    /// Runs the stream socket's receive loop the way a host does: server answers go to the link.
    fn receive_loop(link: Arc<Link>, sock: UdpSocket, stop: Arc<std::sync::atomic::AtomicBool>) {
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
                let _ = link.unwrap(from, &buf[..n]);
            }
        });
    }

    #[test]
    fn stun_reflexive_against_a_fake_server() {
        let server = FakeServer::start();
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        assert_eq!(crate::stun::reflexive(&sock, server.addr), Some(sock.local_addr().unwrap()));
        assert_eq!(server.log(), ["binding"]);
    }

    #[test]
    fn relay_allocation_permit_refresh_and_release() {
        let server = FakeServer::start();
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let link = Arc::new(Link::new());
        let phone = sa("203.0.113.9:4000");
        link.punch([phone, sa("192.168.1.5:1")], Duration::from_secs(5));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        receive_loop(link.clone(), sock.try_clone().unwrap(), stop.clone());
        let flag = stop.clone();
        let relayed = link.start_turn(&sock, move || flag.load(std::sync::atomic::Ordering::Relaxed), server.addr, "u".into(), "p".into(), None);
        assert_eq!(relayed, Some(FakeServer::RELAYED.parse().unwrap()));
        let log = server.log();
        assert!(log.contains(&"allocate".to_string()), "{log:?}");
        assert!(log.contains(&"permit 203.0.113.9:4000".to_string()), "{log:?}");
        assert!(log.contains(&"bind 203.0.113.9:4000".to_string()), "{log:?}");
        assert!(!log.iter().any(|l| l.contains("192.168")), "private candidates are never permitted: {log:?}");
        assert!(link.start_turn(&sock, || true, server.addr, "u".into(), "p".into(), None).is_none(), "one allocation per stream");

        // a candidate that arrives later is permitted off-thread
        link.permit_in_background(&sock, sa("203.0.113.10:4001"));
        std::thread::sleep(Duration::from_millis(400));
        assert!(server.log().contains(&"permit 203.0.113.10:4001".to_string()), "{:?}", server.log());

        // stopping frees the allocation (refresh with lifetime 0)
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(1600));
        assert!(server.log().contains(&"refresh 0".to_string()), "{:?}", server.log());
    }

    #[test]
    fn failed_allocation_leaves_no_relay() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let dead = UdpSocket::bind("127.0.0.1:0").unwrap(); // nobody answers
        let link = Arc::new(Link::new());
        let r = link.start_turn(&sock, || true, dead.local_addr().unwrap(), "u".into(), "p".into(), None);
        assert_eq!(r, None);
        assert!(link.turn().is_none(), "direct sending continues untouched");
    }

    #[test]
    fn relayed_peers_get_wrapped_replies_and_the_rate_gate() {
        let server = FakeServer::start();
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let link = Arc::new(Link::new());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        receive_loop(link.clone(), sock.try_clone().unwrap(), stop.clone());
        let flag = stop.clone();
        link.start_turn(&sock, move || flag.load(std::sync::atomic::Ordering::Relaxed), server.addr, "u".into(), "p".into(), None).unwrap();

        let peer = sa("203.0.113.9:4000");
        let direct = UdpSocket::bind("127.0.0.1:0").unwrap();
        direct.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        // direct before the peer is known to be relayed
        assert!(link.transmit(&sock, direct.local_addr().unwrap(), b"hello"));
        let mut b = [0u8; 64];
        assert_eq!(direct.recv(&mut b).unwrap(), 5);

        // a Data indication from the server: peer address unwrapped, then marked as relayed
        let data = Msg::new(0x0017).add(0x0012, xor_addr(match peer { SocketAddr::V4(v) => v, _ => unreachable!() })).add(0x0013, b"pkt".to_vec()).encode(None);
        let (from, d) = link.unwrap(server.addr, &data).unwrap();
        assert_eq!((from, d.as_deref()), (peer, Some(&b"pkt"[..])));
        assert!(link.mark_relayed(from));
        assert!(!link.mark_relayed(from), "only the first one is news");
        assert!(link.unwrap(direct.local_addr().unwrap(), b"x").unwrap().1.is_none(), "other sources are direct");

        // replies to that peer go to the server, as ChannelData (bound by the permit) or a Send indication
        let before = server.log().len();
        link.permit_in_background(&sock, peer);
        std::thread::sleep(Duration::from_millis(300));
        assert!(link.transmit(&sock, peer, b"reply"));
        std::thread::sleep(Duration::from_millis(100));
        let log = server.log();
        assert!(log[before..].iter().any(|l| l.starts_with("channeldata 4000") || l == "send"), "{log:?}");

        // The rate gate must enforce its minimum interval. Scheduler delays may add time.
        let t0 = Instant::now();
        for _ in 0..300 {
            link.throttle(&peer);
        }
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(80), "{took:?}");
        // Direct packets must bypass the gate without reserving any relay slots.
        // Checking its state avoids imposing a wall-clock deadline on the CI scheduler.
        let gate_before = *link.gate.lock().unwrap();
        for _ in 0..300 {
            link.throttle(&sa("203.0.113.99:1"));
        }
        assert_eq!(*link.gate.lock().unwrap(), gate_before);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn relay_paced_frame_takes_packets_over_rate() {
        let t0 = Instant::now();
        let mut n = 0;
        send_relay_paced(&[0u8; 300], |_| n += 1);
        assert_eq!(n, 300);
        assert!(t0.elapsed() >= Duration::from_millis(80));
    }

    #[test]
    fn punch_copy_only_for_public_candidates_through_a_relay() {
        let server = FakeServer::start();
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let link = Arc::new(Link::new());
        link.transmit_punch_copy(&sock, sa("203.0.113.9:1"), b"x");
        std::thread::sleep(Duration::from_millis(60));
        assert!(server.log().is_empty(), "no relay, no copy");
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        receive_loop(link.clone(), sock.try_clone().unwrap(), stop.clone());
        let flag = stop.clone();
        link.start_turn(&sock, move || flag.load(std::sync::atomic::Ordering::Relaxed), server.addr, "u".into(), "p".into(), None).unwrap();
        let n = server.log().len();
        link.transmit_punch_copy(&sock, sa("192.168.1.5:1"), b"x");
        link.transmit_punch_copy(&sock, sa("203.0.113.9:1"), b"x");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(server.log()[n..], ["send".to_string()], "only the public candidate");
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
