//! Machine-only account credential. User sessions and passwords never enter this service.
use crate::{account_link::ORIGIN, win::dpapi};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Deserialize, Serialize)]
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
    path: PathBuf,
    fingerprint: String,
    client: Client,
    origin: String,
}
impl Account {
    pub fn open(dir: &std::path::Path, fingerprint: &str) -> Result<Arc<Self>, String> {
        let path = dir.join("host-account.bin");
        let registration = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 8192 {
                    return Err("Saved host link is invalid".into());
                }
                let plain =
                    dpapi::unprotect(&bytes).map_err(|_| "Could not unlock saved host link")?;
                let value: Registration =
                    serde_json::from_slice(&plain).map_err(|_| "Saved host link is invalid")?;
                validate(&value, fingerprint)?;
                Some(value)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("Could not read saved host link".into()),
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
            path,
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
        let encrypted = dpapi::protect(&plain).map_err(|_| "Could not protect host link")?;
        let temp = self.path.with_extension("tmp");
        std::fs::write(&temp, encrypted)
            .and_then(|_| std::fs::rename(&temp, &self.path))
            .map_err(|_| "Could not save host link")?;
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
        remove(&self.path)?;
        inner.registration = None;
        inner.state = State::default();
        Ok(inner.state.clone())
    }
    async fn request(
        &self,
        registration: &Registration,
        method: Method,
        path: &str,
    ) -> Result<serde_json::Value, (StatusCode, String)> {
        let mut response = self
            .client
            .request(method, format!("{}{path}", self.origin))
            .bearer_auth(&registration.credential)
            .send()
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
            if bytes.len() + chunk.len() > 8192 {
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
                    let note = remove(&self.path)
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
    pub async fn run(self: Arc<Self>) {
        let mut timer = tokio::time::interval(Duration::from_secs(30));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            self.heartbeat().await;
        }
    }
}
fn remove(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not remove the saved host credential".into()),
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
        || value.computer.len() != 36
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
    #[test]
    fn host_storage_rejects_account_sessions_and_other_certificates() {
        let mut value = Registration {
            computer: "0".repeat(36),
            owner: "fixture".into(),
            certificate_fp: "ab".repeat(32),
            credential: format!("h1_{}", "a".repeat(64)),
        };
        assert!(validate(&value, &value.certificate_fp).is_ok());
        assert!(validate(&value, &"cd".repeat(32)).is_err());
        value.credential = "a".repeat(64);
        assert!(validate(&value, &value.certificate_fp).is_err());
    }
    #[test]
    fn public_state_never_serializes_the_machine_credential() {
        let value = Registration {
            computer: "fixture".into(),
            owner: "player".into(),
            certificate_fp: "public".into(),
            credential: "private-machine-value".into(),
        };
        let json = serde_json::to_string(&view(Some(&value), true, "")).unwrap();
        assert!(!json.contains("private-machine-value"));
        assert!(!json.contains("credential"));
    }
    fn registration() -> Registration {
        Registration {
            computer: "00000000-0000-4000-8000-000000000001".into(),
            owner: "fixture".into(),
            certificate_fp: "ab".repeat(32),
            credential: format!("h1_{}", "a".repeat(64)),
        }
    }

    #[tokio::test]
    async fn saves_only_machine_protected_credentials_and_restores_then_revokes() {
        use wiremock::{
            matchers::{header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let server = MockServer::start().await;
        let value = registration();
        let dir = std::env::temp_dir().join(format!(
            "secpar-host-account-{}-{}",
            std::process::id(),
            crate::tls::hex(&crate::store::random::<8>())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Mock::given(method("GET")).and(path("/v1/host/status")).and(header("authorization",format!("Bearer {}",value.credential)))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"computer":value.computer,"owner":value.owner,"certificate_fp":value.certificate_fp}))).expect(1).mount(&server).await;
        let account = Account {
            inner: Mutex::new(Inner {
                registration: None,
                state: State::default(),
            }),
            path: dir.join("host-account.bin"),
            fingerprint: value.certificate_fp.clone(),
            client: Client::new(),
            origin: server.uri(),
        };
        let state = account.install(value).await.unwrap();
        assert!(state.online);
        let blob = std::fs::read(&account.path).unwrap();
        assert!(!blob
            .windows(67)
            .any(|chunk| chunk == registration().credential.as_bytes()));
        let restored = Account::open(&dir, &registration().certificate_fp).unwrap();
        assert_eq!(restored.state().await.computer, state.computer);
        assert!(!restored.state().await.online);
        assert!(account.allow_owner("other-owner").await.is_err());
        Mock::given(method("POST"))
            .and(path("/v1/host/heartbeat"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        account.heartbeat().await;
        assert!(account.state().await.computer.is_none());
        assert!(!account.path.exists());
        server.verify().await;
        std::fs::remove_dir(&dir).unwrap();
    }
    #[tokio::test]
    async fn refuses_a_credential_for_another_computer_without_saving_it() {
        use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let server = MockServer::start().await;
        let value = registration();
        Mock::given(path("/v1/host/status")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"computer":value.computer,"owner":value.owner,"certificate_fp":"wrong-certificate"}))).mount(&server).await;
        let dir = std::env::temp_dir().join(format!(
            "secpar-host-account-reject-{}",
            crate::tls::hex(&crate::store::random::<8>())
        ));
        let account = Account {
            inner: Mutex::new(Inner {
                registration: None,
                state: State::default(),
            }),
            path: dir.join("host-account.bin"),
            fingerprint: value.certificate_fp.clone(),
            client: Client::new(),
            origin: server.uri(),
        };
        assert!(account.install(value).await.is_err());
        assert!(!dir.exists());
        assert!(account.state().await.computer.is_none());
    }
}
