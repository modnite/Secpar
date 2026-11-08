//! Connect anywhere for the Linux host (docs/remote-protocol.md): the one WebSocket to the
//! rendezvous Worker, the signed bootstrap address for pairing links, and TURN credentials.
//! The protocol itself is `secpar_host_core::rendezvous`, shared with the other desktop hosts.
//! This host's relay key is the private `master` key in its store. Nothing here prints a URL
//! or a key: they carry signatures and, on first contact, the key itself.

use crate::store::Store;
use secpar_host_core::{agent::AgentHandle, relay::TurnServer, rendezvous};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub use rendezvous::url;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Keeps the Worker connection up for the life of the process.
pub fn start(agent: AgentHandle, store: Arc<Mutex<Store>>, base: String) {
    tokio::spawn(async move { rendezvous::run(agent, store, base, |m| eprintln!("secpar: {m}")).await });
}

/// `&pc=<id>&r=<signed phone URL>` for a fresh pairing link, once the Worker knows our key.
/// Without it a phone outside the LAN could not reach the host to pair. None when remote
/// access is off or the Worker has not accepted the key yet.
pub fn pairing_params(store: &Store, base: Option<&str>) -> Option<(String, String)> {
    use secpar_host_core::rendezvous::RelaySource;
    let base = base?;
    if !store.relay_registered() {
        return None;
    }
    let pc = store.pc_id();
    let dev = format!("pair_{}", rendezvous::b64url(&crate::store::random::<16>()));
    let ticket = secpar_host_core::PairingStore::ticket(store, &dev);
    Some((pc.clone(), rendezvous::pairing_relay_url(base, &pc, &dev, &ticket, unix_now())))
}

/// `POST` URL for this host's TURN credentials.
fn turn_request(base: &str, pc: &str, key: &[u8], ts: u64) -> String {
    secpar_host_core::relay::turn_url(base, pc, ts, &rendezvous::host_sig(key, pc, ts))
}

