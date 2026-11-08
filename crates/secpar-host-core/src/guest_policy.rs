//! What the host owner allows each friend. Both desktop hosts decide with this module.
//! The Worker already caps the signed grant. The host checks again, because the grant is
//! only as fresh as the claim and the owner can change a friend at any time.
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};

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
}
#[derive(Deserialize)]
struct Row {
    id: String,
    #[serde(flatten)]
    policy: Policy,
}
/// Accepted, unblocked friends of the host owner and what each may do.
#[derive(Clone, Debug, Default)]
pub struct Roster(HashMap<String, Policy>);
impl Roster {
    pub fn parse(json: &serde_json::Value) -> Result<Self, &'static str> {
        let rows: Vec<Row> = serde_json::from_value(json.clone()).map_err(|_| "Invalid friends response")?;
        if rows.len() > 500 {
            return Err("Invalid friends response");
        }
        Ok(Self(rows.into_iter().map(|r| (r.id, r.policy)).collect()))
    }
    pub fn policy(&self, guest: &str) -> Option<Policy> {
        self.0.get(guest).copied()
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gate(BTreeSet<String>);
impl Gate {
    pub fn has(&self, permission: &str) -> bool {
        self.0.contains(permission)
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
    pub fn permissions(&self) -> &BTreeSet<String> {
        &self.0
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
        .filter(|p| policy.input == Input::Full || !matches!(p.as_str(), "keyboard" | "mouse"))
        .cloned()
        .collect();
    if allowed.is_empty() {
        return Err(Denied::NoPermissions);
    }
    let gate = Gate(allowed);
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
    fn missing_fields_mean_view_only_and_ask() {
        let r = Roster::parse(&json!([{"id":"x"}])).unwrap();
        assert_eq!(r.policy("x"), Some(Policy { input: Input::View, connect: Connect::Ask }));
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
