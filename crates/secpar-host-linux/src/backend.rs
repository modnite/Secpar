//! Local application façade plus authenticated owner sessions.
use crate::{
    identity::Identity,
    media::Media,
    portal::Portal,
    store::{Config, Store},
    stream::{self, Options, SharedPads, Stream},
    thumb::{self, Thumbs},
    uinput,
};
use secpar_host_core::{
    agent::{Agent, AgentHandle, BoxFut, Event, GuestAdmission, Handlers, Outbox, Session},
    guest_policy::Gate,
    files::{self, Incoming, Outgoing},
    media::Cmd,
    pad::{self, PadState, Pads},
    proto::{ClientMsg, PcInfo, ServerMsg},
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{watch, OnceCell};
static HOST: OnceCell<Arc<Backend>> = OnceCell::const_new();
fn config_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        if PathBuf::from(&path).is_absolute() {
            return Ok(PathBuf::from(path).join("secpar"));
        }
    }
    std::env::var_os("HOME")
        .map(|p| PathBuf::from(p).join(".config/secpar"))
        .ok_or_else(|| "No user configuration directory is available".into())
}
fn machine_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "Linux PC".into())
        .trim()
        .chars()
        .take(64)
        .collect()
}
/// The control port. Tests that must not touch a real host on this machine set `SECPAR_HOST_PORT`.
fn host_port() -> u16 {
    std::env::var("SECPAR_HOST_PORT").ok().and_then(|p| p.parse().ok()).filter(|p| *p != 0).unwrap_or(48999)
}
pub async fn call(command: &str, args: Value) -> Result<Value, String> {
    let backend = HOST
        .get_or_try_init(|| async { Backend::start(config_directory()?).await })
        .await?;
    backend.command(command, args).await
}
struct Backend {
    store: Arc<Mutex<Store>>,
    identity: Identity,
    name: String,
    agent: Mutex<Option<AgentHandle>>,
    active: Arc<secpar_host_core::limits::StreamSlots>,
    account: Result<Arc<secpar_host_core::account::Account>, String>,
    /// None when this host is not linked to an account that could vouch for a friend.
    guests: Option<Arc<GuestAdmission>>,
    /// One count of virtual controllers for the whole PC, owner and friends together.
    pad_budget: Arc<pad::Budget>,
    /// Names of friends with a running session, by account id.
    friends: Mutex<std::collections::HashMap<String, String>>,
    /// Files this PC offered to phones, by transfer id, until accepted or rejected.
    outgoing: Outgoing,
    /// Controllers kept for a few seconds after a phone drops, so a game does not lose them.
    lingering: pad::Lingering,
    media: Arc<Media>,
    thumbs: Arc<Thumbs<Outbox>>,
}
impl Backend {
    async fn start(directory: PathBuf) -> Result<Arc<Self>, String> {
        let store =
            Store::open(&directory).map_err(|_| "Could not open private Secpar host settings")?;
        let identity =
            Identity::open(&directory).map_err(|_| "Could not open Secpar host identity")?;
        // An account failure must not interrupt existing manually paired devices.
        let account = crate::account::open(&directory, &identity.fingerprint());
        if let Ok(account) = &account { tokio::spawn(account.clone().run()); }
        let host = Self::new(store, identity, account);
        // The first look at the GStreamer plugin registry takes a while. Every `welcome` lists the
        // host's capabilities, so without this the first device to connect waits for it.
        tokio::task::spawn_blocking(|| { let _ = thumb::available(); });
        let handle=Agent::start(&host.identity,host.store.clone(),host.clone(),std::net::SocketAddr::from(([0, 0, 0, 0], host_port())),Duration::from_secs(60),crate::rendezvous::url().map(str::to_owned)).await.map_err(|_|"Could not start Secpar hosting on its port. Check whether another Secpar instance is open.")?;
        if let Some(base) = crate::rendezvous::url() { crate::rendezvous::start(handle.clone(), host.store.clone(), base.into()); }
        let mut events = handle.events();
        *host.agent.lock().unwrap() = Some(handle);
        let names = Arc::downgrade(&host);
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                let Some(host) = names.upgrade() else { break };
                match event {
                    Event::GuestConnected { guest, handle } => {
                        let who = if handle.is_empty() { "A friend".to_string() } else { format!("@{handle}") };
                        host.friends.lock().unwrap().insert(guest, who);
                    }
                    Event::GuestDisconnected { guest } => {
                        host.friends.lock().unwrap().remove(&guest);
                    }
                    _ => {}
                }
            }
        });
        let weak = Arc::downgrade(&host);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(3));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                let Some(host) = weak.upgrade() else { break };
                let Ok(account) = &host.account else { continue };
                let owner = account.state().await.owner;
                let presence = {
                    let store = host.store.lock().unwrap();
                    secpar_host_core::account::Presence::new(store.config().friends, store.id(), crate::rendezvous::url())
                };
                account.set_presence(presence).await;
                host.sync_account_owner(owner.clone());
                if let Some(owner) = owner {
                    let _ = account.poll_pairings(|id| {
                        let host = host.clone(); let owner = owner.clone();
                        async move { host.owner_pair_link(&id,&owner) }
                    }).await;
                }
            }
        });
        Ok(host)
    }
    fn new(store: Store, identity: Identity, account: Result<Arc<secpar_host_core::account::Account>, String>) -> Arc<Self> {
        let store = Arc::new(Mutex::new(store));
        let active: Arc<secpar_host_core::limits::StreamSlots> = Arc::default();
        let busy = active.clone();
        let guests = account.as_ref().ok().map(|a| GuestAdmission::new(a.clone()));
        Arc::new(Self {
            guests,
            pad_budget: Arc::default(),
            friends: Default::default(),
            thumbs: Arc::new(Thumbs::new(Arc::new(thumb::PortalSource { store: store.clone() }), move || busy.count() > 0)),
            store,
            identity,
            name: machine_name(),
            agent: Mutex::new(None),
            active,
            account,
            outgoing: Outgoing::default(),
            lingering: Default::default(),
            media: Arc::new(Media::system()),
        })
    }
    fn sync_account_owner(&self, owner: Option<String>) {
        let revoked = self.store.lock().unwrap().set_account_owner(owner);
        if let Some(agent) = self.agent.lock().unwrap().as_ref() {
            for device in revoked { agent.revoke(&device); }
        }
    }
    /// `pc` and a signed Worker address, so a phone outside the LAN can pair (once registered).
    fn add_relay_bootstrap(&self, link: &mut url::Url) {
        let params = crate::rendezvous::pairing_params(&self.store.lock().unwrap(), crate::rendezvous::url());
        if let Some((pc, r)) = params { link.query_pairs_mut().append_pair("pc",&pc).append_pair("r",&r); }
    }
    fn owner_pair_link(&self, id: &str, owner: &str) -> Result<String,String> {
        let addresses = stream::local_addresses();
        if addresses.is_empty() { return Err("Connect this computer to a network before pairing".into()); }
        let otp = self.store.lock().unwrap().owner_otp(id,owner)?;
        let mut link = url::Url::parse("secpar://pair").unwrap();
        link.query_pairs_mut().append_pair("n",&self.name).append_pair("h",&addresses.join(",")).append_pair("p",&host_port().to_string()).append_pair("fp",&self.identity.fingerprint()).append_pair("t",&otp);
        self.add_relay_bootstrap(&mut link);
        Ok(link.to_string())
    }
    async fn command(&self, command: &str, args: Value) -> Result<Value, String> {
        match command {
            "get_state" => {
                let connected = self
                    .agent
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|a| a.connected())
                    .unwrap_or_default();
                let guests = self.agent.lock().unwrap().as_ref().map(|a| a.guests()).unwrap_or_default();
                let names = self.friends.lock().unwrap();
                let friends: Vec<Value> = guests
                    .iter()
                    .map(|id| json!({"id":id,"name":names.get(id).cloned().unwrap_or_else(|| "A friend".into()),"connected":true}))
                    .collect();
                let store = self.store.lock().unwrap();
                Ok(
                    json!({"name":self.name,"devices":store.devices().iter().map(|d|json!({"id":d.id,"name":d.name,"connected":connected.contains(&d.id)})).collect::<Vec<_>>(),"friends":friends,"locked":false,"streaming":(self.active.count() > 0)}),
                )
            }
            // `displays` stays empty: the portal picks the monitor, so there is nothing to switch here.
            "get_config" => {
                let pad = uinput::status();
                Ok(json!({"config":self.store.lock().unwrap().config(),"displays":[],"pad":{"ok":pad.ok,"text":pad.text}}))
            }
            "set_config" => {
                let config: Config = serde_json::from_value(args["config"].clone())
                    .map_err(|_| "Invalid hosting settings")?;
                self.store.lock().unwrap().set_config(config).map_err(|_|"Remote hosting is not enabled on Linux yet")?;
                Ok(Value::Null)
            }
            "new_qr" => {
                let addresses = stream::local_addresses();
                if addresses.is_empty() {
                    return Err("Connect this computer to a local network before pairing".into());
                }
                let mut store = self.store.lock().unwrap();
                let mut link = url::Url::parse("secpar://pair").unwrap();
                link.query_pairs_mut()
                    .append_pair("n", &self.name)
                    .append_pair("h", &addresses.join(","))
                    .append_pair("p", &host_port().to_string())
                    .append_pair("fp", &self.identity.fingerprint())
                    .append_pair("t", &store.new_otp());
                drop(store);
                self.add_relay_bootstrap(&mut link);
                Ok(json!({"link":link.as_str()}))
            }
            "revoke" => {
                let id = args["id"].as_str().ok_or("Missing device")?;
                let agent = self.agent.lock().unwrap();
                agent.as_ref().ok_or("Host is not running")?.revoke(id);
                Ok(Value::Null)
            }
            "host_friends" => Ok(json!(self.account.as_ref().map_err(Clone::clone)?.friend_rows().await?)),
            "host_friend_set" => {
                let id = args["id"].as_str().ok_or("Missing friend")?;
                let flag = |key: &str| args[key].as_bool();
                let rows = self.account.as_ref().map_err(Clone::clone)?.set_friend(id, flag("keyboard"), flag("mouse"), args["connect"].as_str()).await?;
                Ok(json!(rows))
            }
            "disconnect_friend" => {
                let id = args["id"].as_str().ok_or("Missing friend")?;
                let agent = self.agent.lock().unwrap();
                agent.as_ref().ok_or("Host is not running")?.disconnect_guest(id);
                Ok(Value::Null)
            }
            "host_account_state" => Ok(json!(self.account.as_ref().map_err(Clone::clone)?.state().await)),
            "host_enrollment_proof" => {
                let challenge = serde_json::from_value::<secpar_host_core::account_link::Challenge>(args["challenge"].clone()).map_err(|_| "Invalid account enrollment challenge")?;
                self.account.as_ref().map_err(Clone::clone)?.allow_owner(&challenge.owner).await?;
                Ok(json!(secpar_host_core::account_link::prove(&self.identity, challenge, &self.name)?))
            }
            "host_account_install" => {
                let registration = serde_json::from_value(args["registration"].clone()).map_err(|_| "Invalid host registration")?;
                let state = self.account.as_ref().map_err(Clone::clone)?.install(registration).await?;
                self.sync_account_owner(state.owner.clone());
                Ok(json!(state))
            }
            "host_account_unlink" => {
                let state = self.account.as_ref().map_err(Clone::clone)?.unlink().await?;
                self.sync_account_owner(None);
                Ok(json!(state))
            }
            _ => Err("This host action is not available on Linux yet".into()),
        }
    }
}
impl Handlers for Backend {
    fn pc_info(&self) -> BoxFut<PcInfo> {
        let pc = PcInfo {
            id: self.store.lock().unwrap().id().into(),
            name: self.name.clone(),
            user: std::env::var("USER").unwrap_or_default(),
            macs: vec![],
            os: "linux".into(),
        };
        Box::pin(async move { pc })
    }
    fn caps(&self) -> Vec<String> {
        let mut caps: Vec<String> = ["stream", "input", "clip", "media", "files", "fsbrowse", "fsbatch"].map(String::from).to_vec();
        if thumb::available() {
            caps.push("thumb".into());
        }
        // Checked on every connection, so fixing the permission needs no restart.
        if uinput::available() {
            caps.push("pad".into());
        }
        caps
    }
    fn session(&self, device: &str, out: Outbox) -> Box<dyn Session> {
        self.build(device, out, None)
    }
    fn guests(&self) -> Option<Arc<GuestAdmission>> {
        self.guests.clone()
    }
    fn guest_refusal(&self) -> Option<&'static str> {
        (!self.store.lock().unwrap().config().friends).then_some("Friends are switched off on this PC.")
    }
    fn guest_locked(&self) -> bool {
        crate::lock::locked_or_unknown()
    }
    fn guest_session(&self, gate: &Gate, guest: &str, out: Outbox) -> Option<Box<dyn Session>> {
        Some(self.build(&format!("guest:{guest}"), out, Some(gate.clone())))
    }
}
impl Backend {
    fn build(&self, device: &str, out: Outbox, gate: Option<Gate>) -> Box<dyn Session> {
        let (cancel, _) = watch::channel(false);
        let pads = if uinput::available() {
            let rumble = out.clone();
            Some(
                Pads::new(
                    Arc::new(uinput::UinputBus),
                    Box::new(move |id, lo, hi| {
                        rumble.send(ServerMsg::Rumble { id, lo, hi });
                    }),
                )
                .for_device(device, self.lingering.clone())
                .with_budget(
                    self.pad_budget.clone(),
                    if gate.is_some() { pad::Who::Friend } else { pad::Who::Owner },
                    if gate.is_some() { pad::MAX_PER_FRIEND } else { pad::MAX_CONTROLLERS },
                ),
            )
        } else {
            None
        };
        let incoming = crate::files::incoming(crate::files::files_dir(&self.store.lock().unwrap().config().files_dir));
        Box::new(OwnerSession {
            pads: Arc::new(Mutex::new(pads)),
            incoming,
            outgoing: self.outgoing.clone(),
            sending: Default::default(),
            media: self.media.clone(),
            media_task: None,
            thumbs: self.thumbs.clone(),
            thumb: None,
            store: self.store.clone(),
            active: self.active.clone(),
            state: Arc::new(Mutex::new(State::default())),
            out,
            cancel,
            clipboard: None,
            relay: crate::rendezvous::url().map(|base| {
                let store = self.store.clone();
                Arc::new(move || crate::rendezvous::turn_server(&store, base)) as stream::RelayProvider
            }),
            gate,
        })
    }
}
#[derive(Default)]
struct State {
    stream: Option<Stream>,
    starting: bool,
    generation: u64,
}
struct OwnerSession {
    /// None inside when /dev/uinput is not writable.
    pads: SharedPads,
    incoming: Incoming,
    outgoing: Outgoing,
    sending: secpar_host_core::files::Sending,
    media: Arc<Media>,
    media_task: Option<tokio::task::JoinHandle<()>>,
    thumbs: Arc<Thumbs<Outbox>>,
    thumb: Option<thumb::Subscription<Outbox>>,
    clipboard: Option<crate::clipboard::Sharing>,
    store: Arc<Mutex<Store>>,
    active: Arc<secpar_host_core::limits::StreamSlots>,
    state: Arc<Mutex<State>>,
    out: Outbox,
    cancel: watch::Sender<bool>,
    /// TURN credentials from the rendezvous Worker, when remote access is built in.
    relay: Option<stream::RelayProvider>,
    /// Set for a friend's session. The agent filters first; this is the second check.
    gate: Option<Gate>,
}
impl OwnerSession {
    fn stop(&self) {
        self.cancel.send_replace(true);
        let mut state = self.state.lock().unwrap();
        state.generation += 1;
        state.starting = false;
        state.stream = None;
    }
    fn start(&self, options: Options) {
        {
            let state = self.state.lock().unwrap();
            if state.starting || state.stream.as_ref().is_some_and(Stream::alive) {
                self.out.send(ServerMsg::StreamError {
                    code: "busy".into(),
                    msg: "This device is already streaming".into(),
                });
                return;
            }
        }
        let Some(guard) = self.active.try_acquire() else {
            self.out.send(ServerMsg::StreamError {
                code: "connection_limit".into(),
                msg: secpar_host_core::limits::LIMIT_MESSAGE.into(),
            });
            return;
        };
        let state = self.state.clone();
        let out = self.out.clone();
        let store = self.store.clone();
        let mut cancel = self.cancel.subscribe();
        let thumbs = self.thumbs.clone();
        let generation = {
            let mut s = state.lock().unwrap();
            s.generation += 1;
            s.starting = true;
            s.generation
        };
        tokio::spawn(async move {
            // The preview holds its own screen-sharing session. Close it so the stream has the screen.
            thumbs.release().await;
            let token = store.lock().unwrap().portal_token();
            let portal =
                tokio::select! {r=Portal::open(token.as_deref())=>r,_=cancel.changed()=>return};
            let result = match portal {
                Ok(portal) => {
                    let _ = store
                        .lock()
                        .unwrap()
                        .set_portal_token(portal.restore_token.clone());
                    Stream::start(portal, options, out.clone(), guard).await
                }
                Err(e) => Err(e),
            };
            let mut s = state.lock().unwrap();
            if s.generation != generation {
                return;
            }
            s.starting = false;
            match result {
                Ok((stream, ready)) => {
                    s.stream = Some(stream);
                    out.send(ready);
                }
                Err(e) => {
                    eprintln!("secpar: the stream could not start: {e}");
                    out.send(ServerMsg::StreamError {
                        code: "capture_failed".into(),
                        msg: e,
                    });
                }
            }
        });
    }
}
impl Session for OwnerSession {
    fn on_binary(&mut self, b: &[u8]) -> Option<ServerMsg> {
        self.incoming.chunk(b)
    }
    fn on_msg(&mut self, msg: ClientMsg) -> Option<ServerMsg> {
        if self.gate.is_some()
            && matches!(msg, ClientMsg::Power { .. } | ClientMsg::Unlock { .. } | ClientMsg::FsList { .. } | ClientMsg::FsGet { .. }
                | ClientMsg::Net | ClientMsg::MediaSub { .. } | ClientMsg::MediaCmd { .. } | ClientMsg::ThumbSub { .. })
        {
            return Some(ServerMsg::error("forbidden", "This is not allowed for a friend.", Some(msg.t())));
        }
        match msg {
            ClientMsg::MediaSub { on } => {
                if let Some(task) = self.media_task.take() {
                    task.abort();
                }
                if on {
                    self.media_task = Some(crate::media::subscribe(self.media.clone(), self.out.clone()));
                }
                None
            }
            ClientMsg::MediaCmd { c, v } => match Cmd::parse(&c, v) {
                Ok(cmd) => {
                    let media = self.media.clone();
                    tokio::spawn(async move { media.run(cmd).await });
                    None
                }
                Err(code) => Some(ServerMsg::error(code, "unknown media command", Some("media.cmd"))),
            },
            ClientMsg::ThumbSub { on } => {
                self.thumb = on.then(|| self.thumbs.subscribe(self.out.clone()));
                None
            }
            ClientMsg::Pad { id, btn, lt, rt, lx, ly, rx, ry } => {
                let mut pads = self.pads.lock().unwrap();
                let Some(p) = pads.as_mut() else {
                    return Some(ServerMsg::error("unsupported", &uinput::status().text, Some("pad")));
                };
                p.pad(id, &PadState { btn, lt, rt, lx, ly, rx, ry }).err().map(|code| ServerMsg::error(code, "gamepad error", Some("pad")))
            }
            ClientMsg::PadOff { id } => {
                if let Some(p) = self.pads.lock().unwrap().as_mut() {
                    p.off(id);
                }
                None
            }
            ClientMsg::FileBegin { id, name, size } => Some(if self.store.lock().unwrap().config().allow_files {
                self.incoming.begin(&id, &name, size)
            } else {
                ServerMsg::FileError { id, code: "denied".into() }
            }),
            ClientMsg::FileEnd { id, sha256 } => Some(self.incoming.end(&id, &sha256)),
            ClientMsg::FileAccept { id, offset } => {
                let Some(path) = self.outgoing.lock().unwrap().remove(&id) else {
                    return Some(ServerMsg::FileError { id, code: "denied".into() });
                };
                let out = self.out.clone();
                let alive = self.sending.start(&id);
                tokio::task::spawn_blocking(move || {
                    let Some(key) = files::parse_uuid(&id) else { return };
                    match files::stream(&path, &key, offset, &mut |f| alive.load(std::sync::atomic::Ordering::Acquire) && out.send_bin_blocking(f)) {
                        Ok(sha256) => out.send_after_bin_blocking(ServerMsg::FileEnd { id, sha256 }),
                        Err(_) => out.send(ServerMsg::FileError { id, code: "denied".into() }),
                    };
                });
                None
            }
            ClientMsg::FileReject { id } => {
                self.outgoing.lock().unwrap().remove(&id);
                None
            }
            ClientMsg::FileCancel { id } => {
                self.sending.cancel(&id);
                self.incoming.cancel(&id);
                self.outgoing.lock().unwrap().remove(&id);
                None
            }
            ClientMsg::FsList { path } => {
                if !self.store.lock().unwrap().config().allow_files {
                    return Some(ServerMsg::error("unsupported", "File transfer is off in Hosting settings", Some("fs.list")));
                }
                match crate::files::list(&path) {
                    Ok(entries) => Some(ServerMsg::FsList { path, entries }),
                    Err(e) => Some(ServerMsg::error("denied", &e.to_string(), Some("fs.list"))),
                }
            }
            ClientMsg::FsGet { path, batch } => {
                if !self.store.lock().unwrap().config().allow_files {
                    return Some(ServerMsg::error("unsupported", "File transfer is off in Hosting settings", Some("fs.get")));
                }
                let (outgoing, out) = (self.outgoing.clone(), self.out.clone());
                tokio::task::spawn_blocking(move || match files::offer_tree(&outgoing, std::path::Path::new(&path)) {
                    Ok(offers) => offers.into_iter().for_each(|mut m| {
                        if let ServerMsg::FileOffer { batch: tag, .. } = &mut m { *tag = batch; }
                        out.send(m);
                    }),
                    Err(e) => {
                        out.send(ServerMsg::error("denied", &e.to_string(), Some("fs.get")));
                    }
                });
                None
            }
            ClientMsg::ClipSub{on}=>{
                if !on {self.clipboard=None;return None;}
                if !self.store.lock().unwrap().config().allow_clip {
                    return Some(ServerMsg::error("unsupported","Clipboard sharing is off in Hosting settings",Some("clip.sub")));
                }
                if self.clipboard.is_none(){self.clipboard=Some(crate::clipboard::Sharing::start(self.store.clone(),self.out.clone()));}
                None
            }
            ClientMsg::Clip{mime,data}=>{
                if mime!="text/plain" || !secpar_clipboard::valid(&data) {
                    return Some(ServerMsg::error("bad_request","Clipboard supports text up to 1 MB",Some("clip")));
                }
                if !self.store.lock().unwrap().config().allow_clip {
                    return Some(ServerMsg::error("unsupported","Clipboard sharing is off in Hosting settings",Some("clip")));
                }
                if self.clipboard.as_ref().is_some_and(|sharing|sharing.set(data)) {None}
                else {Some(ServerMsg::error("clipboard_unavailable","Clipboard sharing is unavailable",Some("clip")))}
            }
            ClientMsg::StreamStart {
                w,
                h,
                fps,
                codecs,
                max_kbps,
                audio,
                cands,
                ..
            } => {
                self.start(Options {
                    width: w,
                    height: h,
                    fps: if fps == 0 { 60 } else { fps.clamp(1, 120) },
                    kbps: if max_kbps == 0 {
                        12000
                    } else {
                        max_kbps.clamp(1000, 50000)
                    },
                    codecs,
                    candidates: cands
                        .iter()
                        .take(16)
                        .filter_map(|c| c.parse().ok())
                        .collect(),
                    audio,
                    relay: self.relay.clone(),
                    pads: self.pads.clone(),
                    gate: self.gate.clone(),
                });
                None
            }
            ClientMsg::StreamStop => {
                self.stop();
                Some(ServerMsg::Ok {
                    of: "stream.stop".into(),
                })
            }
            ClientMsg::StreamSize { w, h } => {
                if let Some(stream) = self.state.lock().unwrap().stream.as_ref() {
                    stream.resize(w, h);
                }
                None
            }
            ClientMsg::StreamCand { c } => {
                if let Ok(peer) = c.parse() {
                    if let Some(stream) = self.state.lock().unwrap().stream.as_ref() {
                        stream.add_candidate(peer)
                    }
                }
                None
            }
            m @ (ClientMsg::MouseMove { .. }
            | ClientMsg::MousePos { .. }
            | ClientMsg::MouseButton { .. }
            | ClientMsg::MouseScroll { .. }
            | ClientMsg::Key { .. }
            | ClientMsg::Text { .. }) => {
                if let Some(stream) = self.state.lock().unwrap().stream.as_ref() {
                    stream.input(m.sanitize());
                }
                None
            }
            other => Some(ServerMsg::error(
                "unsupported",
                "This action is not available on this Linux host",
                Some(other.t()),
            )),
        }
    }
}
impl Drop for OwnerSession {
    fn drop(&mut self) {
        if let Some(task) = self.media_task.take() {
            task.abort();
        }
        if let Some(p) = self.pads.lock().unwrap().as_mut() {
            p.linger(Duration::from_secs(5));
        }
        let _ = self.cancel.send(true);
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secpar_host_core::agent::OutboxRx;
    use secpar_host_core::proto::parse_binary;

    const ID: &str = "0123abcd-0000-4000-8000-00000000beef";

    struct Rig {
        host: Arc<Backend>,
        dir: PathBuf,
        session: Box<dyn Session>,
        rx: OutboxRx,
    }

    fn rig(name: &str, allow_files: bool) -> Rig {
        let dir = std::env::temp_dir().join(format!("secpar-backend-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("config");
        let mut store = Store::open(&config).unwrap();
        let mut c = store.config();
        c.allow_files = allow_files;
        c.files_dir = dir.join("inbox").to_string_lossy().into_owned();
        store.set_config(c).unwrap();
        let identity = Identity::open(&config).unwrap();
        let host = Backend::new(store, identity, Err("offline".into()));
        let (out, rx) = Outbox::pair();
        let session = host.session("phone", out);
        Rig { host, dir, session, rx }
    }

    fn sha(b: &[u8]) -> String {
        crate::store::hex(ring::digest::digest(&ring::digest::SHA256, b).as_ref())
    }

    #[tokio::test]
    async fn welcome_caps_follow_what_this_machine_can_do() {
        let r = rig("caps", false);
        let caps = r.host.caps();
        for c in ["stream", "input", "clip", "media", "files", "fsbrowse"] {
            assert!(caps.iter().any(|x| x == c), "{c} in {caps:?}");
        }
        assert_eq!(caps.iter().any(|c| c == "pad"), uinput::available(), "pad only with /dev/uinput access");
        assert_eq!(caps.iter().any(|c| c == "thumb"), thumb::available());
        let config = r.host.command("get_config", Value::Null).await.unwrap();
        assert_eq!(config["pad"]["ok"], json!(uinput::available()));
        assert!(config["pad"]["text"].as_str().unwrap().starts_with("Gamepad"));
        assert_eq!(config["config"]["allow_files"], json!(false), "this rig turned it off");
        assert!(crate::store::Config::default().allow_files, "on by default, like the Windows host");
    }

    #[tokio::test]
    async fn files_are_refused_until_the_setting_is_on() {
        let mut r = rig("off", false);
        let denied = ServerMsg::FileError { id: ID.into(), code: "denied".into() };
        assert_eq!(r.session.on_msg(ClientMsg::FileBegin { id: ID.into(), name: "a.txt".into(), size: 1 }), Some(denied));
        for m in [ClientMsg::FsList { path: String::new() }, ClientMsg::FsGet { path: "/".into(), batch: None }] {
            let of = m.t();
            assert_eq!(r.session.on_msg(m), Some(ServerMsg::error("unsupported", "File transfer is off in Hosting settings", Some(of))));
        }
    }

    #[tokio::test]
    async fn phone_to_pc_transfer_lands_in_the_configured_folder_with_the_wire_messages() {
        let mut r = rig("upload", true);
        let data = b"hello from the phone".to_vec();
        assert_eq!(
            r.session.on_msg(ClientMsg::FileBegin { id: ID.into(), name: "Trip/a.txt".into(), size: data.len() as u64 }),
            Some(ServerMsg::FileReady { id: ID.into(), offset: 0 })
        );
        let key = files::parse_uuid(ID).unwrap();
        assert_eq!(r.session.on_binary(&secpar_host_core::proto::chunk_frame(&key, 0, &data)), None);
        let Some(ServerMsg::FileDone { path, .. }) = r.session.on_msg(ClientMsg::FileEnd { id: ID.into(), sha256: sha(&data) }) else { panic!("file.done") };
        assert_eq!(PathBuf::from(&path), r.dir.join("inbox").join("Trip").join("a.txt"));
        assert_eq!(std::fs::read(path).unwrap(), data);
        // a wrong hash is rejected and leaves nothing behind
        r.session.on_msg(ClientMsg::FileBegin { id: ID.into(), name: "b.txt".into(), size: 1 });
        r.session.on_binary(&secpar_host_core::proto::chunk_frame(&key, 0, b"x"));
        assert_eq!(
            r.session.on_msg(ClientMsg::FileEnd { id: ID.into(), sha256: sha(b"y") }),
            Some(ServerMsg::FileError { id: ID.into(), code: "bad_hash".into() })
        );
        assert!(!r.dir.join("inbox").join("b.txt").exists());
    }

    #[tokio::test]
    async fn pc_to_phone_browse_offer_accept_and_stream() {
        let mut r = rig("download", true);
        let shared = r.dir.join("share");
        std::fs::create_dir_all(shared.join("sub")).unwrap();
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(shared.join("sub").join("big.bin"), &data).unwrap();
        std::fs::write(shared.join(".hidden"), b"x").unwrap();
        let Some(ServerMsg::FsList { entries, .. }) = r.session.on_msg(ClientMsg::FsList { path: shared.to_string_lossy().into() }) else { panic!("fs.list") };
        assert_eq!(entries.iter().map(|e| (e.name.as_str(), e.dir)).collect::<Vec<_>>(), vec![("sub", true)], "dot-files are hidden");
        let Some(ServerMsg::FsList { entries: roots, .. }) = r.session.on_msg(ClientMsg::FsList { path: String::new() }) else { panic!("roots") };
        assert!(roots.iter().any(|e| e.name == "/") && roots.iter().any(|e| e.name == "Home"));
        assert_eq!(r.session.on_msg(ClientMsg::FsGet { path: shared.to_string_lossy().into(), batch: None }), None);
        // a folder download offers every file inside, dot-files too, in directory order
        let mut offers = vec![];
        while offers.len() < 2 {
            match r.rx.msg() {
                Some(ServerMsg::FileOffer { id, name, size, .. }) => offers.push((id, name, size)),
                Some(other) => panic!("{other:?}"),
                None => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        offers.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(offers.iter().map(|o| (o.1.as_str(), o.2)).collect::<Vec<_>>(), vec![("share/.hidden", 1), ("share/sub/big.bin", data.len() as u64)]);
        let id = offers[1].0.clone();
        // accept from offset 1000: chunks continue there and file.end carries the whole-file hash
        assert_eq!(r.session.on_msg(ClientMsg::FileAccept { id: id.clone(), offset: 1000 }), None);
        let end = loop {
            if let Some(m) = r.rx.msg() {
                break m;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(end, ServerMsg::FileEnd { id: id.clone(), sha256: sha(&data) });
        let mut got = vec![];
        while let Some(frame) = r.rx.bin() {
            let Some(secpar_host_core::proto::Binary::FileChunk { offset, data: chunk, .. }) = parse_binary(&frame) else { panic!() };
            assert_eq!(offset, 1000 + got.len() as u64);
            got.extend(chunk);
        }
        assert_eq!(got, &data[1000..]);
        // an unknown or already used offer is refused
        assert_eq!(r.session.on_msg(ClientMsg::FileAccept { id: id.clone(), offset: 0 }), Some(ServerMsg::FileError { id, code: "denied".into() }));
    }

    #[tokio::test]
    async fn media_subscription_and_commands() {
        let mut r = rig("media", false);
        assert_eq!(r.session.on_msg(ClientMsg::MediaSub { on: true }), None);
        let first = loop {
            if let Some(m) = r.rx.msg() {
                break m;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(matches!(first, ServerMsg::Media { .. }), "{first:?}");
        for c in ["play_pause", "next", "prev", "vol_up", "vol_down", "mute"] {
            assert_eq!(r.session.on_msg(ClientMsg::MediaCmd { c: c.into(), v: None }), None, "{c}");
        }
        assert_eq!(r.session.on_msg(ClientMsg::MediaCmd { c: "vol_set".into(), v: None }), Some(ServerMsg::error("bad_request", "unknown media command", Some("media.cmd"))));
        assert_eq!(r.session.on_msg(ClientMsg::MediaCmd { c: "rewind".into(), v: None }), Some(ServerMsg::error("bad_request", "unknown media command", Some("media.cmd"))));
        assert_eq!(r.session.on_msg(ClientMsg::MediaSub { on: false }), None);
    }

    #[tokio::test]
    async fn preview_without_a_saved_screen_permission_stays_silent() {
        let mut r = rig("thumb", false);
        assert_eq!(r.session.on_msg(ClientMsg::ThumbSub { on: true }), None);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(r.rx.bin().is_none(), "no stored portal token: nothing sent, nothing asked");
        assert_eq!(r.session.on_msg(ClientMsg::ThumbSub { on: false }), None);
    }

    #[tokio::test]
    async fn gamepad_messages_are_answered_by_what_the_machine_allows() {
        let mut r = rig("pad", false);
        let reply = r.session.on_msg(ClientMsg::Pad { id: 0, btn: 0x1000, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 });
        if uinput::available() {
            assert_eq!(reply, None);
            assert!(matches!(r.session.on_msg(ClientMsg::Pad { id: 4, btn: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 }), Some(ServerMsg::Error { code, .. }) if code == "bad_request"));
        } else {
            assert!(matches!(reply, Some(ServerMsg::Error { code, msg, of: Some(of) }) if code == "unsupported" && msg.starts_with("Gamepad unavailable") && of == "pad"));
        }
        assert_eq!(r.session.on_msg(ClientMsg::PadOff { id: 0 }), None);
        assert!(matches!(r.session.on_msg(ClientMsg::Power { a: "lock".into() }), Some(ServerMsg::Error { code, .. }) if code == "unsupported"), "unported actions still say so");
    }
}
