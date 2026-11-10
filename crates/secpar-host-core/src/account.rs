//! Machine-only account credential. User sessions and passwords never enter this service.
use crate::account_link::ORIGIN;
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

/// Platform-owned storage: machine DPAPI on Windows, private user storage on Linux.
/// Replace atomically and never log credentials.
pub trait CredentialStorage: Send + Sync {
    fn load(&self) -> Result<Option<Vec<u8>>, String>;
    fn save(&self, plain: &[u8]) -> Result<(), String>;
    fn remove(&self) -> Result<(), String>;
}
#[derive(Deserialize)]
struct PairRequest { id: String }
fn valid_id(id: &str) -> bool {
    id.len() == 36 && id.bytes().enumerate().all(|(i,b)| if [8,13,18,23].contains(&i) {b == b'-'} else {b.is_ascii_hexdigit()})
}
/// One friend of the owner as the host's settings show them.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FriendRow {
    pub id: String,
    pub handle: String,
    pub connect: String,
    pub keyboard: bool,
    pub mouse: bool,
}
#[derive(Deserialize)]
struct RawFriend {
    id: String,
    #[serde(default)]
    handle: String,
    #[serde(default)]
    input: String,
    #[serde(default)]
    connect: String,
    keyboard: Option<bool>,
    mouse: Option<bool>,
}
impl From<RawFriend> for FriendRow {
    /// A Worker from before keyboard and mouse were separate sends only `input`.
    fn from(r: RawFriend) -> Self {
        let full = r.input == "full";
        FriendRow { id: r.id, handle: r.handle, connect: if r.connect == "always" { "always".into() } else { "ask".into() }, keyboard: r.keyboard.unwrap_or(full), mouse: r.mouse.unwrap_or(full) }
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Registration {
    pub computer: String,
    pub owner: String,
    pub certificate_fp: String,
    pub credential: String,
}
#[derive(Default, Clone, Serialize)]
pub struct State {
    pub computer: Option<String>,
    pub owner: Option<String>,
    pub online: bool,
    pub note: String,
}
#[derive(Deserialize)]
struct RemoteState {
    computer: String,
    owner: String,
    certificate_fp: String,
}
/// What the heartbeat tells the service so friends can find this host. Off until a platform
/// turns it on, and off again whenever the owner switches friends off.
#[derive(Clone, Default, PartialEq, Serialize)]
pub struct Presence {
    pub sharing: bool,
    /// The host's rendezvous id and the relay Worker's origin.
    pub pc: String,
    pub relay: String,
}
impl Presence {
    /// Sharing needs a relay the service will accept: a plain https origin. Anything else means off,
    /// so a misconfigured address can never make the heartbeat fail.
    pub fn new(friends_on: bool, pc: &str, relay: Option<&str>) -> Self {
        let origin = relay.map(|r| r.trim_end_matches('/')).filter(|r| {
            r.strip_prefix("https://").is_some_and(|host| !host.is_empty() && !host.contains(['/', '?', '#', '@', ' ']))
        });
        match origin {
            Some(origin) if friends_on && valid_id(pc) => Self { sharing: true, pc: pc.into(), relay: origin.into() },
            _ => Self::default(),
        }
    }
}
struct Inner {
    registration: Option<Registration>,
    state: State,
    presence: Presence,
}
pub struct Account {
    inner: Mutex<Inner>,
    storage: Arc<dyn CredentialStorage>,
    fingerprint: String,
    client: Client,
    origin: String,
}
impl Account {
    pub fn open(storage: Arc<dyn CredentialStorage>, fingerprint: &str) -> Result<Arc<Self>, String> {
        let registration = match storage.load()? {
            Some(bytes) => {
                if bytes.len() > 8192 { return Err("Saved host link is invalid".into()); }
                let value: Registration = serde_json::from_slice(&bytes).map_err(|_| "Saved host link is invalid")?;
                validate(&value, fingerprint)?;
                Some(value)
            }
            None => None,
        };
        let state = view(registration.as_ref(), false, "");
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| "Could not start secure host networking")?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                registration,
                state,
                presence: Presence::default(),
            }),
            storage,
            fingerprint: fingerprint.into(),
            client,
            origin: ORIGIN.into(),
        }))
    }
    /// Call whenever the owner's friends setting or the relay address changes. Takes effect on
    /// the next heartbeat.
    pub async fn set_presence(&self, presence: Presence) {
        self.inner.lock().await.presence = presence;
    }
    pub async fn state(&self) -> State {
        self.inner.lock().await.state.clone()
    }
    pub async fn allow_owner(&self, owner: &str) -> Result<(), String> {
        let inner = self.inner.lock().await;
        if inner
            .registration
            .as_ref()
            .is_some_and(|r| r.owner != owner)
        {
            return Err("Unlink the current host account first".into());
        }
        Ok(())
    }
    pub async fn install(&self, registration: Registration) -> Result<State, String> {
        validate(&registration, &self.fingerprint)?;
        let mut inner = self.inner.lock().await;
        if inner
            .registration
            .as_ref()
            .is_some_and(|r| r.owner != registration.owner)
        {
            return Err("Unlink the current host account first".into());
        }
        let remote = self
            .request(&registration, Method::GET, "/v1/host/status")
            .await
            .map_err(|(_, e)| e)?;
        let remote: RemoteState =
            serde_json::from_value(remote).map_err(|_| "Invalid host account response")?;
        if remote.computer != registration.computer
            || remote.owner != registration.owner
            || remote.certificate_fp != self.fingerprint
        {
            return Err("Host credential belongs to a different computer".into());
        }
        let plain = serde_json::to_vec(&registration).map_err(|_| "Could not save host link")?;
        self.storage.save(&plain)?;
        inner.state = view(Some(&registration), true, "");
        inner.registration = Some(registration);
        Ok(inner.state.clone())
    }
    pub async fn unlink(&self) -> Result<State, String> {
        let mut inner = self.inner.lock().await;
        if let Some(registration) = &inner.registration {
            match self
                .request(registration, Method::DELETE, "/v1/host/link")
                .await
            {
                Ok(_) | Err((StatusCode::UNAUTHORIZED, _)) => {}
                Err((_, note)) => return Err(note),
            }
        }
        self.storage.remove()?;
        inner.registration = None;
        inner.state = State::default();
        Ok(inner.state.clone())
    }
    async fn request(&self, registration: &Registration, method: Method, path: &str) -> Result<serde_json::Value, (StatusCode, String)> {
        self.request_json(registration, method, path, None).await
    }
    async fn request_json(
        &self,
        registration: &Registration,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, (StatusCode, String)> {
        let request = self
            .client
            .request(method, format!("{}{path}", self.origin))
            .bearer_auth(&registration.credential);
        let request = if let Some(body) = body { request.json(&body) } else { request };
        let mut response = request.send()
            .await
            .map_err(|_| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Could not reach Secpar accounts".into(),
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err((status, "Host link could not be verified".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| (status, "Incomplete host response".into()))?
        {
            if bytes.len() + chunk.len() > 32768 {
                return Err((status, "Invalid host response".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| (status, "Invalid host response".into()))
    }
    async fn heartbeat(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(registration) = &inner.registration {
            let presence = inner.presence.clone();
            let body = presence.sharing.then(|| serde_json::to_value(&presence).unwrap_or_default());
            match self
                .request_json(registration, Method::POST, "/v1/host/heartbeat", body)
                .await
            {
                Ok(_) => inner.state = view(Some(registration), true, ""),
                Err((StatusCode::UNAUTHORIZED, _)) => {
                    let note = self.storage.remove()
                        .err()
                        .unwrap_or_else(|| "Host account was unlinked".into());
                    inner.registration = None;
                    inner.state = State {
                        note,
                        ..State::default()
                    };
                }
                Err((_, note)) => inner.state = view(Some(registration), false, &note),
            }
        }
    }
    /// The installed machine registration's computer id, or None when the host is not linked.
    /// A linked account talking to a fake service, for tests of guest admission.
    #[cfg(test)]
    pub(crate) fn linked_for_test(origin: &str) -> Arc<Self> {
        struct Nothing;
        impl CredentialStorage for Nothing {
            fn load(&self) -> Result<Option<Vec<u8>>, String> { Ok(None) }
            fn save(&self, _: &[u8]) -> Result<(), String> { Ok(()) }
            fn remove(&self) -> Result<(), String> { Ok(()) }
        }
        let registration = Registration {
            computer: crate::guest_grant::tests::COMPUTER.into(),
            owner: "fixture".into(),
            certificate_fp: "ab".repeat(32),
            credential: format!("h1_{}", "a".repeat(64)),
        };
        Arc::new(Self {
            inner: Mutex::new(Inner { state: view(Some(&registration), true, ""), registration: Some(registration), presence: Presence::default() }),
            storage: Arc::new(Nothing),
            fingerprint: "ab".repeat(32),
            client: Client::new(),
            origin: origin.into(),
        })
    }
    pub async fn computer(&self) -> Option<String> {
        self.inner.lock().await.registration.as_ref().map(|r| r.computer.clone())
    }
    /// Secpar's grant signing keys, from the fixed HTTPS origin only. Bounded and never cached.
    pub async fn signing_keys(&self) -> Result<Vec<crate::guest_grant::PublicKey>, String> {
        #[derive(Deserialize)]
        struct Keys { keys: Vec<crate::guest_grant::PublicKey> }
        let mut response = self.client.get(format!("{}/.well-known/jwks.json", self.origin)).send().await
            .map_err(|_| "Could not reach Secpar accounts")?;
        if !response.status().is_success() { return Err("Could not read signing keys".into()); }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "Incomplete signing keys")? {
            if bytes.len() + chunk.len() > 8192 { return Err("Invalid signing keys".into()); }
            bytes.extend_from_slice(&chunk);
        }
        let keys: Keys = serde_json::from_slice(&bytes).map_err(|_| "Invalid signing keys")?;
        if keys.keys.is_empty() || keys.keys.len() > 5 { return Err("Invalid signing keys".into()); }
        Ok(keys.keys)
    }
    /// Friends of the owner and what each may do. Read fresh on every admission.
    pub async fn roster(&self) -> Result<crate::guest_policy::Roster, String> {
        let registration = self.inner.lock().await.registration.clone().ok_or("Host is not linked")?;
        let rows = self.request(&registration, Method::GET, "/v1/host/friends").await.map_err(|(_, note)| note)?;
        crate::guest_policy::Roster::parse(&rows).map_err(Into::into)
    }
    /// Every friend of the owner and what each may do on this PC, for the host's own settings.
    pub async fn friend_rows(&self) -> Result<Vec<FriendRow>, String> {
        let registration = self.inner.lock().await.registration.clone().ok_or("Host is not linked")?;
        let rows = self.request(&registration, Method::GET, "/v1/host/friends").await.map_err(|(_, note)| note)?;
        let raw: Vec<RawFriend> = serde_json::from_value(rows).map_err(|_| "Invalid friends response")?;
        if raw.len() > 500 {
            return Err("Invalid friends response".into());
        }
        let mut rows: Vec<FriendRow> = raw.into_iter().map(Into::into).collect();
        rows.sort_by_key(|r| r.handle.to_lowercase());
        Ok(rows)
    }
    /// Changes one friend. Anything left as `None` stays as it is.
    pub async fn set_friend(&self, friend: &str, keyboard: Option<bool>, mouse: Option<bool>, connect: Option<&str>) -> Result<Vec<FriendRow>, String> {
        if friend.is_empty() || friend.len() > 64 || !friend.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err("Invalid friend".into());
        }
        if connect.is_some_and(|c| c != "ask" && c != "always") {
            return Err("Invalid setting".into());
        }
        let body = serde_json::json!({"keyboard": keyboard, "mouse": mouse, "connect": connect});
        let registration = self.inner.lock().await.registration.clone().ok_or("Host is not linked")?;
        self.request_json(&registration, Method::PUT, &format!("/v1/host/friends/{friend}/settings"), Some(body)).await.map_err(|(_, note)| note)?;
        self.friend_rows().await
    }
    /// The online lease of one claimed guest request.
    pub async fn lease(&self, request: &str) -> Result<crate::guest_policy::Lease, String> {
        if !valid_id(request) { return Err("Invalid request".into()); }
        let registration = self.inner.lock().await.registration.clone().ok_or("Host is not linked")?;
        let row = self.request(&registration, Method::GET, &format!("/v1/host/requests/{request}")).await.map_err(|(_, note)| note)?;
        serde_json::from_value(row).map_err(|_| "Invalid request response".into())
    }
    /// Ask the Worker and the owner's friend list, then decide. Nothing is cached.
    pub async fn admit_guest(
        &self,
        request: &str,
        guest: &str,
        signed: &std::collections::BTreeSet<String>,
    ) -> Result<(crate::guest_policy::Admission, String), String> {
        let (roster, lease) = tokio::try_join!(self.roster(), self.lease(request))?;
        let handle = roster.handle(guest).unwrap_or_default().to_owned();
        crate::guest_policy::admit(&roster, &lease, signed, guest).map(|a| (a, handle)).map_err(|d| d.message().into())
    }
    pub async fn poll_pairings<F, Fut>(&self, mut make_link: F) -> Result<usize, String>
    where F: FnMut(String) -> Fut, Fut: std::future::Future<Output = Result<String, String>> {
        let registration = self.inner.lock().await.registration.clone();
        let Some(registration) = registration else { return Ok(0); };
        let jobs = self.request(&registration, Method::GET, "/v1/host/owner-pairings").await.map_err(|(_, note)| note)?;
        let jobs: Vec<PairRequest> = serde_json::from_value(jobs).map_err(|_| "Invalid owner pairing response")?;
        if jobs.len() > 8 || jobs.iter().any(|job| !valid_id(&job.id)) { return Err("Invalid owner pairing response".into()); }
        let mut delivered = 0;
        for job in jobs {
            let link = make_link(job.id.clone()).await?;
            if link.len() > 8192 { return Err("Invalid host pairing link".into()); }
            self.request_json(&registration, Method::POST, &format!("/v1/host/owner-pairings/{}", job.id), Some(serde_json::json!({"link":link}))).await.map_err(|(_, note)| note)?;
            delivered += 1;
        }
        Ok(delivered)
    }
    pub async fn run(self: Arc<Self>) {
        let mut timer = tokio::time::interval(Duration::from_secs(30));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            self.heartbeat().await;
        }
    }
}
fn view(registration: Option<&Registration>, online: bool, note: &str) -> State {
    State {
        computer: registration.map(|r| r.computer.clone()),
        owner: registration.map(|r| r.owner.clone()),
        online,
        note: note.into(),
    }
}
fn validate(value: &Registration, fingerprint: &str) -> Result<(), String> {
    if value.certificate_fp != fingerprint
        || !valid_id(&value.computer)
        || value.owner.is_empty()
        || value.owner.len() > 80
        || value.credential.len() != 67
        || !value.credential.starts_with("h1_")
        || !value.credential[3..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Invalid credential for this host".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{matchers::{body_bytes, body_json, header, method, path}, Mock, MockServer, ResponseTemplate};
    #[derive(Default)]
    struct Memory(std::sync::Mutex<Option<Vec<u8>>>);
    impl CredentialStorage for Memory {
        fn load(&self) -> Result<Option<Vec<u8>>, String> { Ok(self.0.lock().unwrap().clone()) }
        fn save(&self, bytes: &[u8]) -> Result<(), String> { *self.0.lock().unwrap() = Some(bytes.to_vec()); Ok(()) }
        fn remove(&self) -> Result<(), String> { *self.0.lock().unwrap() = None; Ok(()) }
    }
    fn registration() -> Registration {
        Registration { computer: "00000000-0000-4000-8000-000000000001".into(), owner: "fixture".into(), certificate_fp: "ab".repeat(32), credential: format!("h1_{}", "a".repeat(64)) }
    }
    fn account(server: &MockServer, storage: Arc<Memory>) -> Account {
        // Each fixture must initialize TLS without depending on another test.
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        Account { inner: Mutex::new(Inner { registration: None, state: State::default(), presence: Presence::default() }), storage, fingerprint: registration().certificate_fp, client: Client::new(), origin: server.uri() }
    }
    #[tokio::test]
    async fn the_host_lists_friends_and_changes_one() {
        let server = MockServer::start().await;
        let account = Account::linked_for_test(&server.uri());
        let bob = "00000000-0000-4000-8000-0000000000b0";
        Mock::given(method("GET")).and(path("/v1/host/friends"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": bob, "handle": "Bob", "input": "view", "connect": "ask", "keyboard": true, "mouse": false},
                {"id": "00000000-0000-4000-8000-0000000000a0", "handle": "amy", "input": "full", "connect": "always"}
            ]))).mount(&server).await;
        Mock::given(method("PUT")).and(path(format!("/v1/host/friends/{bob}/settings")))
            .and(body_json(serde_json::json!({"keyboard": null, "mouse": true, "connect": null})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({}))).expect(1).mount(&server).await;
        let rows = account.friend_rows().await.unwrap();
        // Sorted by name without regard to case. An older Worker's `input` fills in keyboard and mouse.
        assert_eq!(rows[0].handle, "amy");
        assert!(rows[0].keyboard && rows[0].mouse && rows[0].connect == "always");
        assert!(rows[1].keyboard && !rows[1].mouse && rows[1].connect == "ask");
        account.set_friend(bob, None, Some(true), None).await.unwrap();
        assert!(account.set_friend("../x", None, None, None).await.is_err());
        assert!(account.set_friend(bob, None, None, Some("never")).await.is_err());
    }
    #[test]
    fn accepts_only_machine_credentials_bound_to_this_identity() {
        let mut value = registration();
        assert!(validate(&value, &value.certificate_fp).is_ok());
        assert!(validate(&value, &"cd".repeat(32)).is_err());
        value.credential = "a".repeat(64);
        assert!(validate(&value, &value.certificate_fp).is_err());
        value = registration(); value.computer = "0".repeat(36);
        assert!(validate(&value, &value.certificate_fp).is_err());
        assert!(!serde_json::to_string(&view(Some(&registration()), true, "")).unwrap().contains("credential"));
    }
    #[tokio::test]
    async fn verified_install_restores_then_revocation_removes_saved_credential() {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let server = MockServer::start().await;
        let value = registration(); let storage = Arc::new(Memory::default());
        Mock::given(method("GET")).and(path("/v1/host/status")).and(header("authorization", format!("Bearer {}", value.credential)))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"computer":value.computer,"owner":value.owner,"certificate_fp":value.certificate_fp}))).expect(1).mount(&server).await;
        let account = account(&server, storage.clone());
        assert!(account.install(value).await.unwrap().online);
        let restored = Account::open(storage.clone(), &registration().certificate_fp).unwrap();
        assert_eq!(restored.state().await.owner.as_deref(), Some("fixture"));
        assert!(!restored.state().await.online);
        assert!(restored.allow_owner("someone-else").await.is_err());
        Mock::given(method("POST")).and(path("/v1/host/heartbeat")).respond_with(ResponseTemplate::new(401)).expect(1).mount(&server).await;
        account.heartbeat().await;
        assert!(account.state().await.owner.is_none()); assert!(storage.load().unwrap().is_none());
        server.verify().await;
    }
    #[tokio::test]
    async fn a_wrong_remote_identity_cannot_be_saved() {
        let server = MockServer::start().await; let storage = Arc::new(Memory::default());
        Mock::given(path("/v1/host/status")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"computer":registration().computer,"owner":"fixture","certificate_fp":"wrong"}))).mount(&server).await;
        let account = account(&server, storage.clone());
        assert!(account.install(registration()).await.is_err());
        assert!(account.state().await.owner.is_none()); assert!(storage.load().unwrap().is_none());
    }
    #[tokio::test]
    async fn polling_uses_only_machine_scope_and_delivers_a_fresh_link_for_each_job() {
        let server = MockServer::start().await; let account = account(&server, Arc::default());
        account.inner.lock().await.registration = Some(registration());
        let id = "00000000-0000-4000-8000-000000000002";
        Mock::given(method("GET")).and(path("/v1/host/owner-pairings")).and(header("authorization",format!("Bearer {}",registration().credential)))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id":id,"device_name":"Laptop"}]))).expect(1).mount(&server).await;
        Mock::given(method("POST")).and(path(format!("/v1/host/owner-pairings/{id}"))).and(wiremock::matchers::body_json(serde_json::json!({"link":"secpar://pair?fixture-only"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ready":true}))).expect(1).mount(&server).await;
        assert_eq!(account.poll_pairings(|request| async move { assert_eq!(request,id); Ok("secpar://pair?fixture-only".into()) }).await.unwrap(),1);
        server.verify().await;
    }
    #[tokio::test]
    async fn guest_admission_reads_the_friend_list_and_lease_with_the_machine_credential_only() {
        use crate::guest_policy::{Admission, Denied};
        use std::collections::BTreeSet;
        let server = MockServer::start().await; let account = account(&server, Arc::default());
        account.inner.lock().await.registration = Some(registration());
        let id = "00000000-0000-4000-8000-000000000009";
        let bearer = format!("Bearer {}", registration().credential);
        Mock::given(method("GET")).and(path("/v1/host/friends")).and(header("authorization", bearer.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id":"bob","handle":"bob","input":"view","connect":"ask"},
                {"id":"cat","handle":"cat","input":"full","connect":"always"}]))).mount(&server).await;
        let lease = |guest: &str, approved: &[&str], active: bool| ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"id":id,"guest":guest,"approved":approved,"state":"claimed","expires":1,"active":active,"input":"full","connect":"always"}));
        Mock::given(method("GET")).and(path(format!("/v1/host/requests/{id}"))).and(header("authorization", bearer))
            .respond_with(lease("cat", &["view","keyboard"], true)).up_to_n_times(1).mount(&server).await;
        let signed: BTreeSet<String> = ["view","keyboard"].map(String::from).into();
        let (got, name) = account.admit_guest(id, "cat", &signed).await.unwrap();
        assert_eq!(name, "cat");
        assert!(matches!(&got, Admission::Allow(g) if g.keyboard() && g.view()));
        // Same signed grant, but the Worker now reports a view only friend: the host drops input.
        server.reset().await;
        Mock::given(method("GET")).and(path("/v1/host/friends")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id":"cat","handle":"cat","input":"view","connect":"ask"}]))).mount(&server).await;
        Mock::given(method("GET")).and(path(format!("/v1/host/requests/{id}"))).respond_with(lease("cat", &["view","keyboard"], true)).mount(&server).await;
        let (got, _) = account.admit_guest(id, "cat", &signed).await.unwrap();
        assert!(matches!(&got, Admission::AskOwner(g) if !g.keyboard() && g.view()));
        // A lease that is no longer active is refused with a plain message.
        server.reset().await;
        Mock::given(method("GET")).and(path("/v1/host/friends")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id":"cat","handle":"cat"}]))).mount(&server).await;
        Mock::given(method("GET")).and(path(format!("/v1/host/requests/{id}"))).respond_with(lease("cat", &["view","keyboard"], false)).mount(&server).await;
        assert_eq!(account.admit_guest(id, "cat", &signed).await.unwrap_err(), Denied::LeaseInactive.message());
        // A request id that is not a UUID never becomes a path.
        assert!(account.lease("../../host/link").await.is_err());
    }
    #[test]
    fn presence_is_on_only_for_a_plain_https_relay_and_a_real_id() {
        let pc = "00000000-0000-4000-8000-0000000000c1";
        let on = Presence::new(true, pc, Some("https://relay.example/"));
        assert!(on.sharing && on.relay == "https://relay.example" && on.pc == pc);
        for relay in [None, Some("http://relay.example"), Some("https://relay.example/v1"), Some("https://u@relay.example"), Some("https://")] {
            assert!(!Presence::new(true, pc, relay).sharing, "{relay:?}");
        }
        assert!(!Presence::new(false, pc, Some("https://relay.example")).sharing);
        assert!(!Presence::new(true, "nope", Some("https://relay.example")).sharing);
    }
    #[tokio::test]
    async fn the_heartbeat_says_whether_friends_are_on_and_nothing_else_secret() {
        let server = MockServer::start().await; let account = account(&server, Arc::default());
        account.inner.lock().await.registration = Some(registration());
        Mock::given(method("POST")).and(path("/v1/host/heartbeat")).and(body_json(serde_json::json!({"sharing":true,"pc":"00000000-0000-4000-8000-0000000000c1","relay":"https://relay.example"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"computer":registration().computer,"owner":"fixture","certificate_fp":registration().certificate_fp,"name":"x","platform":"windows","last_seen":1,"sharing":true})))
            .expect(1).mount(&server).await;
        account.set_presence(Presence::new(true, "00000000-0000-4000-8000-0000000000c1", Some("https://relay.example"))).await;
        account.heartbeat().await;
        assert!(account.state().await.online);
        server.verify().await;
        server.reset().await;
        Mock::given(method("POST")).and(path("/v1/host/heartbeat")).and(body_bytes(Vec::<u8>::new()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"computer":registration().computer,"owner":"fixture","certificate_fp":registration().certificate_fp,"name":"x","platform":"windows","last_seen":1,"sharing":false})))
            .expect(1).mount(&server).await;
        account.set_presence(Presence::default()).await;
        account.heartbeat().await;
        server.verify().await;
    }
    #[tokio::test]
    async fn hostile_job_identifiers_never_reach_the_pairing_callback() {
        let server = MockServer::start().await; let account = account(&server, Arc::default());
        account.inner.lock().await.registration = Some(registration());
        Mock::given(path("/v1/host/owner-pairings")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id":"../../host/link"}]))).mount(&server).await;
        assert!(account.poll_pairings(|_| async { panic!("Invalid job reached host pairing") }).await.is_err());
    }
}

