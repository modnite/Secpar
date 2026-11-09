//! One phone connection: TLS → WebSocket `/v1` (or a relayed connection) → pair|hello → message loop.

use super::guest::{self, Filtered, Link, OwnerWatch};
use super::{Ctx, Event, Out, Outbox, Session, MAX_FRAME};
use crate::proto::{ClientMsg, ServerMsg};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

/// Either half of a phone connection: a direct WebSocket or a relayed one.
pub trait Rx: Stream<Item = Message> + Unpin + Send {}
impl<T: Stream<Item = Message> + Unpin + Send> Rx for T {}
pub trait Tx: Sink<Message> + Unpin + Send {}
impl<T: Sink<Message> + Unpin + Send> Tx for T {}

async fn send(ws: &mut impl Tx, m: &ServerMsg) -> bool {
    ws.send(Message::text(m.json())).await.is_ok()
}

async fn fail(ws: &mut impl Tx, code: &str, msg: &str) {
    let _ = send(ws, &ServerMsg::error(code, msg, None)).await;
    let _ = ws.send(Message::Close(None)).await;
}

enum Frame {
    Text(String),
    Bin(Vec<u8>),
    Other, // ping/pong: activity only
}

// Async cancellation skips code after an await. Keep the connection registry
// consistent even when a relay task is aborted while its writer is blocked.
struct Registration(Arc<Ctx>, u64);
impl Drop for Registration {
    fn drop(&mut self) { self.0.unregister(self.1); }
}

/// The next frame, or None on close, error or `idle` silence.
async fn next_frame(ws: &mut impl Rx, idle: std::time::Duration) -> Option<Frame> {
    match timeout(idle, ws.next()).await {
        Ok(Some(Message::Text(t))) => Some(Frame::Text(t.to_string())),
        Ok(Some(Message::Binary(b))) => Some(Frame::Bin(b.to_vec())),
        Ok(Some(Message::Close(_))) | Ok(None) | Err(_) => None,
        Ok(Some(_)) => Some(Frame::Other),
    }
}

/// `remote` for a paired phone, when a Worker is configured.
fn remote(ctx: &Ctx, dev: &str) -> Option<ServerMsg> {
    let url = ctx.relay_url.as_deref()?;
    let ticket = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        ctx.store.lock().unwrap().ticket(dev),
    );
    Some(ServerMsg::Remote {
        url: url.into(),
        dev: dev.into(),
        ticket,
    })
}

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME))
}

pub(crate) async fn run(tcp: TcpStream, acceptor: tokio_rustls::TlsAcceptor, ctx: Arc<Ctx>) {
    let Ok(Ok(tls)) = timeout(ctx.idle, acceptor.accept(tcp)).await else {
        return;
    };
    #[allow(clippy::result_large_err)] // signature fixed by tungstenite
    let only_v1 = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if req.uri().path() == "/v1" {
            Ok(resp)
        } else {
            let mut e = ErrorResponse::new(None);
            *e.status_mut() = StatusCode::NOT_FOUND;
            Err(e)
        }
    };
    let Ok(Ok(ws)) = timeout(
        ctx.idle,
        tokio_tungstenite::accept_hdr_async_with_config(tls, only_v1, Some(ws_config())),
    )
    .await
    else {
        return;
    };
    let (tx, rx) = ws.split();
    // a read error ends the connection like a close
    let rx = rx
        .take_while(|r| std::future::ready(r.is_ok()))
        .filter_map(|r| std::future::ready(r.ok()));
    serve(rx, tx, ctx).await
}

