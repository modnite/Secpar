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
struct Inner {
    registration: Option<Registration>,
    state: State,
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
            }),
            storage,
            fingerprint: fingerprint.into(),
            client,
            origin: ORIGIN.into(),
        }))
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
            match self
                .request(registration, Method::POST, "/v1/host/heartbeat")
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
    /// Friends of the owner and what each may do. Read fresh on every admission.
    pub async fn roster(&self) -> Result<crate::guest_policy::Roster, String> {
        let registration = self.inner.lock().await.registration.clone().ok_or("Host is not linked")?;
        let rows = self.request(&registration, Method::GET, "/v1/host/friends").await.map_err(|(_, note)| note)?;
        crate::guest_policy::Roster::parse(&rows).map_err(Into::into)
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
    ) -> Result<crate::guest_policy::Admission, String> {
        let (roster, lease) = tokio::try_join!(self.roster(), self.lease(request))?;
        crate::guest_policy::admit(&roster, &lease, signed, guest).map_err(|d| d.message().into())
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
    use wiremock::{matchers::{header, method, path}, Mock, MockServer, ResponseTemplate};
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
        Account { inner: Mutex::new(Inner { registration: None, state: State::default() }), storage, fingerprint: registration().certificate_fp, client: Client::new(), origin: server.uri() }
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
    async fn hostile_job_identifiers_never_reach_the_pairing_callback() {
        let server = MockServer::start().await; let account = account(&server, Arc::default());
        account.inner.lock().await.registration = Some(registration());
        Mock::given(path("/v1/host/owner-pairings")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id":"../../host/link"}]))).mount(&server).await;
        assert!(account.poll_pairings(|_| async { panic!("Invalid job reached host pairing") }).await.is_err());
    }
}

