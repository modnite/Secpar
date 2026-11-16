//! A file sent from one computer's background part to another's hosting side
//! over a real tunnel on loopback. The real writer writes into a temporary
//! folder standing in for `Downloads/Secpar`.
//!
//! A file of about 700 KB takes three app channel frames, arrives byte-identical
//! under its own name and gets a numbered name when that one is taken. A name
//! that points at another folder is refused, and a friend's device is not sent
//! files at all.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::{
    rand::SystemRandom,
    signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
};
use secpar_core::{
    account::{Key, ORIGIN},
    hosting::Access,
    identity::hex,
};
use secpar_hosting::{Engine, Files, Hosting, Pairing, Pending, Progress, Refusal, FILE_CHUNK, MAX_FILE};
use secpar_service::{files, Core, Settings};
use secpar_tunnel::Identity;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct NoEngine;
impl Engine for NoEngine {
    fn approve<'a>(&'a self, _: &'a str, _: &'a str, _: &'a Pairing, _: &'a str) -> Pending<'a, Result<(), Refusal>> {
        Box::pin(async { Err(Refusal::Failed) })
    }
    fn revoke<'a>(&'a self, _: &'a str, _: &'a str) -> Pending<'a, bool> {
        Box::pin(async { true })
    }
    fn granted(&self) -> Pending<'_, Vec<(String, String)>> {
        Box::pin(async { Vec::new() })
    }
}

