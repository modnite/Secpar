//! The Secpar account service, as the app sees it: sign in, enrol this device
//! under the account, and fetch the signed list of who may connect to it.
//!
//! The account is the single source of truth for access. A hosting device keeps
//! the last signed list and the service's public keys, and `verify_access` checks
//! that copy again without the network, so LAN connections keep working while the
//! service is unreachable.
use crate::{
    identity::hex,
    store::{SecretStore, Unavailable},
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use ring::{
    rand::SystemRandom,
    signature::{EcdsaKeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING},
};
use secpar_tunnel::{Fingerprint, Identity};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const ORIGIN: &str = "https://accounts.secpar.win";
const SESSION: &str = "account-session";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No answer from the service. Nothing is known about the account's state.
    Unreachable,
    /// The service answered and refused. `message` is its own wording when it
    /// sent one meant for the user.
    Refused { status: u16, code: String, message: Option<String> },
    /// An answer or a signed list that cannot be trusted or understood.
    Invalid(&'static str),
    Store(Unavailable),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable => f.write_str("The Secpar account service could not be reached"),
            Self::Refused { message: Some(message), .. } => f.write_str(message),
            Self::Refused { code, .. } => write!(f, "The Secpar account service refused: {code}"),
            Self::Invalid(what) => f.write_str(what),
            Self::Store(_) => f.write_str("The system credential store could not be used"),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub handle: String,
}
/// A signed-in session. The token is a secret: keep it in the credential store.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub expires_at: u64,
    pub user: User,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session for {}", self.user.handle)
    }
}
/// `Ok(None)` only when the store answered and nobody is signed in.
pub fn saved_session(store: &dyn SecretStore) -> Result<Option<Session>> {
    match store.load(SESSION).map_err(Error::Store)? {
        None => Ok(None),
        // An unreadable entry is treated as signed out; signing in replaces it.
        Some(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
    }
}
pub fn save_session(store: &dyn SecretStore, session: &Session) -> Result<()> {
    let bytes = serde_json::to_vec(session).map_err(|_| Error::Invalid("Session could not be saved"))?;
    store.save(SESSION, &bytes).map_err(Error::Store)
}
pub fn forget_session(store: &dyn SecretStore) -> Result<()> {
    store.forget(SESSION).map_err(Error::Store)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
    Android,
}
impl Platform {
    fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Android => "android",
        }
    }
    /// The platform this build runs on, if Secpar supports it.
    pub fn current() -> Option<Self> {
        match std::env::consts::OS {
            "windows" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            "android" => Some(Self::Android),
            _ => None,
        }
    }
}
/// One enrolled installation, as the account service lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Device {
    pub id: String,
    pub fingerprint: String,
    pub name: String,
    pub platform: String,
    pub hosts: bool,
    /// A hosting device that reported in recently.
    #[serde(default)]
    pub online: bool,
    /// Where it said it can be reached, while it is online.
    #[serde(default)]
    pub reach: Option<Reach>,
}
/// Where a hosting device can be reached: its tunnel port and the addresses it
/// has on its own network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reach {
    pub port: u16,
    pub addresses: Vec<String>,
}

/// One person on the account's friend list, or a request waiting for an answer.
/// `requester` is the account that asked, so the same row can mean either side.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Friend {
    pub id: String,
    pub handle: String,
    #[serde(default)]
    pub name: String,
    /// "pending" while one side has not answered, then "accepted".
    pub state: String,
    pub requester: String,
    /// "view" or "full".
    pub input: String,
    /// "ask" or "always": whether this friend joins without being asked about.
    pub connect: String,
    #[serde(default)]
    pub keyboard: bool,
    #[serde(default)]
    pub mouse: bool,
}

