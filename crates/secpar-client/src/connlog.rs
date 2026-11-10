//! A small plain-text log of how long each step of a connection took, so a slow connection can be
//! read off a file instead of guessed at. One line per step:
//!
//! `2026-11-09T10:15:02.314Z 1a2b3c4d route_try lan4:.131 connected ms=41 at=96`
//!
//! `ms` is how long the step took. `at` is how long after the connection attempt began it ended.
//!
//! Nothing secret ever gets in. Every field goes through [`scrub`], which keeps a short alphabet,
//! cuts anything long or secret-looking, and shows an address as its kind plus the last octet.
//! The file is rotated: `connect.log` plus one older `connect.log.1`, together under 256 KiB.
//!
//! Nothing is written until an app calls [`init_default`] (or a test calls [`set_path`]).
use std::{
    fs,
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Size at which the active file is moved to `.1`. Two files stay under 256 KiB with room to spare.
pub const ROTATE_AT: u64 = 120 * 1024;
const LINE_MAX: usize = 200;
const DETAIL_MAX: usize = 90;

/// Where the apps keep small per-user files that are not secrets (the log, remembered routes).
pub fn state_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Secpar"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support/Secpar"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .map(|p| p.join("secpar"))
    }
}

/// One log file with rotation.
pub struct Log {
    path: PathBuf,
    limit: u64,
    lock: Mutex<()>,
}
impl Log {
    pub fn new(path: PathBuf, limit: u64) -> Self {
        Self { path, limit, lock: Mutex::new(()) }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Appends one finished line. Failures are ignored: a log must never break a connection.
    pub fn write(&self, line: &str) {
        let Ok(_guard) = self.lock.lock() else { return };
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let size = fs::metadata(&self.path).map_or(0, |m| m.len());
        if size + line.len() as u64 + 1 > self.limit && size > 0 {
            let old = self.path.with_extension("log.1");
            let _ = fs::remove_file(&old);
            let _ = fs::rename(&self.path, &old);
        }
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Lines are handed to a thread that owns the file, so a connection never waits for the disk
/// (opening and appending a file costs several milliseconds on Windows).
enum Msg {
    Line(String),
    Flush(std::sync::mpsc::SyncSender<()>),
}
struct Sink {
    tx: std::sync::mpsc::Sender<Msg>,
}
impl Sink {
    fn start(path: PathBuf) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();
        std::thread::spawn(move || {
            let log = Log::new(path, ROTATE_AT);
            // Ends when the log is switched off or replaced and the queue is empty.
            while let Ok(msg) = rx.recv() {
                match msg {
                    Msg::Line(line) => log.write(&line),
                    Msg::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Self { tx }
    }
}

static GLOBAL: OnceLock<Mutex<Option<Arc<Sink>>>> = OnceLock::new();
fn slot() -> &'static Mutex<Option<Arc<Sink>>> {
    GLOBAL.get_or_init(Default::default)
}
fn current() -> Option<Arc<Sink>> {
    slot().lock().ok().and_then(|g| g.clone())
}
/// Waits (at most a second) until every line handed over so far is on disk.
pub fn flush() {
    if let Some(sink) = current() {
        let (done, wait) = std::sync::mpsc::sync_channel(1);
        if sink.tx.send(Msg::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(std::time::Duration::from_secs(1));
        }
    }
}

/// Turns the log on at the standard place for this system.
pub fn init_default() {
    if std::env::var_os("SECPAR_NO_CONNLOG").is_some() {
        return;
    }
    if let Some(dir) = state_dir() {
        set_path(Some(dir.join("logs").join("connect.log")));
    }
}
/// Turns the log on at `path`, or off with `None`.
pub fn set_path(path: Option<PathBuf>) {
    if let Ok(mut g) = slot().lock() {
        *g = path.map(|p| Arc::new(Sink::start(p)));
    }
}
pub fn enabled() -> bool {
    current().is_some()
}

/// What shows of an address: its kind and the last part. `192.168.100.131` becomes `lan4:.131`.
pub fn addr(host: &str) -> String {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            let kind = if ip.is_link_local() {
                "ll4"
            } else if ip.is_private() {
                "lan4"
            } else if ip.is_loopback() {
                "lo4"
            } else {
                "ip4"
            };
            format!("{kind}:.{}", ip.octets()[3])
        }
        Ok(IpAddr::V6(ip)) => {
            let s = ip.segments();
            let kind = if (s[0] & 0xffc0) == 0xfe80 {
                "ll6"
            } else if ip.is_loopback() {
                "lo6"
            } else if (s[0] & 0xfe00) == 0xfc00 {
                "ula6"
            } else {
                "ip6"
            };
            format!("{kind}:{:x}", s[7])
        }
        Err(_) => "name".into(),
    }
}

fn run_of_secret_chars(token: &str) -> usize {
    let (mut best, mut now) = (0, 0);
    for c in token.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+' || c == '/' {
            now += 1;
            best = best.max(now);
        } else {
            now = 0;
        }
    }
    best
}
fn dotted_quad(token: &str) -> Option<&str> {
    let parts: Vec<&str> = token.split('.').collect();
    (parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))).then(|| parts[3])
}