/// An account service's signing key, standing in for the real one.
fn signer() -> (EcdsaKeyPair, Key) {
    let rng = SystemRandom::new();
    let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref(), &rng).unwrap();
    let point = pair.public_key().as_ref().to_vec();
    let key = Key { kid: "k1".into(), kty: "EC".into(), crv: "P-256".into(), x: URL_SAFE_NO_PAD.encode(&point[1..33]), y: URL_SAFE_NO_PAD.encode(&point[33..]) };
    (pair, key)
}
fn list(pair: &EcdsaKeyPair, host: &Identity, own: &[&Identity], guests: Vec<serde_json::Value>) -> String {
    let now = now();
    let header = serde_json::json!({"alg": "ES256", "typ": "secpar-access+jwt", "kid": "k1"});
    let claims = serde_json::json!({
        "iss": ORIGIN, "aud": "secpar-host", "sub": hex(&host.fingerprint()), "iat": now - 10, "exp": now + 3600, "owner": "id-alice",
        "devices": own.iter().map(|d| serde_json::json!({"fingerprint": hex(&d.fingerprint()), "name": "Laptop"})).collect::<Vec<_>>(),
        "guests": guests,
    });
    let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
    let signature = pair.sign(&SystemRandom::new(), input.as_bytes()).unwrap();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// The real writer, counting the frames it was given, so the test can show the
/// file really took more than one.
struct Counting {
    downloads: Arc<files::Downloads>,
    frames: Mutex<usize>,
}
impl Files for Counting {
    fn receive(&self, from: &str, name: &str, total: u64, offset: u64, data: &[u8]) -> Option<Progress> {
        *self.frames.lock().unwrap() += 1;
        self.downloads.receive(from, name, total, offset, data)
    }
}

/// One piece of a file, as a computer that is not the window would send it.
fn piece(name: &str, size: u64, offset: u64, data: &[u8]) -> Vec<u8> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    let request = json!({"app": {"file": {"name": name, "size": size, "offset": offset, "data": encoded}}});
    serde_json::to_vec(&request).unwrap()
}
async fn answer(tunnel: &secpar_tunnel::Client, request: Vec<u8>) -> serde_json::Value {
    serde_json::from_slice(&tunnel.app_request(&request).await.unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_of_three_frames_arrives_intact_and_guests_and_other_names_are_refused() {
    let folder = tempfile::tempdir().unwrap();
    let (signer, key) = signer();
    let (host, laptop, friend) = (Identity::generate().unwrap(), Identity::generate().unwrap(), Identity::generate().unwrap());
    let access = Arc::new(Access::open(ORIGIN, host.fingerprint(), folder.path().join("access.json"), now()));
    access
        .update(
            &list(
                &signer,
                &host,
                &[&laptop],
                vec![json!({
                    "fingerprint": hex(&friend.fingerprint()), "name": "Phone", "account": "id-bob",
                    "handle": "bob", "trusted": true, "keyboard": true, "mouse": true,
                })],
            ),
            &[key],
            now(),
        )
        .unwrap();

    // The hosting computer writes into a temporary folder standing in for
    // Downloads/Secpar.
    let root = folder.path().join("Downloads").join("Secpar");
    let downloads = Arc::new(files::Downloads::at(&root));
    let received = Arc::new(Counting { downloads: downloads.clone(), frames: Mutex::new(0) });
    let none = secpar_tunnel::Ports { tcp: vec![], udp: vec![] };
    let hosting = Hosting::start(
        "127.0.0.1:0".parse().unwrap(),
        &host,
        access,
        Arc::new(NoEngine),
        Arc::new(secpar_hosting::NoClipboard),
        received.clone(),
        Arc::new(secpar_hosting::NoPreviews),
        "127.0.0.1".parse().unwrap(),
        none.clone(),
    )
    .await
    .unwrap();
    let address = hosting.tunnel().local_addr().unwrap();

    // The owner's laptop: the background part the window asks to send a file.
    let account = secpar_core::account::Service::local_test(ORIGIN, "http://127.0.0.1:9").unwrap();
    let laptop_part = Core::<NoEngine>::start(
        Settings { account, state: folder.path().join("laptop"), bind: "127.0.0.1:0".parse().unwrap() },
        laptop.clone(),
        None,
    )
    .await
    .unwrap();
    // The clipboard stays out of this test, so the real one is never read.
    laptop_part.answer(&json!({"do": "clipboard", "on": false}).to_string()).await;

    // Nothing can be sent before there is a tunnel.
    let path = folder.path().join("report.bin");
    std::fs::write(&path, b"x").unwrap();
    let early = laptop_part.answer(&json!({"do": "send_file", "path": path.to_str().unwrap()}).to_string()).await;
    assert_eq!(early["error"], "not_connected");

    let connected = laptop_part.answer(&json!({"do": "connect", "host": hex(&host.fingerprint()), "addresses": [address]}).to_string()).await;
    assert_eq!(connected["ok"], true, "{connected}");

    // About 700 KB, which is three frames of the sending side's own size.
    let data: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    assert!(data.len() > 2 * FILE_CHUNK && data.len() <= 3 * FILE_CHUNK, "about three frames");
    std::fs::write(&path, &data).unwrap();

    let started = laptop_part.answer(&json!({"do": "send_file", "path": path.to_str().unwrap()}).to_string()).await;
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["transfer"]["name"], "report.bin");
    assert_eq!(started["transfer"]["total"], 700_000);

    // The window follows the transfer until it is done.
    let end = tokio::time::Instant::now() + Duration::from_secs(30);
    let (state, sent) = loop {
        let answer = laptop_part.answer(&json!({"do": "transfers"}).to_string()).await;
        let transfer = answer["transfers"][0].clone();
        let state = transfer["state"].as_str().unwrap_or_default().to_string();
        if state != "sending" {
            break (state, transfer["sent"].as_u64().unwrap_or_default());
        }
        assert!(tokio::time::Instant::now() < end, "the file was still sending after 30 seconds");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!((state.as_str(), sent), ("sent", 700_000));

    // It arrived whole, under its own name, and was written in three pieces.
    assert_eq!(std::fs::read(root.join("report.bin")).unwrap(), data);
    assert_eq!(*received.frames.lock().unwrap(), 3, "the file took three frames");
    assert_eq!(downloads.last().unwrap().name, "report.bin");
    assert!(root.join("report.bin").exists());

    // The same name again is kept apart from the first file.
    let again = laptop_part.answer(&json!({"do": "send_file", "path": path.to_str().unwrap()}).to_string()).await;
    assert_eq!(again["ok"], true, "{again}");
    let end = tokio::time::Instant::now() + Duration::from_secs(30);
    while laptop_part.answer(&json!({"do": "transfers"}).to_string()).await["transfers"][1]["state"] == "sending" {
        assert!(tokio::time::Instant::now() < end, "the second file was still sending after 30 seconds");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(std::fs::read(root.join("report (2).bin")).unwrap(), data);

    // A friend's device may connect and use the keyboard, and is still not sent
    // files.
    let friend_tunnel = secpar_tunnel::connect(address, &friend, host.fingerprint(), "127.0.0.1".parse().unwrap(), none.clone()).await.unwrap();
    let refused = answer(&friend_tunnel, piece("from-friend.txt", 5, 0, b"hello")).await;
    assert_eq!((refused["ok"].clone(), refused["error"].clone()), (json!(false), json!("not_allowed")));
    assert!(!root.join("from-friend.txt").exists());

    // A name that points at another folder, or that the file systems refuse, is
    // turned away before anything is written anywhere.
    let own = secpar_tunnel::connect(address, &laptop, host.fingerprint(), "127.0.0.1".parse().unwrap(), none).await.unwrap();
    for name in ["../escape.txt", "..\\escape.txt", "sub/name.txt", "", ".", "a\u{0}b.txt", "con:1.txt"] {
        let refused = answer(&own, piece(name, 10, 0, b"0123456789")).await;
        assert_eq!(refused["error"], "bad_name", "the name {name:?} is refused");
    }
    assert!(!folder.path().join("Downloads").join("escape.txt").exists());
    assert!(!folder.path().join("escape.txt").exists());
    // A size over the limit and a piece longer than one frame are refused too.
    assert_eq!(answer(&own, piece("huge.bin", MAX_FILE + 1, 0, b"x")).await["error"], "too_large");
    assert_eq!(answer(&own, piece("wide.bin", 2 * FILE_CHUNK as u64, 0, &vec![0u8; FILE_CHUNK + 1])).await["error"], "too_large");
}
