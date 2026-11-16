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
pub mod update;
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
    /// Removes every extra record that shares a name with another, keeping one
    /// and never this computer. For stale records from older versions.
    RemoveDuplicateDevices,
    /// The account's friends and the requests waiting for an answer.
    Friends,
    /// What one friend may do on this account's computers.
    FriendSettings { id: String },
    /// Changes a friend's permissions. Only the parts that are given are sent.
    SaveFriendSettings {
        id: String,
        #[serde(default)]
        input: Option<String>,
        #[serde(default)]
        connect: Option<String>,
        #[serde(default)]
        keyboard: Option<bool>,
        #[serde(default)]
        mouse: Option<bool>,
    },
    /// Asks the account named by `handle` to be friends.
    RequestFriend { handle: String },
    AcceptFriend { id: String },
    RemoveFriend { id: String },
    /// Open a tunnel to one of the account's hosting computers.
    Connect { id: String },
    Disconnect,
    /// The stream settings shown in Settings → Connecting.
    Preferences,
    SavePreferences { preferences: Preferences },
    /// Send the file at this path to the computer this one is connected to.
    SendFile { path: String },
    /// How far the files this computer sends have got, and the last file that
    /// arrived here from another computer.
    Transfers,
    /// A still of one of the account's computers, for its tile. `host` is that
    /// computer's certificate fingerprint and `addresses` where it can be
    /// reached. Answered as the background part answered:
    /// `{"ok": true, "jpeg", "width", "height", "at"}` or `{"ok": false, "err"}`.
    Preview {
        host: String,
        max_width: u32,
        #[serde(default)]
        addresses: Vec<String>,
    },
    /// The version of this copy and the asset a release would install here.
    UpdateState,
    /// Asks the release feed whether a newer release exists.
    UpdateCheck,
    /// Downloads the newest release, checks it, and installs it over this copy.
    UpdateInstall,
}

/// Answers an update request on a runtime of its own, so it works whether or
/// not the window has been set up yet.
fn update_call(request: Request) -> Value {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return failed("unreachable");
    };
    let current = update::CURRENT;
    match request {
        Request::UpdateState => json!({
            "ok": true,
            "current": current,
            "supported": update::asset_name().is_some(),
        }),
        Request::UpdateCheck => match runtime.block_on(update::check()) {
            Ok(Some(release)) => json!({
                "ok": true,
                "current": current,
                "available": {
                    "version": release.version,
                    "tag": release.tag,
                    "size": release.size,
                },
            }),
            Ok(None) => json!({"ok": true, "current": current, "available": null}),
            Err(error) => json!({"ok": false, "error": error}),
        },
        Request::UpdateInstall => match runtime.block_on(update::install()) {
            Ok(version) => json!({"ok": true, "version": version}),
            Err(error) => json!({"ok": false, "error": error}),
        },
        _ => failed("unknown_request"),
    }
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
        // The sentence is written for the person when it is checked here.
        Error::Invalid(text) => json!({"ok": false, "error": "invalid_answer", "message": text}),
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

    async fn save_preferences(&self, preferences: Preferences) -> Value {
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
        // The background part keeps the clipboard in step, so it hears at once.
        self.background(json!({"do": "clipboard", "on": preferences.share_clipboard})).await;
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
                    "fingerprint": d.fingerprint,
                    // Where a still or a stream can be asked for, empty when offline.
                    "addresses": d.reach.as_ref().map(reach_addresses).unwrap_or_default(),
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
        let addresses = reach_addresses(reach);
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

    async fn remove_duplicates(&self) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        // Removing the wrong record would take this computer off its own account,
        // so without the background part's identity nothing is removed.
        let Some(mine) = self.background(json!({"do": "status"})).await.and_then(|status| status["device"].as_str().map(str::to_owned)) else {
            return failed("background_not_running");
        };
        let devices = match self.account.devices(&session).await {
            Ok(devices) => devices,
            Err(error) if self.expired(&error) => return failed("signed_out"),
            Err(error) => return refused(error),
        };
        let ids = duplicate_ids(&devices, &mine);
        for id in &ids {
            if let Err(error) = self.account.remove(&session, id).await {
                return refused(error);
            }
        }
        if !ids.is_empty() {
            self.background(json!({"do": "refresh"})).await;
        }
        json!({"ok": true, "removed": ids.len()})
    }

    async fn friends(&self) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.friends(&session).await {
            Ok(friends) => json!({
                "ok": true,
                "friends": friends.iter().map(|f| json!({
                    "id": f.id, "name": f.name, "handle": f.handle,
                    "state": f.state,
                    // A request this account sent waits for the other side.
                    "incoming": f.state == "pending" && f.requester != session.user.id,
                    // Trusted friends join without being asked about each time.
                    "trusted": f.connect == "always",
                    "input": f.input, "connect": f.connect, "keyboard": f.keyboard, "mouse": f.mouse,
                })).collect::<Vec<_>>(),
            }),
            Err(error) if self.expired(&error) => failed("signed_out"),
            Err(error) => refused(error),
        }
    }

    fn settings_answer(settings: account::FriendSettings) -> Value {
        json!({
            "ok": true,
            "settings": {
                "input": settings.input, "connect": settings.connect,
                "keyboard": settings.keyboard, "mouse": settings.mouse,
            },
        })
    }

    async fn friend_settings(&self, id: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.friend_settings(&session, id).await {
            Ok(settings) => Self::settings_answer(settings),
            Err(error) if self.expired(&error) => failed("signed_out"),
            Err(error) => refused(error),
        }
    }

    async fn save_friend_settings(&self, id: &str, change: account::FriendChange) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.save_friend_settings(&session, id, &change).await {
            Ok(settings) => Self::settings_answer(settings),
            Err(error) if self.expired(&error) => failed("signed_out"),
            Err(error) => refused(error),
        }
    }

    async fn request_friend(&self, handle: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.request_friend(&session, handle).await {
            Ok(()) => json!({"ok": true}),
            Err(error) if self.expired(&error) => failed("signed_out"),
            Err(error) => refused(error),
        }
    }

    async fn accept_friend(&self, id: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.accept_friend(&session, id).await {
            Ok(()) => json!({"ok": true}),
            Err(error) if self.expired(&error) => failed("signed_out"),
            Err(error) => refused(error),
        }
    }

    async fn remove_friend(&self, id: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.remove_friend(&session, id).await {
            Ok(()) => json!({"ok": true}),
            Err(error) if self.expired(&error) => failed("signed_out"),
            Err(error) => refused(error),
        }
    }
}

