//! Stills of a hosting computer's real screen, asked for by the owner's own
//! laptop through its background part.
//!
//! The hosting side runs the real capture of this computer's display, so these
//! tests need an unlocked desktop with a display: a locked screen has nothing to
//! take and answers `not_available`. They are written for Windows, where the
//! still is captured; Linux has no capture path yet.
//!
//! Proved here: a still of the width that was asked for comes from a computer
//! this one is connected to and from one it is not, a peek beside an open stream
//! tunnel leaves that tunnel working, a friend is refused, an oversized width is
//! clamped, an offline computer is never asked, and the same computer is not
//! asked twice in a row.
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
use secpar_hosting::{preview_width, Engine, Hosting, Pairing, Pending, Previews, Refusal, Still, MAX_PREVIEW_FRAME, MAX_PREVIEW_WIDTH};
use secpar_service::{preview, Core, Settings};
use secpar_tunnel::Identity;
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

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

fn signer() -> (EcdsaKeyPair, Key) {
    let rng = SystemRandom::new();
    let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref(), &rng).unwrap();
    let point = pair.public_key().as_ref().to_vec();
    let key = Key { kid: "k1".into(), kty: "EC".into(), crv: "P-256".into(), x: URL_SAFE_NO_PAD.encode(&point[1..33]), y: URL_SAFE_NO_PAD.encode(&point[33..]) };
    (pair, key)
}
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}
fn list(pair: &EcdsaKeyPair, host: &Identity, own: &[&Identity], guests: Vec<Value>) -> String {
    let header = json!({"alg": "ES256", "typ": "secpar-access+jwt", "kid": "k1"});
    let claims = json!({
        "iss": ORIGIN, "aud": "secpar-host", "sub": hex(&host.fingerprint()), "iat": now() - 10, "exp": now() + 3600, "owner": "id-alice",
        "devices": own.iter().map(|d| json!({"fingerprint": hex(&d.fingerprint()), "name": "Laptop"})).collect::<Vec<_>>(),
        "guests": guests,
    });
    let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
    let signature = pair.sign(&SystemRandom::new(), input.as_bytes()).unwrap();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

/// The real capture, counting the pictures it is asked to take.
struct Counting {
    taken: Mutex<Vec<u32>>,
}
impl Previews for Counting {
    fn still(&self, max_width: u32) -> Option<Still> {
        self.taken.lock().unwrap().push(max_width);
        preview::Screen.still(max_width)
    }
}

/// A hosting computer with the real screen, whose list allows `own` and the
/// guests given.
struct Computer {
    identity: Identity,
    hosting: Hosting<NoEngine>,
    screen: Arc<Counting>,
    _folder: tempfile::TempDir,
}
impl Computer {
    async fn start(pair: &EcdsaKeyPair, key: &Key, host: Identity, own: &[&Identity], guests: Vec<Value>) -> Self {
        let folder = tempfile::tempdir().unwrap();
        let access = Arc::new(Access::open(ORIGIN, host.fingerprint(), folder.path().join("access.json"), now()));
        access.update(&list(pair, &host, own, guests), &[key.clone()], now()).unwrap();
        let screen = Arc::new(Counting { taken: Mutex::new(Vec::new()) });
        let none = secpar_tunnel::Ports { tcp: vec![], udp: vec![] };
        let hosting = Hosting::start("127.0.0.1:0".parse().unwrap(), &host, access, Arc::new(NoEngine), Arc::new(secpar_hosting::NoClipboard), Arc::new(secpar_hosting::NoFiles), screen.clone(), "127.0.0.1".parse().unwrap(), none)
            .await
            .unwrap();
        Self { identity: host, hosting, screen, _folder: folder }
    }
    fn address(&self) -> SocketAddr {
        self.hosting.tunnel().local_addr().unwrap()
    }
    fn fingerprint(&self) -> String {
        hex(&self.identity.fingerprint())
    }
    /// How many pictures this computer has been asked to take.
    fn captures(&self) -> usize {
        self.screen.taken.lock().unwrap().len()
    }
}

/// The owner's laptop: the background part the window asks, with the throwaway
/// identity and no engine of its own.
async fn laptop_part(folder: &Path, laptop: &Identity) -> Arc<Core<NoEngine>> {
    let account = secpar_core::account::Service::local_test(ORIGIN, "http://127.0.0.1:9").unwrap();
    let part = Core::<NoEngine>::start(Settings { account, state: folder.join("laptop"), bind: "127.0.0.1:0".parse().unwrap() }, laptop.clone(), None).await.unwrap();
    // The real clipboard stays out of these tests.
    part.answer(&json!({"do": "clipboard", "on": false}).to_string()).await;
    part
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn a_still_of_the_real_screen_goes_to_the_owners_device_only_and_is_kept_for_its_window() {
    let folder = tempfile::tempdir().unwrap();
    let (pair, key) = signer();
    let (laptop, friend) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    // Two computers the laptop may connect to, and a friend who may use the
    // keyboard but is never shown a screen.
    let guest = json!({
        "fingerprint": hex(&friend.fingerprint()), "name": "Phone", "account": "id-bob",
        "handle": "bob", "trusted": true, "keyboard": true, "mouse": true,
    });
    let here = Computer::start(&pair, &key, Identity::generate().unwrap(), &[&laptop], vec![guest.clone()]).await;
    let elsewhere = Computer::start(&pair, &key, Identity::generate().unwrap(), &[&laptop], vec![guest]).await;
    let part = laptop_part(folder.path(), &laptop).await;

    // Nothing can be answered before it is connected.
    let early = part.answer(&json!({"do": "preview", "host": here.fingerprint(), "max_width": 480}).to_string()).await;
    assert_eq!((early["ok"].clone(), early["err"].clone()), (json!(false), json!("not_available")), "no addresses, no still");
    assert_eq!(here.captures(), 0);

    let connected = part.answer(&json!({"do": "connect", "host": here.fingerprint(), "addresses": [here.address()]}).to_string()).await;
    assert_eq!(connected["ok"], true, "{connected}");

    // Over the tunnel that is open, without any address given.
    let first = part.answer(&json!({"do": "preview", "host": here.fingerprint(), "max_width": 480}).to_string()).await;
    assert_eq!(first["ok"], true, "no still was taken; is the screen locked? {first}");
    assert_eq!(first["width"], 480, "the still has the width that was asked for");
    let height = first["height"].as_u64().unwrap() as u32;
    assert!(height > 0, "the still has a height");
    let jpeg = base64::engine::general_purpose::STANDARD.decode(first["jpeg"].as_str().unwrap()).unwrap();
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "a JPEG starts with its own marker");
    assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "and ends with it");
    assert!(jpeg.len() < 320 * 1024, "the picture stays small enough for a tile");
    assert!(first["jpeg"].as_str().unwrap().len() < MAX_PREVIEW_FRAME, "the answer fits the app channel");
    let at = first["at"].as_u64().unwrap();
    assert!(at > 0);
    assert_eq!(here.captures(), 1);

    // Straight away again: the same still, not a second capture.
    let second = part.answer(&json!({"do": "preview", "host": here.fingerprint(), "max_width": 480}).to_string()).await;
    assert_eq!((second["at"].as_u64(), second["jpeg"].as_str()), (Some(at), first["jpeg"].as_str()));

    // A request for a much wider still is answered at the limit.
    let wide = part.answer(&json!({"do": "preview", "host": here.fingerprint(), "max_width": 4000}).to_string()).await;
    assert_eq!(wide["width"], MAX_PREVIEW_WIDTH, "{wide}");

    // The other computer is not connected, so its still comes over a tunnel
    // opened for this one question. That tunnel takes no engine port, so the
    // stream tunnel keeps working: it still answers afterwards.
    let peeked = part.answer(&json!({"do": "preview", "host": elsewhere.fingerprint(), "addresses": [elsewhere.address()], "max_width": 480}).to_string()).await;
    assert_eq!(peeked["ok"], true, "a still from a computer this one is not connected to: {peeked}");
    assert_eq!(peeked["width"], 480);
    assert_eq!(elsewhere.captures(), 1);
    let status = part.answer(&json!({"do": "status"}).to_string()).await;
    assert_eq!(status["connected_to_host"], true, "the peek left the open tunnel alone");
    let after = part.answer(&json!({"do": "preview", "host": here.fingerprint(), "max_width": 480}).to_string()).await;
    assert_eq!((after["ok"].clone(), after["at"].as_u64()), (json!(true), Some(at)), "the stream tunnel still answers");

    // A still is only given for one of the account's own computers.
    let stranger = part.answer(&json!({"do": "preview", "host": hex(&friend.fingerprint()), "addresses": [elsewhere.address()], "max_width": 480}).to_string()).await;
    assert_eq!((stranger["ok"].clone(), stranger["err"].clone()), (json!(false), json!("not_available")));

    // A friend who may use the keyboard is still not shown the screen.
    let friend_tunnel = secpar_tunnel::connect(here.address(), &friend, here.identity.fingerprint(), "127.0.0.1".parse().unwrap(), secpar_tunnel::Ports { tcp: vec![], udp: vec![] }).await.unwrap();
    let request = serde_json::to_vec(&json!({"app": {"preview": {"max_width": 480}}})).unwrap();
    let refused: Value = serde_json::from_slice(&friend_tunnel.app_request(&request).await.unwrap()).unwrap();
    assert_eq!((refused["ok"].clone(), refused["err"].clone()), (json!(false), json!("not_allowed")), "a friend gets no still");

    // The width helper the answer is built on.
    assert_eq!((preview_width(Some(4000)), preview_width(Some(1)), preview_width(None)), (MAX_PREVIEW_WIDTH, 16, 480));
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn a_still_comes_from_a_computer_that_is_not_connected_and_offline_ones_are_never_asked() {
    let folder = tempfile::tempdir().unwrap();
    let (pair, key) = signer();
    let laptop = Identity::generate().unwrap();
    let away = Computer::start(&pair, &key, Identity::generate().unwrap(), &[&laptop], vec![]).await;
    let part = laptop_part(folder.path(), &laptop).await;

    // Nothing is connected, and a computer that is not hosting here can still be
    // asked for a still.
    let peek = |addresses: Vec<SocketAddr>| {
        let part = part.clone();
        let host = away.fingerprint();
        async move { part.answer(&json!({"do": "preview", "host": host, "max_width": 480, "addresses": addresses}).to_string()).await }
    };
    let still = peek(vec![away.address()]).await;
    assert_eq!(still["ok"], true, "no still was taken; is the screen locked? {still}");
    assert_eq!(still["width"], 480);
    assert_eq!(away.captures(), 1);

    // The same computer is not asked again straight away: the picture it showed
    // is still what this turn of the list wanted.
    let again = peek(vec![away.address()]).await;
    assert_eq!((again["ok"].clone(), again["err"].clone()), (json!(false), json!("not_available")));
    assert_eq!(away.captures(), 1, "no second tunnel was opened");

    // A computer the account lists as offline is never asked: the window sends
    // no address for one, and an empty list is refused here too.
    let nothing = peek(vec![]).await;
    assert_eq!((nothing["ok"].clone(), nothing["err"].clone()), (json!(false), json!("not_available")));
    assert_eq!(away.captures(), 1, "an offline computer is not asked");
}
