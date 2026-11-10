//! The Files page: browse a computer's files, download from it, upload to it. One background
//! connection per open computer, like the previews. All protocol work is in
//! `secpar_client::files`. This module keeps the page state and draws it.
use crate::account::OwnedConnection;
use crate::{design, dialogs, icons::{self, Icon}};
use eframe::egui::{self, Color32, RichText};
use secpar_client::control::{Computer, Connected};
use secpar_client::files::{self as proto, Entry, Notice, Phase, Request, Transfer};
use secpar_client::pairing::Pairing;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use tokio::sync::mpsc;

/// The account worker gives up on a pairing after 120 seconds. This is the page's own limit, so
/// no failure can leave it connecting for good.
const PENDING_LIMIT: Duration = Duration::from_secs(135);

const ROW: f32 = 36.;
/// Transfer rows shown at once. The rest are counted.
const VISIBLE: usize = 3;
/// Finished and failed rows kept, newest last.
const KEEP: usize = 400;
const SMALL: f32 = 12.;
/// Height of the buttons inside list and transfer rows.
const SMALL_BUTTON: f32 = 24.;
/// Height of the first line of a transfer row.


/// A computer that can be opened.
#[derive(Clone)]
pub struct Pick {
    pub id: String,
    pub name: String,
    /// Online and linked, so a connection can work.
    pub ready: bool,
    /// The saved credential of a computer paired by hand. Account computers have none here.
    pub saved: Option<Computer>,
}

pub enum Action {
    /// Open this account computer. The account worker hands over its credential.
    Owned(String),
}

/// What a failed connection means for the person, in plain words. The second value is true when the
/// computer refused the saved credential, so asking the account for a new one can fix it.
pub fn explain(error: &str, name: &str) -> (String, bool) {
    let name = if name.is_empty() { "the computer" } else { name };
    let lower = error.to_lowercase();
    if lower.contains("refused or revoked") {
        return (format!("{name} no longer accepts this device."), true);
    }
    let unreachable = ["offline", "timed out", "secure connection", "no connection address", "did not answer", "did not respond", "connection closed", "connection to the computer was lost"];
    if unreachable.iter().any(|k| lower.contains(k)) {
        return (format!("Can't reach {name}. Check that it is on, online and running Secpar."), false);
    }
    // The computer shows another certificate than the one it was paired with: it was set up again.
    if lower.contains("does not match the pairing code") {
        return (format!("{name} was set up again and no longer accepts this device."), true);
    }
    if lower.contains("pairing was refused") {
        return (format!("{name} did not accept the pairing. Open Secpar there and try again."), false);
    }
    if lower.contains("identity") {
        return (format!("{name} does not match what your account has on record. Refresh the list and try again."), false);
    }
    (error.to_string(), false)
}

/// The account computer a connection from the account worker is for, and how to reach it.
pub fn owned_source(connection: OwnedConnection) -> (String, String, Source) {
    match connection {
        OwnedConnection::Cached { owner, id, computer } => (owner, id, Source::Saved(computer)),
        OwnedConnection::Pair { owner, id, pairing } => (owner, id, Source::Pair(pairing)),
    }
}

