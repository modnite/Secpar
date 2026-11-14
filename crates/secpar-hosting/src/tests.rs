//! Real tunnels and a real access list, with a stand-in for the engine.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::{
    rand::SystemRandom,
    signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
};
use secpar_core::account::{Key, ORIGIN};
use std::sync::Mutex;

#[derive(Default)]
struct FakeEngine {
    /// Answers `NotPending` this many times before accepting.
    slow: Mutex<u32>,
    grants: Mutex<Vec<(String, String, Pairing, String)>>,
    revoked: Mutex<Vec<(String, String)>>,
    /// Stream identities that were taken away.
    dead: Mutex<Vec<String>>,
}
impl Engine for FakeEngine {
    fn approve<'a>(&'a self, owner: &'a str, device: &'a str, pairing: &'a Pairing, name: &'a str) -> Pending<'a, Result<(), Refusal>> {
        Box::pin(async move {
            if self.dead.lock().unwrap().contains(&pairing.certificate_sha256) {
                return Err(Refusal::Revoked);
            }
            let mut slow = self.slow.lock().unwrap();
            if *slow > 0 {
                *slow -= 1;
                return Err(Refusal::NotPending);
            }
            self.grants.lock().unwrap().push((owner.into(), device.into(), pairing.clone(), name.into()));
            Ok(())
        })
    }
    fn revoke<'a>(&'a self, owner: &'a str, device: &'a str) -> Pending<'a, bool> {
        Box::pin(async move {
            let mut grants = self.grants.lock().unwrap();
            for grant in grants.iter().filter(|g| g.0 == owner && g.1 == device) {
                self.dead.lock().unwrap().push(grant.2.certificate_sha256.clone());
            }
            grants.retain(|g| !(g.0 == owner && g.1 == device));
            self.revoked.lock().unwrap().push((owner.into(), device.into()));
            true
        })
    }
    fn granted(&self) -> Pending<'_, Vec<(String, String)>> {
        Box::pin(async move { self.grants.lock().unwrap().iter().map(|g| (g.0.clone(), g.1.clone())).collect() })
    }
}

/// A stand-in for the account service's signing key.
struct Signer {
    pair: EcdsaKeyPair,
    key: Key,
}
fn signer() -> Signer {
    let rng = SystemRandom::new();
    let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref(), &rng).unwrap();
    let point = pair.public_key().as_ref().to_vec();
    let key = Key { kid: "k1".into(), kty: "EC".into(), crv: "P-256".into(), x: URL_SAFE_NO_PAD.encode(&point[1..33]), y: URL_SAFE_NO_PAD.encode(&point[33..]) };
    Signer { pair, key }
}
fn guest(device: &Identity, trusted: bool) -> serde_json::Value {
    serde_json::json!({"fingerprint": hex(&device.fingerprint()), "name": "Phone", "account": "id-bob", "handle": "bob", "trusted": trusted, "keyboard": true, "mouse": false})
}
fn list(signer: &Signer, host: &Identity, own: &[&Identity], guests: Vec<serde_json::Value>) -> String {
    let header = serde_json::json!({"alg": "ES256", "typ": "secpar-access+jwt", "kid": "k1"});
    let claims = serde_json::json!({
        "iss": ORIGIN, "aud": "secpar-host", "sub": hex(&host.fingerprint()), "iat": now() - 10, "exp": now() + 3600, "owner": "id-alice",
        "devices": own.iter().map(|d| serde_json::json!({"fingerprint": hex(&d.fingerprint()), "name": "Laptop"})).collect::<Vec<_>>(),
        "guests": guests,
    });
    let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
    let signature = signer.pair.sign(&SystemRandom::new(), input.as_bytes()).unwrap();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}
fn pairing(certificate: &str) -> Pairing {
    Pairing { client_id: "0123456789ABCDEF".into(), certificate_sha256: certificate.repeat(32), pin: "1234".into() }
}

struct World {
    signer: Signer,
    host: Identity,
    laptop: Identity,
    phone: Identity,
    engine: Arc<FakeEngine>,
    hosting: Hosting<FakeEngine>,
    _folder: tempfile::TempDir,
}
impl World {
    /// A hosting device whose list has the owner's laptop and bob's phone.
    async fn new(phone_trusted: bool) -> Self {
        let (signer, host, laptop, phone) = (signer(), Identity::generate().unwrap(), Identity::generate().unwrap(), Identity::generate().unwrap());
        let folder = tempfile::tempdir().unwrap();
        let access = Arc::new(Access::open(ORIGIN, host.fingerprint(), folder.path().join("access.json"), now()));
        access.update(&list(&signer, &host, &[&laptop], vec![guest(&phone, phone_trusted)]), &[signer.key.clone()], now()).unwrap();
        let engine = Arc::new(FakeEngine::default());
        let none = secpar_tunnel::Ports { tcp: vec![], udp: vec![] };
        let hosting = Hosting::start("127.0.0.1:0".parse().unwrap(), &host, access, engine.clone(), "127.0.0.1".parse().unwrap(), none).await.unwrap();
        Self { signer, host, laptop, phone, engine, hosting, _folder: folder }
    }
    async fn connect(&self, device: &Identity) -> secpar_tunnel::Result<secpar_tunnel::Client> {
        let none = secpar_tunnel::Ports { tcp: vec![], udp: vec![] };
        secpar_tunnel::connect(self.hosting.tunnel().local_addr().unwrap(), device, self.host.fingerprint(), "127.0.0.1".parse().unwrap(), none).await
    }
    fn update(&self, own: &[&Identity], guests: Vec<serde_json::Value>) {
        self.hosting.access().update(&list(&self.signer, &self.host, own, guests), &[self.signer.key.clone()], now()).unwrap();
    }
}
async fn gone(tunnel: &secpar_tunnel::Client) -> bool {
    tokio::time::timeout(Duration::from_secs(5), tunnel.closed()).await.is_ok()
}