/// The agent protocol over any connection; returns when it ends.
pub(crate) async fn serve(mut inc: impl Rx, mut ws: impl Tx, ctx: Arc<Ctx>) {
    let first = loop {
        match next_frame(&mut inc, ctx.idle).await {
            None => return,
            Some(Frame::Text(t)) => break serde_json::from_str::<ClientMsg>(&t).ok(),
            Some(Frame::Other) => continue,
            Some(Frame::Bin(_)) => break None,
        }
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut guest: Option<(Arc<guest::GuestAdmission>, guest::Pass)> = None;
    let device = match first {
        Some(ClientMsg::Pair { otp, device, .. }) => {
            let device: String = device.trim().chars().take(40).collect();
            let paired = ctx.store.lock().unwrap().pair_device(&otp, &device);
            let (id, token) = match paired {
                Ok(paired) => paired,
                Err("bad_otp") => return fail(&mut ws, "bad_otp", "pairing code is wrong or expired").await,
                Err(_) => return fail(&mut ws, "internal", "could not save the device").await,
            };
            let _ = ctx.events.send(Event::Paired { device: id.clone() });
            let pc = ctx.handlers.pc_info().await;
            if !send(&mut ws, &ServerMsg::Paired { token, pc }).await {
                return;
            }
            if let Some(m) = remote(&ctx, &id) {
                let _ = send(&mut ws, &m).await;
            }
            id
        }
        Some(ClientMsg::Hello { token, .. }) => {
            let auth = ctx.store.lock().unwrap().auth(&token);
            match auth {
                Ok(id) => {
                    let pc = ctx.handlers.pc_info().await;
                    if !send(
                        &mut ws,
                        &ServerMsg::Welcome {
                            v: 1,
                            pc,
                            caps: ctx.handlers.caps(),
                            locked: ctx.handlers.locked(),
                        },
                    )
                    .await
                    {
                        return;
                    }
                    if let Some(m) = remote(&ctx, &id) {
                        let _ = send(&mut ws, &m).await;
                    }
                    id
                }
                Err(code) => return fail(&mut ws, code, "this device is not paired").await,
            }
        }
        Some(ClientMsg::Guest { grant }) => match admit_guest(&ctx, &grant).await {
            Ok((adm, pass)) => {
                let pc = ctx.handlers.pc_info().await;
                let pc = crate::proto::PcInfo { macs: Vec::new(), user: String::new(), ..pc };
                let welcome = ServerMsg::Welcome {
                    v: 1,
                    pc,
                    caps: guest::caps(&ctx.handlers.caps(), &pass.gate),
                    locked: false,
                };
                if !send(&mut ws, &welcome).await {
                    pass.gate.close();
                    return;
                }
                guest = Some((adm, pass));
                String::new()
            }
            Err((code, msg)) => return fail(&mut ws, code, &msg).await,
        },
        _ => {
            return fail(
                &mut ws,
                "unauthorized",
                "first message must be pair, hello or guest",
            )
            .await
        }
    };

    let (bin_tx, mut bin_rx) = mpsc::channel(4);
    let outbox = Outbox { ctl: tx.clone(), bin: bin_tx };
    // A friend is never registered as an owner device and gets no migration session.
    let (mut session, mut gamestream, registration, link, gate) = match &guest {
        None => {
            let inner = ctx.handlers.session(&device, outbox.clone());
            let session: Box<dyn Session> = Box::new(OwnerWatch { inner, ctx: ctx.clone(), streaming: false });
            let gamestream = ctx.handlers.gamestream_session(&device, outbox);
            let sid = ctx.register(&device, tx);
            (session, gamestream, Some(Registration(ctx.clone(), sid)), None, None)
        }
        Some((adm, pass)) => {
            let Some(inner) = ctx.handlers.guest_session(&pass.gate, outbox) else {
                pass.gate.close();
                return fail(&mut ws, "guest_refused", "This PC does not accept friends yet.").await;
            };
            let session: Box<dyn Session> = Box::new(Filtered { inner, gate: pass.gate.clone(), ctx: ctx.clone() });
            let link = Link::start(ctx.clone(), adm.clone(), pass);
            (session, None, None, Some(link), Some(pass.gate.clone()))
        }
    };
    loop {
        tokio::select! {
            _ = async { gate.as_ref().unwrap().closed().await }, if gate.is_some() => {
                let reason = gate.as_ref().unwrap().reason();
                fail(&mut ws, "guest_ended", guest_reason(reason)).await;
                break;
            },
            f = next_frame(&mut inc, ctx.idle) => {
                let reply = match f {
                    None => break,
                    Some(Frame::Other) => None,
                    Some(Frame::Bin(b)) => session.on_binary(&b),
                    // Unknown types and malformed messages are ignored, per protocol.
                    Some(Frame::Text(t)) => match serde_json::from_str::<ClientMsg>(&t) {
                        Ok(ClientMsg::Ping) => Some(ServerMsg::Pong),
                        Ok(m @ (ClientMsg::Pair { .. } | ClientMsg::Hello { .. })) => {
                            Some(ServerMsg::error("bad_request", "already authenticated", Some(m.t())))
                        }
                        Ok(m @ (ClientMsg::GameStreamInfo { .. } | ClientMsg::GameStreamPair { .. })) => {
                            super::gamestream::dispatch(m, &mut gamestream)
                        }
                        Ok(m) => session.on_msg(m.sanitize()),
                        Err(_) => None,
                    },
                };
                if let Some(r) = reply {
                    if !send(&mut ws, &r).await { break }
                }
            },
            o = rx.recv() => match o {
                Some(Out::Msg(m)) => {
                    if gate.as_ref().is_some_and(|g| !guest::out_allowed(&m, g)) { continue }
                    if !send(&mut ws, &m).await { break }
                }
                Some(Out::Close(code)) => { fail(&mut ws, code, "this phone was removed on the PC").await; break }
                None => break,
            },
            Some(b) = bin_rx.recv() => {
                if gate.as_ref().is_some_and(|g| !g.has("files")) { continue }
                if ws.send(Message::binary(b)).await.is_err() { break }
            },
        }
    }
    drop(registration);
    drop(session); // releases held keys/buttons, stops subscriptions
    drop(link); // closes the gate, stops the lease watcher
    drop(gamestream); // cancels this authenticated device's pending engine grants
}

/// Every online and local check for a friend, then consumes the grant. Order matters: the grant
/// is spent only after the busy and lock checks, so a refused attempt can be retried.
async fn admit_guest(ctx: &Arc<Ctx>, grant: &str) -> Result<(Arc<guest::GuestAdmission>, guest::Pass), (&'static str, String)> {
    let refuse = |code, msg: &str| Err((code, msg.to_string()));
    let Some(adm) = ctx.handlers.guests() else {
        return refuse("guest_refused", "This PC does not accept friends yet.");
    };
    if let Some(why) = ctx.handlers.guest_refusal() {
        return refuse("guest_refused", why);
    }
    let pass = match adm.check(grant).await {
        Ok(pass) => pass,
        Err(e) => return refuse("guest_refused", &e),
    };
    if ctx.handlers.locked() {
        return refuse("locked", "This PC is locked.");
    }
    if ctx.guest.owner_streams.load(std::sync::atomic::Ordering::SeqCst) > 0 || ctx.guest.any() {
        return refuse("busy", "This PC is in use.");
    }
    if adm.consume(&pass).is_err() {
        return refuse("guest_refused", "This grant was already used.");
    }
    Ok((adm, pass))
}

fn guest_reason(reason: &str) -> &'static str {
    match reason {
        "owner" => "The owner ended this session.",
        "locked" => "This PC was locked.",
        "offline" => "Lost contact with Secpar accounts.",
        "revoked" => "Your access was changed or removed.",
        _ => "This session ended.",
    }
}

