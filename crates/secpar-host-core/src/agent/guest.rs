//! Friend sessions: admission, the permission filter and the lease watcher.
//! A friend is never an owner device. This module never touches the pairing store.
use super::{Ctx, Event, Session};
use crate::account::Account;
use crate::guest_grant::{verify, ReplayGuard, VerifiedGrant};
use crate::guest_policy::{admit, Gate};
use crate::proto::{ClientMsg, ServerMsg};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the host asks the Worker whether the lease is still good.
const RECHECK: Duration = Duration::from_secs(10);
/// Consecutive failed rechecks before the session ends. Offline for this long stops output.
const MISSES: u32 = 3;
const LOCK_POLL: Duration = Duration::from_millis(500);

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Verifies a friend's grant against Secpar's signing keys and the owner's friend list.
pub struct GuestAdmission {
    account: Arc<Account>,
    replay: ReplayGuard,
    poll: Duration,
    recheck: Duration,
}

/// A grant that passed every online check but is not yet consumed.
pub struct Pass {
    pub request: String,
    pub guest: String,
    pub handle: String,
    pub gate: Gate,
    grant: VerifiedGrant,
}

impl GuestAdmission {
    pub fn new(account: Arc<Account>) -> Arc<Self> {
        Arc::new(Self { account, replay: ReplayGuard::default(), poll: LOCK_POLL, recheck: RECHECK })
    }

    #[cfg(test)]
    pub(crate) fn with_timing(account: Arc<Account>, poll: Duration, recheck: Duration) -> Arc<Self> {
        Arc::new(Self { account, replay: ReplayGuard::default(), poll, recheck })
    }

    /// Signature, audience, friendship, lease and permissions. Reads the Worker every time.
    pub async fn check(&self, token: &str) -> Result<Pass, String> {
        let computer = self.account.computer().await.ok_or("This PC is not linked to an account.")?;
        let keys = self.account.signing_keys().await?;
        let grant = verify(token, &computer, &keys, now()).map_err(String::from)?;
        let (admission, handle) = self
            .account
            .admit_guest(grant.request(), grant.guest(), grant.permissions())
            .await?;
        Ok(Pass {
            request: grant.request().into(),
            guest: grant.guest().into(),
            handle,
            gate: admission.gate().clone(),
            grant,
        })
    }

    /// One token admits one session. Call after the busy checks, right before registering.
    pub fn consume(&self, pass: &Pass) -> Result<(), &'static str> {
        self.replay.claim(&pass.grant, now())
    }

    /// Reads the lease and friend list again.
    pub async fn recheck(&self, pass: &Pass) -> Recheck {
        let Ok((roster, lease)) = tokio::try_join!(self.account.roster(), self.account.lease(&pass.request)) else {
            return Recheck::Unreachable;
        };
        match admit(&roster, &lease, pass.grant.permissions(), &pass.guest) {
            Ok(fresh) => {
                if !(fresh.gate().keyboard() && fresh.gate().mouse()) {
                    pass.gate.strip_input();
                }
                Recheck::Good
            }
            Err(_) => Recheck::Revoked,
        }
    }
}

pub enum Recheck {
    Good,
    /// Unlinked, blocked, revoked, or the owner changed what this friend may do.
    Revoked,
    /// The Worker could not be reached or answered with an error.
    Unreachable,
}

/// What the host tracks about running friend sessions.
#[derive(Default)]
pub(crate) struct GuestState {
    pub owner_streams: AtomicUsize,
    sessions: Mutex<Vec<(String, Gate)>>,
}

impl GuestState {
    pub fn any(&self) -> bool {
        !self.sessions.lock().unwrap().is_empty()
    }
    pub fn ids(&self) -> Vec<String> {
        self.sessions.lock().unwrap().iter().map(|(g, _)| g.clone()).collect()
    }
    pub fn close_all(&self, reason: &'static str) {
        for (_, gate) in self.sessions.lock().unwrap().iter() {
            gate.close_because(reason);
        }
    }
    pub fn close_one(&self, guest: &str, reason: &'static str) {
        for (g, gate) in self.sessions.lock().unwrap().iter() {
            if g == guest {
                gate.close_because(reason);
            }
        }
    }
}

