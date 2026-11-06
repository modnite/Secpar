//! Independent manual QR and bounded owner-account bootstrap codes.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::rand::{SecureRandom, SystemRandom};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(120);
const MAX_OWNER: usize = 8;
#[derive(Default)]
pub struct PairingCodes {
    manual: Option<(String, Instant)>,
    owner: Vec<(String, String, String, Instant)>,
}
#[derive(Debug, PartialEq)]
pub enum Origin { Manual, Account(String) }
fn random_code() -> String {
    let mut bytes = [0; 16];
    SystemRandom::new().fill(&mut bytes).expect("OS random source");
    URL_SAFE_NO_PAD.encode(bytes)
}
impl PairingCodes {
    pub fn manual(&mut self, now: Instant) -> String {
        let code = random_code();
        self.manual = Some((code.clone(), now));
        code
    }
    pub fn owner(&mut self, request: &str, owner: &str, now: Instant) -> Result<String, &'static str> {
        if request.len() != 36 || !request.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return Err("Invalid owner pairing request");
        }
        if owner.is_empty() || owner.len() > 80 { return Err("Invalid pairing account"); }
        self.owner.retain(|(_, _, _, at)| now.saturating_duration_since(*at) < TTL);
        // A network retry must not invalidate a result already delivered.
        if let Some((_, account, code, _)) = self.owner.iter().find(|(id, _, _, _)| id == request) {
            if account != owner { return Err("Pairing request belongs to another account"); }
            return Ok(code.clone());
        }
        if self.owner.len() == MAX_OWNER { return Err("Too many pending pairing requests"); }
        let code = random_code();
        self.owner.push((request.into(), owner.into(), code.clone(), now));
        Ok(code)
    }
    pub fn take(&mut self, code: &str, now: Instant) -> bool {
        self.consume(code, now).is_some()
    }
    pub fn consume(&mut self, code: &str, now: Instant) -> Option<Origin> {
        if self.manual.as_ref().is_some_and(|(current, at)| current == code && now.saturating_duration_since(*at) < TTL) {
            self.manual = None;
            return Some(Origin::Manual);
        }
        self.owner.retain(|(_, _, _, at)| now.saturating_duration_since(*at) < TTL);
        self.owner.iter().position(|(_, _, current, _)| current == code).map(|i| Origin::Account(self.owner.swap_remove(i).1))
    }
    pub fn clear_owner(&mut self) { self.owner.clear(); }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn id(i: usize) -> String { format!("00000000-0000-4000-8000-{i:012}") }
    #[test]
    fn independent_single_use_codes_preserve_the_manual_qr() {
        let mut codes = PairingCodes::default(); let now = Instant::now();
        let manual = codes.manual(now);
        let first = codes.owner(&id(1), "alice", now).unwrap();
        let second = codes.owner(&id(2), "alice", now).unwrap();
        assert_ne!(first, second);
        assert_eq!(codes.owner(&id(1), "alice", now + Duration::from_secs(60)).unwrap(), first);
        assert!(codes.take(&second, now)); assert!(!codes.take(&second, now));
        assert!(codes.take(&manual, now)); assert!(codes.take(&first, now));
    }
    #[test]
    fn owner_capacity_expiry_and_account_unlink_are_bounded() {
        let mut codes = PairingCodes::default(); let now = Instant::now();
        let first = codes.owner(&id(0), "alice", now).unwrap();
        for i in 1..8 { codes.owner(&id(i), "alice", now).unwrap(); }
        assert!(codes.owner(&id(8), "alice", now).is_err());
        assert!(!codes.take(&first, now + TTL));
        let fresh = codes.owner(&id(8), "alice", now + TTL).unwrap();
        let manual = codes.manual(now + TTL);
        codes.clear_owner();
        assert!(!codes.take(&fresh, now + TTL)); assert!(codes.take(&manual, now + TTL));
    }
}
