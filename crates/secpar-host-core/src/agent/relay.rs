//! Platform-neutral Worker multiplexing. Sessions still use the agent's own auth.
//! Bounded queues and cancellation keep a stalled relay from retaining sessions.
use super::{revoke_envelope, AgentHandle, MAX_FRAME};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, OwnedSemaphorePermit, Semaphore},
    task::{AbortHandle, JoinSet},
};
use tokio_tungstenite::tungstenite::Message;

const MAX_PEERS: usize = 16;
const QUEUED_BYTES: usize = 16 << 20;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

struct Queued {
    message: Message,
    _permit: OwnedSemaphorePermit,
}
fn queued(message: Message, budget: &Arc<Semaphore>) -> Option<Queued> {
    if message.len() > MAX_FRAME + 64 {
        return None;
    }
    let permit = budget
        .clone()
        .try_acquire_many_owned(message.len().max(1).try_into().ok()?)
        .ok()?;
    Some(Queued {
        message,
        _permit: permit,
    })
}
struct Peer {
    input: mpsc::Sender<Queued>,
    abort: AbortHandle,
    generation: u64,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
struct RelayRegistration(AgentHandle);
impl Drop for RelayRegistration {
    fn drop(&mut self) {
        self.0.set_relay(None);
    }
}

#[derive(Debug, PartialEq)]
enum Incoming {
    Open(u32),
    Close(u32),
    Data(u32, Message),
}
fn parse(message: Message) -> Option<Incoming> {
    #[derive(Deserialize)]
    struct Envelope {
        c: u32,
        o: Option<String>,
        m: Option<String>,
    }
    match message {
        Message::Binary(bytes) if bytes.len() >= 4 => Some(Incoming::Data(
            u32::from_be_bytes(bytes[..4].try_into().ok()?),
            Message::Binary(bytes.slice(4..)),
        )),
        Message::Text(text) => {
            let e: Envelope = serde_json::from_str(&text).ok()?;
            match (e.o.as_deref(), e.m) {
                (Some("open"), None) => Some(Incoming::Open(e.c)),
                (Some("close"), None) => Some(Incoming::Close(e.c)),
                (None, Some(text)) => Some(Incoming::Data(e.c, Message::text(text))),
                _ => None,
            }
        }
        _ => None,
    }
}
fn close(cid: u32) -> Message {
    Message::text(serde_json::json!({"c":cid,"o":"close"}).to_string())
}
fn envelope(cid: u32, message: Message) -> Option<Message> {
    match message {
        Message::Text(text) => Some(Message::text(
            serde_json::json!({"c":cid,"m":text.as_str()}).to_string(),
        )),
        Message::Binary(bytes) => {
            let mut data = Vec::with_capacity(4 + bytes.len());
            data.extend_from_slice(&cid.to_be_bytes());
            data.extend_from_slice(&bytes);
            Some(Message::binary(data))
        }
        _ => None,
    }
}
async fn write(writer: &mut (impl Sink<Message> + Unpin), message: Message) -> bool {
    matches!(
        tokio::time::timeout(WRITE_TIMEOUT, writer.send(message)).await,
        Ok(Ok(()))
    )
}

/// Serve one already-authenticated host WebSocket. The platform owns TLS,
/// registration/reconnect policy and persistent revoked ids. Cancellation drops
/// the registration and aborts every child agent session, releasing its inputs.
pub async fn serve<E>(
    agent: AgentHandle,
    mut incoming: impl Stream<Item = Result<Message, E>> + Unpin,
    mut writer: impl Sink<Message> + Unpin,
    revoked: Vec<String>,
) {
    let (control, mut control_rx) = mpsc::channel(32);
    agent.set_relay(Some(control.clone()));
    let _registration = RelayRegistration(agent.clone());
    for device in revoked {
        if !write(&mut writer, revoke_envelope(&device)).await {
            return;
        }
    }
    let (out, mut out_rx) = mpsc::channel::<Queued>(32);
    let input_budget = Arc::new(Semaphore::new(QUEUED_BYTES));
    let output_budget = Arc::new(Semaphore::new(QUEUED_BYTES));
    let mut peers = HashMap::<u32, Peer>::new();
    let mut tasks = JoinSet::new();
    let mut generation = 0u64;
    let mut ping = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(60),
        Duration::from_secs(60),
    );
    let mut last_rx = tokio::time::Instant::now();
    loop {
        tokio::select! {
            message = incoming.next() => {
                let Some(Ok(message)) = message else { break; };
                if message.len() > MAX_FRAME+64 || matches!(message,Message::Close(_)) { break; }
                last_rx = tokio::time::Instant::now();
                match parse(message) {
                    Some(Incoming::Open(cid)) => {
                        if peers.contains_key(&cid) || peers.len() >= MAX_PEERS {
                            peers.remove(&cid);
                            if !write(&mut writer,close(cid)).await { break; }
                            continue;
                        }
                        let (tx,mut rx) = mpsc::channel::<Queued>(8);
                        generation = generation.wrapping_add(1);
                        let current = generation;
                        let (agent,out,budget) = (agent.clone(),out.clone(),output_budget.clone());
                        let abort = tasks.spawn(async move {
                            let close_out = out.clone();
                            let close_budget = budget.clone();
                            let input = futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)).map(|q|q.message);
                            let output = Box::pin(futures_util::sink::unfold(out,move |out,message| {
                                let budget = budget.clone();
                                async move {
                                    if let Some(message) = envelope(cid,message) {
                                        let value = queued(message,&budget).ok_or(())?;
                                        out.send(value).await.map_err(|_|())?;
                                    }
                                    Ok::<_,()>(out)
                                }
                            }));
                            agent.serve(input,output).await;
                            // Keep close behind the session's queued replies; a
                            // separate control queue could overtake an auth error.
                            let message = close(cid);
                            if let Ok(permit) = close_budget.acquire_many_owned(message.len() as u32).await {
                                let _ = close_out.send(Queued {message,_permit:permit}).await;
                            }
                            (cid,current)
                        });
                        peers.insert(cid,Peer {input:tx,abort,generation:current});
                    }
                    Some(Incoming::Close(cid)) => { peers.remove(&cid); }
                    Some(Incoming::Data(cid,message)) => {
                        let Some(peer) = peers.get(&cid) else { continue; };
                        let accepted = queued(message,&input_budget).is_some_and(|q|peer.input.try_send(q).is_ok());
                        if !accepted {
                            peers.remove(&cid);
                            if !write(&mut writer,close(cid)).await { break; }
                        }
                    }
                    None => {} // Includes the Worker's plain "pong".
                }
            }
            Some(message) = control_rx.recv() => if !write(&mut writer,message).await { break; },
            Some(value) = out_rx.recv() => if !write(&mut writer,value.message).await { break; },
            Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                if let Ok((cid,current)) = done {
                    if peers.get(&cid).is_some_and(|p|p.generation==current) { peers.remove(&cid); }
                }
            }
            _ = ping.tick() => {
                if last_rx.elapsed()>Duration::from_secs(150) || !write(&mut writer,Message::text("ping")).await { break; }
            }
        }
    }
    // JoinSet also aborts on cancellation, including while the socket writer stalls.
    peers.clear();
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::super::{BoxFut, Ctx, Handlers, Outbox, Session};
    use super::*;
    use crate::{
        proto::{ClientMsg, PcInfo, ServerMsg},
        PairingStore,
    };
    use std::sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Mutex,
    };
    use tokio::sync::broadcast;

    struct Store {
        revoked: bool,
    }
    impl PairingStore for Store {
        fn take_otp(&mut self, _: &str) -> bool {
            false
        }
        fn add_device(&mut self, _: &str) -> std::io::Result<(String, String)> {
            unreachable!()
        }
        fn auth(&self, token: &str) -> Result<String, &'static str> {
            match token {
                "one" if !self.revoked => Ok("dev1".into()),
                "two" => Ok("dev2".into()),
                _ => Err("unauthorized"),
            }
        }
        fn revoke(&mut self, id: &str) -> std::io::Result<()> {
            if id == "dev1" {
                self.revoked = true;
            }
            Ok(())
        }
        fn ticket(&self, _: &str) -> [u8; 32] {
            [0; 32]
        }
    }
    struct Sessions(Arc<AtomicUsize>);
    impl Handlers for Sessions {
        fn pc_info(&self) -> BoxFut<PcInfo> {
            Box::pin(async {
                PcInfo {
                    id: "fixture".into(),
                    name: "Fixture".into(),
                    user: String::new(),
                    macs: vec![],
                    os: "test".into(),
                }
            })
        }
        fn caps(&self) -> Vec<String> {
            vec!["input".into()]
        }
        fn session(&self, _: &str, _: Outbox) -> Box<dyn Session> {
            Box::new(HeldInput(self.0.clone()))
        }
    }
    struct HeldInput(Arc<AtomicUsize>);
    impl Session for HeldInput {
        fn on_msg(&mut self, _: ClientMsg) -> Option<ServerMsg> {
            None
        }
    }
    impl Drop for HeldInput {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct Fixture {
        agent: AgentHandle,
        input: mpsc::Sender<Message>,
        output: mpsc::Receiver<Message>,
        task: tokio::task::JoinHandle<()>,
        dropped: Arc<AtomicUsize>,
    }
    fn fixture(output_capacity: usize) -> Fixture {
        let dropped = Arc::new(AtomicUsize::new(0));
        let agent = AgentHandle {
            addr: "127.0.0.1:0".parse().unwrap(),
            ctx: Arc::new(Ctx {
                store: Arc::new(Mutex::new(Store { revoked: false })),
                relay_url: None,
                handlers: Arc::new(Sessions(dropped.clone())),
                idle: Duration::from_secs(60),
                sessions: Mutex::default(),
                events: broadcast::channel(32).0,
                next_id: AtomicU64::new(0),
                relay: Mutex::default(),
            }),
        };
        let (input, mut rx) = mpsc::channel(128);
        let (tx, output) = mpsc::channel(output_capacity);
        let incoming =
            futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)).map(Ok::<Message, ()>);
        let writer = Box::pin(futures_util::sink::unfold(tx, |tx, message| async move {
            tx.send(message).await.map(|_| tx)
        }));
        let task = tokio::spawn(serve(agent.clone(), incoming, writer, vec![]));
        Fixture {
            agent,
            input,
            output,
            task,
            dropped,
        }
    }
    async fn connect(f: &Fixture, cid: u32, token: &str) {
        f.input
            .send(Message::text(
                serde_json::json!({"c":cid,"o":"open"}).to_string(),
            ))
            .await
            .unwrap();
        let hello =
            serde_json::json!({"t":"hello","v":1,"token":token,"device":"Fixture"}).to_string();
        f.input
            .send(envelope(cid, Message::text(hello)).unwrap())
            .await
            .unwrap();
    }
    async fn until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !condition() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Relay lifecycle must settle");
    }
    async fn next(f: &mut Fixture) -> Message {
        tokio::time::timeout(Duration::from_secs(2), f.output.recv())
            .await
            .unwrap()
            .unwrap()
    }
    #[test]
    fn worker_framing_and_global_queue_budget_are_bounded() {
        assert_eq!(
            parse(Message::text(r#"{"c":4,"o":"open"}"#)),
            Some(Incoming::Open(4))
        );
        assert_eq!(
            parse(Message::text(r#"{"c":4,"o":"open","m":"bad"}"#)),
            None
        );
        assert_eq!(parse(Message::binary(vec![0, 0, 0])), None);
        assert_eq!(
            parse(envelope(258, Message::binary(vec![9, 8])).unwrap()),
            Some(Incoming::Data(258, Message::binary(vec![9, 8])))
        );
        let budget = Arc::new(Semaphore::new(8));
        let held = queued(Message::binary(vec![0; 8]), &budget).unwrap();
        assert!(queued(Message::binary(vec![0]), &budget).is_none());
        drop(held);
        assert!(queued(Message::binary(vec![0; 8]), &budget).is_some());
    }
    #[tokio::test]
    async fn real_agent_auth_revocation_and_cancellation_are_isolated() {
        let mut f = fixture(32);
        connect(&f, 1, "bad").await;
        let message = next(&mut f).await;
        let Some(Incoming::Data(1, Message::Text(text))) = parse(message) else {
            panic!("Expected authentication refusal")
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["code"],
            "unauthorized"
        );
        until(|| f.agent.connected().is_empty()).await;
        connect(&f, 2, "one").await;
        connect(&f, 3, "two").await;
        until(|| f.agent.connected().len() == 2).await;
        f.agent.revoke("dev1");
        until(|| {
            f.agent.connected() == vec!["dev2".to_string()] && f.dropped.load(Ordering::SeqCst) == 1
        })
        .await;
        f.task.abort();
        let _ = f.task.await;
        until(|| f.agent.connected().is_empty() && f.dropped.load(Ordering::SeqCst) == 2).await;
        assert!(f.agent.ctx.relay.lock().unwrap().is_none());
    }
    #[tokio::test]
    async fn stalled_socket_cancellation_releases_registered_sessions() {
        let f = fixture(1);
        connect(&f, 1, "one").await;
        connect(&f, 2, "two").await;
        until(|| f.agent.connected().len() == 2).await;
        // One welcome fills the socket, the other stalls its writer. Cancellation
        // must still drop both actual sessions and their connection registrations.
        f.task.abort();
        let _ = f.task.await;
        until(|| f.agent.connected().is_empty() && f.dropped.load(Ordering::SeqCst) == 2).await;
    }
    #[tokio::test]
    async fn pending_auth_connections_have_a_hard_limit() {
        let mut f = fixture(32);
        for cid in 0..=MAX_PEERS as u32 {
            f.input
                .send(Message::text(
                    serde_json::json!({"c":cid,"o":"open"}).to_string(),
                ))
                .await
                .unwrap();
        }
        assert_eq!(next(&mut f).await, close(MAX_PEERS as u32));
        assert!(f.agent.connected().is_empty());
        f.task.abort();
        let _ = f.task.await;
    }
    #[tokio::test]
    async fn worker_disconnect_releases_sessions_and_relay_registration() {
        let f = fixture(32);
        connect(&f, 1, "one").await;
        until(|| f.agent.connected().len() == 1).await;
        drop(f.input);
        tokio::time::timeout(Duration::from_secs(2), f.task)
            .await
            .unwrap()
            .unwrap();
        assert!(f.agent.connected().is_empty());
        assert_eq!(f.dropped.load(Ordering::SeqCst), 1);
        assert!(f.agent.ctx.relay.lock().unwrap().is_none());
    }
}