/// What one account allows a friend's devices to do on its computers. The
/// service keeps `input` in step with the two finer permissions: it is "full"
/// only while both keyboard and mouse are allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendSettings {
    /// "view" or "full".
    pub input: String,
    /// "ask" or "always": whether the friend joins without being asked about.
    pub connect: String,
    #[serde(default)]
    pub keyboard: bool,
    #[serde(default)]
    pub mouse: bool,
}

/// A change to a friend's permissions. Parts that are `None` are left as they
/// are. Sending `keyboard` or `mouse` makes the service derive `input` from both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FriendChange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse: Option<bool>,
}
impl FriendChange {
    /// Nothing to send. The service refuses an empty change, so it is caught here.
    pub fn is_empty(&self) -> bool {
        self.input.is_none() && self.connect.is_none() && self.keyboard.is_none() && self.mouse.is_none()
    }
    /// Checks the values the service accepts, so a mistake is caught before it is sent.
    fn checked(&self) -> Result<()> {
        if self.input.as_deref().is_some_and(|input| input != "view" && input != "full") {
            return Err(Error::Invalid("A friend's control setting can only be view or full"));
        }
        if self.connect.as_deref().is_some_and(|connect| connect != "ask" && connect != "always") {
            return Err(Error::Invalid("A friend's connect setting can only be ask or always"));
        }
        if self.is_empty() {
            return Err(Error::Invalid("That friend's permissions were not changed"));
        }
        Ok(())
    }
}

/// What a device needs to use the relay for a few hours.
#[derive(Debug, Clone, Deserialize)]
pub struct RelayAccess {
    pub server: String,
    pub username: String,
    pub credential: String,
}

/// A device asking a hosting device to meet it at the relay, or the answer:
/// the device's fingerprint and the "address:port" to use for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Knock {
    pub device: String,
    pub candidates: Vec<String>,
}

/// A public key the service signs access lists with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub kid: String,
    pub kty: String,
    pub crv: String,
    pub x: String,
    pub y: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnDevice {
    pub fingerprint: Fingerprint,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guest {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub account: String,
    pub handle: String,
    /// Trusted friends connect without being asked about; others must be accepted each time.
    pub trusted: bool,
    pub keyboard: bool,
    pub mouse: bool,
}
/// Who may connect to one hosting device, as signed by the account service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessList {
    pub owner: String,
    pub devices: Vec<OwnDevice>,
    pub guests: Vec<Guest>,
    pub expires_at: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
/// A device or account id from the service: a UUID, never anything else.
fn identifier(text: &str) -> bool {
    text.len() == 36 && text.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}
fn fingerprint_of(text: &str) -> Result<Fingerprint> {
    let bad = Error::Invalid("The access list names a device in a form Secpar does not understand");
    if text.len() != 64 || !text.is_ascii() {
        return Err(bad);
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|_| bad.clone())?;
    }
    Ok(out)
}