/// Makes free text safe for the log. Whatever could be a secret is cut, whatever is left uses a
/// small alphabet, and an address shows only as its kind and last part.
pub fn scrub(text: &str) -> String {
    let mut out: Vec<String> = vec![];
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| ",;()\"'<>".contains(c));
        if token.is_empty() {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        let shown = if token.contains("://") || token.contains('?') || token.contains('@') || lower.starts_with("secpar") || lower.starts_with("bearer") {
            "<url>".to_string()
        } else if let Some(last) = dotted_quad(token.split(':').next().unwrap_or(token)) {
            format!("ip4:.{last}")
        } else if token.matches(':').count() >= 2 && token.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '[' || c == ']' || c == '%') {
            addr(token.split('%').next().unwrap_or(token))
        } else if token.contains('=') {
            // key=value: the value is cut unless it is short and plain
            let (k, v) = token.split_once('=').unwrap_or((token, ""));
            let plain = v.len() <= 8 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.');
            let key_ok = k.len() <= 12 && k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
            if plain && key_ok { format!("{k}={v}") } else { "<x>".to_string() }
        } else if run_of_secret_chars(token) >= 14 || token.len() > 24 {
            "<x>".to_string()
        } else {
            token.chars().map(|c| if c.is_ascii_alphanumeric() || "._:-/+".contains(c) { c } else { '_' }).collect()
        };
        out.push(shown);
    }
    let mut s = out.join(" ");
    if s.len() > DETAIL_MAX {
        s.truncate(DETAIL_MAX); // every branch above produced ASCII
    }
    s
}

/// A short, harmless name for a computer in the log: the first eight id characters.
pub fn short_label(id: &str) -> String {
    let s: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
    if s.is_empty() { "-".into() } else { s.to_ascii_lowercase() }
}
/// The steps the log knows. Anything else is written as `other`, so free text cannot get in here.
pub const PHASES: &[&str] = &[
    "account_fetch", "credential", "pair_request", "pair_wait", "auto_pair", "handoff", "schedule", "route_try", "tcp", "tls", "tls_ws", "ws_upgrade",
    "turn_wait", "hello", "pair", "welcome", "first_reply", "preview", "stream", "cache", "step",
];
fn phase_name(phase: &str) -> String {
    if PHASES.contains(&phase) { phase.to_string() } else { "other".into() }
}

/// `2026-11-09T10:15:02.314Z` from a time since the epoch.
pub fn timestamp(since_epoch: std::time::Duration) -> String {
    let secs = since_epoch.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, since_epoch.subsec_millis())
}

/// The finished text of one line. Every field is cleaned here.
pub fn format_line(now: std::time::Duration, label: &str, phase: &str, detail: &str, ms: u64, at: u64) -> String {
    let mut line = format!("{} {} {} {} ms={ms} at={at}", timestamp(now), short_label(label), phase_name(phase), scrub(detail));
    if line.len() > LINE_MAX {
        line.truncate(LINE_MAX);
    }
    line
}

fn write_line(sink: &Sink, label: &str, phase: &str, detail: &str, ms: u64, at: u64) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let _ = sink.tx.send(Msg::Line(format_line(now, label, phase, detail, ms, at)));
}

/// One connection attempt as the log sees it. Cheap to clone into the tasks of a race.
#[derive(Clone)]
pub struct Conn {
    label: String,
    begin: Instant,
}
impl Conn {
    pub fn new(computer_id: &str) -> Self {
        Self { label: short_label(computer_id), begin: Instant::now() }
    }
    /// Logs a step that began at `started`.
    pub fn step(&self, phase: &str, detail: &str, started: Instant) {
        self.note(phase, detail, started.elapsed().as_millis() as u64);
    }
    /// Logs a step of known length.
    pub fn note(&self, phase: &str, detail: &str, ms: u64) {
        if let Some(log) = current() {
            write_line(&log, &self.label, phase, detail, ms, self.begin.elapsed().as_millis() as u64);
        }
    }
    pub fn began(&self) -> Instant {
        self.begin
    }
}
/// A step outside any one connection, such as the account's own requests.
pub fn note(label: &str, phase: &str, detail: &str, ms: u64) {
    if let Some(log) = current() {
        write_line(&log, label, phase, detail, ms, ms);
    }
}

