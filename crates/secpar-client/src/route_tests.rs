//! Real sockets with fake slow and dead routes. A listener that never accepts a TLS handshake
//! stands for an address that does not answer: the connection stays open and nothing comes back.
//! Times are small (tens of milliseconds) so the suite stays fast, and the assertions leave wide
//! margins around them.
use crate::control::{Computer, Connected, Relay};
use crate::pairing::Pairing;
use crate::route::Timing;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio_rustls::{rustls, rustls::pki_types::PrivatePkcs8KeyDer, TlsAcceptor};
use tokio_tungstenite::tungstenite::Message;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn timing() -> Timing {
    Timing {
        stagger: ms(100),
        relay_delay: ms(500),
        direct_after_relay: ms(100),
        direct_tcp: ms(400),
        direct_open: ms(700),
        relay_open: ms(700),
        answer: ms(1500),
        remote_wait: ms(150),
    }
}

fn token() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7; 32])
}

/// A host that speaks the protocol on the given loopback addresses, all on one port.
struct Host {
    port: u16,
    pin: [u8; 32],
    hellos: Arc<AtomicUsize>,
    pairs: Arc<AtomicUsize>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Host {
    fn drop(&mut self) {
        self.tasks.iter().for_each(|t| t.abort());
    }
}

/// `deny`: answer hello with a refusal instead of welcome.
async fn host(ips: &[&str], deny: bool) -> Host {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let pin: [u8; 32] = ring::digest::digest(&ring::digest::SHA256, cert.der()).as_ref().try_into().unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into())
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let first = TcpListener::bind((ips[0], 0)).await.unwrap();
    let port = first.local_addr().unwrap().port();
    let mut listeners = vec![first];
    for ip in &ips[1..] {
        listeners.push(TcpListener::bind((*ip, port)).await.unwrap());
    }
    let (hellos, pairs) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let tasks = listeners
        .into_iter()
        .map(|listener| {
            let (acceptor, hellos, pairs) = (acceptor.clone(), hellos.clone(), pairs.clone());
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else { return };
                    let (acceptor, hellos, pairs) = (acceptor.clone(), hellos.clone(), pairs.clone());
                    tokio::spawn(async move {
                        let Ok(tls) = acceptor.accept(stream).await else { return };
                        let Ok(mut ws) = tokio_tungstenite::accept_async(tls).await else { return };
                        let Some(Ok(Message::Text(first))) = ws.next().await else { return };
                        let first: Value = serde_json::from_str(&first).unwrap();
                        let reply = match first["t"].as_str() {
                            Some("hello") => {
                                hellos.fetch_add(1, Ordering::SeqCst);
                                json!({"t": if deny {"denied"} else {"welcome"}, "pc": {"id": "pc1", "name": "Test"}, "caps": ["files"]})
                            }
                            Some("pair") => {
                                pairs.fetch_add(1, Ordering::SeqCst);
                                json!({"t": "paired", "pc": {"id": "pc1", "name": "Test"}, "token": token()})
                            }
                            _ => return,
                        };
                        let _ = ws.send(Message::text(reply.to_string())).await;
                        while let Some(Ok(m)) = ws.next().await {
                            if m.is_close() {
                                break;
                            }
                        }
                    });
                }
            })
        })
        .collect();
    Host { port, pin, hellos, pairs, tasks }
}

/// Addresses that accept the TCP connection and then say nothing, which is how a dead route
/// behind a firewall looks after the SYN gets through.
async fn silent(ips: &[&str], port: u16) -> Vec<TcpListener> {
    let mut found = vec![];
    for ip in ips {
        found.push(TcpListener::bind((*ip, port)).await.unwrap());
    }
    found
}

/// A host on 127.0.0.1 and `dead` further addresses that never answer, all on one port. The
/// real one is last. 127.0.0.2 and up are loopback too, on Windows and Linux.
async fn rig(dead: usize, deny: bool) -> (Host, Vec<TcpListener>, Computer) {
    let real = host(&["127.0.0.1"], deny).await;
    let names: Vec<String> = (0..dead).map(|i| format!("127.0.0.{}", i + 2)).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let holes = silent(&refs, real.port).await;
    let mut hosts = names.clone();
    hosts.push("127.0.0.1".into());
    let computer = Computer { id: "pc1".into(), name: "Test".into(), hosts, port: real.port, fingerprint: real.pin, token: token(), relay: None };
    (real, holes, computer)
}

fn relay_to(port: u16) -> Relay {
    Relay { url: format!("wss://127.0.0.1:{port}"), dev: "dev1".into(), ticket: token() }
}

#[tokio::test]
async fn silent_addresses_do_not_add_up() {
    let (_real, _holes, computer) = rig(4, false).await;
    let started = Instant::now();
    let c = Connected::reconnect_with(computer, "Test", &timing()).await.unwrap();
    let took = started.elapsed();
    assert_eq!(c.direct_host.as_deref(), Some("127.0.0.1"));
    // One after another, each of the four would hold the connection for 700 ms: 2.8 s.
    assert!(took < ms(1200), "took {took:?}");
    let cancelled = c.route_log.iter().filter(|t| t.outcome == "cancelled").count();
    assert_eq!(cancelled, 4, "{:?}", c.route_log);
}

#[tokio::test]
async fn a_route_that_never_answers_is_cut_off_at_its_limit() {
    let real = host(&["127.0.0.1"], false).await;
    let holes = silent(&["127.0.0.2"], real.port).await;
    let computer = Computer { id: "pc1".into(), name: "Test".into(), hosts: vec!["127.0.0.2".into()], port: real.port, fingerprint: real.pin, token: token(), relay: None };
    let started = Instant::now();
    let error = Connected::reconnect_with(computer, "Test", &timing()).await.err().unwrap();
    let took = started.elapsed();
    assert_eq!(error, "Connection timed out");
    assert!(took >= ms(650) && took < ms(1500), "took {took:?}");
    drop(holes);
}

