//! Finds out how the internet can reach a stream socket: its STUN-reflexive address and a
//! router (UPnP) mapping of the same UDP port. Both run at once; used by hosts that bind a
//! fresh port per stream (the Windows host maps its fixed port ahead of time instead).

use crate::stun;
use crate::upnp::{self, Gateway, Mapped, Proto};
use std::net::{SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a "no UPnP router here" answer is trusted, so a stream start does not wait for it again.
const MISS_TTL: Duration = Duration::from_secs(300);
const DISCOVER: Duration = Duration::from_millis(1800);

#[derive(Debug, Default)]
pub struct Reach {
    /// This socket's public address as a STUN server sees it.
    pub reflexive: Option<SocketAddr>,
    /// The router now forwards this UDP port here; the public IP is in `mapped.public`.
    pub mapped: Option<Mapped>,
}

enum Cached {
    Found(Gateway),
    Missing(Instant),
}

static GATEWAY: Mutex<Option<Cached>> = Mutex::new(None);

fn gateway() -> Result<Gateway, String> {
    match &*GATEWAY.lock().unwrap() {
        Some(Cached::Found(g)) => return Ok(g.clone()),
        Some(Cached::Missing(at)) if at.elapsed() < MISS_TTL => return Err("no UPnP router (cached)".into()),
        _ => {}
    }
    let r = Gateway::discover(DISCOVER);
    *GATEWAY.lock().unwrap() = Some(match &r {
        Ok(g) => Cached::Found(g.clone()),
        Err(_) => Cached::Missing(Instant::now()),
    });
    r
}

/// Blocking, about 2 s at most. `sock` must not be read by anyone else meanwhile (STUN reads its
/// answer from it). `port` is the port `sock` is bound to; the router maps the same external port.
/// `upnp_on` false skips the router mapping.
pub fn gather(sock: &UdpSocket, port: u16, upnp_on: bool) -> Reach {
    let mapping = upnp_on.then(|| std::thread::spawn(move || gateway().and_then(|g| upnp::map_udp_on(g, port))));
    let reflexive = stun::public_addr(sock);
    let mapped = mapping.and_then(|h| h.join().ok()).and_then(|r| match r {
        Ok(m) => Some(m),
        Err(e) => {
            log::debug!("upnp: {e}");
            None
        }
    });
    Reach { reflexive, mapped }
}

/// Removes the mapping made by [`gather`] (best effort, blocking).
pub fn release(m: &Mapped) {
    let _ = m.gateway.unmap(Proto::Udp, m.port);
}