/// How to reach the computer.
pub enum Source {
    Saved(Computer),
    Pair(Pairing),
    /// A friend's computer, with a grant for the files permission. It works once.
    Guest(crate::account::Grant),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Conn {
    Idle,
    Connecting,
    Online,
    Closed(String),
}

/// Everything the page shows, apart from the connection itself. No I/O, so it is tested directly.
#[derive(Clone)]
pub struct View {
    pub conn: Conn,
    pub browse: bool,
    pub upload: bool,
    pub path: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub problem: Option<String>,
    back: Vec<String>,
    pub transfers: Vec<Transfer>,
}
impl Default for View {
    fn default() -> Self {
        Self { conn: Conn::Idle, browse: true, upload: true, path: String::new(), entries: vec![], loading: false, problem: None, back: vec![], transfers: vec![] }
    }
}
impl View {
    pub fn apply(&mut self, notice: Notice) -> Option<Request> {
        match notice {
            Notice::Ready { browse, upload } => {
                self.conn = Conn::Online;
                self.browse = browse;
                self.upload = upload;
                if browse {
                    self.loading = true;
                    return Some(Request::List(String::new()));
                }
                None
            }
            Notice::Listed { path, entries } => {
                if path == self.path {
                    self.entries = entries;
                    self.loading = false;
                    self.problem = None;
                }
                None
            }
            Notice::Problem(text) => {
                // The host turned file transfer off, so an upload would be refused too.
                if text == proto::OFF {
                    self.upload = false;
                }
                self.loading = false;
                self.problem = Some(text);
                None
            }
            Notice::Transfer(t) => {
                match self.transfers.iter_mut().find(|x| x.id == t.id) {
                    Some(old) => *old = t,
                    None => {
                        self.transfers.push(t);
                        if self.transfers.len() > KEEP {
                            if let Some(i) = self.transfers.iter().position(Transfer::finished) {
                                self.transfers.remove(i);
                            }
                        }
                    }
                }
                None
            }
            Notice::Closed(why) => {
                self.conn = Conn::Closed(if why == "Disconnected" { String::new() } else { why });
                self.loading = false;
                None
            }
        }
    }
    /// Opens a folder, remembering where we were.
    pub fn go(&mut self, path: &str) -> Request {
        if path != self.path {
            self.back.push(std::mem::replace(&mut self.path, path.to_string()));
        }
        self.loading = true;
        self.problem = None;
        Request::List(path.to_string())
    }
    pub fn go_back(&mut self) -> Option<Request> {
        let path = self.back.pop()?;
        self.path = path.clone();
        self.loading = true;
        self.problem = None;
        Some(Request::List(path))
    }
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn refresh(&mut self) -> Request {
        self.loading = true;
        self.problem = None;
        Request::List(self.path.clone())
    }
    /// Rows to draw: unfinished first, then the newest finished ones. Returns them and how many more exist.
    pub fn visible(&self, max: usize) -> (Vec<&Transfer>, usize) {
        let mut rows: Vec<&Transfer> = self.transfers.iter().filter(|t| !t.finished()).collect();
        rows.extend(self.transfers.iter().rev().filter(|t| t.finished()));
        let max = max.min(VISIBLE);
        let more = rows.len().saturating_sub(max);
        rows.truncate(max);
        (rows, more)
    }
    /// One line for when there is no room for rows: what is running and what failed.
    pub fn summary(&self) -> String {
        let running = self.transfers.iter().filter(|t| !t.finished()).count();
        let failed = self.transfers.iter().filter(|t| t.phase == Phase::Failed && t.error != "Cancelled").count();
        match (running, failed) {
            (0, 0) => format!("{} finished", self.transfers.len()),
            (0, f) => format!("{f} failed"),
            (r, 0) => format!("{r} running"),
            (r, f) => format!("{r} running, {f} failed"),
        }
    }
    pub fn clear_finished(&mut self) {
        self.transfers.retain(|t| !t.finished());
    }
}

/// `(label, path)` for each step of a path, starting with the computer's root list.
pub fn crumbs(path: &str) -> Vec<(String, String)> {
    let mut out = vec![("Computer".to_string(), String::new())];
    if path.is_empty() {
        return out;
    }
    if let Some(rest) = path.strip_prefix("\\\\") {
        let mut acc = String::from("\\\\");
        for (i, part) in rest.split('\\').filter(|p| !p.is_empty()).enumerate() {
            if i > 0 {
                acc.push('\\');
            }
            acc.push_str(part);
            out.push((part.to_string(), acc.clone()));
        }
        return out;
    }
    if path.starts_with('/') {
        out.push(("/".into(), "/".into()));
        let mut acc = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            acc.push('/');
            acc.push_str(part);
            out.push((part.to_string(), acc.clone()));
        }
        return out;
    }
    let mut acc = String::new();
    for (i, part) in path.split('\\').filter(|p| !p.is_empty()).enumerate() {
        if i > 0 {
            acc.push('\\');
        }
        acc.push_str(part);
        // A drive is "C:\", not "C:".
        let full = if i == 0 && part.ends_with(':') { format!("{acc}\\") } else { acc.clone() };
        out.push((part.to_string(), full));
    }
    out
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024. && unit < UNITS.len() - 1 {
        v /= 1024.;
        unit += 1;
    }
    if v >= 100. {
        format!("{v:.0} {}", UNITS[unit])
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// `2026-11-09` for Unix seconds. Empty for 0, which hosts use for "unknown".
pub fn date(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Where downloads go unless the user picked another folder.
pub fn default_save_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join("Downloads").join("Secpar")
}

enum Event {
    Paired(Computer),
    Failed(String),
}
enum Picked {
    Files(Result<Vec<PathBuf>, String>),
    Folder(Result<Option<PathBuf>, String>),
}
struct Session {
    id: String,
    tx: mpsc::UnboundedSender<Request>,
    notices: mpsc::UnboundedReceiver<Notice>,
    events: mpsc::UnboundedReceiver<Event>,
    /// Account that owns the credential, when this is an account computer.
    owner: Option<String>,
}

pub struct Panel {
    ctx: egui::Context,
    save_dir: PathBuf,
    save_changed: Option<String>,
    chosen: Option<String>,
    /// An account computer waiting for the account worker to hand over its credential.
    pending: Option<String>,
    pending_since: Option<Instant>,
    /// When the person asked to open a computer, until its first folder listing is on screen.
    asked_at: Option<Instant>,
    /// A refused saved credential to drop before asking the account again.
    stale: Option<(String, String)>,
    /// The account computer that already got one automatic new credential.
    renewed: Option<String>,
    name: String,
    session: Option<Session>,
    pub view: View,
    dialog: Option<std_mpsc::Receiver<Picked>>,
    paired: Option<(String, String, Computer)>,
    message: String,
    /// The connection was opened by a stream so files the computer sends meanwhile are received.
    auto: bool,
    #[cfg(feature = "ui-review")]
    inert: bool,
    /// Screenshot review against a real host: steps to run, and when to wait until.
    #[cfg(feature = "ui-review")]
    live: bool,
    #[cfg(feature = "ui-review")]
    script: std::collections::VecDeque<String>,
    #[cfg(feature = "ui-review")]
    script_wait: Option<std::time::Instant>,
}

impl Panel {
    pub fn new(ctx: egui::Context, save_dir: Option<&str>) -> Self {
        let save_dir = save_dir.filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(default_save_dir);
        Self {
            ctx,
            save_dir,
            save_changed: None,
            chosen: None,
            pending: None,
            pending_since: None,
            asked_at: None,
            stale: None,
            renewed: None,
            name: String::new(),
            session: None,
            view: View::default(),
            dialog: None,
            paired: None,
            message: String::new(),
            auto: false,
            #[cfg(feature = "ui-review")]
            inert: false,
            #[cfg(feature = "ui-review")]
            live: false,
            #[cfg(feature = "ui-review")]
            script: Default::default(),
            #[cfg(feature = "ui-review")]
            script_wait: None,
        }
    }
    /// Screenshot review against a real host. Pairs with the one-time link in `link_file`, then runs
    /// `script`: `cd:PATH`, `get:NAME`, `put:LOCAL`, `wait:MS`, `idle`, separated by `;`.
    #[cfg(feature = "ui-review")]
    pub fn live(ctx: egui::Context, link_file: &str, script: &str, save_dir: &str) -> Self {
        let mut panel = Self::new(ctx, Some(save_dir));
        let link = std::fs::read_to_string(link_file).expect("pairing link file");
        let pairing = Pairing::parse(link.trim()).expect("pairing link");
        panel.open("live", "Review host", None, Source::Pair(pairing));
        panel.live = true;
        panel.script = script.split(';').filter(|s| !s.is_empty()).map(String::from).collect();
        panel
    }
    /// True once a live review has run its whole script.
    #[cfg(feature = "ui-review")]
    pub fn review_ready(&self) -> bool {
        !self.live || self.script.is_empty()
    }
    #[cfg(feature = "ui-review")]
    fn run_script(&mut self) {
        if !self.live || self.view.conn != Conn::Online || self.view.loading {
            return;
        }
        self.ctx.request_repaint_after(std::time::Duration::from_millis(100));
        if let Some(until) = self.script_wait {
            if std::time::Instant::now() < until {
                return;
            }
            self.script_wait = None;
        }
        let Some(step) = self.script.front().cloned() else { return };
        let (cmd, arg) = step.split_once(':').unwrap_or((step.as_str(), ""));
        match cmd {
            "cd" => {
                let request = self.view.go(arg);
                self.send(request);
            }
            "get" => match self.view.entries.iter().find(|e| e.name == arg).cloned() {
                Some(e) => self.send(Request::Get(e.path)),
                None => eprintln!("review script: {arg} is not in the listing"),
            },
            "put" => self.upload(vec![PathBuf::from(arg)]),
            "wait" => self.script_wait = Some(std::time::Instant::now() + std::time::Duration::from_millis(arg.parse().unwrap_or(0))),
            "idle" => {
                if self.view.transfers.is_empty() || self.view.transfers.iter().any(|t| !t.finished()) {
                    return;
                }
            }
            _ => {}
        }
        self.script.pop_front();
    }
    /// A page with made-up contents for the screenshot review. No connection exists.
    #[cfg(feature = "ui-review")]
    pub fn sample(ctx: egui::Context, page: &str) -> Self {
        let mut panel = Self::new(ctx, Some("C:\\Users\\Player\\Downloads\\Secpar"));
        panel.inert = true;
        panel.name = "Linux laptop".into();
        panel.chosen = Some("2".into());
        let file = |name: &str, size: u64, mtime: u64| Entry { name: name.into(), path: format!("/home/player/Documents/{name}"), dir: false, size, mtime };
        let dir = |name: &str, mtime: u64| Entry { name: name.into(), path: format!("/home/player/Documents/{name}"), dir: true, size: 0, mtime };
        let v = &mut panel.view;
        match page {
            "files-empty" => {}
            "files-off" => {
                v.conn = Conn::Online;
                v.problem = Some(proto::OFF.into());
                v.upload = false;
            }
            "files-closed" => v.conn = Conn::Closed(explain("Could not establish a secure connection", "Linux laptop").0),
            _ => {
                v.conn = Conn::Online;
                v.path = "/home/player/Documents".into();
                v.back = vec![String::new(), "/home/player".into()];
                v.entries = vec![
                    dir("Invoices 2026", 1_793_000_000),
                    dir("Photos from the trip to the coast with a very long folder name", 1_790_000_000),
                    file("budget.xlsx", 48_213, 1_794_000_000),
                    file("holiday-video-final-cut.mp4", 734_003_200, 1_792_000_000),
                    file("notes.txt", 812, 1_794_500_000),
                    file("scan 0042.pdf", 2_411_724, 1_790_500_000),
                ];
                let t = |id: &str, name: &str, up: bool, size: u64, done: u64, phase: Phase, path: &str, error: &str| Transfer { id: id.into(), name: name.into(), upload: up, size, done, phase, path: path.into(), error: error.into() };
                v.transfers = vec![
                    t("a", "holiday-video-final-cut.mp4", false, 734_003_200, 441_450_496, Phase::Active, "", ""),
                    t("b", "notes.txt", true, 812, 812, Phase::Done, "/home/player/Secpar/notes.txt", ""),
                    t("c", "scan 0042.pdf", true, 2_411_724, 0, Phase::Failed, "", "The computer refused the file. File transfer may be turned off there."),
                    t("d", "budget.xlsx", false, 48_213, 0, Phase::Queued, "", ""),
                ];
                if page == "files-downloads" {
                    v.path = "/home/player/Documents/Photos from the coast".into();
                    v.transfers = (0..8).map(|i| t(&format!("folder-{i}"), &format!("Photos from the coast/photo-{i:03}.jpg"), false, 8_388_608, if i == 0 { 2_097_152 } else { 0 }, if i == 0 { Phase::Active } else { Phase::Queued }, "", "")).collect();
                    v.transfers.push(t("upload", "notes.txt", true, 812, 400, Phase::Active, "", ""));
                }

            }
        }
        panel
    }

    /// A new save folder to remember, once.
    pub fn take_save_dir(&mut self) -> Option<String> {
        self.save_changed.take()
    }
    /// The credential made while pairing an account computer, to be remembered by the account worker.
    pub fn take_paired(&mut self) -> Option<(String, String, Computer)> {
        self.paired.take()
    }
    /// The saved credential of an account computer that refused it: (account, computer).
    pub fn take_stale(&mut self) -> Option<(String, String)> {
        self.stale.take()
    }
    /// A connection from the account worker. Opens it if this page asked for that computer and
    /// `signed_in` is the account it belongs to.
    pub fn receive(&mut self, connection: OwnedConnection, signed_in: Option<&str>, name_of: impl Fn(&str) -> String) -> bool {
        let (owner, id, source) = owned_source(connection);
        if !self.wants(&id) || signed_in != Some(owner.as_str()) {
            return false;
        }
        let name = name_of(&id);
        self.open(&id, &name, Some(owner), source);
        true
    }
    /// True while this page waits for the account worker's answer about `id`.
    pub fn wants(&self, id: &str) -> bool {
        self.pending.as_deref() == Some(id)
    }

    /// Opens a computer. `owner` is the account that holds its credential, if any.
    pub fn open(&mut self, id: &str, name: &str, owner: Option<String>, source: Source) {
        self.close();
        self.auto = false;
        self.name = name.to_string();
        self.chosen = Some(id.to_string());
        self.pending = None;
        self.pending_since = None;
        self.message.clear();
        self.view.conn = Conn::Connecting;
        #[cfg(feature = "ui-review")]
        if self.inert {
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let (note_tx, notices) = mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::unbounded_channel();
        let (save, ctx) = (self.save_dir.clone(), self.ctx.clone());
        // A preview may already hold an authenticated connection to this computer.
        let borrow_key = owner.as_deref().and_then(|owner| crate::account::owned_key(owner, id).ok());
        std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                let _ = event_tx.send(Event::Failed("Could not start the file connection".into()));
                ctx.request_repaint();
                return;
            };
            runtime.block_on(async move {
                let device = crate::device_name();
                let paired = matches!(source, Source::Pair(_));
                let connected = match source {
                    Source::Saved(computer) => {
                        let asked = Instant::now();
                        let borrowed = match &borrow_key {
                            Some(key) => crate::previews::handoffs().take(key, crate::previews::HANDOFF_WAIT).await.filter(|c| c.computer.token == computer.token && c.computer.fingerprint == computer.fingerprint),
                            None => None,
                        };
                        match borrowed {
                            Some(connected) => {
                                secpar_client::connlog::note(&secpar_client::connlog::short_label(&connected.computer.id), "handoff", "preview", asked.elapsed().as_millis() as u64);
                                Ok(connected)
                            }
                            None => Connected::reconnect(computer, &device).await,
                        }
                    }
                    Source::Pair(pairing) => Connected::pair_lan_first(pairing, &device).await,
                    Source::Guest(grant) => Connected::guest(&grant.relay, &grant.pc, &grant.grant, &grant.name).await,
                };
                match connected {
                    Ok(connected) => {
                        if paired {
                            let _ = event_tx.send(Event::Paired(connected.computer.clone()));
                        }
                        let wake = ctx.clone();
                        proto::run(connected, save, rx, note_tx, Arc::new(move || wake.request_repaint())).await;
                    }
                    Err(error) => {
                        let _ = event_tx.send(Event::Failed(error));
                    }
                }
                ctx.request_repaint();
            });
        });
        self.session = Some(Session { id: id.to_string(), tx, notices, events, owner });
    }
    /// While a stream to a computer runs, keep a files connection to it too, so files that computer
    /// offers (Send to Secpar) arrive in the save folder. Does nothing if the page is already in use.
    pub fn background(&mut self, id: &str, name: &str, owner: Option<String>, source: Source) {
        if self.session.is_none() && self.pending.is_none() {
            self.open(id, name, owner, source);
            self.auto = true;
        }
    }
    /// The stream ended: close the connection `background` opened. Finished rows stay listed.
    pub fn end_background(&mut self) {
        if std::mem::take(&mut self.auto) {
            self.close();
        }
    }
    /// Waits for the account worker to hand over a credential for `id`.
    pub fn wait_for(&mut self, id: &str, name: &str) {
        self.close();
        self.name = name.to_string();
        self.chosen = Some(id.to_string());
        self.pending = Some(id.to_string());
        self.pending_since = Some(Instant::now());
        self.message.clear();
        self.view.conn = Conn::Connecting;
    }
    /// Ends the connection. Anything still moving is cancelled and its partial file removed.
    pub fn close(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = session.tx.send(Request::Stop);
        }
        self.pending = None;
        self.pending_since = None;
        // What already finished stays listed until it is cleared.
        let mut finished = std::mem::take(&mut self.view.transfers);
        finished.retain(Transfer::finished);
        self.view = View { transfers: finished, ..View::default() };
    }
    /// The account signed out: drop connections that used its credentials.
    pub fn account_gone(&mut self) {
        if self.session.as_ref().is_some_and(|s| s.owner.is_some()) || self.pending.is_some() {
            self.close();
        }
    }
    /// Reports a failure to get a credential from the account worker.
    pub fn fail(&mut self, why: &str) {
        if self.pending.take().is_some() {
            self.pending_since = None;
            self.asked_at = None;
            self.view.conn = Conn::Closed(why.to_string());
        }
    }

    fn send(&mut self, request: Request) {
        if let Some(session) = &self.session {
            let _ = session.tx.send(request);
        }
    }

    pub fn poll(&mut self) {
        let mut follow = vec![];
        let mut ended = false;
        let mut renew = None;
        if let Some(session) = self.session.as_mut() {
            while let Ok(event) = session.events.try_recv() {
                match event {
                    Event::Paired(computer) => {
                        if let Some(owner) = session.owner.clone() {
                            self.paired = Some((owner, session.id.clone(), computer));
                        }
                    }
                    Event::Failed(why) => {
                        let (text, refused) = explain(&why, &self.name);
                        match session.owner.clone() {
                            // The saved credential is out of date, for example after the computer
                            // was set up again. Get a new one through the account, once.
                            Some(owner) if refused && self.renewed.as_deref() != Some(session.id.as_str()) => {
                                self.renewed = Some(session.id.clone());
                                renew = Some((owner, session.id.clone()));
                            }
                            _ => self.view.conn = Conn::Closed(text),
                        }
                        ended = true;
                    }
                }
            }
            while let Ok(notice) = session.notices.try_recv() {
                let lost = matches!(&notice, Notice::Closed(why) if why != "Disconnected");
                if let (Notice::Listed { .. }, Some(asked)) = (&notice, self.asked_at.take()) {
                    // Everything between the click and the first folder on screen, in one line.
                    let label = self.chosen.as_deref().unwrap_or("");
                    secpar_client::connlog::note(&secpar_client::connlog::short_label(label), "first_reply", "click_to_listing", asked.elapsed().as_millis() as u64);
                }
                let applied = self.view.apply(notice);
                if let (true, Conn::Closed(why)) = (lost, &mut self.view.conn) {
                    *why = explain(why, &self.name).0;
                }
                if let Some(request) = applied {
                    follow.push(request);
                }
            }
            if matches!(self.view.conn, Conn::Closed(_)) {
                ended = true;
            }
        }
        for request in follow {
            self.send(request);
        }
        #[cfg(feature = "ui-review")]
        self.run_script();
        if ended {
            self.session = None;
            self.asked_at = None;
        }
        if let Some((owner, id)) = renew {
            let name = self.name.clone();
            self.wait_for(&id, &name);
            self.stale = Some((owner, id));
        }
        if self.pending_since.is_some_and(|since| since.elapsed() > PENDING_LIMIT) {
            self.fail("Secpar did not get an answer from the computer through your account. Try again.");
        }
        if let Some(rx) = &self.dialog {
            if let Ok(picked) = rx.try_recv() {
                self.dialog = None;
                match picked {
                    Picked::Files(Ok(paths)) => self.upload(paths),
                    Picked::Folder(Ok(Some(dir))) => {
                        self.save_changed = Some(dir.to_string_lossy().into_owned());
                        self.save_dir = dir.clone();
                        self.send(Request::SaveTo(dir));
                    }
                    Picked::Folder(Ok(None)) => {}
                    Picked::Files(Err(e)) | Picked::Folder(Err(e)) => self.message = e,
                }
            }
        }
    }

    fn upload(&mut self, paths: Vec<PathBuf>) {
        if !self.view.upload {
            self.message = if self.view.problem.as_deref() == Some(proto::OFF) { proto::OFF } else { "Update Secpar on this computer to send files to it." }.into();
            return;
        }
        for (path, name) in proto::expand(&paths) {
            self.send(Request::SendAs(path, name));
        }
    }

    fn pick(&mut self, folder: bool) {
        if self.dialog.is_some() {
            return;
        }
        self.message.clear();
        let (tx, rx) = std_mpsc::channel();
        let (ctx, start) = (self.ctx.clone(), self.save_dir.clone());
        std::thread::spawn(move || {
            let _ = tx.send(if folder { Picked::Folder(dialogs::pick_folder(&start)) } else { Picked::Files(dialogs::pick_files()) });
            ctx.request_repaint();
        });
        self.dialog = Some(rx);
    }

    /// Draws the page. `note` is what the account worker last said, shown while it pairs.
    pub fn ui(&mut self, ui: &mut egui::Ui, picks: &[Pick], note: &str) -> Option<Action> {
        self.poll();
        // A failed account request ends the wait.
        if self.pending.is_some() && !note.is_empty() && !note.starts_with("Pairing with") {
            let why = explain(note, &self.name).0;
            self.fail(&why);
        }
        let mut action = None;
        design::page_header(ui, "Files", |ui| {
            if self.view.conn != Conn::Idle && !matches!(self.view.conn, Conn::Closed(_)) && ui.add(egui::Button::new(if self.view.conn == Conn::Online { "Disconnect" } else { "Cancel" })).clicked() {
                self.close();
            }
            if self.view.conn == Conn::Online {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    design::dot(ui, design::OK);
                    ui.add(egui::Label::new(&self.name).truncate()).on_hover_text(format!("Connected to {}", self.name));
                });
            }
        });
        ui.add_space(6.);
        match self.view.conn.clone() {
            Conn::Idle | Conn::Closed(_) => {
                if let Conn::Closed(why) = &self.view.conn {
                    if !why.is_empty() {
                        ui.label(RichText::new(why).color(design::BAD));
                    }
                }
                action = self.picker(ui, picks);
            }
            Conn::Connecting => {
                self.connection_row(ui, "Connecting", design::WARN);
                if !note.is_empty() && self.pending.is_some() {
                    ui.label(RichText::new(note).color(design::MUTED));
                }
            }
            Conn::Online => self.online(ui),
        }
        action
    }

    fn picker(&mut self, ui: &mut egui::Ui, picks: &[Pick]) -> Option<Action> {
        if picks.is_empty() {
            ui.label(RichText::new("No computers to browse").color(design::MUTED));
            return None;
        }
        let current = self.chosen.as_ref().and_then(|id| picks.iter().find(|p| &p.id == id)).or_else(|| picks.iter().find(|p| p.ready)).or(picks.first()).cloned();
        let mut chosen = current.clone();
        let mut connect = false;
        ui.horizontal(|ui| {
            let gap = ui.spacing().item_spacing.x;
            let width = (ui.available_width() - 144. - gap).max(120.);
            egui::ComboBox::from_id_salt("files-computer")
                .width(width)
                .selected_text(chosen.as_ref().map_or(String::new(), |p| p.name.clone()))
                .show_ui(ui, |ui| {
                    for pick in picks {
                        let label = if pick.ready { pick.name.clone() } else { format!("{} (offline)", pick.name) };
                        if ui.selectable_label(chosen.as_ref().is_some_and(|c| c.id == pick.id), label).clicked() {
                            chosen = Some(pick.clone());
                        }
                    }
                });
            let failed = matches!(&self.view.conn, Conn::Closed(why) if !why.is_empty());
            connect = ui.add_enabled(chosen.as_ref().is_some_and(|p| p.ready), design::primary(if failed { "Retry" } else { "Open" })).clicked();
        });
        self.chosen = chosen.as_ref().map(|p| p.id.clone());
        let pick = chosen?;
        if !connect {
            return None;
        }
        self.renewed = None;
        match pick.saved {
            Some(computer) => {
                self.asked_at = Some(Instant::now());
                self.open(&pick.id, &pick.name, None, Source::Saved(computer));
                None
            }
            None => {
                self.wait_for(&pick.id, &pick.name);
                self.asked_at = Some(Instant::now());
                Some(Action::Owned(pick.id))
            }
        }
    }

    fn connection_row(&mut self, ui: &mut egui::Ui, status: &str, color: Color32) {
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), design::CONTROL), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            design::dot(ui, color);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(status).size(SMALL).color(design::MUTED));
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&self.name).strong()).truncate());
                });
            });
        });
    }

    fn online(&mut self, ui: &mut egui::Ui) {
        let mut requests = vec![];
        // Room for transfer rows after the path bar, the bottom bar and a list worth looking at.
        let budget = ui.available_height() - 36. - 110. - 64.;
        let rows = (budget / 76.).floor().clamp(0., VISIBLE as f32) as usize;
        // Reserve the complete control area even in short windows. The panel's default
        // half-height limit otherwise clips cancellation while the file list takes its minimum.
        let bottom_height = if self.view.transfers.is_empty() { 78. } else if rows == 0 { 102. } else { 98. + rows as f32 * 76. };
        egui::TopBottomPanel::bottom("files-bottom").exact_height(bottom_height).frame(egui::Frame::new()).show_separator_line(false).show_inside(ui, |ui| {
            ui.add_space(6.);
            self.transfers(ui, &mut requests, rows);
            self.bottom_bar(ui);
        });
        self.navigation(ui, &mut requests);
        self.listing(ui, &mut requests);
        // Files dropped on the window go up.
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            self.upload(dropped);
        }
        for request in requests {
            self.send(request);
        }
    }

    fn navigation(&mut self, ui: &mut egui::Ui, requests: &mut Vec<Request>) {
        if !self.view.browse {
            return;
        }
        let mut go = None;
        ui.horizontal(|ui| {
            if icons::button(ui, Icon::Back, "Back", self.view.can_go_back()) {
                requests.extend(self.view.go_back());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Refresh, "Refresh folder", !self.view.loading) {
                    requests.push(self.view.refresh());
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let steps = crumbs(&self.view.path);
                    let label = steps.last().map(|s| s.0.as_str()).unwrap_or("Computer");
                    let response = ui.add_sized([ui.available_width(), 32.], egui::Button::new(label).truncate());
                    response.clone().on_hover_text(if self.view.path.is_empty() { "Computer" } else { &self.view.path });
                    egui::Popup::menu(&response).show(|ui| {
                        for (label, path) in &steps {
                            if ui.selectable_label(path == &self.view.path, label).clicked() {
                                go = Some(path.clone());
                                ui.close();
                            }
                        }
                    });
                });
            });
        });
        if let Some(path) = go { requests.push(self.view.go(&path)); }
        ui.add_space(2.);
    }

    fn listing(&mut self, ui: &mut egui::Ui, requests: &mut Vec<Request>) {
        let width = ui.available_width();
        // The frame takes exactly what is left, so rows can never run past its border.
        let inner = (ui.available_height() - 2.).max(0.);
        design::surface().inner_margin(0).show(ui, |ui| {
            ui.set_width(width - 2.);
            ui.set_min_height(inner);
            ui.set_max_height(inner);
            if !self.view.browse {
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    ui.add_space(12.);
                    ui.label(RichText::new("Update Secpar on this computer to browse its files.").color(design::MUTED));
                });
                return;
            }
            if let Some(problem) = &self.view.problem {
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    ui.add_space(12.);
                    ui.label(RichText::new(problem).color(design::BAD));
                });
                return;
            }
            if self.view.entries.is_empty() {
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    ui.add_space(12.);
                    ui.label(RichText::new(if self.view.loading { "Loading…" } else { "This folder is empty" }).color(design::MUTED));
                });
                return;
            }
            ui.spacing_mut().item_spacing.y = 0.;
            let columns = Columns::new(ui.available_width());
            let heading_height = if inner >= 72. { header(ui, &columns); 26. } else { 0. };
            let mut open = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).min_scrolled_height(0.).max_height((inner - heading_height).max(0.)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                for entry in &self.view.entries {
                    match row(ui, &columns, entry) {
                        RowAction::Open => open = Some(entry.path.clone()),
                        RowAction::Download => requests.push(Request::Get(entry.path.clone())),
                        RowAction::None => {}
                    }
                }
            });
            if let Some(path) = open {
                requests.push(self.view.go(&path));
            }
        });
    }

    fn transfers(&mut self, ui: &mut egui::Ui, requests: &mut Vec<Request>, max: usize) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Transfers").size(SMALL).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel downloads").on_hover_text("Cancel all downloads, including pending folder contents. Uploads continue.").clicked() {
                    requests.push(Request::CancelDownloads);
                }
                if self.view.transfers.iter().any(Transfer::finished) && icons::button(ui, Icon::Close, "Clear finished transfers", true) {
                    self.view.clear_finished();
                }
            });
        });
        if self.view.transfers.is_empty() { return; }
        if max == 0 {
            ui.label(RichText::new(self.view.summary()).size(SMALL).color(design::MUTED));
            return;
        }
        let (rows, more) = self.view.visible(max);
        design::surface().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.;
            for t in rows {
                if transfer_row(ui, t) {
                    requests.push(Request::Cancel(t.id.clone()));
                }
            }
            if more > 0 {
                ui.label(RichText::new(format!("and {more} more")).size(SMALL).color(design::MUTED));
            }
        });
        ui.add_space(6.);
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(self.view.upload && self.dialog.is_none(), design::primary("Upload files")).on_hover_text("Send files to this computer").clicked() {
                    self.pick(false);
                }
                if ui.add_enabled(self.dialog.is_none(), egui::Button::new("Change")).on_hover_text("Choose the download folder").clicked() { self.pick(true); }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let text = self.save_dir.to_string_lossy();
                    ui.add(egui::Label::new(RichText::new(format!("Save to {text}")).size(SMALL).color(design::MUTED)).truncate()).on_hover_text(text.as_ref());
                });
            });
        });
        if !self.message.is_empty() {
            ui.label(RichText::new(&self.message).color(design::BAD));
        }
    }

}

