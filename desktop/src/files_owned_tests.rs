//! The Files page through the account-owned path, against a scripted host and a scripted account
//! worker: the credential the worker hands over, the page opening it, the quiet second connection
//! next to a preview, and every way that can fail.
use super::*;
use crate::account::{self, OwnedComputer};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::{rustls, rustls::pki_types::PrivatePkcs8KeyDer, TlsAcceptor};
use tokio_tungstenite::tungstenite::Message;

const OWNER: &str = "00000000-0000-4000-8000-0000000000a1";
/// What the account calls the computer.
const ACCOUNT_ID: &str = "11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
/// What the host calls itself in `welcome`. A different id, as in production.
const PC_ID: &str = "22222222-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const TOKEN: &str = "dG9rZW4tdG9rZW4tdG9rZW4tdG9rZW4tdG9rZW4tMTI";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    /// Knows no token and no pairing code.
    Refuses,
    /// Only a pairing request works. The saved token is out of date.
    PairOnly,
    /// File transfer is off in the host's settings.
    FilesOff,
    /// An older host: no `fsbrowse`.
    NoBrowse,
}

struct Host {
    port: u16,
    pin: [u8; 32],
    connections: Arc<AtomicUsize>,
}

async fn start_host(mode: Mode) -> Host {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let pin: [u8; 32] = ring::digest::digest(&ring::digest::SHA256, cert.der()).as_ref().try_into().unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into())
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let connections = Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else { return };
            let acceptor = acceptor.clone();
            let count = count.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(socket).await else { return };
                let Ok(mut ws) = tokio_tungstenite::accept_async(tls).await else { return };
                let Some(Ok(first)) = ws.next().await else { return };
                let Ok(first) = serde_json::from_str::<Value>(first.to_text().unwrap_or("")) else { return };
                let pc = json!({"id": PC_ID, "name": "Office"});
                let caps: Vec<&str> = if mode == Mode::NoBrowse { vec!["stream", "files"] } else { vec!["stream", "files", "fsbrowse", "thumb"] };
                let ok = match first["t"].as_str() {
                    Some("hello") => first["token"] == TOKEN && !matches!(mode, Mode::Refuses | Mode::PairOnly),
                    Some("pair") => first["otp"] == "once" && mode != Mode::Refuses,
                    _ => false,
                };
                if !ok {
                    let _ = ws.send(Message::text(json!({"t":"error","code":"unauthorized","msg":"this device is not paired"}).to_string())).await;
                    return;
                }
                let reply = if first["t"] == "hello" { json!({"t":"welcome","v":1,"pc":pc,"caps":caps}) } else { json!({"t":"paired","pc":pc,"token":TOKEN}) };
                let _ = ws.send(Message::text(reply.to_string())).await;
                count.fetch_add(1, Ordering::SeqCst);
                while let Some(Ok(message)) = ws.next().await {
                    let Ok(text) = message.to_text() else { continue };
                    let Ok(v) = serde_json::from_str::<Value>(text) else { continue };
                    let answer = match v["t"].as_str() {
                        Some("ping") => json!({"t":"pong"}),
                        Some("fs.list") if mode == Mode::FilesOff => json!({"t":"error","code":"unsupported","msg":"file transfer is off on this PC","of":"fs.list"}),
                        Some("fs.list") => {
                            let entries = if v["path"] == "" {
                                json!([{"name":"C:","path":"C:\\","dir":true,"size":0,"mtime":0}])
                            } else {
                                json!([{"name":"report.txt","path":"C:\\report.txt","dir":false,"size":1200,"mtime":5}])
                            };
                            json!({"t":"fs.list","path":v["path"],"entries":entries})
                        }
                        _ => continue,
                    };
                    if ws.send(Message::text(answer.to_string())).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    Host { port, pin, connections }
}

