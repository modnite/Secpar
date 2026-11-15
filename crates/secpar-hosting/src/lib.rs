//! The hosting side of Secpar put together: the tunnel only lets in the devices
//! the account's access list allows, and the engine behind it only pairs with a
//! device that came in through the tunnel.
//!
//! The engine listens on this computer only. A device reaches it through the
//! tunnel, where its certificate has been checked against the list, and asks for
//! pairing over the tunnel's own channel. So the engine's pairing is always tied
//! to one device of one account, and is taken away again when that device leaves
//! the list.
use secpar_core::{
    account::{Knock, Reach, Service},
    hosting::{Access, Peer, Refreshed},
    identity::{hex, unhex},
};
use secpar_tunnel::{Fingerprint, Identity};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

mod engine;
pub mod outside;
#[cfg(test)]
mod tests;

/// The engine's base port on every Secpar hosting computer. Chosen away from the
/// usual range of this engine family, so it runs beside an installed one.
pub const ENGINE_BASE: u16 = 46989;

pub type Pending<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The stream component's own identity towards the engine, as it shows up in
/// the engine's pending pairings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pairing {
    pub client_id: String,
    pub certificate_sha256: String,
    pub pin: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The engine has not seen this pairing attempt yet. Worth trying again shortly.
    NotPending,
    /// This identity was taken away before. The device must make a new one.
    Revoked,
    Failed,
}
/// The part of the engine's supervisor that hosting needs.
pub trait Engine: Send + Sync + 'static {
    /// Completes a pairing for the device `device` of the account `owner`.
    fn approve<'a>(&'a self, owner: &'a str, device: &'a str, pairing: &'a Pairing, name: &'a str) -> Pending<'a, Result<(), Refusal>>;
    /// Takes away everything granted to this device. Returns whether it worked.
    fn revoke<'a>(&'a self, owner: &'a str, device: &'a str) -> Pending<'a, bool>;
    /// The `(owner, device)` pairs that hold a grant now.
    fn granted(&self) -> Pending<'_, Vec<(String, String)>>;
}

#[derive(Deserialize)]
enum Request {
    #[serde(rename = "pair")]
    Pair(Pairing),
}
/// What a device hears back on the tunnel's own channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Paired,
    /// The device should make a new stream identity and pair again.
    Revoked,
    NotAllowed,
    Failed,
    Unknown,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

struct Inner<E> {
    access: Arc<Access>,
    engine: Arc<E>,
}
impl<E: Engine> Inner<E> {
    async fn answer(&self, device: Fingerprint, request: Vec<u8>) -> Answer {
        let Ok(Request::Pair(pairing)) = serde_json::from_slice(&request) else { return Answer::Unknown };
        // The tunnel let this device in, but the list may have changed since.
        let (Some(peer), Some((owner, _))) = (self.access.peer(&device, now()), self.access.listed()) else {
            return Answer::NotAllowed;
        };
        // The name and the account come from the signed list, never from the device.
        let (account, name) = match peer {
            Peer::Own { name } => (owner, name),
            Peer::Guest { name, account, handle, .. } => (account, format!("{handle}: {name}")),
        };
        let device = hex(&device);
        // The device starts pairing with the engine and asks here at the same
        // time, so the engine may need a moment to show the attempt.
        for _ in 0..40 {
            match self.engine.approve(&account, &device, &pairing, &name).await {
                Ok(()) => return Answer::Paired,
                Err(Refusal::NotPending) => tokio::time::sleep(Duration::from_millis(250)).await,
                Err(Refusal::Revoked) => return Answer::Revoked,
                Err(Refusal::Failed) => return Answer::Failed,
            }
        }
        Answer::Failed
    }
}

/// A running hosting side. Dropping it closes the tunnel.
pub struct Hosting<E: Engine> {
    inner: Arc<Inner<E>>,
    tunnel: secpar_tunnel::Host,
    /// This network's address on the internet, once found, and whether the
    /// router agreed to pass the tunnel's port through.
    outside: std::sync::Mutex<(Option<IpAddr>, bool)>,
    /// Devices to keep sending small packets to, and until when, so the
    /// routers in between let their packets in.
    punching: std::sync::Mutex<Vec<(tokio::time::Instant, SocketAddr)>>,
}
impl<E: Engine> Hosting<E> {
    /// Opens the tunnel on `bind` for the devices `access` allows right now, in
    /// front of an engine that listens on `engine_ip` with `ports`.
    pub async fn start(bind: SocketAddr, identity: &Identity, access: Arc<Access>, engine: Arc<E>, engine_ip: IpAddr, ports: secpar_tunnel::Ports) -> secpar_tunnel::Result<Self> {
        let allowed = secpar_tunnel::Allowed::new(access.allowed(now()));
        let tunnel = secpar_tunnel::serve(bind, identity, allowed, engine_ip, ports).await?;
        let inner = Arc::new(Inner { access, engine });
        let answering = inner.clone();
        tunnel.on_request(Arc::new(move |device, request| {
            let answering = answering.clone();
            Box::pin(async move { serde_json::to_vec(&answering.answer(device, request).await).unwrap_or_default() })
        }));
        Ok(Self { inner, tunnel, outside: Default::default(), punching: Default::default() })
    }
    pub fn tunnel(&self) -> &secpar_tunnel::Host {
        &self.tunnel
    }
    pub fn access(&self) -> &Access {
        &self.inner.access
    }