struct Columns {
    width: f32,
    button: f32,
    size: f32,
    date: f32,
}
const PAD: f32 = 12.;
impl Columns {
    fn new(width: f32) -> Self {
        Self { width, button: 32., size: 65., date: if width >= 590. { 88. } else { 0. } }
    }
    fn right(&self) -> f32 {
        self.width - PAD
    }
    fn button_x(&self) -> f32 {
        self.right() - self.button
    }
    fn size_right(&self) -> f32 {
        self.button_x() - 12.
    }
    fn date_x(&self) -> f32 {
        self.size_right() - self.size - 12. - self.date
    }
    /// Left edge of the modified column, or None when the window is too narrow for it.
    fn name_right(&self) -> f32 {
        if self.date > 0. { self.date_x() - 12. } else { self.size_right() - self.size - 12. }
    }
}

fn header(ui: &mut egui::Ui, c: &Columns) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(c.width, 26.), egui::Sense::hover());
    let p = ui.painter();
    let font = egui::FontId::proportional(SMALL);
    let y = rect.center().y;
    p.text(egui::pos2(rect.left() + PAD + 18., y), egui::Align2::LEFT_CENTER, "Name", font.clone(), design::MUTED);
    if c.date > 0. { p.text(egui::pos2(rect.left() + c.date_x(), y), egui::Align2::LEFT_CENTER, "Modified", font.clone(), design::MUTED); }
    p.text(egui::pos2(rect.left() + c.size_right(), y), egui::Align2::RIGHT_CENTER, "Size", font, design::MUTED);
    p.hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1_f32, design::LINE));
}