/// Checks a signed access list: signed by one of `keys`, issued by `origin` for
/// the hosting device `host`, and not expired at `at` (seconds since 1970).
pub fn verify_access(access: &str, keys: &[Key], origin: &str, host: &Fingerprint, at: u64) -> Result<AccessList> {
    const BAD: Error = Error::Invalid("The access list is not validly signed by the Secpar account service");
    #[derive(Deserialize)]
    struct Header {
        alg: String,
        typ: String,
        kid: String,
    }
    #[derive(Deserialize)]
    struct Named {
        fingerprint: String,
        name: String,
    }
    #[derive(Deserialize)]
    struct Friend {
        fingerprint: String,
        name: String,
        account: String,
        handle: String,
        trusted: bool,
        keyboard: bool,
        mouse: bool,
    }
    #[derive(Deserialize)]
    struct Claims {
        iss: String,
        aud: String,
        sub: String,
        iat: u64,
        exp: u64,
        owner: String,
        devices: Vec<Named>,
        guests: Vec<Friend>,
    }
    let mut parts = access.split('.');
    let (Some(head), Some(body), Some(signature), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(BAD);
    };
    let decode = |part: &str| URL_SAFE_NO_PAD.decode(part).map_err(|_| BAD);
    let header: Header = serde_json::from_slice(&decode(head)?).map_err(|_| BAD)?;
    if header.alg != "ES256" || header.typ != "secpar-access+jwt" {
        return Err(BAD);
    }
    let key = keys.iter().find(|k| k.kid == header.kid && k.kty == "EC" && k.crv == "P-256").ok_or(BAD)?;
    let (x, y) = (decode(&key.x)?, decode(&key.y)?);
    if x.len() != 32 || y.len() != 32 {
        return Err(BAD);
    }
    let point = [&[4u8][..], &x, &y].concat();
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
        .verify(&access.as_bytes()[..head.len() + 1 + body.len()], &decode(signature)?)
        .map_err(|_| BAD)?;
    let claims: Claims = serde_json::from_slice(&decode(body)?).map_err(|_| BAD)?;
    if claims.iss != origin || claims.aud != "secpar-host" {
        return Err(BAD);
    }
    if claims.sub != hex(host) {
        return Err(Error::Invalid("The access list is for another device"));
    }
    // Ten minutes of allowance for a clock that runs behind the service.
    if claims.exp <= at || claims.iat > at.saturating_add(600) {
        return Err(Error::Invalid("The access list has expired"));
    }
    Ok(AccessList {
        owner: claims.owner,
        devices: claims
            .devices
            .into_iter()
            .map(|d| Ok(OwnDevice { fingerprint: fingerprint_of(&d.fingerprint)?, name: d.name }))
            .collect::<Result<_>>()?,
        guests: claims
            .guests
            .into_iter()
            .map(|g| {
                Ok(Guest {
                    fingerprint: fingerprint_of(&g.fingerprint)?,
                    name: g.name,
                    account: g.account,
                    handle: g.handle,
                    trusted: g.trusted,
                    keyboard: g.keyboard,
                    mouse: g.mouse,
                })
            })
            .collect::<Result<_>>()?,
        expires_at: claims.exp,
    })
}

/// A one-use enrolment challenge from the service. Not a secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub challenge: String,
    pub owner: String,
    pub origin: String,
    pub issued_at: u64,
    pub expires_at: u64,
}
/// A device certificate with a token signed by its key. Not a secret, and only
/// usable for the one purpose the token names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proof {
    pub certificate: String,
    pub proof: String,
}
/// A token signed with the device certificate's own key.
fn token(identity: &Identity, typ: &str, claims: serde_json::Value) -> Result<String> {
    let header = serde_json::json!({"alg": "ES256", "typ": typ});
    let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
    let rng = SystemRandom::new();
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, identity.key_der(), &rng)
        .map_err(|_| Error::Invalid("The device identity cannot sign for the account service"))?;
    let signature = key.sign(&rng, input.as_bytes()).map_err(|_| Error::Invalid("The device identity cannot sign for the account service"))?;
    Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref())))
}
/// The enrolment proof: names the account, the one-use challenge and this
/// certificate. On a hosting computer the part that holds the device key makes
/// this, and the signed-in app hands it to the service, so neither the password
/// nor the session ever reaches the part that runs before login.
pub fn enrolment(identity: &Identity, challenge: &Challenge, origin: &str, name: &str, platform: Platform, hosts: bool) -> Result<Proof> {
    const BAD: Error = Error::Invalid("The account service sent an enrolment challenge Secpar cannot use");
    let lower_hex = |s: &str| s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if challenge.origin != origin
        || challenge.challenge.len() != 64
        || !lower_hex(&challenge.challenge)
        || challenge.owner.is_empty()
        || challenge.expires_at <= challenge.issued_at
        || challenge.expires_at - challenge.issued_at > 300
    {
        return Err(BAD);
    }
    let name = name.trim();
    if name.is_empty() || name.encode_utf16().count() > 80 {
        return Err(Error::Invalid("Use a device name of 1 to 80 characters"));
    }
    let claims = serde_json::json!({
        "iss": "secpar-device", "aud": origin, "sub": challenge.owner,
        "purpose": "secpar-device", "challenge": challenge.challenge,
        "fingerprint": hex(&identity.fingerprint()),
        "name": name, "platform": platform.as_str(), "hosts": hosts,
        // The service's own time, so a wrong clock here cannot spoil the proof.
        "iat": challenge.issued_at, "exp": (challenge.issued_at + 90).min(challenge.expires_at),
    });
    Ok(Proof { certificate: pem(identity.certificate_der()), proof: token(identity, "secpar-device+jwt", claims)? })
}
fn pem(certificate: &[u8]) -> String {
    let encoded = STANDARD.encode(certificate);
    let lines: Vec<&str> = encoded.as_bytes().chunks(64).map(|line| std::str::from_utf8(line).unwrap_or_default()).collect();
    format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", lines.join("\n"))
}