#[cfg(test)]
mod guest_tests {
    use super::*;
    use crate::agent::{BoxFut, Handlers};
    use crate::guest_grant::tests::{signed, COMPUTER};
    use crate::guest_policy::{admit, Gate, Lease, Roster};
    use crate::proto::PcInfo;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const GUEST: &str = "00000000-0000-4000-8000-0000000000aa";
    const REQUEST: &str = "00000000-0000-4000-8000-0000000000bb";

    struct Store;
    impl crate::PairingStore for Store {
        fn take_otp(&mut self, _: &str) -> bool { false }
        fn add_device(&mut self, _: &str) -> std::io::Result<(String, String)> { Err(std::io::Error::other("friends never pair")) }
        fn auth(&self, _: &str) -> Result<String, &'static str> { Err("unauthorized") }
        fn revoke(&mut self, _: &str) -> std::io::Result<()> { Ok(()) }
        fn ticket(&self, _: &str) -> [u8; 32] { [0; 32] }
    }
    struct Recorder(Arc<Mutex<Vec<&'static str>>>);
    impl Session for Recorder {
        fn on_msg(&mut self, m: ClientMsg) -> Option<ServerMsg> {
            self.0.lock().unwrap().push(m.t());
            None
        }
    }
    struct Host {
        admission: Arc<guest::GuestAdmission>,
        seen: Arc<Mutex<Vec<&'static str>>>,
        locked: AtomicBool,
        gates: Mutex<Vec<Gate>>,
    }
    impl Handlers for Host {
        fn pc_info(&self) -> BoxFut<PcInfo> {
            Box::pin(async { PcInfo { id: "pc".into(), name: "Studio".into(), user: "owner".into(), macs: vec!["aa".into()], os: "test".into() } })
        }
        fn caps(&self) -> Vec<String> {
            ["input", "power", "files", "clip", "stream", "unlock", "fsbrowse", "net"].map(String::from).to_vec()
        }
        fn session(&self, _: &str, _: Outbox) -> Box<dyn Session> {
            Box::new(Recorder(Default::default()))
        }
        fn locked(&self) -> bool {
            self.locked.load(Ordering::SeqCst)
        }
        fn guests(&self) -> Option<Arc<guest::GuestAdmission>> {
            Some(self.admission.clone())
        }
        fn guest_session(&self, gate: &Gate, _: Outbox) -> Option<Box<dyn Session>> {
            self.gates.lock().unwrap().push(gate.clone());
            Some(Box::new(Recorder(self.seen.clone())))
        }
    }

    struct Rig {
        host: Arc<Host>,
        ctx: Arc<Ctx>,
        server: MockServer,
    }
    async fn rig(input: &str, connect: &str, lease_active: bool) -> Rig {
        let server = MockServer::start().await;
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../cloud/accounts/test/fixtures/guest-grant.json")).unwrap();
        Mock::given(method("GET")).and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "keys": fixture["keys"] })))
            .mount(&server).await;
        friends(&server, input, connect).await;
        lease(&server, lease_active).await;
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let account = crate::account::Account::linked_for_test(&server.uri());
        let admission = guest::GuestAdmission::with_timing(account, Duration::from_millis(20), Duration::from_millis(60));
        let host = Arc::new(Host { admission, seen: Default::default(), locked: AtomicBool::new(false), gates: Default::default() });
        let ctx = Arc::new(Ctx {
            store: Arc::new(Mutex::new(Store)),
            relay_url: None,
            handlers: host.clone(),
            idle: Duration::from_secs(5),
            sessions: Mutex::default(),
            events: tokio::sync::broadcast::channel(32).0,
            next_id: AtomicU64::new(0),
            relay: Mutex::default(),
            guest: Default::default(),
        });
        Rig { host, ctx, server }
    }
    async fn friends(server: &MockServer, input: &str, connect: &str) {
        Mock::given(method("GET")).and(path("/v1/host/friends"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id":GUEST,"handle":"cat","input":input,"connect":connect}])))
            .mount(server).await;
    }
    async fn lease(server: &MockServer, active: bool) {
        Mock::given(method("GET")).and(path(format!("/v1/host/requests/{REQUEST}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(
                {"id":REQUEST,"guest":GUEST,"approved":["keyboard","view"],"state":"claimed","expires":1,"active":active})))
            .mount(server).await;
    }
    fn token() -> String {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let (token, _) = signed(|_, c| {
            c["sub"] = GUEST.into();
            c["jti"] = REQUEST.into();
            c["aud"] = COMPUTER.into();
            c["iat"] = now.into();
            c["exp"] = (now + 30).into();
            c["permissions"] = serde_json::json!(["keyboard", "view"]);
        });
        token
    }

    struct Conn {
        tx: mpsc::Sender<Message>,
        rx: mpsc::Receiver<Message>,
    }
    fn connect(ctx: &Arc<Ctx>) -> Conn {
        let (tx, mut in_rx) = mpsc::channel::<Message>(64);
        let (out_tx, rx) = mpsc::channel::<Message>(64);
        let incoming = futures_util::stream::poll_fn(move |cx| in_rx.poll_recv(cx));
        let writer = Box::pin(futures_util::sink::unfold(out_tx, |out, m| async move {
            out.send(m).await.map(|_| out).map_err(|_| ())
        }));
        tokio::spawn(serve(incoming, writer, ctx.clone()));
        Conn { tx, rx }
    }
    impl Conn {
        async fn say(&self, m: serde_json::Value) {
            self.tx.send(Message::text(m.to_string())).await.unwrap();
        }
        async fn next(&mut self) -> Option<serde_json::Value> {
            loop {
                match tokio::time::timeout(Duration::from_secs(3), self.rx.recv()).await {
                    Ok(Some(Message::Text(t))) => return serde_json::from_str(&t).ok(),
                    Ok(Some(_)) => continue,
                    _ => return None,
                }
            }
        }
    }
    fn start() -> ClientMsg {
        ClientMsg::StreamStart { w: 0, h: 0, fps: 0, codecs: vec![], max_kbps: 0, audio: false, cands: vec![], display: String::new() }
    }

    #[tokio::test]
    async fn a_friend_gets_a_filtered_session_and_nothing_an_owner_gets() {
        let r = rig("full", "always", true).await;
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":token()})).await;
        let welcome = c.next().await.unwrap();
        assert_eq!(welcome["t"], "welcome");
        assert_eq!(welcome["caps"], serde_json::json!(["input", "stream"]));
        assert_eq!(welcome["pc"]["user"], "");
        assert_eq!(welcome["pc"]["macs"], serde_json::json!([]));
        c.say(serde_json::json!({"t":"key","vk":65,"down":true})).await;
        c.say(serde_json::json!({"t":"power","a":"sleep"})).await;
        let denied = c.next().await.unwrap();
        assert_eq!((denied["t"].as_str(), denied["code"].as_str()), (Some("error"), Some("forbidden")));
        c.say(serde_json::json!({"t":"fs.list","path":"C:/"})).await;
        assert_eq!(c.next().await.unwrap()["code"], "forbidden");
        c.say(serde_json::json!({"t":"ping"})).await;
        assert_eq!(c.next().await.unwrap()["t"], "pong");
        assert_eq!(*r.host.seen.lock().unwrap(), vec!["key"]);
        assert!(r.ctx.guest.any());
    }

    #[tokio::test]
    async fn a_view_only_friend_cannot_type_even_with_a_signed_keyboard_grant() {
        let r = rig("view", "always", true).await;
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":token()})).await;
        assert_eq!(c.next().await.unwrap()["caps"], serde_json::json!(["stream"]));
        c.say(serde_json::json!({"t":"key","vk":65,"down":true})).await;
        c.say(serde_json::json!({"t":"ping"})).await;
        assert_eq!(c.next().await.unwrap()["t"], "pong");
        assert!(r.host.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_grant_works_once() {
        let r = rig("full", "always", true).await;
        let grant = token();
        let mut first = connect(&r.ctx);
        first.say(serde_json::json!({"t":"guest","grant":grant})).await;
        assert_eq!(first.next().await.unwrap()["t"], "welcome");
        drop(first);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut again = connect(&r.ctx);
        again.say(serde_json::json!({"t":"guest","grant":grant})).await;
        assert_eq!(again.next().await.unwrap()["code"], "guest_refused");
    }

    #[tokio::test]
    async fn a_locked_pc_or_a_second_friend_is_refused_and_the_grant_is_not_spent() {
        let r = rig("full", "always", true).await;
        r.host.locked.store(true, Ordering::SeqCst);
        let grant = token();
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":grant})).await;
        assert_eq!(c.next().await.unwrap()["code"], "locked");
        r.host.locked.store(false, Ordering::SeqCst);
        let mut again = connect(&r.ctx);
        again.say(serde_json::json!({"t":"guest","grant":grant})).await;
        assert_eq!(again.next().await.unwrap()["t"], "welcome");
        let mut second = connect(&r.ctx);
        second.say(serde_json::json!({"t":"guest","grant":token()})).await;
        assert_eq!(second.next().await.unwrap()["code"], "busy");
    }

    #[tokio::test]
    async fn an_inactive_lease_or_a_garbage_grant_never_starts_a_session() {
        let r = rig("full", "always", false).await;
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":token()})).await;
        assert_eq!(c.next().await.unwrap()["code"], "guest_refused");
        assert!(!r.ctx.guest.any());
        let mut bad = connect(&r.ctx);
        bad.say(serde_json::json!({"t":"guest","grant":"not.a.token"})).await;
        assert_eq!(bad.next().await.unwrap()["code"], "guest_refused");
    }

    #[tokio::test]
    async fn revoking_the_lease_ends_a_running_session_and_closes_its_gate() {
        let r = rig("full", "always", true).await;
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":token()})).await;
        assert_eq!(c.next().await.unwrap()["t"], "welcome");
        r.server.reset().await;
        Mock::given(method("GET")).and(path("/v1/host/friends"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([]))).mount(&r.server).await;
        lease(&r.server, true).await;
        let end = c.next().await.unwrap();
        assert_eq!((end["code"].as_str(), end["msg"].as_str()), (Some("guest_ended"), Some("Your access was changed or removed.")));
        assert!(!r.host.gates.lock().unwrap()[0].is_open());
    }

    #[tokio::test]
    async fn lowering_a_friend_to_view_only_stops_typing_without_ending_the_session() {
        let r = rig("full", "always", true).await;
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":token()})).await;
        c.next().await.unwrap();
        r.server.reset().await;
        friends(&r.server, "view", "always").await;
        lease(&r.server, true).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        c.say(serde_json::json!({"t":"key","vk":65,"down":true})).await;
        c.say(serde_json::json!({"t":"ping"})).await;
        assert_eq!(c.next().await.unwrap()["t"], "pong");
        assert!(r.host.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn locking_the_pc_ends_a_running_session() {
        let r = rig("full", "always", true).await;
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":token()})).await;
        c.next().await.unwrap();
        r.host.locked.store(true, Ordering::SeqCst);
        assert_eq!(c.next().await.unwrap()["msg"], "This PC was locked.");
    }

    #[tokio::test]
    async fn losing_contact_with_accounts_ends_a_running_session() {
        let r = rig("full", "always", true).await;
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":token()})).await;
        c.next().await.unwrap();
        r.server.reset().await;
        Mock::given(method("GET")).and(path("/v1/host/friends")).respond_with(ResponseTemplate::new(503)).mount(&r.server).await;
        assert_eq!(c.next().await.unwrap()["msg"], "Lost contact with Secpar accounts.");
    }

    #[tokio::test]
    async fn the_owner_streaming_closes_a_friend_and_a_friend_cannot_start_over_the_owner() {
        let r = rig("full", "always", true).await;
        let mut c = connect(&r.ctx);
        c.say(serde_json::json!({"t":"guest","grant":token()})).await;
        c.next().await.unwrap();
        let mut owner = OwnerWatch { inner: Box::new(Recorder(Default::default())), ctx: r.ctx.clone(), streaming: false };
        owner.on_msg(start());
        assert_eq!(c.next().await.unwrap()["msg"], "The owner ended this session.");
        let set: std::collections::BTreeSet<String> = ["view".to_string()].into();
        let roster = Roster::parse(&serde_json::json!([{"id":"x","handle":"x"}])).unwrap();
        let lease = Lease { guest: "x".into(), approved: set.clone(), active: true };
        let gate = admit(&roster, &lease, &set, "x").unwrap().gate().clone();
        let mut f = Filtered { inner: Box::new(Recorder(Default::default())), gate, ctx: r.ctx.clone() };
        assert_eq!(f.on_msg(start()), Some(ServerMsg::error("busy", "The owner is using this PC.", Some("stream.start"))));
        drop(owner);
        assert_eq!(r.ctx.guest.owner_streams.load(Ordering::SeqCst), 0);
    }
}
