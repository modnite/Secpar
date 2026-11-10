//! What the host owner allows each friend. Both desktop hosts decide with this module.
//! The Worker already caps the signed grant. The host checks again, because the grant is
//! only as fresh as the claim and the owner can change a friend at any time.
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Input {
    #[default]
    View,
    Full,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Connect {
    #[default]
    Ask,
    Always,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Policy {
    pub input: Input,
    pub connect: Connect,
    /// Set when the owner chose keyboard and mouse separately. Absent means follow `input`.
    pub keyboard: Option<bool>,
    pub mouse: Option<bool>,
}
impl Policy {
    pub fn allows_keyboard(&self) -> bool {
        self.keyboard.unwrap_or(self.input == Input::Full)
    }
    pub fn allows_mouse(&self) -> bool {
        self.mouse.unwrap_or(self.input == Input::Full)
    }
}
#[derive(Deserialize)]
struct Row {
    id: String,
    #[serde(default)]
    handle: String,
    #[serde(flatten)]
    policy: Policy,
}
/// Accepted, unblocked friends of the host owner and what each may do.
#[derive(Clone, Debug, Default)]
pub struct Roster(HashMap<String, (Policy, String)>);
impl Roster {
    pub fn parse(json: &serde_json::Value) -> Result<Self, &'static str> {
        let rows: Vec<Row> = serde_json::from_value(json.clone()).map_err(|_| "Invalid friends response")?;
        if rows.len() > 500 {
            return Err("Invalid friends response");
        }
        Ok(Self(rows.into_iter().map(|r| (r.id, (r.policy, r.handle))).collect()))
    }
    pub fn policy(&self, guest: &str) -> Option<Policy> {
        self.0.get(guest).map(|(policy, _)| *policy)
    }
    pub fn handle(&self, guest: &str) -> Option<&str> {
        self.0.get(guest).map(|(_, handle)| handle.as_str()).filter(|h| !h.is_empty())
    }
}
/// The lease the Worker reports for one claimed request.
#[derive(Debug, Deserialize)]
pub struct Lease {
    pub guest: String,
    pub approved: BTreeSet<String>,
    pub active: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    NotAFriend,
    LeaseInactive,
    NoPermissions,
    ExceedsApproval,
}
impl Denied {
    pub fn message(&self) -> &'static str {
        match self {
            Denied::NotAFriend => "This person is not your friend.",
            Denied::LeaseInactive => "The request is no longer active.",
            Denied::NoPermissions => "This friend may not do what they asked for.",
            Denied::ExceedsApproval => "The request asks for more than you approved.",
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    /// Connect at any time. The host starts the session without asking.
    Allow(Gate),
    /// The host must show its own consent prompt before it starts the session.
    AskOwner(Gate),
}
impl Admission {
    pub fn gate(&self) -> &Gate {
        match self {
            Admission::Allow(g) | Admission::AskOwner(g) => g,
        }
    }
}
/// The permissions of one running guest session. Every video, audio and input path asks the gate.
/// Clones share state: closing one closes them all, which is how a revoked lease stops a session.
#[derive(Clone, Debug)]
pub struct Gate(Arc<GateState>);
#[derive(Debug)]
struct GateState {
    permissions: BTreeSet<String>,
    open: AtomicBool,
    keyboard_off: AtomicBool,
    mouse_off: AtomicBool,
    streaming: AtomicBool,
    announced: AtomicBool,
    handle: std::sync::Mutex<String>,
    closed: tokio::sync::Notify,
    reason: std::sync::Mutex<&'static str>,
}
impl PartialEq for Gate {
    fn eq(&self, other: &Self) -> bool {
        self.0.permissions == other.0.permissions
    }
}
impl Eq for Gate {}
impl Gate {
    fn new(permissions: BTreeSet<String>) -> Self {
        Self(Arc::new(GateState { permissions, open: AtomicBool::new(true), keyboard_off: AtomicBool::new(false), mouse_off: AtomicBool::new(false), streaming: AtomicBool::new(false), announced: AtomicBool::new(false), handle: std::sync::Mutex::new(String::new()), closed: tokio::sync::Notify::new(), reason: std::sync::Mutex::new("ended") }))
    }
    pub fn has(&self, permission: &str) -> bool {
        self.0.open.load(Ordering::SeqCst)
            && self.0.permissions.contains(permission)
            && !(self.0.keyboard_off.load(Ordering::SeqCst) && permission == "keyboard")
            && !(self.0.mouse_off.load(Ordering::SeqCst) && permission == "mouse")
    }
    pub fn view(&self) -> bool {
        self.has("view")
    }
    pub fn keyboard(&self) -> bool {
        self.has("keyboard")
    }
    pub fn mouse(&self) -> bool {
        self.has("mouse")
    }
    /// True once, the first time it is called. The host tells its owner about a friend only then, so a card
    /// preview stays quiet until the friend really starts using the PC.
    pub fn announce_once(&self) -> bool {
        !self.0.announced.swap(true, Ordering::SeqCst)
    }
    pub fn was_announced(&self) -> bool {
        self.0.announced.load(Ordering::SeqCst)
    }
    pub fn set_handle(&self, handle: &str) {
        *self.0.handle.lock().unwrap() = handle.to_string();
    }
    pub fn handle(&self) -> String {
        self.0.handle.lock().unwrap().clone()
    }
    /// The friend started a stream, so this is more than a preview.
    pub fn mark_streaming(&self) {
        self.0.streaming.store(true, Ordering::SeqCst);
    }
    /// A session that holds only the view permission and never started a stream. It is a card preview and
    /// gives way to a real connection, so it never takes a seat from one.
    pub fn is_preview(&self) -> bool {
        !self.0.streaming.load(Ordering::SeqCst) && self.0.permissions.len() == 1 && self.0.permissions.contains("view")
    }
    pub fn permissions(&self) -> &BTreeSet<String> {
        &self.0.permissions
    }
    /// Ends every permission at once. Never reopens.
    pub fn close(&self) {
        self.0.open.store(false, Ordering::SeqCst);
        self.0.closed.notify_waiters();
    }
    /// Closes with the first reason given: ended, owner, locked, offline or revoked.
    pub fn close_because(&self, reason: &'static str) {
        if self.is_open() {
            *self.0.reason.lock().unwrap() = reason;
        }
        self.close();
    }
    pub fn reason(&self) -> &'static str {
        *self.0.reason.lock().unwrap()
    }
    /// True when both handles share one session.
    pub fn same(&self, other: &Gate) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    /// Completes once the gate is closed.
    pub async fn closed(&self) {
        loop {
            let waiting = self.0.closed.notified();
            if !self.is_open() {
                return;
            }
            waiting.await;
        }
    }
    pub fn is_open(&self) -> bool {
        self.0.open.load(Ordering::SeqCst)
    }
    /// The owner lowered the friend to view only. Keyboard and mouse stop, the rest stays.
    pub fn strip_input(&self) {
        self.strip_keyboard();
        self.strip_mouse();
    }
    pub fn strip_keyboard(&self) {
        self.0.keyboard_off.store(true, Ordering::SeqCst);
    }
    pub fn strip_mouse(&self) {
        self.0.mouse_off.store(true, Ordering::SeqCst);
    }
}
/// `signed` is the permission set of the verified grant.
pub fn admit(
    roster: &Roster,
    lease: &Lease,
    signed: &BTreeSet<String>,
    guest: &str,
) -> Result<Admission, Denied> {
    if lease.guest != guest {
        return Err(Denied::LeaseInactive);
    }
    let policy = roster.policy(guest).ok_or(Denied::NotAFriend)?;
    if !lease.active {
        return Err(Denied::LeaseInactive);
    }
    if signed != &lease.approved {
        return Err(Denied::ExceedsApproval);
    }
    let allowed: BTreeSet<String> = signed
        .iter()
        .filter(|p| match p.as_str() {
            "keyboard" => policy.allows_keyboard(),
            "mouse" => policy.allows_mouse(),
            _ => true,
        })
        .cloned()
        .collect();
    if allowed.is_empty() {
        return Err(Denied::NoPermissions);
    }
    let gate = Gate::new(allowed);
    Ok(match policy.connect {
        Connect::Always => Admission::Allow(gate),
        Connect::Ask => Admission::AskOwner(gate),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }
    fn roster() -> Roster {
        Roster::parse(&json!([
            {"id":"bob","handle":"bob","input":"view","connect":"ask"},
            {"id":"cat","handle":"cat","input":"full","connect":"always"},
            {"id":"dan","handle":"dan","input":"full","connect":"ask"},
        ]))
        .unwrap()
    }
    fn lease(guest: &str, approved: &[&str], active: bool) -> Lease {
        Lease { guest: guest.into(), approved: set(approved), active }
    }
    #[test]
    fn closing_or_lowering_a_gate_reaches_every_clone() {
        let Admission::Allow(gate) = admit(&roster(), &lease("cat", &["view", "keyboard", "mouse"], true), &set(&["view", "keyboard", "mouse"]), "cat").unwrap() else { panic!() };
        let other = gate.clone();
        gate.strip_input();
        assert!(other.view() && !other.keyboard() && !other.mouse());
        gate.close();
        assert!(!other.view() && !other.is_open());
    }
    #[test]
    fn missing_fields_mean_view_only_and_ask() {
        let r = Roster::parse(&json!([{"id":"x"}])).unwrap();
        assert_eq!(r.policy("x"), Some(Policy { input: Input::View, connect: Connect::Ask, ..Default::default() }));
        assert_eq!(r.policy("y"), None);
    }
    #[test]
    fn rejects_unknown_values_instead_of_guessing() {
        assert!(Roster::parse(&json!([{"id":"x","input":"admin"}])).is_err());
        assert!(Roster::parse(&json!([{"id":"x","connect":"never"}])).is_err());
        assert!(Roster::parse(&json!({"id":"x"})).is_err());
    }
    #[test]
    fn view_only_friend_never_holds_keyboard_or_mouse() {
        let signed = set(&["view", "keyboard", "mouse", "audio"]);
        let got = admit(&roster(), &lease("bob", &["view", "keyboard", "mouse", "audio"], true), &signed, "bob").unwrap();
        assert!(matches!(got, Admission::AskOwner(_)));
        let g = got.gate();
        assert!(g.view() && g.has("audio"));
        assert!(!g.keyboard() && !g.mouse());
    }
    #[test]
    fn full_friend_keeps_input_and_always_skips_the_prompt() {
        let signed = set(&["view", "keyboard", "mouse"]);
        let got = admit(&roster(), &lease("cat", &["view", "keyboard", "mouse"], true), &signed, "cat").unwrap();
        assert!(matches!(&got, Admission::Allow(g) if g.keyboard() && g.mouse() && g.view()));
        let asked = admit(&roster(), &lease("dan", &["view", "keyboard", "mouse"], true), &signed, "dan").unwrap();
        assert!(matches!(asked, Admission::AskOwner(_)));
    }
    #[test]
    fn keyboard_and_mouse_can_be_allowed_apart() {
        let signed = set(&["view", "keyboard", "mouse"]);
        let lease = lease("eve", &["view", "keyboard", "mouse"], true);
        let keys_only = Roster::parse(&json!([{"id":"eve","input":"view","connect":"always","keyboard":true,"mouse":false}])).unwrap();
        let got = admit(&keys_only, &lease, &signed, "eve").unwrap();
        assert!(matches!(&got, Admission::Allow(g) if g.keyboard() && !g.mouse() && g.view()));
        let mouse_only = Roster::parse(&json!([{"id":"eve","input":"view","connect":"ask","keyboard":false,"mouse":true}])).unwrap();
        let got = admit(&mouse_only, &lease, &signed, "eve").unwrap();
        assert!(matches!(&got, Admission::AskOwner(g) if !g.keyboard() && g.mouse()));
        // Without the new fields the old single setting decides.
        let old_full = Roster::parse(&json!([{"id":"eve","input":"full","connect":"ask"}])).unwrap();
        assert!(matches!(admit(&old_full, &lease, &signed, "eve").unwrap(), Admission::AskOwner(g) if g.keyboard() && g.mouse()));
    }
    #[test]
    fn lowering_one_of_the_two_keeps_the_other() {
        let gate = Gate::new(set(&["view", "keyboard", "mouse"]));
        gate.strip_mouse();
        assert!(gate.keyboard() && !gate.mouse() && gate.view());
    }
    #[test]
    fn input_only_grant_for_a_view_only_friend_is_refused() {
        let signed = set(&["keyboard"]);
        assert_eq!(
            admit(&roster(), &lease("bob", &["keyboard"], true), &signed, "bob"),
            Err(Denied::NoPermissions)
        );
    }
    #[test]
    fn strangers_inactive_leases_and_mismatches_are_refused() {
        let signed = set(&["view"]);
        assert_eq!(admit(&roster(), &lease("eve", &["view"], true), &signed, "eve"), Err(Denied::NotAFriend));
        assert_eq!(admit(&roster(), &lease("bob", &["view"], false), &signed, "bob"), Err(Denied::LeaseInactive));
        assert_eq!(admit(&roster(), &lease("cat", &["view"], true), &signed, "bob"), Err(Denied::LeaseInactive));
        assert_eq!(
            admit(&roster(), &lease("bob", &["view"], true), &set(&["view", "audio"]), "bob"),
            Err(Denied::ExceedsApproval)
        );
    }
    #[test]
    fn a_lowered_friend_loses_input_at_the_next_admission_even_with_an_old_grant() {
        let signed = set(&["view", "keyboard"]);
        let before = Roster::parse(&json!([{"id":"bob","input":"full","connect":"ask"}])).unwrap();
        assert!(admit(&before, &lease("bob", &["view", "keyboard"], true), &signed, "bob").unwrap().gate().keyboard());
        let after = Roster::parse(&json!([{"id":"bob","input":"view","connect":"ask"}])).unwrap();
        assert!(!admit(&after, &lease("bob", &["view", "keyboard"], true), &signed, "bob").unwrap().gate().keyboard());
    }
}