/// Where a computer can be reached: its tunnel port on each of its addresses.
fn reach_addresses(reach: &account::Reach) -> Vec<String> {
    reach.addresses.iter().map(|a| if a.contains(':') { format!("[{a}]:{}", reach.port) } else { format!("{a}:{}", reach.port) }).collect()
}

/// The device records to remove as duplicates. One record is kept per name, and/// this computer is always the one kept when an older record shares its name.
fn duplicate_ids(devices: &[account::Device], mine: &str) -> Vec<String> {
    // Better to keep: this computer, then one that hosts, then one that reported in.
    fn rank(device: &account::Device, mine: &str) -> (bool, bool, bool) {
        (device.fingerprint == mine, device.hosts, device.online)
    }
    let mut kept: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, device) in devices.iter().enumerate() {
        let name = device.name.trim().to_lowercase();
        let better = kept.get(&name).is_none_or(|&at| rank(device, mine) > rank(&devices[at], mine));
        if better {
            kept.insert(name, index);
        }
    }
    let kept: std::collections::HashSet<usize> = kept.into_values().collect();
    devices.iter().enumerate().filter(|(index, _)| !kept.contains(index)).map(|(_, device)| device.id.clone()).collect()
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
    // The background part shares the clipboard unless the person turned it off,
    // and it starts on, so it is told as soon as the window is set up.
    let _ = runtime.block_on(channel::ask(&channel, &json!({"do": "clipboard", "on": preferences.share_clipboard})));
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
    // Update requests stand on their own: they need the release feed, not the
    // account or the background part.
    if matches!(request, Request::UpdateState | Request::UpdateCheck | Request::UpdateInstall) {
        return update_call(request);
    }
    let Some(app) = APP.get() else { return failed("not_configured") };
    app.runtime.block_on(async {
        match request {
            Request::Configure { .. } => failed("already_configured"),
            Request::UpdateState | Request::UpdateCheck | Request::UpdateInstall => failed("unknown_request"),
            Request::State => app.state().await,
            Request::SignIn { email, password, computer } => app.sign_in(&email, &password, &computer).await,
            Request::SignOut => app.sign_out().await,
            Request::Devices => app.devices().await,
            Request::RemoveDevice { id } => app.remove_device(&id).await,
            Request::RemoveDuplicateDevices => app.remove_duplicates().await,
            Request::Friends => app.friends().await,
            Request::FriendSettings { id } => app.friend_settings(&id).await,
            Request::SaveFriendSettings { id, input, connect, keyboard, mouse } => {
                app.save_friend_settings(&id, account::FriendChange { input, connect, keyboard, mouse }).await
            }
            Request::RequestFriend { handle } => app.request_friend(&handle).await,
            Request::AcceptFriend { id } => app.accept_friend(&id).await,
            Request::RemoveFriend { id } => app.remove_friend(&id).await,
            Request::Connect { id } => app.connect(&id).await,
            Request::Preferences => {
                let preferences = app.preferences.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
                json!({"ok": true, "preferences": preferences})
            }
            Request::SavePreferences { preferences } => app.save_preferences(preferences).await,
            Request::SendFile { path } => {
                let answer = app.background(json!({"do": "send_file", "path": path})).await;
                answer.unwrap_or_else(|| failed("background_not_running"))
            }
            Request::Transfers => {
                let answer = app.background(json!({"do": "transfers"})).await;
                answer.unwrap_or_else(|| failed("background_not_running"))
            }
            Request::Preview { host, max_width, addresses } => {
                // The background part peeks at that computer for one answer, or
                // uses the tunnel it already has; the window keeps its icon when
                // there is none.
                let answer = app.background(json!({"do": "preview", "host": host, "max_width": max_width, "addresses": addresses})).await;
                answer.unwrap_or_else(|| json!({"ok": false, "err": "not_available"}))
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, fingerprint: &str) -> account::Device {
        account::Device { id: id.into(), fingerprint: fingerprint.into(), name: name.into(), platform: "windows".into(), hosts: false, online: false, reach: None }
    }

    #[test]
    fn duplicate_records_keep_one_name_and_never_this_computer() {
        let devices = vec![
            device("1", "x250", "aa"),
            device("2", "X250 ", "bb"),
            device("3", "Desktop", "cc"),
            // This computer, listed after an older record with the same name.
            device("4", "desktop", "dd"),
            device("5", "Laptop", "ee"),
        ];
        assert_eq!(duplicate_ids(&devices, "dd"), vec!["2", "3"]);
    }

    #[test]
    fn a_record_that_hosts_is_the_one_kept_for_that_name() {
        let mut hosting = device("2", "x250", "bb");
        hosting.hosts = true;
        assert_eq!(duplicate_ids(&[device("1", "x250", "aa"), hosting], "zz"), vec!["1"]);
        // Different names and a list without duplicates remove nothing.
        assert!(duplicate_ids(&[device("1", "x250", "aa"), device("2", "desktop", "bb")], "aa").is_empty());
    }
}