    /// Brings the tunnel and the engine in line with the list as it is now:
    /// closes the tunnels of devices that may no longer connect, and takes the
    /// engine pairing away from devices the list no longer names at all.
    ///
    /// A friend whose acceptance ran out only loses the tunnel. Their pairing
    /// stays, because the engine cannot be reached without the tunnel and they
    /// may be accepted again.
    pub async fn apply(&self) {
        self.tunnel.set_allowed(self.inner.access.allowed(now()));
        // Without a list nothing is known about who left it, so nothing is revoked.
        let Some((_, listed)) = self.inner.access.listed() else { return };
        let listed: Vec<String> = listed.iter().map(hex).collect();
        for (owner, device) in self.inner.engine.granted().await {
            if !listed.contains(&device) {
                self.inner.engine.revoke(&owner, &device).await;
            }
        }
    }

    /// Where other devices can reach this tunnel: its port, and the address this
    /// computer uses on its network (or the one address the tunnel is bound to).
    pub fn reach(&self) -> Option<Reach> {
        let bound = self.tunnel.local_addr().ok()?;
        let address = if bound.ip().is_unspecified() {
            // No packet is sent; this only asks the system which address it would use.
            let probe = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
            probe.connect("192.0.2.1:9").ok()?;
            probe.local_addr().ok()?.ip()
        } else {
            bound.ip()
        };
        let mut addresses = vec![address.to_string()];
        // The home address first: it is tried first and is the faster way.
        if let (Some(public), true) = (self.outside.lock().unwrap().0, bound.ip().is_unspecified()) {
            addresses.push(public.to_string());
        }
        Some(Reach { port: bound.port(), addresses })
    }