fn saved(host: &Host, token: &str) -> Computer {
    Computer { id: PC_ID.into(), name: "Office".into(), hosts: vec!["127.0.0.1".into()], port: host.port, fingerprint: host.pin, token: token.into(), relay: None }
}
fn pairing(host: &Host) -> Pairing {
    Pairing { name: "Office".into(), hosts: vec!["127.0.0.1".into()], port: host.port, fingerprint: host.pin, otp: "once".into(), relay_url: None, pc_id: Some(PC_ID.into()) }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn panel() -> Panel {
    let dir = std::env::temp_dir().join(format!("secpar-files-owned-{}-{}", std::process::id(), std::thread::current().name().unwrap_or("t").replace("::", "-")));
    Panel::new(egui::Context::default(), dir.to_str())
}
/// Polls the page the way the app does, until `done` or ten seconds pass.
fn run_until(panel: &mut Panel, done: impl Fn(&Panel) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        panel.poll();
        if done(panel) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out; connection is {:?}", panel.view.conn);
}

/// A scripted account worker: the computer list and the pairing request, over plain HTTP.
async fn account_worker(fingerprint: &[u8; 32]) -> (account::Api, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let pairing_requests = Arc::new(AtomicUsize::new(0));
    let (count, fp) = (pairing_requests.clone(), hex(fingerprint));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let (count, fp) = (count.clone(), fp.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = head.lines().next().unwrap_or("").to_string();
                let (status, body) = if line.starts_with("GET /v1/computers") {
                    (200, json!([{"id": ACCOUNT_ID, "name": "Office", "platform": "windows", "certificate_fp": fp, "linked": 1, "online": 1}]))
                } else if line.starts_with("POST /v1/owner-pairings") {
                    count.fetch_add(1, Ordering::SeqCst);
                    (201, json!({"id": "33333333-cccc-4ccc-8ccc-cccccccccccc", "state": "pending"}))
                } else {
                    (404, json!({"error": "not_found"}))
                };
                let body = body.to_string();
                let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    (account::Api::for_test(base), pairing_requests)
}

/// Asks the scripted worker to open the computer, like the Files page's Open button does, and
/// returns what it handed back.
async fn ask_worker(host: &Host, vault: Option<Computer>) -> (account::OwnedConnection, Option<account::Event>, usize) {
    ask_worker_pin(&host.pin, vault).await
}
async fn ask_worker_pin(pin: &[u8; 32], vault: Option<Computer>) -> (account::OwnedConnection, Option<account::Event>, usize) {
    let (api, requests) = account_worker(pin).await;
    let (events, mut output) = tokio::sync::mpsc::channel(8);
    let mut pending = None;
    let result = account::connect_owned_for_test(&api, OWNER, ACCOUNT_ID, vault, &mut pending, &events).await;
    result.expect("the worker accepts the request");
    drop(events);
    let first = output.recv().await;
    match first {
        Some(account::Event::OwnedConnection(connection)) => (connection, None, requests.load(Ordering::SeqCst)),
        other => (
            account::OwnedConnection::Pair { owner: OWNER.into(), id: ACCOUNT_ID.into(), pairing: Pairing { name: "x".into(), hosts: vec![], port: 0, fingerprint: [0; 32], otp: String::new(), relay_url: None, pc_id: None } },
            other,
            requests.load(Ordering::SeqCst),
        ),
    }
}

fn computers() -> Vec<OwnedComputer> {
    vec![OwnedComputer { id: ACCOUNT_ID.into(), name: "Office".into(), platform: "windows".into(), certificate_fp: None, linked: 1, online: 1 }]
}
fn name_of(id: &str) -> String {
    computers().iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default()
}

#[test]
fn the_ids_of_the_account_and_the_host_differ_and_the_page_waits_for_the_account_one() {
    let mut p = panel();
    p.wait_for(ACCOUNT_ID, "Office");
    let host_side = OwnedConnection::Cached { owner: OWNER.into(), id: ACCOUNT_ID.into(), computer: Computer { id: PC_ID.into(), name: "Office".into(), hosts: vec![], port: 1, fingerprint: [0; 32], token: String::new(), relay: None } };
    let (_, id, _) = owned_source(host_side);
    assert_eq!(id, ACCOUNT_ID, "a saved credential is opened under the account's id, not the host's");
    assert!(p.wants(&id));
    assert!(!p.wants(PC_ID));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_computer_with_a_saved_credential_opens_next_to_a_preview() {
    let host = start_host(Mode::Normal).await;
    // The preview worker keeps its own connection with the same token.
    let preview = Connected::reconnect(saved(&host, TOKEN), "Desktop").await.unwrap();
    let (connection, _, pairings) = ask_worker(&host, Some(saved(&host, TOKEN))).await;
    assert_eq!(pairings, 0, "a saved credential needs no pairing request");
    assert!(matches!(&connection, account::OwnedConnection::Cached { id, .. } if id == ACCOUNT_ID));
    tokio::task::spawn_blocking(move || {
        let mut p = panel();
        p.wait_for(ACCOUNT_ID, "Office");
        assert!(p.receive(connection, Some(OWNER), name_of), "the page takes the credential it asked for");
        run_until(&mut p, |p| p.view.conn == Conn::Online && !p.view.loading);
        assert_eq!(p.view.entries.len(), 1);
        assert_eq!(p.view.entries[0].name, "C:");
        assert!(p.view.browse && p.view.upload);
        // Opening a folder works through the same connection.
        let request = p.view.go("C:\\");
        p.send(request);
        run_until(&mut p, |p| p.view.entries.first().is_some_and(|e| e.name == "report.txt"));
        p.close();
    })
    .await
    .unwrap();
    assert_eq!(host.connections.load(Ordering::SeqCst), 2, "the preview and the files page are two connections");
    drop(preview);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_credential_for_another_account_or_computer_is_not_opened() {
    let host = start_host(Mode::Normal).await;
    let (connection, _, _) = ask_worker(&host, Some(saved(&host, TOKEN))).await;
    tokio::task::spawn_blocking(move || {
        let mut p = panel();
        p.wait_for("some-other-computer", "Other");
        assert!(!p.receive(connection, Some(OWNER), name_of));
        assert_eq!(p.view.conn, Conn::Connecting);
        assert!(p.session.is_none());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_saved_credential_the_account_is_asked_to_pair() {
    let host = start_host(Mode::Normal).await;
    let (_, event, pairings) = ask_worker(&host, None).await;
    assert_eq!(pairings, 1);
    assert!(matches!(event, Some(account::Event::Message(note)) if note == account::pairing_with("Office")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_credential_the_host_no_longer_accepts_is_renewed_once_through_the_account() {
    let host = start_host(Mode::PairOnly).await;
    let (connection, _, _) = ask_worker(&host, Some(saved(&host, "b2xkLXRva2Vu"))).await;
    tokio::task::spawn_blocking(move || {
        let mut p = panel();
        p.wait_for(ACCOUNT_ID, "Office");
        assert!(p.receive(connection, Some(OWNER), name_of));
        run_until(&mut p, |p| p.stale.is_some());
        assert_eq!(p.take_stale(), Some((OWNER.into(), ACCOUNT_ID.into())));
        assert_eq!(p.view.conn, Conn::Connecting, "no error is shown while a new credential is requested");
        assert!(p.wants(ACCOUNT_ID));
        // The account answers the new pairing request with a link.
        let pair = OwnedConnection::Pair { owner: OWNER.into(), id: ACCOUNT_ID.into(), pairing: pairing(&host) };
        assert!(p.receive(pair, Some(OWNER), name_of));
        run_until(&mut p, |p| p.view.conn == Conn::Online && !p.view.loading);
        let (owner, id, computer) = p.take_paired().expect("the new credential is handed back to be saved");
        assert_eq!((owner.as_str(), id.as_str()), (OWNER, ACCOUNT_ID));
        assert_eq!(computer.token, TOKEN);
        p.close();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_refusal_is_shown_and_does_not_loop() {
    let host = start_host(Mode::Refuses).await;
    let (connection, _, _) = ask_worker(&host, Some(saved(&host, "b2xkLXRva2Vu"))).await;
    tokio::task::spawn_blocking(move || {
        let mut p = panel();
        p.wait_for(ACCOUNT_ID, "Office");
        assert!(p.receive(connection, Some(OWNER), name_of));
        run_until(&mut p, |p| p.stale.is_some());
        p.take_stale();
        let pair = OwnedConnection::Pair { owner: OWNER.into(), id: ACCOUNT_ID.into(), pairing: pairing(&host) };
        assert!(p.receive(pair, Some(OWNER), name_of));
        run_until(&mut p, |p| matches!(p.view.conn, Conn::Closed(_)));
        assert_eq!(p.view.conn, Conn::Closed("Office did not accept the pairing. Open Secpar there and try again.".into()));
        assert!(p.take_stale().is_none());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_computer_says_so_in_plain_words() {
    let host = start_host(Mode::Normal).await;
    let mut gone = saved(&host, TOKEN);
    // Nothing listens on this port.
    gone.port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    tokio::task::spawn_blocking(move || {
        let mut p = panel();
        p.wait_for(ACCOUNT_ID, "Office");
        let connection = OwnedConnection::Cached { owner: OWNER.into(), id: ACCOUNT_ID.into(), computer: gone };
        assert!(p.receive(connection, Some(OWNER), name_of));
        run_until(&mut p, |p| matches!(p.view.conn, Conn::Closed(_)));
        assert_eq!(p.view.conn, Conn::Closed("Can't reach Office. Check that it is on, online and running Secpar.".into()));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn file_transfer_turned_off_on_the_computer_is_named() {
    let host = start_host(Mode::FilesOff).await;
    let (connection, _, _) = ask_worker(&host, Some(saved(&host, TOKEN))).await;
    tokio::task::spawn_blocking(move || {
        let mut p = panel();
        p.wait_for(ACCOUNT_ID, "Office");
        assert!(p.receive(connection, Some(OWNER), name_of));
        run_until(&mut p, |p| p.view.problem.is_some());
        assert_eq!(p.view.problem.as_deref(), Some(proto::OFF));
        assert!(!p.view.upload);
        p.close();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_old_host_without_browsing_is_named() {
    let host = start_host(Mode::NoBrowse).await;
    let (connection, _, _) = ask_worker(&host, Some(saved(&host, TOKEN))).await;
    tokio::task::spawn_blocking(move || {
        let mut p = panel();
        p.wait_for(ACCOUNT_ID, "Office");
        assert!(p.receive(connection, Some(OWNER), name_of));
        run_until(&mut p, |p| p.view.conn == Conn::Online);
        assert!(!p.view.browse);
        p.close();
    })
    .await
    .unwrap();
}

#[test]
fn waiting_for_the_account_has_an_end() {
    let mut p = panel();
    p.wait_for(ACCOUNT_ID, "Office");
    p.poll();
    assert_eq!(p.view.conn, Conn::Connecting);
    p.pending_since = Some(Instant::now() - PENDING_LIMIT - Duration::from_secs(1));
    p.poll();
    assert!(matches!(&p.view.conn, Conn::Closed(why) if why.contains("did not get an answer")));
    assert!(!p.wants(ACCOUNT_ID));
}

#[test]
fn failures_become_plain_reasons() {
    let o = "Office";
    assert_eq!(explain("Computer is offline", o).0, "Can't reach Office. Check that it is on, online and running Secpar.");
    assert_eq!(explain("Computer did not respond. Check that Secpar is open there.", o).0, "Can't reach Office. Check that it is on, online and running Secpar.");
    assert_eq!(explain("Connection timed out", o).1, false);
    assert_eq!(explain("Could not establish a secure connection", o).0, "Can't reach Office. Check that it is on, online and running Secpar.");
    assert_eq!(explain("Access was refused or revoked. Pair this computer again.", o), ("Office no longer accepts this device.".into(), true));
    assert_eq!(explain("Host identity changed", o).0, "Office does not match what your account has on record. Refresh the list and try again.");
    assert_eq!(explain("Connection closed", "").0, "Can't reach the computer. Check that it is on, online and running Secpar.");
    assert_eq!(explain("Sign in to connect to your computers", o).0, "Sign in to connect to your computers");
}

/// Against real hosts, by hand: `SECPAR_REAL_LINK` is a file with a fresh pairing link and
/// `SECPAR_REAL_WORK` an empty folder. Pairs once (the first, manual-looking connection), hands the
/// credential to a scripted account worker, opens the computer through the real Files page code as
/// an account computer, browses, uploads about 800 KB, downloads it back and compares hashes.
#[test]
#[ignore = "needs a real host and a one-time pairing link"]
fn real_host_through_the_account_path() {
    let link = std::fs::read_to_string(std::env::var("SECPAR_REAL_LINK").unwrap()).unwrap();
    let work = PathBuf::from(std::env::var("SECPAR_REAL_WORK").unwrap());
    let pairing = Pairing::parse(link.trim()).unwrap();
    let pin = pairing.fingerprint;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let paired = runtime.block_on(async { Connected::pair_lan_first(pairing, "files-check").await.unwrap() });
    let mut computer = paired.computer.clone();
    println!("paired with {} ({} addresses, relay {})", computer.name, computer.hosts.len(), computer.relay.is_some());
    // The host's own id is not the account's id.
    computer.id = computer.id.clone();
    let host_id = computer.id.clone();
    runtime.block_on(async { let mut s = paired.socket; let _ = s.close(None).await; });
    // SECPAR_REAL_RELAY_ONLY=1 drops the direct addresses, so only the relay can answer.
    if std::env::var_os("SECPAR_REAL_RELAY_ONLY").is_some() {
        computer.hosts.clear();
    }
    // The preview worker's connection stays open next to the page.
    let preview = runtime.block_on(async { Connected::reconnect(computer.clone(), "files-check-preview").await.unwrap() });
    let (connection, _, pairings) = runtime.block_on(ask_worker_pin(&pin, Some(computer)));
    assert_eq!(pairings, 0);
    assert!(matches!(&connection, account::OwnedConnection::Cached { id, computer, .. } if id == ACCOUNT_ID && computer.id == host_id && host_id != ACCOUNT_ID));
    let dl = work.join("dl");
    std::fs::create_dir_all(&dl).unwrap();
    let src = work.join("probe-800k.bin");
    let mut x = 22u64 | 1;
    let data: Vec<u8> = (0..800_000).map(|_| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; (x >> 24) as u8 }).collect();
    std::fs::write(&src, &data).unwrap();
    let mut p = Panel::new(egui::Context::default(), dl.to_str());
    p.wait_for(ACCOUNT_ID, "Real host");
    assert!(p.receive(connection, Some(OWNER), |_| "Real host".into()));
    run_until(&mut p, |p| p.view.conn == Conn::Online && !p.view.loading);
    println!("online. browse={} upload={} roots: {}", p.view.browse, p.view.upload, p.view.entries.iter().map(|e| e.name.clone()).take(6).collect::<Vec<_>>().join(", "));
    if std::env::var_os("SECPAR_REAL_FILES_OFF").is_some() {
        // File transfer is switched off in the host's settings: the page must say so.
        println!("problem: {:?}, upload allowed: {}", p.view.problem, p.view.upload);
        assert_eq!(p.view.problem.as_deref(), Some(proto::OFF));
        assert!(!p.view.upload);
        p.close();
        return;
    }
    assert!(!p.view.entries.is_empty(), "the roots list is empty");
    let root = p.view.entries.iter().find(|e| e.dir).expect("a folder among the roots").clone();
    let request = p.view.go(&root.path);
    p.send(request);
    run_until(&mut p, |p| !p.view.loading && p.view.path == root.path);
    println!("opened {} ({} entries)", root.name, p.view.entries.len());
    p.upload(vec![src.clone()]);
    run_until(&mut p, |p| p.view.transfers.iter().any(|t| t.upload && t.finished()));
    let up = p.view.transfers.iter().find(|t| t.upload).unwrap().clone();
    assert_eq!(up.phase, Phase::Done, "upload failed: {}", up.error);
    println!("uploaded {} bytes to the computer, stored at {}", up.size, up.path);
    // Browse to where it landed and download it from the listing.
    let parent = up.path.rsplit_once(['\\', '/']).map(|(dir, _)| dir.to_string()).unwrap();
    let request = p.view.go(&parent);
    p.send(request);
    run_until(&mut p, |p| !p.view.loading && p.view.path == parent && !p.view.entries.is_empty());
    let name = up.path.rsplit(['\\', '/']).next().unwrap().to_string();
    let entry = p.view.entries.iter().find(|e| e.name == name).expect("the uploaded file is listed").clone();
    println!("listed {} in {} ({} bytes)", entry.name, parent, entry.size);
    p.send(Request::Get(entry.path.clone()));
    run_until(&mut p, |p| p.view.transfers.iter().any(|t| !t.upload && t.finished()));
    let down = p.view.transfers.iter().find(|t| !t.upload).unwrap().clone();
    assert_eq!(down.phase, Phase::Done, "download failed: {}", down.error);
    let sha = |b: &[u8]| hex(ring::digest::digest(&ring::digest::SHA256, b).as_ref());
    let (a, b) = (sha(&data), sha(&std::fs::read(&down.path).unwrap()));
    println!("downloaded {} bytes; sha256 {} {}", down.size, &b[..16], if a == b { "MATCH" } else { "MISMATCH" });
    assert_eq!(a, b);
    p.close();
    drop(preview);
}