/// Registration of one running friend session. Dropping it ends everything it started.
pub(crate) struct Link {
    ctx: Arc<Ctx>,
    guest: String,
    gate: Gate,
    watcher: tokio::task::JoinHandle<()>,
}

impl Link {
    pub fn start(ctx: Arc<Ctx>, admission: Arc<GuestAdmission>, pass: &Pass) -> Self {
        ctx.guest.sessions.lock().unwrap().push((pass.guest.clone(), pass.gate.clone()));
        let _ = ctx.events.send(Event::GuestConnected { guest: pass.guest.clone(), handle: pass.handle.clone() });
        let gate = pass.gate.clone();
        let watcher = tokio::spawn(watch(ctx.clone(), admission, Pass {
            request: pass.request.clone(),
            guest: pass.guest.clone(),
            handle: pass.handle.clone(),
            gate: pass.gate.clone(),
            grant: pass.grant.clone(),
        }));
        Self { ctx, guest: pass.guest.clone(), gate, watcher }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.gate.close_because("ended");
        self.watcher.abort();
        self.ctx.guest.sessions.lock().unwrap().retain(|(_, g)| !g.same(&self.gate));
        let _ = self.ctx.events.send(Event::GuestDisconnected { guest: self.guest.clone() });
    }
}

/// Ends the session on a lock, a failed lease or lost contact with the Worker.
async fn watch(ctx: Arc<Ctx>, admission: Arc<GuestAdmission>, pass: Pass) {
    let (mut since_check, mut misses) = (Duration::ZERO, 0u32);
    loop {
        tokio::time::sleep(admission.poll).await;
        if !pass.gate.is_open() {
            return;
        }
        if ctx.handlers.locked() {
            pass.gate.close_because("locked");
            return;
        }
        since_check += admission.poll;
        if since_check < admission.recheck {
            continue;
        }
        since_check = Duration::ZERO;
        match admission.recheck(&pass).await {
            Recheck::Good => misses = 0,
            Recheck::Unreachable => {
                misses += 1;
                if misses >= MISSES {
                    pass.gate.close_because("offline");
                    return;
                }
            }
            Recheck::Revoked => {
                pass.gate.close_because("revoked");
                return;
            }
        }
    }
}

pub(crate) enum Verdict {
    Allow,
    /// Ignored without a reply, for high rate input.
    Drop,
    Deny,
}

/// Whether a message from a friend may reach the host session.
pub(crate) fn verdict(m: &ClientMsg, gate: &Gate) -> Verdict {
    use ClientMsg::*;
    use Verdict::*;
    let need = |p: &str| if gate.has(p) { Allow } else { Deny };
    let quiet = |ok: bool| if ok { Allow } else { Drop };
    match m {
        Ping => Allow,
        // Releases always pass, so lowering input never leaves a key or button held.
        Key { down: false, .. } | MouseButton { down: false, .. } => Allow,
        MouseMove { .. } | MouseButton { .. } | MouseScroll { .. } | MousePos { .. } => quiet(gate.mouse()),
        Key { .. } | Text { .. } => quiet(gate.keyboard()),
        Pad { .. } | PadOff { .. } => quiet(gate.has("gamepad")),
        ClipSub { .. } | Clip { .. } => need("clipboard"),
        FileBegin { .. } | FileEnd { .. } | FileAccept { .. } | FileReject { .. } | FileCancel { .. } => need("files"),
        StreamStart { .. } | StreamDisplay { .. } | StreamSize { .. } | StreamCand { .. } => need("view"),
        StreamStop => Allow,
        // Owner only, whatever the grant says.
        Pair { .. } | Hello { .. } | Guest { .. } | GameStreamInfo { .. } | GameStreamPair { .. } | MediaSub { .. }
        | MediaCmd { .. } | Power { .. } | ThumbSub { .. } | Net | Unlock { .. } | FsList { .. } | FsGet { .. } => Deny,
    }
}

/// Whether a message from the host may go to a friend.
pub(crate) fn out_allowed(m: &ServerMsg, gate: &Gate) -> bool {
    use ServerMsg::*;
    match m {
        Welcome { .. } | Pong | Error { .. } | Ok { .. } | Lock { .. } | Status { .. } => true,
        Clip { .. } => gate.has("clipboard"),
        FileReady { .. } | FileDone { .. } | FileError { .. } | FileOffer { .. } | FileEnd { .. } => gate.has("files"),
        Rumble { .. } => gate.has("gamepad"),
        StreamReady { .. } | StreamError { .. } | StreamDisplays { .. } | CursorShape { .. } | StreamCand { .. } => {
            gate.view()
        }
        Paired { .. } | GameStreamInfo { .. } | GameStreamPaired { .. } | GameStreamError { .. } | Media { .. }
        | Net { .. } | Remote { .. } | UnlockResult { .. } | FsList { .. } => false,
    }
}