/// A connection to the account service.
pub struct Service {
    origin: String,
    address: String,
    http: reqwest::Client,
}
impl Service {
    /// `origin` is `ORIGIN` in the app.
    pub fn new(origin: &str) -> Result<Self> {
        Self::build(origin, origin)
    }
    /// A test service on this computer: it calls itself `origin`, and is reached
    /// over plain HTTP at `address`, which must be on 127.0.0.1.
    pub fn local_test(origin: &str, address: &str) -> Result<Self> {
        match address.strip_prefix("http://127.0.0.1:").map(str::parse::<u16>) {
            Some(Ok(_)) => Self::build(origin, address),
            _ => Err(Error::Invalid("The account service address is not usable")),
        }
    }
    fn build(origin: &str, address: &str) -> Result<Self> {
        let local = address != origin;
        if !origin.starts_with("https://") || origin.ends_with('/') {
            return Err(Error::Invalid("The account service address is not usable"));
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let builder = reqwest::Client::builder();
        // Android: check the service's certificate against the bundled list of
        // certificate authorities instead of asking the system through Java.
        #[cfg(target_os = "android")]
        let builder = {
            let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|_| Error::Invalid("The connection to the account service could not be set up"))?
                .with_root_certificates(roots)
                .with_no_client_auth();
            builder.tls_backend_preconfigured(tls)
        };
        let http = builder
            .https_only(!local)
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Secpar")
            .build()
            .map_err(|_| Error::Invalid("The connection to the account service could not be set up"))?;
        Ok(Self { origin: origin.into(), address: address.into(), http })
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        #[derive(Deserialize)]
        struct Refusal {
            error: Option<String>,
            message: Option<String>,
        }
        let answer = request.send().await.map_err(|_| Error::Unreachable)?;
        if answer.status().is_success() {
            return Ok(answer);
        }
        let status = answer.status().as_u16();
        let refusal: Option<Refusal> = answer.json().await.ok();
        Err(Error::Refused {
            status,
            code: refusal.as_ref().and_then(|r| r.error.clone()).unwrap_or_else(|| "unknown".into()),
            message: refusal.and_then(|r| r.message),
        })
    }
    async fn json<T: DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> Result<T> {
        self.send(request).await?.json().await.map_err(|_| Error::Invalid("The account service sent an answer Secpar does not understand"))
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.address)
    }

    pub async fn sign_in(&self, email: &str, password: &str) -> Result<Session> {
        let body = serde_json::json!({"email": email, "password": password});
        self.json(self.http.post(self.url("/v1/app/sign-in")).json(&body)).await
    }

    /// Enrols this installation under the signed-in account. Doing it again with
    /// the same identity updates the name and role; it never makes a second device.
    pub async fn enrol(&self, session: &Session, identity: &Identity, name: &str, platform: Platform, hosts: bool) -> Result<Device> {
        let challenge = self.challenge(session).await?;
        self.submit(session, &enrolment(identity, &challenge, &self.origin, name, platform, hosts)?).await
    }
    /// The first half of enrolling, for when another part holds the device key.
    pub async fn challenge(&self, session: &Session) -> Result<Challenge> {
        let challenge: Challenge = self.json(self.http.post(self.url("/v1/devices/challenge")).bearer_auth(&session.token)).await?;
        if challenge.owner != session.user.id {
            return Err(Error::Invalid("The account service sent an enrolment challenge Secpar cannot use"));
        }
        Ok(challenge)
    }
    /// The second half: hands the proof made by `enrolment` to the service.
    pub async fn submit(&self, session: &Session, proof: &Proof) -> Result<Device> {
        self.json(self.http.post(self.url("/v1/devices")).bearer_auth(&session.token).json(proof)).await
    }

    /// Fetches and checks this hosting device's access list using only its own
    /// key, so it works before anyone has logged in to the computer. Returns the
    /// signed text to save, and what it says.
    ///
    /// `reach` tells the account's other devices where this one can be reached
    /// from now on; without it this device shows as offline.
    pub async fn device_access(&self, identity: &Identity, keys: &[Key], reach: Option<&Reach>) -> Result<(String, AccessList)> {
        #[derive(Deserialize)]
        struct Time {
            now: u64,
        }
        #[derive(Deserialize)]
        struct Signed {
            access: String,
        }
        // Dated with the service's time, so a wrong clock here does not matter.
        let Time { now: at } = self.json(self.http.get(self.url("/v1/device/time"))).await?;
        let host = identity.fingerprint();
        let claims = serde_json::json!({
            "iss": "secpar-device", "aud": self.origin, "sub": hex(&host),
            "purpose": "secpar-access", "iat": at, "exp": at + 60, "reach": reach,
        });
        let body = Proof { certificate: pem(identity.certificate_der()), proof: token(identity, "secpar-device-request+jwt", claims)? };
        let signed: Signed = self.json(self.http.post(self.url("/v1/device/access")).json(&body)).await?;
        let list = verify_access(&signed.access, keys, &self.origin, &host, at)?;
        Ok((signed.access, list))
    }

    /// A request signed with the device key, for a hosting device.
    async fn as_device<T: DeserializeOwned>(&self, identity: &Identity, path: &str, purpose: &str, answers: &[Knock]) -> Result<T> {
        #[derive(Deserialize)]
        struct Time {
            now: u64,
        }
        let Time { now: at } = self.json(self.http.get(self.url("/v1/device/time"))).await?;
        let claims = serde_json::json!({
            "iss": "secpar-device", "aud": self.origin, "sub": hex(&identity.fingerprint()),
            "purpose": purpose, "iat": at, "exp": at + 60, "answers": answers,
        });
        let body = Proof { certificate: pem(identity.certificate_der()), proof: token(identity, "secpar-device-request+jwt", claims)? };
        self.json(self.http.post(self.url(path)).timeout(Duration::from_secs(40)).json(&body)).await
    }
    /// Relay access for a hosting device.
    pub async fn device_relay(&self, identity: &Identity) -> Result<RelayAccess> {
        self.as_device(identity, "/v1/device/relay", "secpar-relay", &[]).await
    }
    /// Hands in answers to earlier knocks, then waits up to twenty seconds for
    /// devices asking this hosting device to meet them at the relay.
    pub async fn device_knocks(&self, identity: &Identity, answers: &[Knock]) -> Result<Vec<Knock>> {
        #[derive(Deserialize)]
        struct Knocks {
            knocks: Vec<Knock>,
        }
        Ok(self.as_device::<Knocks>(identity, "/v1/device/knocks", "secpar-knocks", answers).await?.knocks)
    }
    /// Relay access for the signed-in person's device.
    pub async fn relay(&self, session: &Session) -> Result<RelayAccess> {
        self.json(self.http.post(self.url("/v1/devices/relay")).bearer_auth(&session.token)).await
    }
    /// Asks the hosting device `host` to meet this device, which is at the
    /// addresses `at` (direct first, then its relay address if it has one), and
    /// waits for the host's addresses in the same order.
    pub async fn knock(&self, session: &Session, host: &str, this: &Fingerprint, at: &[std::net::SocketAddr]) -> Result<Vec<std::net::SocketAddr>> {
        #[derive(Deserialize)]
        struct Answer {
            candidates: Vec<String>,
        }
        if host.len() != 36 || !host.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return Err(Error::Invalid("Unknown device"));
        }
        let body = serde_json::json!({"fingerprint": hex(this), "candidates": at.iter().map(|a| a.to_string()).collect::<Vec<_>>()});
        let request = self.http.post(self.url(&format!("/v1/devices/{host}/knock"))).bearer_auth(&session.token).timeout(Duration::from_secs(40)).json(&body);
        Ok(self.json::<Answer>(request).await?.candidates.iter().filter_map(|c| c.parse().ok()).collect())
    }

    pub async fn devices(&self, session: &Session) -> Result<Vec<Device>> {
        #[derive(Deserialize)]
        struct List {
            devices: Vec<Device>,
        }
        Ok(self.json::<List>(self.http.get(self.url("/v1/devices")).bearer_auth(&session.token)).await?.devices)
    }

    /// Removes a device from the account. Hosting devices drop it at their next list.
    pub async fn remove(&self, session: &Session, device: &str) -> Result<()> {
        if !identifier(device) {
            return Err(Error::Invalid("Unknown device"));
        }
        self.send(self.http.delete(self.url(&format!("/v1/devices/{device}"))).bearer_auth(&session.token)).await.map(|_| ())
    }

    /// Everyone on the account's friend list, including requests that have not
    /// been answered yet.
    pub async fn friends(&self, session: &Session) -> Result<Vec<Friend>> {
        self.json(self.http.get(self.url("/v1/friends")).bearer_auth(&session.token)).await
    }

    /// Asks the account named `handle` (with or without the @) to be friends.
    /// The request waits for that account to accept it.
    pub async fn request_friend(&self, session: &Session, handle: &str) -> Result<()> {
        let handle = handle.trim().trim_start_matches('@').to_lowercase();
        if handle.is_empty() || handle.len() > 30 || !handle.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return Err(Error::Invalid("Use an account name of 1 to 30 letters, digits or underscores"));
        }
        self.send(self.http.put(self.url(&format!("/v1/friends/by-username/{handle}"))).bearer_auth(&session.token)).await.map(|_| ())
    }

    /// Accepts a request from `account`. Requests this account sent itself are
    /// refused by the service, not here.
    pub async fn accept_friend(&self, session: &Session, account: &str) -> Result<()> {
        if !identifier(account) {
            return Err(Error::Invalid("Unknown account"));
        }
        self.send(self.http.post(self.url(&format!("/v1/friends/{account}/accept"))).bearer_auth(&session.token)).await.map(|_| ())
    }

    /// Removes a friend, or turns down a request. Any access it had ends with it.
    pub async fn remove_friend(&self, session: &Session, account: &str) -> Result<()> {
        if !identifier(account) {
            return Err(Error::Invalid("Unknown account"));
        }
        self.send(self.http.delete(self.url(&format!("/v1/friends/{account}"))).bearer_auth(&session.token)).await.map(|_| ())
    }

    /// What this account allows `account`'s devices to do on its computers.
    pub async fn friend_settings(&self, session: &Session, account: &str) -> Result<FriendSettings> {
        if !identifier(account) {
            return Err(Error::Invalid("Unknown account"));
        }
        self.json(self.http.get(self.url(&format!("/v1/friends/{account}/settings"))).bearer_auth(&session.token)).await
    }

    /// Changes what `account`'s devices may do. Answers the settings now in force,
    /// which may differ from the change when one permission decides another.
    pub async fn save_friend_settings(&self, session: &Session, account: &str, change: &FriendChange) -> Result<FriendSettings> {
        if !identifier(account) {
            return Err(Error::Invalid("Unknown account"));
        }
        change.checked()?;
        self.json(self.http.put(self.url(&format!("/v1/friends/{account}/settings"))).bearer_auth(&session.token).json(change)).await
    }

    /// The keys access lists are signed with. Keep them beside the saved list.
    pub async fn keys(&self) -> Result<Vec<Key>> {
        #[derive(Deserialize)]
        struct Set {
            keys: Vec<Key>,
        }
        Ok(self.json::<Set>(self.http.get(self.url("/.well-known/jwks.json"))).await?.keys)
    }

    /// Fetches and checks the access list of the hosting device `device`, whose
    /// identity is `host`. Returns the signed text to save, and what it says.
    pub async fn access(&self, session: &Session, device: &Device, host: &Fingerprint, keys: &[Key]) -> Result<(String, AccessList)> {
        #[derive(Deserialize)]
        struct Signed {
            access: String,
        }
        let signed: Signed = self.json(self.http.get(self.url(&format!("/v1/devices/{}/access", device.id))).bearer_auth(&session.token)).await?;
        let list = verify_access(&signed.access, keys, &self.origin, host, now())?;
        Ok((signed.access, list))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    fn session() -> Session {
        Session { token: "t".repeat(64), expires_at: 2_000_000_000, user: User { id: "id-1".into(), handle: "alice".into() } }
    }

    #[test]
    fn a_session_is_saved_read_back_and_forgotten() {
        let store = MemoryStore::default();
        assert_eq!(saved_session(&store), Ok(None));
        save_session(&store, &session()).unwrap();
        assert_eq!(saved_session(&store), Ok(Some(session())));
        forget_session(&store).unwrap();
        assert_eq!(saved_session(&store), Ok(None));
    }

    #[test]
    fn a_store_that_cannot_be_read_is_not_signed_out() {
        let store = MemoryStore::default();
        save_session(&store, &session()).unwrap();
        store.fail_reads.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(saved_session(&store), Err(Error::Store(Unavailable)));
    }

    #[test]
    fn the_session_token_is_not_shown_when_printed() {
        assert!(!format!("{:?}", session()).contains("tttt"));
    }

    #[test]
    fn only_https_or_a_test_service_on_this_computer_is_accepted() {
        assert!(Service::new(ORIGIN).is_ok());
        assert!(Service::new("http://127.0.0.1:8791").is_err());
        assert!(Service::new("http://accounts.secpar.win").is_err());
        assert!(Service::local_test(ORIGIN, "http://127.0.0.1:8791").is_ok());
        assert!(Service::local_test(ORIGIN, "http://127.0.0.1:8791.example.com").is_err());
        assert!(Service::local_test(ORIGIN, "http://192.168.1.5:8791").is_err());
    }

    #[test]
    fn a_friend_change_only_accepts_the_service_s_values() {
        let mut change = FriendChange::default();
        assert!(change.is_empty() && change.checked().is_err());
        change.connect = Some("always".into());
        assert!(change.checked().is_ok());
        change.connect = Some("sometimes".into());
        assert!(change.checked().is_err());
        // Control is view or full; the fine permissions are separate.
        let input = FriendChange { input: Some("full".into()), ..Default::default() };
        assert!(input.checked().is_ok());
        assert_eq!(serde_json::to_value(&input).unwrap(), serde_json::json!({"input": "full"}));
        let both = FriendChange { keyboard: Some(true), mouse: Some(true), ..Default::default() };
        assert_eq!(serde_json::to_value(&both).unwrap(), serde_json::json!({"keyboard": true, "mouse": true}));
        assert!(FriendChange { input: Some("keyboard".into()), ..Default::default() }.checked().is_err());
    }

    /// A stand-in for the service's signing key, to check lists without a network.
    struct Signer {
        pair: EcdsaKeyPair,
        key: Key,
    }
    fn signer() -> Signer {
        let rng = SystemRandom::new();
        let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref(), &rng).unwrap();
        let point = ring::signature::KeyPair::public_key(&pair).as_ref().to_vec();
        let key = Key { kid: "k1".into(), kty: "EC".into(), crv: "P-256".into(), x: URL_SAFE_NO_PAD.encode(&point[1..33]), y: URL_SAFE_NO_PAD.encode(&point[33..]) };
        Signer { pair, key }
    }
    fn signed(signer: &Signer, typ: &str, claims: serde_json::Value) -> String {
        let header = serde_json::json!({"alg": "ES256", "typ": typ, "kid": "k1"});
        let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
        let signature = signer.pair.sign(&SystemRandom::new(), input.as_bytes()).unwrap();
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
    }
    fn claims(host: &Fingerprint) -> serde_json::Value {
        serde_json::json!({
            "iss": ORIGIN, "aud": "secpar-host", "sub": hex(host), "iat": 1000, "exp": 2000, "owner": "id-1",
            "devices": [{"fingerprint": "11".repeat(32), "name": "Laptop"}],
            "guests": [{"fingerprint": "22".repeat(32), "name": "Phone", "account": "id-2", "handle": "bob", "trusted": true, "keyboard": true, "mouse": false}],
        })
    }

    #[test]
    fn a_signed_list_is_read() {
        let (signer, host) = (signer(), [7u8; 32]);
        let list = verify_access(&signed(&signer, "secpar-access+jwt", claims(&host)), &[signer.key.clone()], ORIGIN, &host, 1500).unwrap();
        assert_eq!(list.owner, "id-1");
        assert_eq!(list.devices, vec![OwnDevice { fingerprint: [0x11; 32], name: "Laptop".into() }]);
        assert_eq!(list.guests[0].fingerprint, [0x22; 32]);
        assert!(list.guests[0].trusted && list.guests[0].keyboard && !list.guests[0].mouse);
        assert_eq!(list.expires_at, 2000);
    }

    #[test]
    fn a_list_that_is_expired_altered_or_meant_for_something_else_is_refused() {
        let (signer, host) = (signer(), [7u8; 32]);
        let keys = [signer.key.clone()];
        let good = signed(&signer, "secpar-access+jwt", claims(&host));
        assert!(verify_access(&good, &keys, ORIGIN, &host, 1500).is_ok());
        // Expired, and issued further ahead than a slow clock explains.
        assert!(verify_access(&good, &keys, ORIGIN, &host, 2000).is_err());
        assert!(verify_access(&good, &keys, ORIGIN, &host, 300).is_err());
        // Another hosting device, another service, another signer.
        assert!(verify_access(&good, &keys, ORIGIN, &[8u8; 32], 1500).is_err());
        assert!(verify_access(&good, &keys, "https://accounts.example.test", &host, 1500).is_err());
        assert!(verify_access(&good, &[self::signer().key], ORIGIN, &host, 1500).is_err());
        // Another kind of token from the same key.
        assert!(verify_access(&signed(&signer, "secpar-grant+jwt", claims(&host)), &keys, ORIGIN, &host, 1500).is_err());
        let mut other = claims(&host);
        other["aud"] = "secpar-guest".into();
        assert!(verify_access(&signed(&signer, "secpar-access+jwt", other), &keys, ORIGIN, &host, 1500).is_err());
        // A changed body keeps the old signature.
        let mut wider = claims(&host);
        wider["guests"][0]["mouse"] = true.into();
        let forged = signed(&signer, "secpar-access+jwt", wider);
        let body = forged.split('.').nth(1).unwrap();
        let mut parts: Vec<&str> = good.split('.').collect();
        parts[1] = body;
        assert!(verify_access(&parts.join("."), &keys, ORIGIN, &host, 1500).is_err());
        assert!(verify_access("not a token", &keys, ORIGIN, &host, 1500).is_err());
    }
}
