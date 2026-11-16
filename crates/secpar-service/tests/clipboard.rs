//! Two background parts' clipboard sides over a real tunnel on loopback, with
//! stand-ins for the two OS clipboards: text copied on one computer appears on
//! the other, a guest who may not use the keyboard shares nothing, and text over
//! the limit is refused.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::{
    rand::SystemRandom,
    signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
};
use secpar_core::{
    account::{Key, ORIGIN},
    hosting::Access,
};
use secpar_clipboard::Bridge;
use secpar_hosting::{Engine, Hosting, Pairing, Pending, Refusal};
use secpar_service::clipboard;
use secpar_tunnel::Identity;
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
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let header = serde_json::json!({"alg": "ES256", "typ": "secpar-access+jwt", "kid": "k1"});
    let claims = serde_json::json!({
        "iss": ORIGIN, "aud": "secpar-host", "sub": secpar_core::identity::hex(&host.fingerprint()), "iat": now - 10, "exp": now + 3600, "owner": "id-alice",
        "devices": own.iter().map(|d| serde_json::json!({"fingerprint": secpar_core::identity::hex(&d.fingerprint()), "name": "Laptop"})).collect::<Vec<_>>(),
        "guests": guests,
    });
    let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
    let signature = pair.sign(&SystemRandom::new(), input.as_bytes()).unwrap();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

/// One computer's clipboard, outside the OS.
#[derive(Clone)]
struct Text(Arc<Mutex<String>>);
impl Text {
    fn new(text: &str) -> Self {
        Self(Arc::new(Mutex::new(text.into())))
    }
    fn copied(&self) -> String {
        self.0.lock().unwrap().clone()
    }
    fn copy(&self, text: &str) {
        *self.0.lock().unwrap() = text.into();
    }
    /// This computer's clipboard as the background part uses it.
    fn shared(&self) -> Arc<clipboard::Shared> {
        let text = self.clone();
        clipboard::Shared::with(true, move || Bridge::on({ let text = text.clone(); move || Ok(text.clone()) }))
    }
}
impl secpar_clipboard::Backend for Text {
    fn read(&mut self) -> Option<String> {
        Some(self.copied())
    }
    fn write(&mut self, text: &str) -> bool {
        self.copy(text);
        true
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let end = tokio::time::Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(tokio::time::Instant::now() < end, "did not happen in time: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn clipboard_text_travels_both_ways_and_guests_and_oversized_text_are_refused() {
    let folder = tempfile::tempdir().unwrap();
    let (signer, key) = signer();
    let (host, laptop, guest) = (Identity::generate().unwrap(), Identity::generate().unwrap(), Identity::generate().unwrap());
    let (desktop, phone) = (Text::new("desktop start"), Text::new("laptop start"));
    let (desktop_part, laptop_part) = (desktop.shared(), phone.shared());
    let access = Arc::new(Access::open(ORIGIN, host.fingerprint(), folder.path().join("access.json"), now()));
    access
        .update(
            &list(&signer, &host, &[&laptop], vec![serde_json::json!({
                "fingerprint": secpar_core::identity::hex(&guest.fingerprint()), "name": "Phone", "account": "id-bob",
                "handle": "bob", "trusted": true, "keyboard": false, "mouse": false,
            })]),
            &[key],
            now(),
        )
        .unwrap();
    let none = secpar_tunnel::Ports { tcp: vec![], udp: vec![] };
    let hosting = Hosting::start(
        "127.0.0.1:0".parse().unwrap(),
        &host,
        access,
        Arc::new(NoEngine),
        desktop_part.clone(),
        Arc::new(secpar_hosting::NoFiles),
        Arc::new(secpar_hosting::NoPreviews),
        "127.0.0.1".parse().unwrap(),
        none.clone(),
    )
    .await
    .unwrap();
    let address = hosting.tunnel().local_addr().unwrap();
    let hosting = Arc::new(hosting);
    {
        let hosting = hosting.clone();
        tokio::spawn(async move { hosting.keep_clipboard().await });
    }

    // The owner's laptop connects and keeps its clipboard in step.
    let tunnel = Arc::new(
        secpar_tunnel::connect(address, &laptop, host.fingerprint(), "127.0.0.1".parse().unwrap(), none.clone()).await.unwrap(),
    );
    tokio::spawn(clipboard::follow(laptop_part.clone(), Arc::downgrade(&tunnel)));

    // What is already there is never shared.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(desktop.copied(), "desktop start");
    assert_eq!(phone.copied(), "laptop start");

    // Copied on the laptop, appears on the desktop.
    phone.copy("from the laptop");
    until("the laptop's text reaches the desktop", || desktop.copied() == "from the laptop").await;

    // Copied on the desktop, appears on the laptop.
    desktop.copy("from the desktop");
    until("the desktop's text reaches the laptop", || phone.copied() == "from the desktop").await;

    // A friend who may not use the keyboard shares nothing.
    let other = secpar_tunnel::connect(address, &guest, host.fingerprint(), "127.0.0.1".parse().unwrap(), none).await.unwrap();
    assert!(!secpar_hosting::set_clipboard(&other, "not for you").await);
    assert!(secpar_hosting::clipboard_text(&other).await.is_none());
    assert_eq!(desktop.copied(), "from the desktop");

    // Text over the limit is refused, and what is in the clipboard stays.
    let too_long = "x".repeat(secpar_clipboard::MAX_TEXT + 1);
    assert!(!secpar_hosting::set_clipboard(&tunnel, &too_long).await);
    assert_eq!(desktop.copied(), "from the desktop");
    assert!(secpar_hosting::set_clipboard(&tunnel, "small enough").await);
    until("a later copy still arrives", || desktop.copied() == "small enough").await;
}