enum RowAction {
    None,
    Open,
    Download,
}
fn row(ui: &mut egui::Ui, c: &Columns, e: &Entry) -> RowAction {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(c.width, ROW), if e.dir { egui::Sense::click() } else { egui::Sense::hover() });
    let mut action = RowAction::None;
    if response.hovered() {
        ui.painter().rect_filled(rect, 0., Color32::from_rgb(31, 39, 51));
    }
    if e.dir && response.clicked() {
        action = RowAction::Open;
    }
    let y = rect.center().y;
    icons::paint(ui, if e.dir { Icon::Folder } else { Icon::File }, egui::Rect::from_center_size(egui::pos2(rect.left() + PAD + 4., y), egui::vec2(16., 16.)), design::MUTED);
    response.widget_info(|| egui::WidgetInfo::labeled(if e.dir { egui::WidgetType::Button } else { egui::WidgetType::Label }, true, &e.name));
    response.on_hover_text(&e.name);
    let name_left = rect.left() + PAD + 18.;
    let max = (rect.left() + c.name_right() - name_left).max(40.);
    let mut job = egui::text::LayoutJob::simple_singleline(e.name.clone(), egui::FontId::proportional(14.), design::TEXT);
    job.wrap = egui::text::TextWrapping { max_width: max, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let p = ui.painter();
    p.galley(egui::pos2(name_left, y - galley.size().y / 2.), galley, design::TEXT);
    let font = egui::FontId::proportional(SMALL);
    if c.date > 0. { p.text(egui::pos2(rect.left() + c.date_x(), y), egui::Align2::LEFT_CENTER, date(e.mtime), font.clone(), design::MUTED); }
    if !e.dir {
        p.text(egui::pos2(rect.left() + c.size_right(), y), egui::Align2::RIGHT_CENTER, human_size(e.size), font, design::MUTED);
    }
    p.hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1_f32, Color32::from_rgb(33, 41, 53)));
    let button = egui::Rect::from_center_size(egui::pos2(rect.left() + c.button_x() + c.button / 2., y), egui::vec2(c.button, 32.));
    // A child ui, so the button does not move the row cursor.
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
    child.set_clip_rect(child.clip_rect().intersect(rect));
    child.spacing_mut().interact_size.y = SMALL_BUTTON;
    child.spacing_mut().button_padding.y = 3.;
    if icons::button(&mut child, Icon::Download, &format!("Download {}{}", if e.dir { "folder " } else { "" }, e.name), true) {
        action = RowAction::Download;
    }
    action
}

