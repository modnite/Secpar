//! TLS WebSocket agent on 48999 (docs/protocol.md).

mod session;
mod gamestream;
pub mod guest;
pub mod engine;
pub mod relay;
pub use guest::{GuestAdmission, Pass};
pub use session::{Rx, Tx};

use crate::proto::{ClientMsg, PcInfo, ServerMsg};
use crate::{IdentitySource, PairingStore};
type DeviceId = String;

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio_rustls::rustls::{self, pki_types::PrivatePkcs8KeyDer};

pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Largest WebSocket message accepted: an 8 MB clipboard image as base64, plus slack.
pub const MAX_FRAME: usize = 12 << 20;

/// One authenticated phone connection's state on the host side. Dropping it ends
/// everything the session started (held keys, subscriptions, pads, transfers).
pub trait Session: Send {
    /// An authenticated, sanitised message other than ping/pair/hello.
    fn on_msg(&mut self, m: ClientMsg) -> Option<ServerMsg>;
    /// A binary frame (file chunk).
    fn on_binary(&mut self, _b: &[u8]) -> Option<ServerMsg> {
        None
    }
}

/// Lets a session push to its phone at any time. Binary frames go through a small
/// bounded queue, so a fast producer (a file upload) waits for the socket.
#[derive(Clone)]
pub struct Outbox {
    ctl: mpsc::UnboundedSender<Out>,
    bin: mpsc::Sender<Vec<u8>>,
}

impl Outbox {
    /// False once the session is gone.
    pub fn send(&self, m: ServerMsg) -> bool {
        self.ctl.send(Out::Msg(m)).is_ok()
    }
    pub async fn send_bin(&self, b: Vec<u8>) -> bool {
        self.bin.send(b).await.is_ok()
    }
    /// For blocking threads (file upload).
    pub fn send_bin_blocking(&self, b: Vec<u8>) -> bool {
        self.bin.blocking_send(b).is_ok()
    }
    pub fn is_closed(&self) -> bool {
        self.ctl.is_closed()
    }

    /// An outbox with no socket behind it, and the end that reads what a session sent. For tests
    /// of `Session` implementations in other crates.
    pub fn pair() -> (Outbox, OutboxRx) {
        let (ctl, ctl_rx) = mpsc::unbounded_channel();
        let (bin, bin_rx) = mpsc::channel(64);
        (Outbox { ctl, bin }, OutboxRx { ctl: ctl_rx, bin: bin_rx })
    }
}

/// What a test `Outbox` received so far (see `Outbox::pair`).
pub struct OutboxRx {
    ctl: mpsc::UnboundedReceiver<Out>,
    bin: mpsc::Receiver<Vec<u8>>,
}

impl OutboxRx {
    /// The next control message, if one is waiting.
    pub fn msg(&mut self) -> Option<ServerMsg> {
        loop {
            match self.ctl.try_recv().ok()? {
                Out::Msg(m) => return Some(m),
                Out::Close(_) => continue,
            }
        }
    }
    /// The next binary frame, if one is waiting.
    pub fn bin(&mut self) -> Option<Vec<u8>> {
        self.bin.try_recv().ok()
    }
}

/// What the agent needs from the rest of the host.
pub trait Handlers: Send + Sync + 'static {
    fn pc_info(&self) -> BoxFut<PcInfo>;
    /// Capabilities for `welcome`.
    fn caps(&self) -> Vec<String>;
    fn session(&self, device: &str, out: Outbox) -> Box<dyn Session>;
    /// Only called AFTER owner-device authentication. The default keeps migration
    /// unavailable; platforms must complete certificate binding/revocation first.
    fn gamestream_session(&self, _device: &str, _out: Outbox) -> Option<Box<dyn Session>> { None }
    /// For `welcome.locked`.
    fn locked(&self) -> bool {
        false
    }
    /// Friend sessions. None (the default) refuses every guest.
    fn guests(&self) -> Option<Arc<GuestAdmission>> {
        None
    }
    /// A local reason to refuse friends right now (sharing switched off), as a short message.
    fn guest_refusal(&self) -> Option<&'static str> {
        None
    }
    /// The platform session for a friend. The agent filters it by `gate`; the platform also
    /// hands `gate` to its stream and UDP input paths.
    fn guest_session(&self, _gate: &crate::guest_policy::Gate, _out: Outbox) -> Option<Box<dyn Session>> {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Paired { device: DeviceId },
    Connected { device: DeviceId },
    Disconnected { device: DeviceId },
    GuestConnected { guest: String, handle: String },
    GuestDisconnected { guest: String },
}

pub(crate) enum Out {
    Msg(ServerMsg),
    Close(&'static str),
}

pub(crate) struct Ctx {
    pub store: Arc<Mutex<dyn PairingStore>>,
    pub relay_url: Option<String>,
    pub handlers: Arc<dyn Handlers>,
    pub idle: Duration,
    pub sessions: Mutex<HashMap<u64, (DeviceId, mpsc::UnboundedSender<Out>)>>,
    pub events: broadcast::Sender<Event>,
    next_id: AtomicU64,
    /// The rendezvous socket's outbox while it's connected.
    relay: Mutex<Option<mpsc::Sender<tokio_tungstenite::tungstenite::Message>>>,
    pub guest: guest::GuestState,
}

impl Ctx {
    pub fn register(&self, dev: &DeviceId, tx: mpsc::UnboundedSender<Out>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.sessions.lock().unwrap().insert(id, (dev.clone(), tx));
        let _ = self.events.send(Event::Connected {
            device: dev.clone(),
        });
        id
    }

