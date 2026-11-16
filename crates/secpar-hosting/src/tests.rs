//! Real tunnels and a real access list, with a stand-in for the engine.
use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
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
    /// Stream sizes a device asked for, in order.
    sizes: Mutex<Vec<(u32, u32)>>,
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
    fn resolution(&self, width: u32, height: u32) -> Pending<'_, Result<bool, Refusal>> {
        Box::pin(async move {
            self.sizes.lock().unwrap().push((width, height));
            Ok(true)
        })
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

/// A stand-in for this computer's screen: it counts the pictures it is asked to
/// take and describes each one with its own width.
#[derive(Default)]
struct FakeScreen {
    taken: Mutex<Vec<u32>>,
}
impl Previews for FakeScreen {
    fn still(&self, max_width: u32) -> Option<Still> {
        self.taken.lock().unwrap().push(max_width);
        // The smallest JPEG the answer check accepts: a start and an end marker.
        Some(Still { jpeg: vec![0xFF, 0xD8, 0xFF, 0xD9], width: max_width, height: max_width * 9 / 16, at: now() })
    }
}

struct World {
    signer: Signer,
    host: Identity,
    laptop: Identity,
    phone: Identity,
    engine: Arc<FakeEngine>,
    screen: Arc<FakeScreen>,
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
        let screen = Arc::new(FakeScreen::default());
        let none = secpar_tunnel::Ports { tcp: vec![], udp: vec![] };
        let hosting = Hosting::start("127.0.0.1:0".parse().unwrap(), &host, access, engine.clone(), Arc::new(NoClipboard), Arc::new(NoFiles), screen.clone(), "127.0.0.1".parse().unwrap(), none).await.unwrap();
        Self { signer, host, laptop, phone, engine, screen, hosting, _folder: folder }
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
async fn only_the_owners_own_device_changes_the_stream_size() {
    let world = World::new(false).await;
    let laptop = world.connect(&world.laptop).await.unwrap();
    assert_eq!(set_resolution(&laptop, 1280, 720).await, Ok(true));
    assert_eq!(*world.engine.sizes.lock().unwrap(), vec![(1280, 720)]);
    // A friend watching does not choose the size of the picture on this computer.
    assert!(world.hosting.accept(&world.phone.fingerprint(), now() + 600).await);
    let phone = world.connect(&world.phone).await.unwrap();
    assert_eq!(set_resolution(&phone, 640, 480).await, Err("not_allowed"));
    assert_eq!(world.engine.sizes.lock().unwrap().len(), 1);
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

#[tokio::test]
async fn a_still_of_the_screen_is_given_to_the_owners_own_device_only() {
    let world = World::new(false).await;
    let laptop = world.connect(&world.laptop).await.unwrap();
    let still = preview(&laptop, 480).await.expect("the owner's laptop sees this computer's screen");
    assert_eq!((still.width, still.height), (480, 270));
    assert_eq!(&still.jpeg[..2], &[0xFF, 0xD8]);
    assert!(still.at > 0);
    assert_eq!(*world.screen.taken.lock().unwrap(), vec![480]);
    // A friend who may use the keyboard is still not shown the screen.
    assert!(world.hosting.accept(&world.phone.fingerprint(), now() + 600).await);
    let phone = world.connect(&world.phone).await.unwrap();
    assert!(preview(&phone, 480).await.is_none());
    assert_eq!(*world.screen.taken.lock().unwrap(), vec![480], "a friend's request takes no picture");
}

#[tokio::test]
async fn a_still_is_kept_for_its_window_and_the_width_is_clamped_both_ways() {
    let world = World::new(false).await;
    let laptop = world.connect(&world.laptop).await.unwrap();
    assert_eq!(preview(&laptop, 4000).await.unwrap().width, MAX_PREVIEW_WIDTH);
    assert_eq!(preview(&laptop, 1).await.unwrap().width, MIN_PREVIEW_WIDTH);
    assert_eq!(preview(&laptop, 0).await.unwrap().width, DEFAULT_PREVIEW_WIDTH);
    // Each width is its own picture, and each was taken once.
    assert_eq!(*world.screen.taken.lock().unwrap(), vec![MAX_PREVIEW_WIDTH, MIN_PREVIEW_WIDTH, DEFAULT_PREVIEW_WIDTH]);
    // Asking again straight away is answered from the cache: no second capture.
    assert_eq!(preview(&laptop, 440).await.unwrap().width, 440);
    assert!(preview(&laptop, 440).await.is_some());
    assert!(preview(&laptop, 440).await.is_some());
    let taken = world.screen.taken.lock().unwrap().clone();
    assert_eq!(taken, vec![MAX_PREVIEW_WIDTH, MIN_PREVIEW_WIDTH, DEFAULT_PREVIEW_WIDTH, 440], "one capture per width inside the window");
}

#[tokio::test]
async fn a_computer_with_no_screen_to_show_answers_not_available() {
    let (signer, host, laptop) = (signer(), Identity::generate().unwrap(), Identity::generate().unwrap());
    let folder = tempfile::tempdir().unwrap();
    let access = Arc::new(Access::open(ORIGIN, host.fingerprint(), folder.path().join("access.json"), now()));
    access.update(&list(&signer, &host, &[&laptop], vec![]), &[signer.key.clone()], now()).unwrap();
    let none = secpar_tunnel::Ports { tcp: vec![], udp: vec![] };
    let hosting = Hosting::start(
        "127.0.0.1:0".parse().unwrap(),
        &host,
        access,
        Arc::new(FakeEngine::default()),
        Arc::new(NoClipboard),
        Arc::new(NoFiles),
        Arc::new(NoPreviews),
        "127.0.0.1".parse().unwrap(),
        none.clone(),
    )
    .await
    .unwrap();
    let tunnel = secpar_tunnel::connect(hosting.tunnel().local_addr().unwrap(), &laptop, host.fingerprint(), "127.0.0.1".parse().unwrap(), none).await.unwrap();
    let request = serde_json::to_vec(&serde_json::json!({"app": {"preview": {"max_width": 480}}})).unwrap();
    let answer: serde_json::Value = serde_json::from_slice(&tunnel.app_request(&request).await.unwrap()).unwrap();
    // A computer with nothing to capture gives the protocol's refusal, not an error.
    assert_eq!((answer["ok"].clone(), answer["err"].clone()), (serde_json::json!(false), serde_json::json!("not_available")));
    assert!(preview(&tunnel, 480).await.is_none());
}
