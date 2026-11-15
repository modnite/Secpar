//! What the Secpar window does besides drawing: signing in, keeping the session
//! in the system credential store, enrolling this computer, and listing the
//! account's devices. The window passes one JSON request and gets one JSON
//! answer; each call blocks, so the window makes it off its drawing thread.
//!
//! This part holds the session and never the device key. For anything the
//! device key is needed for, it asks the background part over the local channel.
use secpar_core::{
    account::{self, Error, Proof, Service, Session},
    store::{MemoryStore, SecretStore, SystemStore},
};
use secpar_service::channel;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

mod preferences;
mod stream;
use preferences::Preferences;

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Request {
    /// First call. `channel` is the background part's channel. `test_account`
    /// is `[origin, address]` of a test account service on this computer, and
    /// `memory_store` keeps the session out of the real credential store.
    Configure {
        channel: String,
        #[serde(default)]
        test_account: Option<(String, String)>,
        #[serde(default)]
        memory_store: bool,
        /// The stream component's executable and Secpar's data folder for this
        /// person. Without both, Connect opens the tunnel and starts no stream.
        #[serde(default)]
        stream_component: Option<String>,
        #[serde(default)]
        data_folder: Option<String>,
    },
    State,
    /// Signs in and enrols this computer under the account as `computer`.
    SignIn { email: String, password: String, computer: String },
    /// Takes this computer off the account and forgets the session.
    SignOut,
    Devices,
    RemoveDevice { id: String },
    /// Open a tunnel to one of the account's hosting computers.
    Connect { id: String },
    Disconnect,
    /// The stream settings shown in Settings → Connecting.
    Preferences,
    SavePreferences { preferences: Preferences },
}

struct App {
    runtime: tokio::runtime::Runtime,
    account: Service,
    store: Box<dyn SecretStore>,
    channel: String,
    stream: Option<Arc<stream::Settings>>,
    running: stream::Running,
    preferences: Mutex<Preferences>,
    preferences_path: Option<PathBuf>,
}
static APP: OnceLock<App> = OnceLock::new();
/// One request at a time: sign-in and sign-out must not interleave.
static TURN: Mutex<()> = Mutex::new(());

fn failed(code: &str) -> Value {
    json!({"ok": false, "error": code})
}
fn refused(error: Error) -> Value {
    match error {
        Error::Unreachable => failed("unreachable"),
        Error::Refused { code, message, .. } => json!({"ok": false, "error": code, "message": message}),
        Error::Invalid(_) => failed("invalid_answer"),
        Error::Store(_) => failed("store_unavailable"),
    }
}

impl App {
    async fn background(&self, request: Value) -> Option<Value> {
        channel::ask(&self.channel, &request).await
    }
    /// The saved session, or the answer to give instead.
    fn session(&self) -> Result<Session, Value> {
        match account::saved_session(self.store.as_ref()) {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err(failed("signed_out")),
            Err(error) => Err(refused(error)),
        }
    }
    /// A session the service no longer accepts is forgotten.
    fn expired(&self, error: &Error) -> bool {
        let gone = matches!(error, Error::Refused { status: 401, .. });
        if gone {
            let _ = account::forget_session(self.store.as_ref());
        }
        gone
    }

    async fn state(&self) -> Value {
        let session = match account::saved_session(self.store.as_ref()) {
            Ok(session) => session,
            Err(error) => return refused(error),
        };
        let background = self.background(json!({"do": "status"})).await;
        let preferences = self.preferences.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        let hosting = background.as_ref().map(|status| json!({
            "hosts": status["hosts"].clone(),
            "allowed": status["allowed"].clone(),
            "access_expires_at": status["access_expires_at"].clone(),
            "tunnel_port": status["tunnel_port"].clone(),
        }));
        json!({
            "ok": true,
            "signed_in": session.is_some(),
            "handle": session.map(|s| s.user.handle),
            "background": background.is_some(),
            "hosts": background.as_ref().map(|b| b["hosts"].clone()),
            "device": background.as_ref().map(|b| b["device"].clone()),
            "connected_to_host": background.as_ref().map(|b| b["connected_to_host"].clone()),
            "streaming": self.running.lock().await.is_some(),
            "hosting": hosting,
            "preferences": preferences,
        })
    }

    fn save_preferences(&self, preferences: Preferences) -> Value {
        let preferences = match preferences.checked() {
            Ok(preferences) => preferences,
            Err(field) => return json!({"ok": false, "error": format!("invalid_{field}")}),
        };
        if let Some(path) = &self.preferences_path {
            if preferences.save(path).is_err() {
                return failed("preferences_not_saved");
            }
        }
        *self.preferences.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = preferences.clone();
        json!({"ok": true, "preferences": preferences})
    }