    /// Finds this network's address on the internet and asks the router to pass
    /// the tunnel's port through. Returns the address, if found, and whether the
    /// router agreed. Does nothing when the tunnel is bound to one address only.
    pub async fn reach_outside(&self) -> (Option<IpAddr>, bool) {
        let Ok(bound) = self.tunnel.local_addr() else { return (None, false) };
        if !bound.ip().is_unspecified() {
            return (None, false);
        }
        let local = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|probe| probe.connect("192.0.2.1:9").and_then(|_| probe.local_addr()));
        let opened = match local {
            Ok(SocketAddr::V4(local)) => outside::open_on_router(*local.ip(), bound.port()).await,
            _ => false,
        };
        let found = (outside::public_address().await, opened);
        *self.outside.lock().unwrap() = found;
        found
    }

    /// Fetches a new list with the device's own key and applies it.
    pub async fn refresh(&self, service: &Service, identity: &Identity) -> Refreshed {
        let reach = self.reach();
        let result = self.inner.access.refresh(service, identity, reach.as_ref()).await;
        if result == Refreshed::Removed {
            // This computer is no longer a hosting device of the account.
            self.tunnel.set_allowed([]);
            for (owner, device) in self.inner.engine.granted().await {
                self.inner.engine.revoke(&owner, &device).await;
            }
        } else {
            self.apply().await;
        }
        result
    }

    /// The owner accepted a friend who is asked about each time, until `until`.
    pub async fn accept(&self, device: &Fingerprint, until: u64) -> bool {
        let accepted = self.inner.access.accept(device, until);
        self.apply().await;
        accepted
    }
    /// The owner ended a friend's visit.
    pub async fn dismiss(&self, device: &Fingerprint) {
        self.inner.access.dismiss(device);
        self.apply().await;
    }

    /// Keeps everything current: applies time limits every 20 seconds and asks
    /// the account service for a new list every 15 minutes. Never returns.
    ///
    /// It also asks for a new list when a device that is not allowed tries to
    /// connect, at most once in ten seconds: a computer the owner has just
    /// signed in on is not on the old list yet, and should not have to wait.
    pub async fn keep_current(&self, service: &Service, identity: &Identity) {
        tokio::join!(self.keep_list(service, identity), self.meet(service, identity), self.punch());
    }

    async fn punch(&self) {
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let to: Vec<SocketAddr> = {
                let mut punching = self.punching.lock().unwrap();
                punching.retain(|(until, _)| *until > tokio::time::Instant::now());
                punching.iter().map(|(_, to)| *to).collect()
            };
            self.tunnel.punch(&to);
        }
    }

    /// Waits for devices that cannot reach this computer directly. Each says
    /// where it is on the internet and, if it has one, its relay address; this
    /// computer answers with its own two, sends packets towards the device so
    /// the routers let it in, and adds the device at the relay. Only devices on
    /// the access list get an answer, and what then arrives is the tunnel, which
    /// checks them again. The relay address is given back ten minutes after the
    /// last use. In both lists the first entry is the direct address
    /// (0.0.0.0:9 when unknown) and the second, if any, the relay address.
    async fn meet(&self, service: &Service, identity: &Identity) {
        let relay = self.tunnel.relay();
        let mut answers: Vec<Knock> = Vec::new();
        let mut used = tokio::time::Instant::now();
        let mut kept = used;
        loop {
            let knocks = match service.device_knocks(identity, &answers).await {
                Ok(knocks) => knocks,
                Err(_) => {
                    answers.clear();
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
            };
            answers.clear();
            for knock in knocks {
                let peers: Vec<SocketAddr> = knock.candidates.iter().filter_map(|c| c.parse().ok()).collect();
                if !unhex(&knock.device).is_some_and(|device| self.inner.access.allowed(now()).contains(&device)) {
                    // Perhaps a device the list does not have yet.
                    self.refresh(service, identity).await;
                    if !unhex(&knock.device).is_some_and(|device| self.inner.access.allowed(now()).contains(&device)) {
                        continue;
                    }
                }
                let Some(direct) = peers.first().copied() else { continue };
                // Direct: where this socket is seen from outside, and packets towards the device.
                let seen = match tokio::net::lookup_host(outside::STUN).await.ok().and_then(|mut found| found.find(SocketAddr::is_ipv4)) {
                    Some(stun) => {
                        let asking = relay.clone();
                        tokio::task::spawn_blocking(move || asking.seen_from_outside(stun)).await.ok().flatten()
                    }
                    None => None,
                };
                if !direct.ip().is_unspecified() {
                    self.punching.lock().unwrap().push((tokio::time::Instant::now() + Duration::from_secs(12), direct));
                }
                // The relay, when the device has a relay address and this service offers one.
                if let Some(there) = peers.get(1).copied() {
                    if relay.address().is_none() {
                        if let Ok(access) = service.device_relay(identity).await {
                            if let Some(server) = tokio::net::lookup_host(&access.server).await.ok().and_then(|mut found| found.find(SocketAddr::is_ipv4)) {
                                let starting = relay.clone();
                                let _ = tokio::task::spawn_blocking(move || starting.start(server, access.username, access.credential)).await;
                                kept = tokio::time::Instant::now();
                            }
                        }
                    }
                    if relay.address().is_some() {
                        let adding = relay.clone();
                        let _ = tokio::task::spawn_blocking(move || adding.add(there)).await;
                    }
                }
                let mut candidates = vec![seen.map_or("0.0.0.0:9".to_string(), |a| a.to_string())];
                candidates.extend(relay.address().filter(|_| peers.len() > 1).map(|a| a.to_string()));
                if seen.is_none() && candidates.len() < 2 {
                    continue;
                }
                answers.push(Knock { device: knock.device, candidates });
                used = tokio::time::Instant::now();
            }
            if relay.address().is_some() {
                if !self.tunnel.connected().is_empty() {
                    used = tokio::time::Instant::now();
                }
                if used.elapsed() > Duration::from_secs(600) {
                    relay.stop();
                } else if kept.elapsed() > Duration::from_secs(200) {
                    kept = tokio::time::Instant::now();
                    let relay = relay.clone();
                    let _ = tokio::task::spawn_blocking(move || relay.keep()).await;
                }
            }
        }
    }

    async fn keep_list(&self, service: &Service, identity: &Identity) {
        let mut tick = 0u32;
        let mut knocked = tokio::time::Instant::now() - Duration::from_secs(10);
        loop {
            if tick % 45 == 0 {
                // Every 15 minutes, so the router's hour never runs out.
                self.reach_outside().await;
                self.refresh(service, identity).await;
            } else {
                self.apply().await;
            }
            tick = tick.wrapping_add(1);
            let next = tokio::time::sleep(Duration::from_secs(20));
            tokio::pin!(next);
            loop {
                tokio::select! {
                    _ = &mut next => break,
                    _ = self.tunnel.turned_away() => {
                        if knocked.elapsed() >= Duration::from_secs(10) {
                            knocked = tokio::time::Instant::now();
                            self.refresh(service, identity).await;
                        }
                    }
                }
            }
        }
    }
}

/// The connecting side of the same exchange: asks the host, through the tunnel,
/// to complete the pairing its stream component has just started.
pub async fn request_pairing(tunnel: &secpar_tunnel::Client, pairing: &Pairing) -> Answer {
    let Ok(request) = serde_json::to_vec(&serde_json::json!({ "pair": pairing })) else { return Answer::Failed };
    match tunnel.request(&request).await {
        Ok(answer) => serde_json::from_slice(&answer).unwrap_or(Answer::Failed),
        Err(_) => Answer::Failed,
    }
}
