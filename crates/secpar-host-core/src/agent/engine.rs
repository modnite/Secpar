//! Authenticated-device adapter between the agent and an upstream streaming
//! engine controller. The agent has already authenticated `device`; this layer
//! scopes every grant to it, never exposes a generic engine API and lets in-flight
//! work finish its journal write even if the phone disconnects.
use super::{BoxFut, Outbox, Session};
use crate::proto::{ClientMsg, ServerMsg};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineInfo {
    pub http_port: u16,
    pub https_port: u16,
    pub certificate_sha256: String,
}

/// Implemented by the platform glue around the engine controller.
pub trait EngineBackend: Send + Sync + 'static {
    fn info(&self) -> EngineInfo;
    /// Approve exactly this client certificate for `device` (the authenticated Secpar
    /// device) under `owner` (the computer's account).
    fn approve(
        &self,
        owner: &str,
        device: &str,
        client_id: &str,
        certificate_sha256: &str,
        pin: &str,
    ) -> BoxFut<Result<(), &'static str>>;
    /// Durable per-device revocation; returns how many client grants were removed.
    fn revoke(&self, owner: &str, device: &str) -> BoxFut<Result<usize, &'static str>>;
}

#[derive(Clone)]
pub struct Broker {
    backend: Arc<dyn EngineBackend>,
    owner: Arc<str>,
}

impl Broker {
    pub fn new(backend: Arc<dyn EngineBackend>, owner: &str) -> Self {
        Self {
            backend,
            owner: owner.into(),
        }
    }
    /// For `Handlers::gamestream_session`, called only after owner-device authentication.
    pub fn session(&self, device: &str, out: Outbox) -> Box<dyn Session> {
        Box::new(EngineSession {
            broker: self.clone(),
            device: device.into(),
            out,
            cancelled: Arc::new(AtomicBool::new(false)),
            busy: Arc::new(AtomicBool::new(false)),
        })
    }
    /// For the platform's device-removal path. Call it together with `AgentHandle::revoke`.
    pub async fn revoke_device(&self, device: &str) -> Result<usize, &'static str> {
        self.backend.revoke(&self.owner, device).await
    }
}

struct EngineSession {
    broker: Broker,
    device: String,
    out: Outbox,
    cancelled: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}

fn error(id: &str, code: &str, msg: &str) -> ServerMsg {
    ServerMsg::GameStreamError {
        id: id.into(),
        code: code.into(),
        msg: msg.into(),
    }
}

impl Session for EngineSession {
    fn on_msg(&mut self, m: ClientMsg) -> Option<ServerMsg> {
        match m {
            ClientMsg::GameStreamInfo { id } => {
                let info = self.broker.backend.info();
                Some(ServerMsg::GameStreamInfo {
                    id,
                    http_port: info.http_port,
                    https_port: info.https_port,
                    certificate_sha256: info.certificate_sha256,
                })
            }
            ClientMsg::GameStreamPair {
                id,
                client_id,
                certificate_sha256,
                pin,
            } => {
                // One enrollment at a time per device: a second grant cannot
                // overwrite or race the first.
                if self.busy.swap(true, Ordering::SeqCst) {
                    return Some(error(&id, "busy", "A pairing is already in progress"));
                }
                if self.cancelled.load(Ordering::SeqCst) {
                    return None;
                }
                let broker = self.broker.clone();
                let device = self.device.clone();
                let out = self.out.clone();
                let busy = self.busy.clone();
                tokio::spawn(async move {
                    let result = broker
                        .backend
                        .approve(&broker.owner, &device, &client_id, &certificate_sha256, &pin)
                        .await;
                    busy.store(false, Ordering::SeqCst);
                    out.send(match result {
                        Ok(()) => ServerMsg::GameStreamPaired { id },
                        Err(_) => error(&id, "pair_failed", "Pairing did not complete"),
                    });
                });
                None
            }
            _ => None,
        }
    }
}

