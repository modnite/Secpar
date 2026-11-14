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
    account::Service,
    hosting::{Access, Peer, Refreshed},
    identity::hex,
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
#[cfg(test)]
mod tests;

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
        Ok(Self { inner, tunnel })
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

    /// Fetches a new list with the device's own key and applies it.
    pub async fn refresh(&self, service: &Service, identity: &Identity) -> Refreshed {
        let result = self.inner.access.refresh(service, identity).await;
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
    pub async fn keep_current(&self, service: &Service, identity: &Identity) {
        let mut tick = 0u32;
        loop {
            if tick % 45 == 0 {
                self.refresh(service, identity).await;
            } else {
                self.apply().await;
            }
            tick = tick.wrapping_add(1);
            tokio::time::sleep(Duration::from_secs(20)).await;
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