    /// Enrols this computer: the background part signs, this part hands it in.
    async fn enrol(&self, session: &Session, computer: &str) -> Result<account::Device, Value> {
        let challenge = self.account.challenge(session).await.map_err(refused)?;
        let answer = self.background(json!({"do": "enrolment", "challenge": challenge, "name": computer})).await.ok_or_else(|| failed("background_not_running"))?;
        let proof: Proof = serde_json::from_value(answer["proof"].clone()).map_err(|_| failed("enrolment_not_signed"))?;
        let device = self.account.submit(session, &proof).await.map_err(refused)?;
        // A hosting computer fetches its access list right away.
        if device.hosts {
            self.background(json!({"do": "refresh"})).await;
        }
        Ok(device)
    }

    async fn sign_in(&self, email: &str, password: &str, computer: &str) -> Value {
        let session = match self.account.sign_in(email, password).await {
            Ok(session) => session,
            Err(error) => return refused(error),
        };
        if let Err(error) = account::save_session(self.store.as_ref(), &session) {
            return refused(error);
        }
        // Signed in even if enrolling fails; the window shows why and can retry.
        let enrolled = self.enrol(&session, computer).await;
        let mut state = self.state().await;
        state["enrolled"] = json!(enrolled.is_ok());
        if let Err(reason) = enrolled {
            state["enrol_error"] = reason["error"].clone();
        }
        state
    }

    async fn sign_out(&self) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        // Take this computer off the account first, so it stops hosting for it.
        let mine = self.background(json!({"do": "status"})).await.map(|b| b["device"].clone());
        let mut removed = false;
        if let (Some(mine), Ok(devices)) = (mine, self.account.devices(&session).await) {
            if let Some(device) = devices.iter().find(|d| mine == d.fingerprint.as_str()) {
                removed = self.account.remove(&session, &device.id).await.is_ok();
            }
        }
        if let Err(error) = account::forget_session(self.store.as_ref()) {
            return refused(error);
        }
        self.background(json!({"do": "disconnect"})).await;
        self.background(json!({"do": "refresh"})).await;
        json!({"ok": true, "device_removed": removed})
    }

    async fn devices(&self) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        let mine = self.background(json!({"do": "status"})).await.map(|b| b["device"].clone());
        match self.account.devices(&session).await {
            Ok(devices) => json!({
                "ok": true,
                "devices": devices.iter().map(|d| json!({
                    "id": d.id, "name": d.name, "platform": d.platform, "hosts": d.hosts, "online": d.online,
                    "this": mine.as_ref().is_some_and(|m| *m == d.fingerprint.as_str()),
                })).collect::<Vec<_>>(),
            }),
            Err(error) if self.expired(&error) => failed("signed_out"),
            Err(error) => refused(error),
        }
    }

    async fn connect(&self, id: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        // Ask now: the computer's address may have changed since the list was shown.
        let devices = match self.account.devices(&session).await {
            Ok(devices) => devices,
            Err(error) if self.expired(&error) => return failed("signed_out"),
            Err(error) => return refused(error),
        };
        let Some(device) = devices.iter().find(|d| d.id == id) else { return failed("device_not_found") };
        let Some(reach) = &device.reach else { return failed("offline") };
        let addresses: Vec<String> = reach
            .addresses
            .iter()
            .map(|a| if a.contains(':') { format!("[{a}]:{}", reach.port) } else { format!("{a}:{}", reach.port) })
            .collect();
        let request = json!({"do": "connect", "host": device.fingerprint, "addresses": addresses});
        // Connecting to a computer replaces any stream and tunnel already open.
        stream::stop(&self.running).await;
        self.background(json!({"do": "disconnect"})).await;
        // A computer that has not heard of this device yet fetches a new list
        // when it is refused, so a second and third try are worth making.
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            match self.background(request.clone()).await {
                None => return failed("background_not_running"),
                Some(answer) if answer["ok"] == true => {
                    let Some(settings) = &self.stream else {
                        return json!({"ok": true, "id": id, "streaming": false, "stream_address": answer["stream_address"]});
                    };
                    let address = answer["stream_address"].as_str().unwrap_or_default().to_string();
                    let preferences = self.preferences.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
                    return match stream::start(settings, &preferences, &device.fingerprint, &address, &self.channel, &self.running).await {
                        Ok(()) => {
                            tokio::spawn(stream::watch(settings.clone(), device.fingerprint.clone(), self.channel.clone(), self.running.clone()));
                            json!({"ok": true, "id": id, "streaming": true})
                        }
                        Err(reason) => {
                            // No tunnel is left open without a stream using it.
                            self.background(json!({"do": "disconnect"})).await;
                            failed(reason)
                        }
                    };
                }
                Some(_) => {}
            }
        }
        failed("not_reachable")
    }

    async fn remove_device(&self, id: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.remove(&session, id).await {
            Ok(()) => {
                // If this computer hosts, it drops the removed device now, not at its next refresh.
                self.background(json!({"do": "refresh"})).await;
                json!({"ok": true})
            }
            Err(error) if self.expired(&error) => failed("signed_out"),
            Err(error) => refused(error),
        }
    }
}