impl Drop for EngineSession {
    fn drop(&mut self) {
        // New requests stop here. An approval already sent to the engine still
        // completes and is journaled, so no certificate is trusted without a record.
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        approvals: Mutex<Vec<(String, String, String, String)>>,
        revocations: Mutex<Vec<(String, String)>>,
        fail: AtomicBool,
    }
    impl EngineBackend for Fake {
        fn info(&self) -> EngineInfo {
            EngineInfo {
                http_port: 1,
                https_port: 2,
                certificate_sha256: "a".repeat(64),
            }
        }
        fn approve(&self, o: &str, d: &str, c: &str, f: &str, _p: &str) -> BoxFut<Result<(), &'static str>> {
            self.approvals
                .lock()
                .unwrap()
                .push((o.into(), d.into(), c.into(), f.into()));
            let fail = self.fail.load(Ordering::SeqCst);
            Box::pin(async move { if fail { Err("no") } else { Ok(()) } })
        }
        fn revoke(&self, o: &str, d: &str) -> BoxFut<Result<usize, &'static str>> {
            self.revocations.lock().unwrap().push((o.into(), d.into()));
            Box::pin(async { Ok(1) })
        }
    }

    fn outbox() -> (Outbox, tokio::sync::mpsc::UnboundedReceiver<super::super::Out>) {
        let (ctl, rx) = tokio::sync::mpsc::unbounded_channel();
        let (bin, _) = tokio::sync::mpsc::channel(1);
        (Outbox { ctl, bin }, rx)
    }
    fn pair(id: &str) -> ClientMsg {
        ClientMsg::GameStreamPair {
            id: id.into(),
            client_id: "cid".into(),
            certificate_sha256: "b".repeat(64),
            pin: "1234".into(),
        }
    }

    #[tokio::test]
    async fn grants_use_the_authenticated_device_and_report_the_result() {
        let fake = Arc::new(Fake::default());
        let broker = Broker::new(fake.clone(), "acct");
        let (out, mut rx) = outbox();
        let mut session = broker.session("device-1", out);
        assert!(matches!(
            session.on_msg(ClientMsg::GameStreamInfo { id: "i".into() }),
            Some(ServerMsg::GameStreamInfo { http_port: 1, .. })
        ));
        assert!(session.on_msg(pair("p1")).is_none());
        match rx.recv().await.unwrap() {
            super::super::Out::Msg(ServerMsg::GameStreamPaired { id }) => assert_eq!(id, "p1"),
            _ => panic!("expected paired"),
        }
        let approvals = fake.approvals.lock().unwrap();
        assert_eq!(approvals.len(), 1);
        assert_eq!((approvals[0].0.as_str(), approvals[0].1.as_str()), ("acct", "device-1"));
    }

    #[tokio::test]
    async fn failures_are_correlated_and_a_second_grant_is_refused_while_busy() {
        let fake = Arc::new(Fake::default());
        fake.fail.store(true, Ordering::SeqCst);
        let broker = Broker::new(fake.clone(), "acct");
        let (out, mut rx) = outbox();
        let mut session = broker.session("device-1", out);
        assert!(session.on_msg(pair("p1")).is_none());
        match session.on_msg(pair("p2")) {
            Some(ServerMsg::GameStreamError { id, code, .. }) => {
                assert_eq!((id.as_str(), code.as_str()), ("p2", "busy"))
            }
            other => panic!("expected busy, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            super::super::Out::Msg(ServerMsg::GameStreamError { id, code, .. }) => {
                assert_eq!((id.as_str(), code.as_str()), ("p1", "pair_failed"))
            }
            _ => panic!("expected failure"),
        }
        assert_eq!(fake.approvals.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn revocation_is_scoped_to_the_named_device() {
        let fake = Arc::new(Fake::default());
        let broker = Broker::new(fake.clone(), "acct");
        assert_eq!(broker.revoke_device("device-1").await, Ok(1));
        assert_eq!(
            *fake.revocations.lock().unwrap(),
            vec![("acct".to_string(), "device-1".to_string())]
        );
    }
}
