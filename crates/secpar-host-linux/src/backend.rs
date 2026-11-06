//! Local application façade plus authenticated owner sessions.
use crate::{
    identity::Identity,
    portal::Portal,
    store::{Config, Store},
    stream::{self, Options, Stream},
};
use secpar_host_core::{
    agent::{Agent, AgentHandle, BoxFut, Handlers, Outbox, Session},
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
        let host = Arc::new(Self {
            store: Arc::new(Mutex::new(store)),
            identity,
            name: machine_name(),
            agent: Mutex::new(None),
            active: Arc::default(),
            account,
        });
        let handle=Agent::start(&host.identity,host.store.clone(),host.clone(),"0.0.0.0:48999".parse().unwrap(),Duration::from_secs(60),None).await.map_err(|_|"Could not start Secpar hosting on port 48999. Check whether another Secpar instance is open.")?;
        *host.agent.lock().unwrap() = Some(handle);
        let weak = Arc::downgrade(&host);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(3));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                let Some(host) = weak.upgrade() else { break };
                let Ok(account) = &host.account else { continue };
                let owner = account.state().await.owner;
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
    fn sync_account_owner(&self, owner: Option<String>) {
        let revoked = self.store.lock().unwrap().set_account_owner(owner);
        if let Some(agent) = self.agent.lock().unwrap().as_ref() {
            for device in revoked { agent.revoke(&device); }
        }
    }
    fn owner_pair_link(&self, id: &str, owner: &str) -> Result<String,String> {
        let addresses = stream::local_addresses();
        if addresses.is_empty() { return Err("Connect this computer to a network before pairing".into()); }
        let otp = self.store.lock().unwrap().owner_otp(id,owner)?;
        let mut link = url::Url::parse("secpar://pair").unwrap();
        link.query_pairs_mut().append_pair("n",&self.name).append_pair("h",&addresses.join(",")).append_pair("p","48999").append_pair("fp",&self.identity.fingerprint()).append_pair("t",&otp);
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
                let store = self.store.lock().unwrap();
                Ok(
                    json!({"name":self.name,"devices":store.devices().iter().map(|d|json!({"id":d.id,"name":d.name,"connected":connected.contains(&d.id)})).collect::<Vec<_>>(),"locked":false,"streaming":(self.active.count() > 0)}),
                )
            }
            "get_config" => Ok(json!({"config":self.store.lock().unwrap().config(),"displays":[]})),
            "set_config" => {
                let config: Config = serde_json::from_value(args["config"].clone())
                    .map_err(|_| "Invalid hosting settings")?;
                self.store.lock().unwrap().set_config(config).map_err(|_|"This Linux preview supports local owner streaming; remote relay, files and clipboard are not enabled")?;
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
                    .append_pair("p", "48999")
                    .append_pair("fp", &self.identity.fingerprint())
                    .append_pair("t", &store.new_otp());
                Ok(json!({"link":link.as_str()}))
            }
            "revoke" => {
                let id = args["id"].as_str().ok_or("Missing device")?;
                let agent = self.agent.lock().unwrap();
                agent.as_ref().ok_or("Host is not running")?.revoke(id);
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
        vec!["stream".into(), "input".into()]
    }
    fn session(&self, _device: &str, out: Outbox) -> Box<dyn Session> {
        let (cancel, _) = watch::channel(false);
        Box::new(OwnerSession {
            store: self.store.clone(),
            active: self.active.clone(),
            state: Arc::new(Mutex::new(State::default())),
            out,
            cancel,
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
    store: Arc<Mutex<Store>>,
    active: Arc<secpar_host_core::limits::StreamSlots>,
    state: Arc<Mutex<State>>,
    out: Outbox,
    cancel: watch::Sender<bool>,
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
        let generation = {
            let mut s = state.lock().unwrap();
            s.generation += 1;
            s.starting = true;
            s.generation
        };
        tokio::spawn(async move {
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
    fn on_msg(&mut self, msg: ClientMsg) -> Option<ServerMsg> {
        match msg {
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
                });
                None
            }
            ClientMsg::StreamStop => {
                self.stop();
                Some(ServerMsg::Ok {
                    of: "stream.stop".into(),
                })
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
        let _ = self.cancel.send(true);
        self.stop();
    }
}
