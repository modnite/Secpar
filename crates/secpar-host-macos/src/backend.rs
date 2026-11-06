//! Local application façade plus the agent on port 48999; mirrors the Linux commands.
use crate::{
    platform::{Permission, Platform},
    session::OwnerSession,
};
use secpar_host_core::{
    agent::{Agent, AgentHandle, BoxFut, Handlers, Outbox, Session},
    limits::StreamSlots,
    proto::PcInfo,
};
use secpar_host_linux::{identity::Identity, store::{Config, Store}};
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

pub struct Backend {
    store: Arc<Mutex<Store>>,
    identity: Identity,
    platform: Arc<dyn Platform>,
    name: String,
    agent: Mutex<Option<AgentHandle>>,
    active: Arc<StreamSlots>,
}
pub fn local_addresses() -> Vec<String> {
    let mut all: Vec<String> = get_if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip.to_string()),
            _ => None,
        })
        .collect();
    all.sort();
    all.dedup();
    all
}
impl Backend {
    /// `bind` is 0.0.0.0:48999 in the app; tests pass an ephemeral loopback port.
    pub async fn start(directory: &Path, platform: Arc<dyn Platform>, bind: SocketAddr) -> Result<Arc<Self>, String> {
        let store = Store::open(directory).map_err(|_| "Could not open private Secpar host settings")?;
        let identity = Identity::open(directory).map_err(|_| "Could not open Secpar host identity")?;
        let host = Arc::new(Self {
            store: Arc::new(Mutex::new(store)),
            identity,
            name: platform.machine_name().chars().take(64).collect(),
            platform,
            agent: Mutex::new(None),
            active: Arc::default(),
        });
        let handle = Agent::start(&host.identity, host.store.clone(), host.clone(), bind, Duration::from_secs(60), None)
            .await
            .map_err(|_| "Could not start Secpar hosting on port 48999. Check whether another Secpar instance is open.")?;
        *host.agent.lock().unwrap() = Some(handle);
        Ok(host)
    }
    pub fn agent(&self) -> Option<AgentHandle> {
        self.agent.lock().unwrap().clone()
    }
    pub fn streaming(&self) -> usize {
        self.active.count()
    }
    fn port(&self) -> u16 {
        self.agent().map(|a| a.addr.port()).unwrap_or(48999)
    }
    fn pair_link(&self, token: &str) -> Result<String, String> {
        let addresses = local_addresses();
        if addresses.is_empty() {
            return Err("Connect this computer to a local network before pairing".into());
        }
        let mut link = url::Url::parse("secpar://pair").unwrap();
        link.query_pairs_mut()
            .append_pair("n", &self.name)
            .append_pair("h", &addresses.join(","))
            .append_pair("p", &self.port().to_string())
            .append_pair("fp", &self.identity.fingerprint())
            .append_pair("t", token);
        Ok(link.to_string())
    }
    pub async fn command(&self, command: &str, args: Value) -> Result<Value, String> {
        match command {
            "get_state" => {
                let connected = self.agent().map(|a| a.connected()).unwrap_or_default();
                let store = self.store.lock().unwrap();
                Ok(json!({
                    "name": self.name,
                    "devices": store.devices().iter().map(|d| json!({"id":d.id,"name":d.name,"connected":connected.contains(&d.id)})).collect::<Vec<_>>(),
                    "locked": false,
                    "streaming": self.active.count() > 0,
                    "permissions": self.platform.permissions(),
                }))
            }
            "get_config" => Ok(json!({"config": self.store.lock().unwrap().config(), "displays": self.platform.displays()})),
            "set_config" => {
                let config: Config = serde_json::from_value(args["config"].clone()).map_err(|_| "Invalid hosting settings")?;
                self.store.lock().unwrap().set_config(config).map_err(|_| "This Mac host supports local owner streaming; remote relay, files and clipboard are not enabled")?;
                Ok(Value::Null)
            }
            "request_permission" => {
                let which = match args["which"].as_str() {
                    Some("screen_recording") => Permission::ScreenRecording,
                    Some("accessibility") => Permission::Accessibility,
                    _ => return Err("Unknown permission".into()),
                };
                self.platform.request_permission(which);
                Ok(json!(self.platform.permissions()))
            }
            "new_qr" => {
                let token = self.store.lock().unwrap().new_otp();
                Ok(json!({"link": self.pair_link(&token)?}))
            }
            "revoke" => {
                let id = args["id"].as_str().ok_or("Missing device")?;
                self.agent().ok_or("Host is not running")?.revoke(id);
                Ok(Value::Null)
            }
            _ => Err("This host action is not available on macOS yet".into()),
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
            os: "macos".into(),
        };
        Box::pin(async move { pc })
    }
    fn caps(&self) -> Vec<String> {
        vec!["stream".into(), "input".into()]
    }
    fn session(&self, _device: &str, out: Outbox) -> Box<dyn Session> {
        Box::new(OwnerSession::new(self.platform.clone(), self.active.clone(), Arc::new(out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Permissions, Sink, StreamHandle, StreamRequest};
    use secpar_host_core::{limits::{StreamPermit, LIMIT_MESSAGE}, proto::{ClientMsg, ServerMsg}, Display};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Channel(Mutex<Vec<ServerMsg>>);
    impl Sink for Channel {
        fn send(&self, m: ServerMsg) -> bool { self.0.lock().unwrap().push(m); true }
    }
    impl Channel {
        fn new() -> Arc<Self> { Arc::new(Self(Mutex::default())) }
        fn codes(&self) -> Vec<String> {
            self.0.lock().unwrap().iter().map(|m| match m { ServerMsg::StreamError { code, .. } => code.clone(), _ => "other".into() }).collect()
        }
    }
    struct Fake { perms: Permissions, delay: Duration, input: Arc<Mutex<u32>>, live: Arc<Mutex<i32>> }
    struct Handle { _guard: StreamPermit, input: Arc<Mutex<u32>>, live: Arc<Mutex<i32>>, alive: AtomicBool }
    impl StreamHandle for Handle {
        fn alive(&self) -> bool { self.alive.load(Ordering::SeqCst) }
        fn input(&self, _: ClientMsg) { *self.input.lock().unwrap() += 1; }
        fn add_candidate(&self, _: SocketAddr) {}
    }
    impl Drop for Handle { fn drop(&mut self) { *self.live.lock().unwrap() -= 1; } }
    fn fake(screen: bool, ax: bool, delay_ms: u64) -> Arc<Fake> {
        Arc::new(Fake { perms: Permissions { screen_recording: screen, accessibility: ax }, delay: Duration::from_millis(delay_ms), input: Arc::default(), live: Arc::default() })
    }
    impl Platform for Fake {
        fn machine_name(&self) -> String { "Test Mac".into() }
        fn permissions(&self) -> Permissions { self.perms }
        fn request_permission(&self, _: Permission) {}
        fn displays(&self) -> Vec<Display> { vec![Display { id: "1".into(), name: "Display 1".into(), primary: true, rect: [0, 0, 1, 1] }] }
        fn start_stream(&self, _: StreamRequest, _: Arc<dyn Sink>, guard: StreamPermit) -> BoxFut<Result<(Box<dyn StreamHandle>, ServerMsg), String>> {
            let (delay, input, live) = (self.delay, self.input.clone(), self.live.clone());
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                *live.lock().unwrap() += 1;
                let ready = ServerMsg::Ok { of: "stream.ready".into() };
                Ok((Box::new(Handle { _guard: guard, input, live, alive: AtomicBool::new(true) }) as Box<dyn StreamHandle>, ready))
            })
        }
    }
    fn start_msg() -> ClientMsg {
        serde_json::from_value(json!({"t":"stream.start","w":1280,"h":720,"fps":60,"codecs":["h264"],"max_kbps":8000,"audio":false,"cands":[]})).unwrap()
    }
    fn session(p: &Arc<Fake>, slots: &Arc<StreamSlots>, out: &Arc<Channel>) -> OwnerSession {
        OwnerSession::new(p.clone(), slots.clone(), out.clone())
    }
    async fn settle() { tokio::time::sleep(Duration::from_millis(120)).await; }

    #[tokio::test]
    async fn screen_recording_denial_is_reported_and_reserves_nothing() {
        let (p, slots, out) = (fake(false, true, 0), Arc::new(StreamSlots::default()), Channel::new());
        let mut s = session(&p, &slots, &out);
        s.on_msg(start_msg());
        assert_eq!(out.codes(), ["permission_denied"]);
        assert_eq!(slots.count(), 0);
        assert_eq!(*p.live.lock().unwrap(), 0);
    }
    #[tokio::test]
    async fn three_devices_stream_a_fourth_is_refused_and_disconnect_frees_a_slot() {
        let (p, slots) = (fake(true, true, 0), Arc::new(StreamSlots::default()));
        let outs: Vec<_> = (0..4).map(|_| Channel::new()).collect();
        let mut sessions: Vec<_> = outs.iter().map(|o| session(&p, &slots, o)).collect();
        for s in &mut sessions { s.on_msg(start_msg()); }
        assert_eq!(outs[3].codes(), ["connection_limit"]);
        assert!(outs[3].0.lock().unwrap().iter().any(|m| matches!(m, ServerMsg::StreamError{msg,..} if msg==LIMIT_MESSAGE)));
        settle().await;
        assert_eq!((slots.count(), *p.live.lock().unwrap()), (3, 3));
        sessions.remove(0);
        assert_eq!((slots.count(), *p.live.lock().unwrap()), (2, 2));
        sessions[2].on_msg(start_msg());
        settle().await;
        assert_eq!(slots.count(), 3);
    }
    #[tokio::test]
    async fn stop_during_startup_discards_the_late_stream() {
        let (p, slots, out) = (fake(true, true, 40), Arc::new(StreamSlots::default()), Channel::new());
        let mut s = session(&p, &slots, &out);
        s.on_msg(start_msg());
        s.on_msg(ClientMsg::StreamStop);
        settle().await;
        assert_eq!((slots.count(), *p.live.lock().unwrap()), (0, 0));
        assert!(out.0.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn input_needs_accessibility_and_stops_with_the_session() {
        for (ax, expected) in [(false, 0), (true, 1)] {
            let (p, slots, out) = (fake(true, ax, 0), Arc::new(StreamSlots::default()), Channel::new());
            let mut s = session(&p, &slots, &out);
            s.on_msg(start_msg());
            settle().await;
            s.on_msg(serde_json::from_value(json!({"t":"mouse.move","dx":1,"dy":1})).unwrap());
            assert_eq!(*p.input.lock().unwrap(), expected);
            drop(s);
            assert_eq!((slots.count(), *p.live.lock().unwrap()), (0, 0));
        }
    }
    #[tokio::test]
    async fn identity_and_pairing_persist_and_pairing_link_pins_the_same_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let a = Backend::start(dir.path(), fake(true, true, 0), bind).await.unwrap();
        let fp = a.identity.fingerprint();
        let id = a.store.lock().unwrap().id().to_string();
        drop(a);
        let b = Backend::start(dir.path(), fake(true, true, 0), bind).await.unwrap();
        assert_eq!((b.identity.fingerprint(), b.store.lock().unwrap().id().to_string()), (fp.clone(), id));
        if let Ok(v) = b.command("new_qr", Value::Null).await {
            let link = url::Url::parse(v["link"].as_str().unwrap()).unwrap();
            let q: std::collections::HashMap<_, _> = link.query_pairs().into_owned().collect();
            assert_eq!(q["fp"], fp);
            assert!(!q["t"].is_empty());
        }
        let state = b.command("get_state", Value::Null).await.unwrap();
        assert_eq!(state["permissions"]["screen_recording"], true);
        assert!(b.command("nope", Value::Null).await.is_err());
    }
}
