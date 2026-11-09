//! Whether the desktop session is locked, from systemd's LockedHint. A friend's video and input
//! stop when it says yes or cannot be read.
use std::process::Command;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

const UNKNOWN: u8 = 0;
const UNLOCKED: u8 = 1;
const LOCKED: u8 = 2;
static STATE: AtomicU8 = AtomicU8::new(UNKNOWN);
static START: Once = Once::new();

/// `yes`/`no` from `loginctl show-session -p LockedHint --value`. Anything else is unknown.
pub fn parse(output: &str) -> Option<bool> {
    match output.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn read() -> Option<bool> {
    let mut command = Command::new("loginctl");
    command.args(["show-session"]);
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            command.arg(id);
        }
    }
    let out = command.args(["-p", "LockedHint", "--value"]).output().ok()?;
    out.status.success().then(|| parse(&String::from_utf8_lossy(&out.stdout))).flatten()
}

fn store(state: Option<bool>) {
    STATE.store(
        match state {
            Some(true) => LOCKED,
            Some(false) => UNLOCKED,
            None => UNKNOWN,
        },
        Ordering::SeqCst,
    );
}

/// Locked, or the state cannot be read. The first call reads once before answering, then a
/// thread refreshes twice a second, so a lock reaches a running stream within about half a second.
pub fn locked_or_unknown() -> bool {
    START.call_once(|| {
        store(read());
        std::thread::spawn(|| loop {
            let started = Instant::now();
            store(read());
            std::thread::sleep(Duration::from_millis(500).saturating_sub(started.elapsed()));
        });
    });
    STATE.load(Ordering::SeqCst) != UNLOCKED
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_yes_and_no_are_understood() {
        assert_eq!(parse("yes\n"), Some(true));
        assert_eq!(parse("no\n"), Some(false));
        assert_eq!(parse(""), None);
        assert_eq!(parse("maybe"), None);
    }
}