/// One transfer. Returns true when Cancel was clicked.
fn transfer_row(ui: &mut egui::Ui, t: &Transfer) -> bool {
    let mut cancel = false;
    ui.horizontal(|ui| {
        icons::status(ui, if t.upload { Icon::Upload } else { Icon::Download }, if t.upload { "Upload" } else { "Download" }, true);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !t.finished() { cancel = icons::button(ui, Icon::Close, "Cancel this transfer", true); }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(&t.name).truncate()).on_hover_text(&t.name);
            });
        });
    });
    let status = match t.phase {
        Phase::Queued => "Waiting".into(),
        Phase::Active => format!("{} of {}", human_size(t.done), human_size(t.size)),
        Phase::Done => "Done".into(),
        Phase::Failed => if t.error == "Cancelled" { "Cancelled".into() } else { "Failed".into() },
    };
    ui.label(RichText::new(status).size(SMALL).color(design::MUTED));
    match t.phase {
        Phase::Active | Phase::Queued => {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2., Color32::from_rgb(33, 41, 53));
            let fraction = if t.size == 0 { 0. } else { (t.done as f32 / t.size as f32).clamp(0., 1.) };
            let mut fill = rect;
            fill.set_width(rect.width() * fraction);
            ui.painter().rect_filled(fill, 2., design::ACCENT);
        }
        Phase::Done if !t.path.is_empty() => {
            ui.add(egui::Label::new(RichText::new(format!("{} {}", if t.upload { "Sent to" } else { "Saved to" }, t.path)).size(SMALL).color(design::MUTED)).truncate());
        }
        Phase::Failed if !t.error.is_empty() && t.error != "Cancelled" => {
            ui.add(egui::Label::new(RichText::new(&t.error).size(SMALL).color(design::BAD)).truncate()).on_hover_text(&t.error);
        }
        _ => {}
    }
    cancel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, dir: bool) -> Entry {
        Entry { name: name.into(), path: format!("/x/{name}"), dir, size: 1, mtime: 0 }
    }
    fn transfer(id: &str, phase: Phase) -> Transfer {
        Transfer { id: id.into(), name: id.into(), upload: false, size: 10, done: 0, phase, path: String::new(), error: String::new() }
    }

    #[test]
    fn crumbs_for_windows_paths() {
        let c = crumbs("C:\\Users\\c\\Documents");
        let labels: Vec<_> = c.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(labels, ["Computer", "C:", "Users", "c", "Documents"]);
        assert_eq!(c[0].1, "");
        assert_eq!(c[1].1, "C:\\", "a drive keeps its trailing slash");
        assert_eq!(c[2].1, "C:\\Users");
        assert_eq!(c[4].1, "C:\\Users\\c\\Documents");
        assert_eq!(crumbs("C:\\").len(), 2);
    }
    #[test]
    fn crumbs_for_unix_and_network_paths() {
        let c = crumbs("/home/player/Documents");
        let labels: Vec<_> = c.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(labels, ["Computer", "/", "home", "player", "Documents"]);
        assert_eq!(c[1].1, "/");
        assert_eq!(c[3].1, "/home/player");
        assert_eq!(crumbs("/").len(), 2);
        let unc = crumbs("\\\\server\\share\\dir");
        assert_eq!(unc[1].1, "\\\\server");
        assert_eq!(unc[3].1, "\\\\server\\share\\dir");
        assert_eq!(crumbs("").len(), 1);
    }
    #[test]
    fn sizes_and_dates_read_plainly() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(250 * 1024), "250 KB");
        assert_eq!(human_size(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(date(0), "");
        assert_eq!(date(86_400 * 365), "1971-01-01");
        assert_eq!(date(1_709_164_800), "2024-02-29");
        assert_eq!(date(1_794_000_000), "2026-11-06");
    }
    #[test]
    fn the_roots_list_loads_when_the_host_is_ready() {
        let mut v = View::default();
        assert!(matches!(v.apply(Notice::Ready { browse: true, upload: true }), Some(Request::List(p)) if p.is_empty()));
        assert!(v.loading && v.conn == Conn::Online);
        v.apply(Notice::Listed { path: String::new(), entries: vec![entry("a", true)] });
        assert!(!v.loading);
        assert_eq!(v.entries.len(), 1);
    }
    #[test]
    fn a_host_without_browsing_asks_for_nothing() {
        let mut v = View::default();
        assert!(v.apply(Notice::Ready { browse: false, upload: true }).is_none());
        assert!(!v.browse && v.upload);
    }
    #[test]
    fn folders_open_and_back_returns_in_order() {
        let mut v = View::default();
        v.apply(Notice::Ready { browse: true, upload: true });
        assert!(matches!(v.go("/home"), Request::List(p) if p == "/home"));
        v.go("/home/me");
        assert_eq!(v.path, "/home/me");
        assert!(matches!(v.go_back(), Some(Request::List(p)) if p == "/home"));
        assert!(matches!(v.go_back(), Some(Request::List(p)) if p.is_empty()));
        assert!(v.go_back().is_none() && !v.can_go_back());
        // Opening the folder already shown does not stack history.
        v.go("");
        assert!(!v.can_go_back());
    }
    #[test]
    fn a_late_listing_for_another_folder_is_ignored() {
        let mut v = View::default();
        v.go("/a");
        v.go("/b");
        v.apply(Notice::Listed { path: "/a".into(), entries: vec![entry("old", false)] });
        assert!(v.entries.is_empty() && v.loading);
        v.apply(Notice::Listed { path: "/b".into(), entries: vec![entry("new", false)] });
        assert_eq!(v.entries[0].name, "new");
    }
    #[test]
    fn a_refused_listing_shows_the_reason_and_stops_loading() {
        let mut v = View::default();
        v.go("/a");
        v.apply(Notice::Problem(proto::OFF.into()));
        assert!(!v.loading && !v.upload);
        assert_eq!(v.problem.as_deref(), Some("File transfer is turned off on this computer."));
        v.refresh();
        assert!(v.problem.is_none());
    }
    #[test]
    fn transfer_updates_replace_their_row() {
        let mut v = View::default();
        v.apply(Notice::Transfer(transfer("a", Phase::Queued)));
        let mut t = transfer("a", Phase::Active);
        t.done = 5;
        v.apply(Notice::Transfer(t));
        v.apply(Notice::Transfer(transfer("b", Phase::Queued)));
        assert_eq!(v.transfers.len(), 2);
        assert_eq!(v.transfers[0].done, 5);
    }
    #[test]
    fn unfinished_rows_come_first_and_extra_rows_are_counted() {
        let mut v = View::default();
        for (id, phase) in [("a", Phase::Done), ("b", Phase::Failed), ("c", Phase::Active), ("d", Phase::Queued), ("e", Phase::Done)] {
            v.apply(Notice::Transfer(transfer(id, phase)));
        }
        let (rows, more) = v.visible(3);
        let ids: Vec<_> = rows.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["c", "d", "e"], "newest finished first after the running ones");
        assert_eq!(more, 2);
        v.clear_finished();
        assert_eq!(v.transfers.len(), 2);
    }
    #[test]
    fn the_summary_counts_what_is_running_and_what_failed() {
        let mut v = View::default();
        for (id, phase) in [("a", Phase::Done), ("b", Phase::Failed), ("c", Phase::Active)] {
            v.apply(Notice::Transfer(transfer(id, phase)));
        }
        assert_eq!(v.summary(), "1 running, 1 failed");
        v.clear_finished();
        assert_eq!(v.summary(), "1 running");
        assert_eq!(v.visible(0).1, 1);
    }
    #[test]
    fn the_transfer_list_is_bounded_by_dropping_old_finished_rows() {
        let mut v = View::default();
        for i in 0..(KEEP + 50) {
            v.apply(Notice::Transfer(transfer(&format!("t{i}"), Phase::Done)));
        }
        assert_eq!(v.transfers.len(), KEEP);
        assert_eq!(v.transfers[0].id, "t50");
    }
    #[test]
    fn closing_keeps_the_reason_and_hides_a_plain_disconnect() {
        let mut v = View::default();
        v.apply(Notice::Closed("Connection closed".into()));
        assert_eq!(v.conn, Conn::Closed("Connection closed".into()));
        v.apply(Notice::Closed("Disconnected".into()));
        assert_eq!(v.conn, Conn::Closed(String::new()));
    }
    #[test]
    fn the_default_save_folder_is_inside_downloads() {
        let p = default_save_dir();
        assert!(p.ends_with("Downloads/Secpar") || p.ends_with("Downloads\\Secpar"));
    }

    /// Clicks at `at` (relative to the row's top left) on a row drawn in an 800 wide window.
    fn click_row(e: &Entry, at: egui::Vec2) -> &'static str {
        let ctx = egui::Context::default();
        design::configure(&ctx);
        let mut out = "none";
        let mut origin = egui::pos2(0., 0.);
        for frame in 0..3 {
            let mut events = vec![];
            if frame == 2 {
                let pos = origin + at;
                events = vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
                ];
            }
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(800., 600.))), events, ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::new()).show(ctx, |ui| {
                    origin = ui.cursor().min;
                    let columns = Columns::new(ui.available_width());
                    out = match row(ui, &columns, e) {
                        RowAction::Open => "open",
                        RowAction::Download => "download",
                        RowAction::None => "none",
                    };
                });
            });
        }
        out
    }
    #[test]
    fn clicking_a_folder_opens_it_and_its_button_downloads_it() {
        let c = Columns::new(800.);
        let folder = entry("Photos", true);
        assert_eq!(click_row(&folder, egui::vec2(100., ROW / 2.)), "open");
        assert_eq!(click_row(&folder, egui::vec2(c.button_x() + c.button / 2., ROW / 2.)), "download");
    }
    #[test]
    fn clicking_a_file_name_does_nothing_and_its_button_downloads_it() {
        let c = Columns::new(800.);
        let file = entry("a.txt", false);
        assert_eq!(click_row(&file, egui::vec2(100., ROW / 2.)), "none");
        assert_eq!(click_row(&file, egui::vec2(c.button_x() + c.button / 2., ROW / 2.)), "download");
    }

    /// Draws the page `width` wide and returns the heights of its rows.
    fn lay_out(width: f32, height: f32, view: View) -> (f32, f32) {
        let ctx = egui::Context::default();
        design::configure(&ctx);
        let mut panel = Panel::new(ctx.clone(), Some("/tmp/x"));
        panel.view = view;
        panel.name = "Linux laptop".into();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(width, height))), ..Default::default() };
        let mut used = (0., 0.);
        for _ in 0..3 {
            let output = ctx.run(input.clone(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panel.ui(ui, &[], "");
                    used = (ui.min_rect().width(), ui.min_rect().height());
                });
            });
            if panel.view.conn == Conn::Online {
                for label in ["Cancel downloads", "Upload files"] {
                    let shape = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some((shape.clip_rect, text)),
                        _ => None,
                    }).unwrap_or_else(|| panic!("missing {label} at {width}x{height}"));
                    let bounds = egui::Rect::from_min_size(shape.1.pos, shape.1.galley.size());
                    assert!(shape.0.contains_rect(bounds), "clipped {label} at {width}x{height}");
                }
            }
        }
        used
    }
    #[test]
    fn the_page_never_grows_wider_or_taller_than_the_window() {
        let mut view = View { conn: Conn::Online, path: "/x".into(), ..View::default() };
        view.entries = (0..60).map(|i| entry(&format!("a file with quite a long name number {i}.mp4"), i % 3 == 0)).collect();
        for i in 0..8 {
            view.transfers.push(Transfer { error: "The computer refused the file. File transfer may be turned off there.".into(), ..transfer(&format!("t{i}"), if i % 2 == 0 { Phase::Failed } else { Phase::Active }) });
        }
        // 480 x 245 is what the page gets inside a 480 x 360 window after the toolbar and tabs.
        for (w, h) in [(420., 245.), (640., 285.), (480., 245.), (480., 360.), (1280., 720.), (1366., 768.)] {
            let (used_w, used_h) = lay_out(w, h, view.clone());
            assert!(used_w <= w + 0.5, "{w}x{h}: width {used_w}");
            assert!(used_h <= h + 0.5, "{w}x{h}: height {used_h}");
        }
    }
}

#[cfg(test)]
#[path = "files_owned_tests.rs"]
mod owned_tests;