#[tokio::test]
async fn an_own_device_pairs_through_the_tunnel_under_the_owners_account() {
    let world = World::new(false).await;
    let tunnel = world.connect(&world.laptop).await.unwrap();
    // The engine needs a moment before it shows the attempt.
    *world.engine.slow.lock().unwrap() = 2;
    assert_eq!(request_pairing(&tunnel, &pairing("aa")).await, Answer::Paired);
    let grants = world.engine.grants.lock().unwrap().clone();
    assert_eq!(grants.len(), 1);
    assert_eq!((grants[0].0.as_str(), grants[0].1.clone(), grants[0].3.as_str()), ("id-alice", hex(&world.laptop.fingerprint()), "Laptop"));
    // Anything else on the channel is not understood.
    assert_eq!(tunnel.request(b"{\"stop\":1}").await.unwrap(), b"\"unknown\"");
}

#[tokio::test]
async fn a_friend_who_is_asked_about_gets_in_only_while_accepted_and_keeps_the_pairing() {
    let world = World::new(false).await;
    assert!(world.connect(&world.phone).await.is_err());
    assert!(world.hosting.accept(&world.phone.fingerprint(), now() + 600).await);
    let tunnel = world.connect(&world.phone).await.unwrap();
    assert_eq!(request_pairing(&tunnel, &pairing("bb")).await, Answer::Paired);
    {
        let grants = world.engine.grants.lock().unwrap();
        // Recorded under the friend's own account, with a name from the list.
        assert_eq!((grants[0].0.as_str(), grants[0].3.as_str()), ("id-bob", "bob: Phone"));
    }
    world.hosting.dismiss(&world.phone.fingerprint()).await;
    assert!(gone(&tunnel).await);
    assert!(world.connect(&world.phone).await.is_err());
    // Out, but still a friend on the list: the pairing is not taken away.
    assert!(world.engine.revoked.lock().unwrap().is_empty());
    // A stranger cannot be accepted at all.
    let stranger = Identity::generate().unwrap();
    assert!(!world.hosting.accept(&stranger.fingerprint(), now() + 600).await);
    assert!(world.connect(&stranger).await.is_err());
}

#[tokio::test]
async fn a_device_that_leaves_the_list_loses_the_tunnel_and_the_pairing() {
    let world = World::new(true).await;
    let (laptop, phone) = (world.connect(&world.laptop).await.unwrap(), world.connect(&world.phone).await.unwrap());
    assert_eq!(request_pairing(&laptop, &pairing("aa")).await, Answer::Paired);
    assert_eq!(request_pairing(&phone, &pairing("bb")).await, Answer::Paired);
    // The laptop is removed from the account; bob stays a trusted friend.
    world.update(&[], vec![guest(&world.phone, true)]);
    world.hosting.apply().await;
    assert!(gone(&laptop).await);
    assert!(!gone_soon(&phone).await);
    assert_eq!(*world.engine.revoked.lock().unwrap(), vec![("id-alice".to_string(), hex(&world.laptop.fingerprint()))]);
    assert!(world.connect(&world.laptop).await.is_err());
    // Back on the account later: the old stream identity stays dead, a new one works.
    world.update(&[&world.laptop], vec![guest(&world.phone, true)]);
    world.hosting.apply().await;
    let again = world.connect(&world.laptop).await.unwrap();
    assert_eq!(request_pairing(&again, &pairing("aa")).await, Answer::Revoked);
    assert_eq!(request_pairing(&again, &pairing("cc")).await, Answer::Paired);
}
async fn gone_soon(tunnel: &secpar_tunnel::Client) -> bool {
    tokio::time::timeout(Duration::from_millis(300), tunnel.closed()).await.is_ok()
}

#[tokio::test]
async fn a_request_after_access_ended_is_refused_even_on_an_open_tunnel() {
    let world = World::new(true).await;
    let tunnel = world.connect(&world.phone).await.unwrap();
    // The list changes, and the device asks before the change was applied to the tunnel.
    world.update(&[&world.laptop], vec![guest(&world.phone, false)]);
    assert_eq!(request_pairing(&tunnel, &pairing("bb")).await, Answer::NotAllowed);
    assert!(world.engine.grants.lock().unwrap().is_empty());
}

#[tokio::test]
async fn without_a_list_nobody_gets_in_and_nothing_is_revoked() {
    let world = World::new(true).await;
    let tunnel = world.connect(&world.laptop).await.unwrap();
    assert_eq!(request_pairing(&tunnel, &pairing("aa")).await, Answer::Paired);
    world.hosting.access().clear();
    world.hosting.apply().await;
    assert!(gone(&tunnel).await);
    assert!(world.connect(&world.laptop).await.is_err());
    // Losing the list is not the same as the device leaving it.
    assert!(world.engine.revoked.lock().unwrap().is_empty());
}