/// `welcome.caps` for a friend: only what the grant allows.
pub(crate) fn caps(all: &[String], gate: &Gate) -> Vec<String> {
    all.iter()
        .filter(|c| match c.as_str() {
            "stream" => gate.view(),
            "input" => gate.keyboard() || gate.mouse(),
            "clip" => gate.has("clipboard"),
            "files" => gate.has("files"),
            "pad" => gate.has("gamepad"),
            _ => false,
        })
        .cloned()
        .collect()
}

/// The platform session behind the permission filter.
pub(crate) struct Filtered {
    pub inner: Box<dyn Session>,
    pub gate: Gate,
    pub ctx: Arc<Ctx>,
}

impl Session for Filtered {
    fn on_binary(&mut self, b: &[u8]) -> Option<ServerMsg> {
        if self.gate.has("files") {
            self.inner.on_binary(b)
        } else {
            None
        }
    }
    fn on_msg(&mut self, mut m: ClientMsg) -> Option<ServerMsg> {
        let of = m.t();
        match verdict(&m, &self.gate) {
            Verdict::Allow => {}
            Verdict::Drop => return None,
            Verdict::Deny => return Some(ServerMsg::error("forbidden", "This is not allowed for a friend.", Some(of))),
        }
        if let ClientMsg::StreamStart { audio, .. } = &mut m {
            if self.ctx.guest.owner_streams.load(Ordering::SeqCst) > 0 {
                return Some(ServerMsg::error("busy", "The owner is using this PC.", Some(of)));
            }
            *audio = *audio && self.gate.has("audio");
        }
        self.inner.on_msg(m)
    }
}

/// Sits in front of every owner session. When the owner starts a stream, friends are closed.
pub(crate) struct OwnerWatch {
    pub inner: Box<dyn Session>,
    pub ctx: Arc<Ctx>,
    pub streaming: bool,
}