/// Fetches TURN credentials from the Worker. Blocking: call from a plain thread.
pub fn turn_server(store: &Mutex<Store>, base: &str) -> Option<TurnServer> {
    use secpar_host_core::rendezvous::RelaySource;
    let url = {
        let s = store.lock().unwrap();
        turn_request(base, &s.pc_id(), &s.relay_key(), unix_now())
    };
    match secpar_host_core::relay::fetch_turn_blocking(&url) {
        Ok(server) => Some(server),
        Err(e) => {
            eprintln!("secpar: turn: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use futures_util::{SinkExt, StreamExt};
    use secpar_host_core::agent::{Agent, BoxFut, Handlers, Outbox, Session};
    use secpar_host_core::proto::{ClientMsg, PcInfo, ServerMsg};
    use secpar_host_core::rendezvous::{bin_env, text_env, RelaySource};
    use secpar_host_core::PairingStore;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    use tokio_tungstenite::tungstenite::http::StatusCode;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::WebSocketStream;

    struct Quiet;
    impl Session for Quiet {
        fn on_msg(&mut self, _: ClientMsg) -> Option<ServerMsg> {
            None
        }
        fn on_binary(&mut self, b: &[u8]) -> Option<ServerMsg> {
            Some(ServerMsg::Ok { of: format!("bin:{}", b.len()) })
        }
    }
    struct Handler(String);
    impl Handlers for Handler {
        fn pc_info(&self) -> BoxFut<PcInfo> {
            let id = self.0.clone();
            Box::pin(async move { PcInfo { id, name: "Fixture".into(), user: String::new(), macs: vec![], os: "linux".into() } })
        }
        fn caps(&self) -> Vec<String> {
            vec!["stream".into()]
        }
        fn session(&self, _: &str, _: Outbox) -> Box<dyn Session> {
            Box::new(Quiet)
        }
    }

    type Worker = WebSocketStream<TcpStream>;

    /// Accepts the host's next connection like the Worker: 401 when `refuse`, else the socket.
    async fn accept(l: &TcpListener, refuse: bool) -> (HashMap<String, String>, Option<Worker>) {
        let (tcp, _) = l.accept().await.unwrap();
        let mut q = HashMap::new();
        let cb = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
            assert_eq!(req.uri().path(), "/v1/host");
            for kv in req.uri().query().unwrap_or_default().split('&') {
                let (k, v) = kv.split_once('=').unwrap();
                q.insert(k.to_string(), v.to_string());
            }
            if refuse {
                let mut e = ErrorResponse::new(None);
                *e.status_mut() = StatusCode::UNAUTHORIZED;
                Err(e)
            } else {
                Ok(resp)
            }
        };
        let ws = tokio_tungstenite::accept_hdr_async(tcp, cb).await.ok();
        (q, ws)
    }

    async fn env(w: &mut Worker) -> Value {
        match tokio::time::timeout(Duration::from_secs(5), w.next()).await.expect("worker recv timed out") {
            Some(Ok(Message::Text(t))) => {
                let mut v: Value = serde_json::from_str(&t).unwrap();
                if let Some(m) = v["m"].as_str() {
                    v["m"] = serde_json::from_str(m).unwrap();
                }
                v
            }
            other => panic!("worker got {other:?}"),
        }
    }

    #[tokio::test]
    async fn registers_serves_a_relayed_phone_and_sends_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::open(dir.path()).unwrap();
        let store = Arc::new(Mutex::new(Store::open(dir.path()).unwrap()));
        let (dev, token) = store.lock().unwrap().add_device("Pixel").unwrap();
        let (gone, _) = store.lock().unwrap().add_device("Old phone").unwrap();
        store.lock().unwrap().revoke(&gone).unwrap();
        let (pc, key) = {
            let s = store.lock().unwrap();
            (s.pc_id(), s.relay_key())
        };
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("ws://{}", l.local_addr().unwrap());
        let agent = Agent::start(&identity, store.clone(), Arc::new(Handler(pc.clone())), "127.0.0.1:0".parse().unwrap(), Duration::from_secs(30), Some(base.clone()))
            .await
            .unwrap();
        start(agent.clone(), store.clone(), base.clone());

        // First contact registers the key, signed with it.
        let (q, w) = accept(&l, false).await;
        let mut w = w.unwrap();
        assert_eq!(q["pc"], pc);
        assert_eq!(q["register"], rendezvous::b64url(&key));
        assert_eq!(q["sig"], rendezvous::host_sig(&key, &pc, q["ts"].parse().unwrap()));
        assert_eq!(env(&mut w).await, json!({"o":"revoke","dev":gone}), "earlier removals are repeated");
        for _ in 0..50 {
            if store.lock().unwrap().relay_registered() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(store.lock().unwrap().relay_registered(), "the accepted key is remembered");
        assert!(pairing_params(&store.lock().unwrap(), Some(&base)).is_some());

        // A phone opens connection 7: hello, welcome, then the `remote` message with its ticket.
        w.send(Message::text(r#"{"c":7,"o":"open"}"#)).await.unwrap();
        w.send(text_env(7, &json!({"t":"hello","v":1,"token":token,"device":"Pixel"}).to_string())).await.unwrap();
        let welcome = env(&mut w).await;
        assert_eq!((welcome["c"].as_u64(), welcome["m"]["t"].as_str()), (Some(7), Some("welcome")));
        let remote = env(&mut w).await;
        assert_eq!(remote["m"]["t"], "remote");
        assert_eq!((remote["m"]["url"].as_str(), remote["m"]["dev"].as_str()), (Some(base.as_str()), Some(dev.as_str())));
        assert_eq!(remote["m"]["ticket"], rendezvous::b64url(&rendezvous::ticket(&key, &pc, &dev)));
        w.send(text_env(7, r#"{"t":"ping"}"#)).await.unwrap();
        assert_eq!(env(&mut w).await["m"]["t"], "pong");
        w.send(bin_env(7, &[1, 2, 3])).await.unwrap();
        assert_eq!(env(&mut w).await, json!({"c":7,"m":{"t":"ok","of":"bin:3"}}));
        assert_eq!(agent.connected(), vec![dev.clone()]);

        // Removing the phone: the Worker hears it, the relayed session ends.
        agent.revoke(&dev);
        let mut got = vec![env(&mut w).await, env(&mut w).await, env(&mut w).await];
        got.sort_by_key(|v| v.to_string());
        assert_eq!(got[0]["m"]["code"], "revoked");
        assert_eq!(got[1], json!({"c":7,"o":"close"}));
        assert_eq!(got[2], json!({"dev":dev,"o":"revoke"}));

        // The Worker socket drops: back within the first backoff step, no `register` this time,
        // both removals repeated.
        drop(w);
        let (q, w) = accept(&l, false).await;
        let mut w = w.unwrap();
        assert!(!q.contains_key("register"));
        let mut again = vec![env(&mut w).await["dev"].as_str().unwrap().to_string(), env(&mut w).await["dev"].as_str().unwrap().to_string()];
        again.sort();
        let mut want = vec![gone, dev];
        want.sort();
        assert_eq!(again, want);
    }

    #[test]
    fn pairing_bootstrap_needs_a_registered_key_and_never_exports_the_ticket() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        assert!(pairing_params(&s, Some("wss://w.example")).is_none(), "not before the Worker knows the key");
        s.set_relay_registered().unwrap();
        assert!(pairing_params(&s, None).is_none(), "off without a Worker");
        let (pc, r) = pairing_params(&s, Some("wss://w.example")).unwrap();
        assert_eq!(pc, s.pc_id());
        assert!(r.starts_with(&format!("wss://w.example/v1/phone?pc={pc}&dev=pair_")));
        assert!(!r.contains(&rendezvous::b64url(&s.relay_key())));
    }

    #[test]
    fn turn_request_is_signed_for_the_host_role() {
        let key = [7u8; 32];
        let u = turn_request("wss://w.example", "pc1", &key, 1_700_000_000);
        assert_eq!(u, format!("https://w.example/v1/turn?role=host&pc=pc1&ts=1700000000&sig={}", rendezvous::host_sig(&key, "pc1", 1_700_000_000)));
    }

    #[test]
    fn store_ticket_matches_the_shared_derivation() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        let (id, _) = s.add_device("Phone").unwrap();
        assert_eq!(s.ticket(&id), rendezvous::ticket(&s.relay_key(), &s.pc_id(), &id));
    }
}