/// The log is process-wide, so tests that switch it on or off take turns.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("secpar-connlog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }
    /// Cheap deterministic noise, so the fuzz is repeatable.
    struct Noise(u64);
    impl Noise {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[(self.next() % items.len() as u64) as usize]
        }
        fn text(&mut self, alphabet: &str, len: usize) -> String {
            let a: Vec<char> = alphabet.chars().collect();
            (0..len).map(|_| a[(self.next() % a.len() as u64) as usize]).collect()
        }
    }

    #[test]
    fn addresses_show_kind_and_last_octet_only() {
        assert_eq!(addr("192.168.100.131"), "lan4:.131");
        assert_eq!(addr("169.254.7.9"), "ll4:.9");
        assert_eq!(addr("203.0.113.77"), "ip4:.77");
        assert_eq!(addr("fe80::1234:abcd"), "ll6:abcd");
        assert_eq!(addr("[2001:db8::5]"), "ip6:5");
        assert_eq!(addr("host.example.net"), "name");
    }

    #[test]
    fn a_line_has_the_agreed_shape() {
        let t = std::time::Duration::from_millis(1_794_218_102_314);
        let line = format_line(t, "1A2B3C4D-aaaa-bbbb", "route_try", "lan4:.131 connected", 41, 96);
        assert_eq!(line, "2026-11-09T09:55:02.314Z 1a2b3c4d route_try lan4:.131 connected ms=41 at=96");
    }

    #[test]
    fn secrets_never_reach_a_line_even_with_hostile_input() {
        let mut n = Noise(0x9E3779B97F4A7C15);
        let b64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        for _ in 0..2000 {
            let hex = n.text("0123456789abcdef", 64);
            let token = n.text(b64, 43);
            let otp = n.text("ABCDEFGHJKLMNPQRSTUVWXYZ23456789", 8);
            let secrets = [hex.clone(), token.clone(), format!("h1_{hex}"), format!("secpar://pair?n=x&t={token}&fp={hex}"), format!("wss://relay.example/v1/phone?sig={hex}&ts=1"), format!("Bearer {token}"), format!("token={token}"), format!("ticket:{token}"), format!("otp={otp}-{hex}")];
            let secret = n.pick(&secrets.iter().map(String::as_str).collect::<Vec<_>>()).to_string();
            let ip = format!("{}.{}.{}.{}", 1 + n.next() % 223, n.next() % 256, n.next() % 256, 1 + n.next() % 254);
            let noise = n.text("abc xyz,;:()<>\"'=/?&@#%^*", 12);
            let detail = format!("{noise} {secret} {ip}:48999 {noise}");
            let line = format_line(std::time::Duration::from_secs(1), &format!("{secret}{ip}"), &format!("phase {secret}"), &detail, 1, 2);
            for piece in [&hex, &token, &otp] {
                // any 12 characters of a secret in a row would be a leak
                for w in piece.as_bytes().windows(12) {
                    assert!(!line.contains(std::str::from_utf8(w).unwrap()), "leak in {line}");
                }
            }
            assert!(!line.contains("://") && !line.contains(&ip), "leak in {line}");
            assert!(line.len() <= LINE_MAX);
        }
    }

    #[test]
    fn plain_words_and_short_values_survive() {
        assert_eq!(scrub("lan4:.131 connected"), "lan4:.131 connected");
        assert_eq!(scrub("timeout after 2500 ms=2500"), "timeout after 2500 ms=2500");
        assert_eq!(scrub("failed 10.0.0.5:48999"), "failed ip4:.5");
        assert_eq!(scrub("tls pinned"), "tls pinned");
    }

    #[test]
    fn the_log_rotates_and_stays_small() {
        let dir = temp("rotate");
        let log = Log::new(dir.join("logs").join("connect.log"), 4096);
        for i in 0..2000 {
            log.write(&format_line(std::time::Duration::from_secs(i), "abcd1234", "route_try", "lan4:.131 connected", i, i));
        }
        let size = |p: &Path| fs::metadata(p).map_or(0, |m| m.len());
        let (a, b) = (size(log.path()), size(&log.path().with_extension("log.1")));
        assert!(a > 0 && b > 0, "both files exist");
        assert!(a <= 4096 && b <= 4096 + 200, "{a} {b}");
        let newest = fs::read_to_string(log.path()).unwrap();
        assert!(newest.lines().last().unwrap().contains("ms=1999"), "the newest line is kept");
        let default_limit = ROTATE_AT * 2 + 2 * LINE_MAX as u64;
        assert!(default_limit < 256 * 1024);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_is_written_until_a_path_is_set() {
        let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp("off");
        set_path(None);
        Conn::new("abcdef0123").note("hello", "x", 1);
        assert!(!dir.exists());
        set_path(Some(dir.join("connect.log")));
        let conn = Conn::new("ABCDEF0123456789");
        conn.step("tcp", "lan4:.131", Instant::now());
        flush();
        set_path(None);
        let text = fs::read_to_string(dir.join("connect.log")).unwrap();
        assert!(text.contains(" abcdef01 tcp lan4:.131 ") && text.contains("ms="));
        let _ = fs::remove_dir_all(dir);
    }
}