impl OwnerWatch {
    fn stopped(&mut self) {
        if std::mem::take(&mut self.streaming) {
            self.ctx.guest.owner_streams.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Session for OwnerWatch {
    fn on_binary(&mut self, b: &[u8]) -> Option<ServerMsg> {
        self.inner.on_binary(b)
    }
    fn on_msg(&mut self, m: ClientMsg) -> Option<ServerMsg> {
        match &m {
            ClientMsg::StreamStart { .. } if !self.streaming => {
                self.streaming = true;
                self.ctx.guest.owner_streams.fetch_add(1, Ordering::SeqCst);
                self.ctx.guest.close_all("owner");
            }
            ClientMsg::StreamStop => self.stopped(),
            _ => {}
        }
        self.inner.on_msg(m)
    }
}

impl Drop for OwnerWatch {
    fn drop(&mut self) {
        self.stopped();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest_policy::{admit, Admission, Lease, Roster};
    use std::collections::BTreeSet;

    fn gate(perms: &[&str], input: &str) -> Gate {
        let set: BTreeSet<String> = perms.iter().map(|s| s.to_string()).collect();
        let roster = Roster::parse(&serde_json::json!([{"id":"g","handle":"g","input":input,"connect":"always"}])).unwrap();
        let lease = Lease { guest: "g".into(), approved: set.clone(), active: true };
        match admit(&roster, &lease, &set, "g").unwrap() {
            Admission::Allow(g) | Admission::AskOwner(g) => g,
        }
    }
    fn allowed(m: ClientMsg, g: &Gate) -> bool {
        matches!(verdict(&m, g), Verdict::Allow)
    }
    fn key() -> ClientMsg {
        ClientMsg::Key { vk: 65, down: true }
    }
    fn power() -> ClientMsg {
        ClientMsg::Power { a: "sleep".into() }
    }

    #[test]
    fn owner_only_messages_are_denied_even_with_every_permission() {
        let g = gate(&["view", "audio", "gamepad", "keyboard", "mouse", "clipboard", "files"], "full");
        for m in [
            power(),
            ClientMsg::Unlock { secret: "x".into() },
            ClientMsg::FsList { path: "C:/".into() },
            ClientMsg::FsGet { path: "C:/x".into() },
            ClientMsg::Net,
            ClientMsg::MediaSub { on: true },
            ClientMsg::ThumbSub { on: true },
            ClientMsg::GameStreamInfo { id: "x".into() },
        ] {
            assert!(matches!(verdict(&m, &g), Verdict::Deny), "{}", m.t());
        }
    }

    #[test]
    fn view_only_friends_get_no_keyboard_or_mouse_and_no_reply_storm() {
        let g = gate(&["view", "keyboard", "mouse"], "view");
        assert!(matches!(verdict(&key(), &g), Verdict::Drop));
        assert!(matches!(verdict(&ClientMsg::MouseMove { dx: 1, dy: 1 }, &g), Verdict::Drop));
        assert!(allowed(ClientMsg::StreamStop, &g));
        let full = gate(&["view", "keyboard", "mouse"], "full");
        assert!(allowed(key(), &full));
    }

    #[test]
    fn lowering_input_still_lets_held_keys_be_released() {
        let g = gate(&["view", "keyboard", "mouse"], "full");
        g.strip_input();
        assert!(matches!(verdict(&key(), &g), Verdict::Drop));
        assert!(allowed(ClientMsg::Key { vk: 65, down: false }, &g));
        assert!(allowed(ClientMsg::MouseButton { b: "left".into(), down: false }, &g));
        assert!(matches!(verdict(&ClientMsg::MouseButton { b: "left".into(), down: true }, &g), Verdict::Drop));
    }

    #[test]
    fn a_gamepad_only_grant_gets_no_desktop_view_or_keyboard() {
        let g = gate(&["gamepad"], "view");
        assert!(allowed(ClientMsg::Pad { id: 0, btn: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 }, &g));
        assert!(matches!(verdict(&ClientMsg::StreamStart { w: 0, h: 0, fps: 0, codecs: vec![], max_kbps: 0, audio: false, cands: vec![], display: String::new() }, &g), Verdict::Deny));
        assert!(matches!(verdict(&key(), &g), Verdict::Drop));
        assert!(!out_allowed(&ServerMsg::StreamError { code: "x".into(), msg: "x".into() }, &g));
    }

    #[test]
    fn closing_the_gate_stops_everything_including_releases_staying_harmless() {
        let g = gate(&["view", "clipboard", "files"], "view");
        g.close();
        assert!(!allowed(ClientMsg::Clip { mime: "text/plain".into(), data: "x".into() }, &g));
        assert!(!out_allowed(&ServerMsg::Clip { mime: "text/plain".into(), data: "x".into() }, &g));
    }

    #[test]
    fn outgoing_owner_data_never_reaches_a_friend() {
        let g = gate(&["view", "audio", "gamepad", "keyboard", "mouse", "clipboard", "files"], "full");
        assert!(!out_allowed(&ServerMsg::Remote { url: "u".into(), dev: "d".into(), ticket: "t".into() }, &g));
        assert!(!out_allowed(&ServerMsg::Paired { token: "t".into(), pc: crate::proto::PcInfo { id: "i".into(), name: "n".into(), user: "u".into(), macs: vec![], os: "o".into() } }, &g));
        assert!(!out_allowed(&ServerMsg::FsList { path: "C:/".into(), entries: vec![] }, &g));
        assert!(out_allowed(&ServerMsg::Lock { locked: true }, &g));
    }

    #[test]
    fn welcome_lists_only_what_the_grant_allows() {
        let all: Vec<String> = ["input", "power", "media", "files", "clip", "thumb", "net", "stream", "unlock", "fsbrowse", "pad"].map(String::from).to_vec();
        assert_eq!(caps(&all, &gate(&["view"], "view")), vec!["stream"]);
        assert_eq!(caps(&all, &gate(&["view", "keyboard"], "view")), vec!["stream"]);
        assert_eq!(caps(&all, &gate(&["view", "keyboard"], "full")), vec!["input", "stream"]);
        assert_eq!(caps(&all, &gate(&["gamepad"], "view")), vec!["pad"]);
    }
}