fn configure(channel: String, test_account: Option<(String, String)>, memory_store: bool, stream: Option<stream::Settings>) -> Value {
    let account = match &test_account {
        Some((origin, address)) => Service::local_test(origin, address),
        None => Service::new(account::ORIGIN),
    };
    let (Ok(account), Ok(runtime)) = (account, tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()) else {
        return failed("not_configured");
    };
    let store: Box<dyn SecretStore> = if memory_store { Box::new(MemoryStore::default()) } else { Box::new(SystemStore) };
    let preferences_path = stream.as_ref().map(|settings| settings.data.join("preferences.json"));
    let preferences = preferences_path.as_deref().map(Preferences::load).unwrap_or_default();
    let app = App {
        runtime,
        account,
        store,
        channel,
        stream: stream.map(Arc::new),
        running: Default::default(),
        preferences: Mutex::new(preferences),
        preferences_path,
    };
    match APP.set(app) {
        Ok(()) => json!({"ok": true}),
        Err(_) => failed("already_configured"),
    }
}

/// Answers one request. Blocks until the answer is ready.
pub fn call(request: &str) -> Value {
    let Ok(request) = serde_json::from_str::<Request>(request) else { return failed("unknown_request") };
    let _turn = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Request::Configure { channel, test_account, memory_store, stream_component, data_folder } = request {
        let stream = match (stream_component, data_folder) {
            (Some(component), Some(data)) => Some(stream::Settings { component: component.into(), data: data.into() }),
            _ => None,
        };
        return configure(channel, test_account, memory_store, stream);
    }
    let Some(app) = APP.get() else { return failed("not_configured") };
    app.runtime.block_on(async {
        match request {
            Request::Configure { .. } => failed("already_configured"),
            Request::State => app.state().await,
            Request::SignIn { email, password, computer } => app.sign_in(&email, &password, &computer).await,
            Request::SignOut => app.sign_out().await,
            Request::Devices => app.devices().await,
            Request::RemoveDevice { id } => app.remove_device(&id).await,
            Request::Connect { id } => app.connect(&id).await,
            Request::Preferences => {
                let preferences = app.preferences.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
                json!({"ok": true, "preferences": preferences})
            }
            Request::SavePreferences { preferences } => app.save_preferences(preferences),
            Request::Disconnect => {
                stream::stop(&app.running).await;
                app.background(json!({"do": "disconnect"})).await.unwrap_or_else(|| failed("background_not_running"))
            }
        }
    })
}

/// For the window: `request` is a terminated UTF-8 JSON string. The answer is
/// a new string the caller gives back to `secpar_app_free`.
///
/// # Safety
/// `request` must point at a terminated string that stays valid for the call.
#[no_mangle]
pub unsafe extern "C" fn secpar_app_call(request: *const std::ffi::c_char) -> *mut std::ffi::c_char {
    let answer = if request.is_null() {
        failed("unknown_request")
    } else {
        // SAFETY: the caller promises a terminated string.
        match unsafe { std::ffi::CStr::from_ptr(request) }.to_str() {
            Ok(text) => std::panic::catch_unwind(|| call(text)).unwrap_or_else(|_| failed("internal_error")),
            Err(_) => failed("unknown_request"),
        }
    };
    std::ffi::CString::new(answer.to_string()).map(std::ffi::CString::into_raw).unwrap_or(std::ptr::null_mut())
}
/// # Safety
/// `answer` must come from `secpar_app_call` and not have been freed before.
#[no_mangle]
pub unsafe extern "C" fn secpar_app_free(answer: *mut std::ffi::c_char) {
    if !answer.is_null() {
        // SAFETY: the pointer came from CString::into_raw in secpar_app_call.
        drop(unsafe { std::ffi::CString::from_raw(answer) });
    }
}