    pub fn unregister(&self, id: u64) {
        if let Some((dev, _)) = self.sessions.lock().unwrap().remove(&id) {
            let _ = self.events.send(Event::Disconnected { device: dev });
        }
    }
}

#[derive(Clone)]
pub struct AgentHandle {
    pub addr: SocketAddr,
    ctx: Arc<Ctx>,
}

impl AgentHandle {
    /// Revokes the device and closes only its sessions with `revoked`. The Worker hears it now,
    /// or on its next connection.
    pub fn revoke(&self, device: &str) {
        let _ = self.ctx.store.lock().unwrap().revoke(device);
        if let Some(tx) = self.ctx.relay.lock().unwrap().as_ref() {
            let _ = tx.try_send(revoke_envelope(device));
        }
        for (dev, tx) in self.ctx.sessions.lock().unwrap().values() {
            if dev == device {
                let _ = tx.send(Out::Close("revoked"));
            }
        }
    }

    /// Account ids of friends with a running session.
    pub fn guests(&self) -> Vec<String> {
        self.ctx.guest.ids()
    }

    /// Ends a friend's session now.
    pub fn disconnect_guest(&self, guest: &str) {
        self.ctx.guest.close_one(guest, "owner");
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.ctx.events.subscribe()
    }

    /// Sends to every open session of every phone.
    pub fn broadcast(&self, m: ServerMsg) {
        for (_, tx) in self.ctx.sessions.lock().unwrap().values() {
            let _ = tx.send(Out::Msg(m.clone()));
        }
    }

    /// Sends to every open session of `device`; returns how many got it.
    pub fn send(&self, device: &str, m: ServerMsg) -> usize {
        let sessions = self.ctx.sessions.lock().unwrap();
        sessions
            .values()
            .filter(|(d, tx)| d == device && tx.send(Out::Msg(m.clone())).is_ok())
            .count()
    }

    pub fn set_relay(&self, tx: Option<mpsc::Sender<tokio_tungstenite::tungstenite::Message>>) {
        *self.ctx.relay.lock().unwrap() = tx;
    }

    /// Runs the agent protocol over a relayed connection, exactly as over a direct one.
    pub async fn serve(&self, rx: impl session::Rx, tx: impl session::Tx) {
        session::serve(rx, tx, self.ctx.clone()).await
    }

    /// Device ids with an open session (one entry per session).
    pub fn connected(&self) -> Vec<DeviceId> {
        self.ctx
            .sessions
            .lock()
            .unwrap()
            .values()
            .map(|(d, _)| d.clone())
            .collect()
    }
}

pub struct Agent;

impl Agent {
    pub async fn start(
        identity: &dyn IdentitySource,
        store: Arc<Mutex<dyn PairingStore>>,
        handlers: Arc<dyn Handlers>,
        bind: SocketAddr,
        idle: Duration,
        relay_url: Option<String>,
    ) -> std::io::Result<AgentHandle> {
        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(std::io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.certificate().to_vec().into()],
            PrivatePkcs8KeyDer::from(identity.private_key().to_vec()).into(),
        )
        .map_err(std::io::Error::other)?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener = tokio::net::TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let ctx = Arc::new(Ctx {
            store,
            relay_url,
            handlers,
            idle,
            sessions: Mutex::default(),
            events: broadcast::channel(32).0,
            next_id: AtomicU64::new(0),
            relay: Mutex::default(),
            guest: Default::default(),
        });
        let c = ctx.clone();
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    continue;
                };
                let _ = tcp.set_nodelay(true);
                tokio::spawn(session::run(tcp, acceptor.clone(), c.clone()));
            }
        });
        Ok(AgentHandle { addr, ctx })
    }
}

/// Existing rendezvous envelope; no platform-specific host state is required.
pub fn revoke_envelope(device: &str) -> tokio_tungstenite::tungstenite::Message {
    tokio_tungstenite::tungstenite::Message::text(
        serde_json::json!({"o":"revoke","dev":device}).to_string(),
    )
}

#[cfg(test)]
mod outbox_tests {
    use super::*;

    #[tokio::test]
    async fn test_outbox_delivers_messages_and_frames_in_order_and_reports_a_closed_peer() {
        let (out, mut rx) = Outbox::pair();
        assert!(rx.msg().is_none() && rx.bin().is_none());
        assert!(out.send(ServerMsg::Pong));
        assert!(out.send(ServerMsg::Ok { of: "x".into() }));
        assert!(out.send_bin(vec![2, 1]).await);
        assert_eq!(rx.msg(), Some(ServerMsg::Pong));
        assert_eq!(rx.msg(), Some(ServerMsg::Ok { of: "x".into() }));
        assert_eq!(rx.bin(), Some(vec![2, 1]));
        assert!(!out.is_closed());
        drop(rx);
        assert!(out.is_closed());
        assert!(!out.send(ServerMsg::Pong));
    }
}