#[tokio::test]
async fn a_failed_route_hands_over_to_the_next_without_waiting_for_the_stagger() {
    let real = host(&["127.0.0.1"], false).await;
    // Nothing listens on 127.0.0.2 here: the connection is refused. The stagger is long.
    let computer = Computer { id: "pc1".into(), name: "Test".into(), hosts: vec!["127.0.0.2".into(), "127.0.0.1".into()], port: real.port, fingerprint: real.pin, token: token(), relay: None };
    let slow = Timing { stagger: Duration::from_secs(5), ..timing() };
    let started = Instant::now();
    let c = Connected::reconnect_with(computer, "Test", &slow).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert_eq!(c.direct_host.as_deref(), Some("127.0.0.1"));
}

#[tokio::test]
async fn the_relay_joins_only_after_its_delay() {
    let (_real, _holes, mut computer) = rig(2, false).await;
    // Take away the working address, so only silent ones and the relay are left.
    computer.hosts.retain(|h| h != "127.0.0.1");
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    computer.relay = Some(relay_to(relay.local_addr().unwrap().port()));
    let started = Instant::now();
    let watcher = tokio::spawn(async move {
        let _held = relay.accept().await.unwrap();
        started.elapsed()
    });
    let error = Connected::reconnect_with(computer, "Test", &timing()).await.err().unwrap();
    let contact = watcher.await.unwrap();
    assert!(contact >= ms(450), "the relay was contacted after {contact:?}, before its delay");
    assert!(contact < ms(900), "the relay was contacted after {contact:?}");
    assert!(error.contains("timed out") || error.contains("secure"), "{error}");
}

#[tokio::test]
async fn a_lan_that_works_never_contacts_the_relay() {
    let (_real, _holes, mut computer) = rig(1, false).await;
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    computer.relay = Some(relay_to(relay.local_addr().unwrap().port()));
    let started = Instant::now();
    let c = Connected::reconnect_with(computer, "Test", &timing()).await.unwrap();
    assert!(started.elapsed() < ms(500));
    assert_eq!(c.direct_host.as_deref(), Some("127.0.0.1"));
    // Wait past the relay delay: it must not be started late either.
    assert!(tokio::time::timeout(ms(900), relay.accept()).await.is_err(), "the relay was contacted");
    assert!(c.route_log.iter().all(|t| t.label != "relay"), "{:?}", c.route_log);
}

#[tokio::test]
async fn the_route_that_worked_is_tried_first_next_time() {
    let (_real, _holes, computer) = rig(2, false).await;
    let slow = Timing { stagger: ms(400), ..timing() };
    let first = Connected::reconnect_with(computer.clone(), "Test", &slow).await.unwrap();
    // The working address was last in the list, so the first connection waited for two staggers.
    assert!(first.route_log.len() == 3, "{:?}", first.route_log);
    drop(first);
    let started = Instant::now();
    let second = Connected::reconnect_with(computer, "Test", &slow).await.unwrap();
    assert_eq!(second.route_log.len(), 1, "{:?}", second.route_log);
    assert_eq!(second.route_log[0].label, "127.0.0.1");
    assert!(started.elapsed() < ms(350), "{:?}", started.elapsed());
}

#[tokio::test]
async fn a_host_that_says_no_ends_the_race() {
    let (real, _holes, computer) = rig(1, true).await;
    let started = Instant::now();
    let error = Connected::reconnect_with(computer, "Test", &timing()).await.err().unwrap();
    assert!(error.starts_with("Access was refused"), "{error}");
    assert!(started.elapsed() < ms(600));
    assert_eq!(real.hellos.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn two_working_routes_spend_the_pairing_code_once() {
    let real = host(&["127.0.0.1", "127.0.0.2"], false).await;
    let p = Pairing {
        name: "Test".into(),
        hosts: vec!["127.0.0.1".into(), "127.0.0.2".into()],
        port: real.port,
        fingerprint: real.pin,
        otp: "once".into(),
        relay_url: None,
        pc_id: Some("pc1".into()),
    };
    // Both routes start together, so both open and both would pair if nothing held one back.
    let simultaneous = Timing { stagger: Duration::ZERO, ..timing() };
    let c = Connected::pair_with(p, "Test", true, &simultaneous).await.unwrap();
    tokio::time::sleep(ms(200)).await;
    assert_eq!(real.pairs.load(Ordering::SeqCst), 1);
    assert_eq!(c.computer.id, "pc1");
}

#[tokio::test]
async fn pairing_with_a_silent_address_in_front_is_quick() {
    let real = host(&["127.0.0.1"], false).await;
    let _holes = silent(&["127.0.0.2", "127.0.0.3"], real.port).await;
    let p = Pairing {
        name: "Test".into(),
        hosts: vec!["127.0.0.2".into(), "127.0.0.3".into(), "127.0.0.1".into()],
        port: real.port,
        fingerprint: real.pin,
        otp: "once".into(),
        relay_url: None,
        pc_id: Some("pc1".into()),
    };
    let started = Instant::now();
    let c = Connected::pair_with(p, "Test", true, &timing()).await.unwrap();
    // 200 ms of stagger and the wait for remote credentials, which this host has none to send.
    assert!(started.elapsed() < ms(900), "{:?}", started.elapsed());
    assert_eq!(c.direct_host.as_deref(), Some("127.0.0.1"));
}
