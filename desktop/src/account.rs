//! Account credentials remain on this worker; UI events never contain a bearer token.
use crate::{local_host, previews, vault};
use reqwest::{Client, Method, StatusCode};
use secpar_client::control::Computer;
use secpar_client::pairing::Pairing;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const ORIGIN: &str = "https://accounts.secpar.win";
const CREDENTIAL: &str = "account-session";

#[derive(Clone, Deserialize)]
pub struct User {
    pub id: String,
    pub handle: String,
    #[serde(default)]
    pub visible: bool,
}
/// One registered user who chose to appear in search.
#[derive(Clone, Deserialize)]
pub struct Person {
    pub handle: String,
    pub cursor: String,
    pub relation: String,
}
/// A connection request waiting for this account's answer.
#[derive(Clone, Deserialize)]
pub struct Incoming {
    pub id: String,
    pub owner: String,
    pub guest_handle: String,
    #[serde(default)]
    pub computer_name: Option<String>,
    pub requested: String,
    pub state: String,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answer { AllowConnection, DenyConnection, AcceptFriend, DeclineFriend }
fn default_input() -> String { "view".into() }
fn default_connect() -> String { "ask".into() }
#[derive(Clone, Deserialize)]
pub struct Friend {
    pub id: String,
    pub handle: String,
    pub state: String,
    pub requester: String,
    #[serde(default = "default_input")]
    pub input: String,
    #[serde(default = "default_connect")]
    pub connect: String,
    /// Set by a service that allows keyboard and mouse apart. Absent means `input` decides both.
    #[serde(default)]
    pub keyboard: Option<bool>,
    #[serde(default)]
    pub mouse: Option<bool>,
}
impl Friend {
    pub fn keyboard(&self) -> bool { self.keyboard.unwrap_or(self.input == "full") }
    pub fn mouse(&self) -> bool { self.mouse.unwrap_or(self.input == "full") }
}
/// What this friend may do on the computers you host. Shown beside the name.
pub fn friend_summary(friend: &Friend) -> String {
    let control = match (friend.keyboard(), friend.mouse()) {
        (true, true) => "Keyboard and mouse",
        (true, false) => "Keyboard only",
        (false, true) => "Mouse only",
        (false, false) => "View only",
    };
    format!("{} · {control}", if friend.connect == "always" { "Any time" } else { "Ask first" })
}
#[derive(Clone, Deserialize)]
pub struct OwnedComputer {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub certificate_fp: Option<String>,
    pub linked: u8,
    pub online: u8,
}
/// A computer of an accepted friend that is online and lets friends connect.
#[derive(Clone, Deserialize)]
pub struct FriendComputer {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub platform: String,
    pub owner_handle: String,
    /// What the owner lets this account do there. An older service leaves these out.
    #[serde(default = "default_connect")]
    pub connect: String,
    #[serde(default)]
    pub keyboard: bool,
    #[serde(default)]
    pub mouse: bool,
}
impl FriendComputer {
    /// The owner's settings for you, in the words of the friends list.
    pub fn access(&self) -> String {
        let control = match (self.keyboard, self.mouse) {
            (true, true) => "Keyboard and mouse",
            (true, false) => "Keyboard only",
            (false, true) => "Mouse only",
            (false, false) => "View only",
        };
        format!("{} · {control}", if self.connect == "always" { "Any time" } else { "Ask first" })
    }
}
/// What a friend session needs once the owner said yes. The grant works once for 60 seconds.
pub struct Grant {
    pub relay: String,
    pub pc: String,
    pub grant: String,
    pub name: String,
    pub owner_handle: String,
    pub permissions: Vec<String>,
}
impl Grant {
    pub fn allows(&self, permission: &str) -> bool { self.permissions.iter().any(|p| p == permission) }
}
/// How a request for a friend's computer is going.
pub enum Guest {
    Waiting(String),
    Refused(String),
    Grant(Grant),
}
pub const PERMISSION_LABELS: [&str; 5] = ["View", "Audio", "Keyboard and mouse", "Controller", "Clipboard"];
pub const PERMISSION_DEFAULTS: [bool; 5] = [true, true, true, false, false];
const PERMISSIONS: [&str; 7] = ["view", "audio", "gamepad", "keyboard", "mouse", "clipboard", "files"];
/// The permission names for the five checkboxes: View, Audio, Keyboard and mouse, Controller, Clipboard.
pub fn guest_permissions(flags: [bool; 5]) -> Vec<String> {
    let mut list = Vec::new();
    for (on, names) in flags.iter().zip([&["view"][..], &["audio"], &["keyboard", "mouse"], &["gamepad"], &["clipboard"]]) {
        if *on { list.extend(names.iter().map(|n| n.to_string())); }
    }
    list
}
fn valid_handle(handle: &str) -> bool {
    (1..=30).contains(&handle.len()) && handle.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
/// The permission list from a request row. The service stores lists as JSON text.
fn permission_list(value: &Value) -> Option<Vec<String>> {
    let list: Vec<String> = match value {
        Value::String(text) => serde_json::from_str(text).ok()?,
        Value::Array(_) => serde_json::from_value(value.clone()).ok()?,
        _ => return None,
    };
    (list.len() <= 7 && list.iter().all(|p| PERMISSIONS.contains(&p.as_str()))).then_some(list)
}
/// Plain words for a request that ended without a session.
fn guest_refusal(state: &str, owner: &str, was_approved: bool) -> String {
    match (state, was_approved) {
        ("rejected" | "revoked", false) => format!("@{owner} said no."),
        ("rejected" | "revoked", true) => "Access was removed.".into(),
        _ => "This request was already used. Ask again.".into(),
    }
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Presence { Online, Offline, Streaming, Locked }
/// Status text and dot colour for a computer card. Online is OK, streaming is ACCENT, locked and
/// "Host update required" are WARN, offline is MUTED.
pub fn computer_status(linked: bool, presence: Presence) -> (&'static str, eframe::egui::Color32) {
    use crate::design::{ACCENT, MUTED, OK, WARN};
    if !linked { return ("Host update required", WARN); }
    match presence {
        Presence::Streaming => ("Streaming", ACCENT),
        Presence::Locked => ("Locked", WARN),
        Presence::Online => ("Online", OK),
        Presence::Offline => ("Offline", MUTED),
    }
}
pub const SIGN_IN_CONFIRM: &str = "Confirm this code in your browser";
pub const NO_COMPUTERS: &str = "No computers yet";
pub const NO_OTHER_COMPUTERS: &str = "No other computers yet";
/// The computers you can connect to. The one this app is hosting from is left out: you are already
/// on it and cannot connect to yourself.
pub fn remote_computers<'a>(computers: &'a [OwnedComputer], local: Option<&str>) -> Vec<&'a OwnedComputer> {
    computers.iter().filter(|c| local != Some(c.id.as_str())).collect()
}
pub fn pairing_with(name: &str) -> String { format!("Pairing with {name}…") }
pub enum OwnedConnection {
    /// `id` is the account's id for the computer. The credential's own id is the host's, a different one.
    Cached { owner: String, id: String, computer: Computer },
    Pair { owner: String, id: String, pairing: Pairing },
}
pub enum Action {
    Invite(String),
    Accept(String),
    Remove(String),
    Block(String),
    Unblock(String),
    /// Keyboard and mouse ("view" or "full") and connect policy ("ask" or "always") for one friend.
    Setting { id: String, input: Option<&'static str>, connect: Option<&'static str> },
    /// Keyboard and mouse, each on its own.
    Input { id: String, keyboard: Option<bool>, mouse: Option<bool> },
}
pub enum Command {
    HostRefresh,
    LinkHost,
    UnlinkHost,
    SignIn,
    Cancel,
    Refresh,
    Action(Action),
    SignOut,
    Remember(Computer),
    ForgetComputer,
    ConnectOwned(String),
    RememberOwned { owner: String, id: String, computer: Computer },
    Search(String),
    SetVisible(bool),
    Answer(Answer, String),
    RequestGuest { computer: FriendComputer, permissions: Vec<String> },
    CancelGuest,
    ForgetOwned { owner: String, id: String },
}
pub enum Event {
    Host(Result<local_host::State, String>),
    SignedOut(String),
    Waiting {
        code: String,
        url: String,
    },
    SignedIn {
        user: User,
        friends: Vec<Friend>,
        blocked: Vec<User>,
        note: String,
    },
    Message(String),
    Busy(bool),
    Restored(Computer),
    ForgotComputer,
    StorageNote(String),
    Owned(Vec<OwnedComputer>),
    OwnedConnection(OwnedConnection),
    Directory(Result<Vec<Person>, String>),
    Live { friends: Vec<Friend>, requests: Vec<Incoming> },
    FriendComputers(Vec<FriendComputer>),
    Guest(Guest),
}

pub struct Panel {
    host: Option<Result<local_host::State, String>>,
    pub restored_computer: Option<Computer>,
    pub forgot_computer: bool,
    pub storage_note: Option<String>,
    pub user: Option<User>,
    pub computers: Vec<OwnedComputer>,
    pub owned_connection: Option<OwnedConnection>,
    /// The next owned connection is for the Files page, not for a stream.
    files_wanted: bool,
    pub files_connection: Option<OwnedConnection>,
    pub signed_out: bool,
    previews: previews::Previews,
    textures: previews::Cache<eframe::egui::TextureHandle>,
    friends: Vec<Friend>,
    blocked: Vec<User>,
    waiting: Option<(String, String)>,
    note: String,
    query: String,
    asked: Option<String>,
    query_at: Option<Instant>,
    people: Vec<Person>,
    requests: Vec<Incoming>,
    friend_computers: Vec<FriendComputer>,
    guest_choice: Option<(FriendComputer, [bool; 5])>,
    guest_waiting: Option<String>,
    /// Set by the window: a session is open, so no second one starts.
    pub session_open: bool,
    /// Taken by the window, which opens the session.
    pub guest_grant: Option<Grant>,
    busy: bool,
    tx: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
}
impl Panel {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        #[allow(unused_variables)]
        let preview_ctx = ctx.clone();
        let (tx, events) = {
            #[cfg(feature = "ui-review")]
            if std::env::var_os("SECPAR_REVIEW_SCREENSHOT_TO").is_some() {
                let (tx, _rx) = mpsc::channel(1);
                let (_events, rx) = mpsc::channel(1);
                let sample =
                    std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p.ends_with("-sample"));
                let own = "00000000-0000-4000-8000-000000000001".to_string();
                return Self {
                    host: Some(Ok(
                        if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p=="host-sample" || p.contains("owned")) {
                            local_host::State {
                                computer: Some("00000000-0000-4000-8000-000000000010".into()),
                                owner: Some(own.clone()),
                                online: true,
                                note: String::new(),
                            }
                        } else {
                            local_host::State::default()
                        },
                    )),
                    restored_computer: None,
                    computers: if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("owned")) {
                        let long=std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("long"));
                        let mut list=vec![("00000000-0000-4000-8000-000000000010","Studio PC","windows",1),("00000000-0000-4000-8000-000000000011","Linux laptop","linux",1),("00000000-0000-4000-8000-000000000012","Workshop","linux",0)];
                        if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("only")) { list.truncate(1); }
                        if long {
                            list[1].1="Brother's Gaming Laptop (Linux)";
                            list.extend([("00000000-0000-4000-8000-000000000013","Living room media center","windows",1),("00000000-0000-4000-8000-000000000014","Dad's old desktop","windows",0),("00000000-0000-4000-8000-000000000015","Office build server","linux",1),("00000000-0000-4000-8000-000000000016","Garage PC","windows",1)]);
                        }
                        list.into_iter().map(|(id,name,platform,online)|OwnedComputer{id:id.into(),name:name.into(),platform:platform.into(),certificate_fp:Some("ab".repeat(32)),linked:1,online}).collect()
                    }else{vec![]},
                    owned_connection: None,
                    files_wanted: false,
                    files_connection: None,
                    signed_out: false,
                    previews: previews::Previews::inert(),
                    textures: review_textures(&ctx),
                    forgot_computer: false,
                    storage_note: None,
                    user: sample.then(|| User {
                        id: own.clone(),
                        handle: "Player".into(),
                        visible: true,
                    }),
                    friends: if sample {
                        vec![
                            Friend {
                                id: "00000000-0000-4000-8000-000000000002".into(),
                                handle: "Jamie".into(),
                                state: "accepted".into(),
                                requester: own.clone(),
                                input: "view".into(),
                                connect: "ask".into(),
                                keyboard: None,
                                mouse: None,
                            },
                            Friend {
                                id: "00000000-0000-4000-8000-000000000003".into(),
                                handle: "Morgan".into(),
                                state: "pending".into(),
                                requester: "00000000-0000-4000-8000-000000000003".into(),
                                input: "view".into(),
                                connect: "ask".into(),
                                keyboard: None,
                                mouse: None,
                            },
                            Friend {
                                id: "00000000-0000-4000-8000-000000000004".into(),
                                handle: "Riley".into(),
                                state: "pending".into(),
                                requester: own.clone(),
                                input: "view".into(),
                                connect: "ask".into(),
                                keyboard: None,
                                mouse: None,
                            },
                        ]
                    } else {
                        vec![]
                    },
                    blocked: vec![],
                    waiting: None,
                    note: String::new(),
                    query: String::new(),
                    asked: None,
                    query_at: None,
                    people: vec![],
                    requests: vec![],
                    busy: false,
                    tx,
                    events: rx,
                };
            }
            start(ctx)
        };
        Self {
            host: None,
            restored_computer: None,
            computers: vec![],
            owned_connection: None,
            files_wanted: false,
            files_connection: None,
            signed_out: false,
            previews: previews::Previews::new(preview_ctx),
            textures: Default::default(),
            forgot_computer: false,
            storage_note: None,
            user: None,
            friends: vec![],
            blocked: vec![],
            waiting: None,
            note: String::new(),
            query: String::new(),
            asked: None,
            query_at: None,
            people: vec![],
            requests: vec![],
            friend_computers: Vec::new(),
            guest_choice: None,
            guest_waiting: None,
            session_open: false,
            guest_grant: None,
            busy: false,
            tx,
            events,
        }
    }
    fn send(&mut self, command: Command) {
        if self.tx.try_send(command).is_ok() {
            self.busy = true;
        } else {
            self.note = "An account request is already in progress. Please wait.".into();
        }
    }
    pub fn remember(&mut self, computer: Computer) {
        self.send(Command::Remember(computer));
    }
    pub fn forget_computer(&mut self) {
        self.send(Command::ForgetComputer);
    }
    pub fn connect_owned(&mut self, id: String) { self.files_wanted = false; self.send(Command::ConnectOwned(id)); }
    /// Same as `connect_owned`, but the credential goes to the Files page instead of a stream.
    pub fn connect_owned_for_files(&mut self, id: String) {
        self.note.clear();
        self.files_wanted = true;
        self.send(Command::ConnectOwned(id));
    }
    /// What the account worker last said, such as pairing progress or why a request failed.
    pub fn note(&self) -> &str { &self.note }
    /// Computers the Files page can open: the other computers on the account and one paired by hand.
    pub fn picks(&self, manual: Option<&Computer>) -> Vec<crate::files::Pick> {
        let mut out: Vec<crate::files::Pick> = if self.user.is_some() {
            remote_computers(&self.computers, self.local_id()).into_iter()
                .map(|c| crate::files::Pick { id: c.id.clone(), name: c.name.clone(), ready: c.online == 1 && c.linked == 1, saved: None })
                .collect()
        } else { vec![] };
        if let Some(c) = manual.filter(|c| out.iter().all(|p| p.id != c.id)) {
            out.push(crate::files::Pick { id: c.id.clone(), name: c.name.clone(), ready: true, saved: Some(c.clone()) });
        }
        out
    }
    pub fn remember_owned(&mut self, owner: String, id: String, computer: Computer) {
        self.send(Command::RememberOwned { owner, id, computer });
    }
    /// Drops a saved credential the computer no longer accepts, so the next connection pairs again.
    pub fn forget_owned(&mut self, owner: String, id: String) {
        self.send(Command::ForgetOwned { owner, id });
    }
    /// Takes new preview frames and tells the preview worker which computers to watch. A computer
    /// being streamed, this computer and offline ones get no preview.
    pub fn sync_previews(&mut self, ctx: &eframe::egui::Context, streaming: Option<&str>, manual: Option<&Computer>) {
        let local = self.host.as_ref().and_then(|state| state.as_ref().ok()).and_then(|state| state.computer.as_deref());
        let owner = self.user.as_ref().map(|user| user.id.as_str());
        let shown = |c: &OwnedComputer| c.online == 1 && c.linked == 1 && Some(c.id.as_str()) != local && Some(c.id.as_str()) != streaming;
        let targets = owner.map(|owner| {
            self.computers.iter().filter(|c| shown(c)).filter_map(|c| {
                let fingerprint = secpar_client::pairing::fingerprint(c.certificate_fp.as_deref()?).ok()?;
                Some(previews::Target { id: c.id.clone(), key: owned_key(owner, &c.id).ok()?, fingerprint })
            }).collect::<Vec<_>>()
        });
        let mut targets = targets.unwrap_or_default();
        // A computer paired by link or QR on this device is watched the same way, from its saved credential.
        let manual = manual.filter(|c| Some(c.id.as_str()) != streaming && targets.iter().all(|t| t.id != c.id));
        if let Some(c) = manual {
            targets.push(previews::Target { id: c.id.clone(), key: "paired-computer".into(), fingerprint: c.fingerprint });
        }
        self.previews.set(previews::Wanted { targets });
        self.textures.apply(self.previews.take(), |id, image| ctx.load_texture(format!("preview {id}"), image, eframe::egui::TextureOptions::LINEAR));
        let computers = &self.computers;
        let manual_id = manual.map(|c| c.id.as_str());
        self.textures.retain(|id| manual_id == Some(id) || computers.iter().any(|c| c.id == id && shown(c)));
    }
    /// The newest preview picture of a computer, if it gave one.
    pub fn preview(&self, id: &str) -> Option<&eframe::egui::TextureHandle> {
        self.textures.get(id)
    }
    /// The computer this app hosts from, when the local host is linked to the account.
    pub fn local_id(&self) -> Option<&str> {
        self.host.as_ref().and_then(|state| state.as_ref().ok()).and_then(|state| state.computer.as_deref())
    }
    /// Computers that friends share with you, drawn like your own.
    fn friend_cards(&mut self, ui: &mut eframe::egui::Ui) {
        use crate::design;
        for computer in self.friend_computers.clone() {
            let platform = match computer.platform.as_str() { "windows" => "Windows", "linux" => "Linux", "macos" => "macOS", _ => "" };
            let status = format!("@{} · {}", computer.owner_handle, computer.access());
            let enabled = !self.busy && !self.session_open;
            let clicked = crate::card::show(ui, crate::card::Card { name: &computer.name, status: Some((status.as_str(), design::OK)), platform, preview: None, connect: Some(enabled), menu: None });
            if clicked {
                self.guest_choice = Some((computer, PERMISSION_DEFAULTS));
            }
        }
    }
    pub fn computers_ui(&mut self, ui: &mut eframe::egui::Ui) {
        use crate::design;
        use eframe::egui;
        design::page_header(ui,"Computers",|ui| {
            if let Some(user) = self.user.clone() {
                if ui.add_enabled(!self.busy,egui::Button::new("Sign out")).clicked() { self.send(Command::SignOut); }
                if ui.add_enabled(!self.busy,egui::Button::new("Refresh")).clicked() { self.send(Command::Refresh); }
                ui.add(egui::Label::new(user.handle).truncate());
            } else if ui.add_enabled(!self.busy && self.waiting.is_none(),egui::Button::new("Sign in")).clicked() {
                self.send(Command::SignIn);
            }
        });
        ui.add_space(6.);
        if let Some((code,url)) = self.waiting.clone() {
            design::surface().show(ui,|ui| {
                ui.horizontal(|ui| {
                    ui.label(SIGN_IN_CONFIRM); ui.strong(code);
                    if ui.button("Open browser").clicked() { std::thread::spawn(move || { let _=open::that(url); }); }
                    if ui.button("Cancel").clicked() { self.send(Command::Cancel); }
                });
            });
        }
        if self.user.is_some() {
            let mut connect = None;
            ui.spacing_mut().item_spacing.y=6.;
            let shown = remote_computers(&self.computers,self.local_id());
            for computer in &shown {
                let preview=self.textures.get(&computer.id).filter(|_| computer.online==1);
                let presence=if computer.online==1 {Presence::Online}else{Presence::Offline};
                let (status,color)=computer_status(computer.linked==1,presence);
                let platform=match computer.platform.as_str(){"windows"=>"Windows","linux"=>"Linux","macos"=>"macOS",_=>""};
                let clicked=crate::card::show(ui,crate::card::Card{name:&computer.name,status:Some((status,color)),platform,preview,connect:Some(!self.busy && computer.linked==1),menu:None});
                if clicked { connect=Some(computer.id.clone()); }
            }
            let none_of_ours = shown.is_empty();
            let waiting = self.guest_flow_ui(ui);
            if !waiting { self.friend_cards(ui); }
            ui.spacing_mut().item_spacing.y=8.;
            if none_of_ours && self.friend_computers.is_empty() { ui.label(egui::RichText::new(if self.computers.is_empty() {NO_COMPUTERS}else{NO_OTHER_COMPUTERS}).color(design::MUTED)); }
            if let Some(id)=connect { self.connect_owned(id); }
            if !self.note.is_empty() { ui.label(&self.note); }
            ui.add_space(6.);
        }
    }
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Host(state) => self.host = Some(state),
                Event::SignedOut(note) => {
                    self.computers.clear();
                    self.owned_connection = None;
                    self.files_connection = None;
                    self.signed_out = true;
                    self.user = None;
                    self.friends.clear();
                    self.requests.clear();
                    self.friend_computers.clear();
                    self.guest_choice = None;
                    self.guest_waiting = None;
                    self.guest_grant = None;
                    self.people.clear();
                    self.asked = None;
                    self.blocked.clear();
                    self.waiting = None;
                    self.note = note;
                }
                Event::Waiting { code, url } => {
                    self.waiting = Some((code, url));
                    self.note.clear();
                }
                Event::SignedIn {
                    user,
                    friends,
                    blocked,
                    note,
                } => {
                    self.user = Some(user);
                    self.friends = friends;
                    self.blocked = blocked;
                    self.waiting = None;
                    self.note = note;
                    self.asked = None;
                }
                Event::Directory(Ok(people)) => self.people = people,
                Event::Directory(Err(note)) => { self.people.clear(); self.note = note; }
                Event::Live { friends, requests } => { self.friends = friends; self.requests = requests; }
                Event::FriendComputers(list) => self.friend_computers = list,
                Event::Guest(Guest::Waiting(owner)) => { self.guest_waiting = Some(owner); self.note.clear(); }
                Event::Guest(Guest::Refused(note)) => { self.guest_waiting = None; self.note = note; }
                Event::Guest(Guest::Grant(grant)) => { self.guest_waiting = None; self.note.clear(); self.guest_grant = Some(grant); }
                Event::Message(note) => self.note = note,
                Event::Busy(busy) => self.busy = busy,
                Event::Restored(computer) => self.restored_computer = Some(computer),
                Event::ForgotComputer => self.forgot_computer = true,
                Event::StorageNote(note) => self.storage_note = Some(note),
                Event::Owned(computers) => self.computers = computers,
                Event::OwnedConnection(connection) => {
                    if std::mem::take(&mut self.files_wanted) { self.files_connection = Some(connection); }
                    else { self.owned_connection = Some(connection); }
                }
            }
        }
    }
    pub fn host_ui(&mut self, ui: &mut eframe::egui::Ui) {
        use crate::design;
        use eframe::egui;
        design::surface().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("This computer").size(15.).strong());
                ui.label(
                    egui::RichText::new("Friend sharing off")
                        .size(12.)
                        .color(design::MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if cfg!(windows)
                        && ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(egui::RichText::new("Refresh").size(12.))
                                    .frame(false),
                            )
                            .clicked()
                    {
                        self.send(Command::HostRefresh);
                    }
                });
            });
            match self.host.clone() {
                Some(Ok(state)) if state.computer.is_some() => {
                    let own = self
                        .user
                        .as_ref()
                        .is_some_and(|u| Some(&u.id) == state.owner.as_ref());
                    ui.horizontal(|ui| {
                        design::dot(
                            ui,
                            if state.online {
                                design::ACCENT
                            } else {
                                design::MUTED
                            },
                        );
                        ui.label(if own {
                            "Linked to your account"
                        } else {
                            "Host account linked"
                        });
                        ui.label(
                            egui::RichText::new(if state.online { "Online" } else { "Offline" })
                                .size(12.)
                                .color(design::MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    !self.busy,
                                    egui::Button::new("Unlink host")
                                        .min_size(egui::vec2(116., design::CONTROL)),
                                )
                                .clicked()
                            {
                                self.send(Command::UnlinkHost);
                            }
                        });
                    });
                    if !state.note.is_empty() {
                        ui.label(state.note);
                    }
                }
                Some(Ok(_)) => {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Account not linked").color(design::MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    !self.busy && self.user.is_some(),
                                    egui::Button::new("Link my account"),
                                )
                                .on_disabled_hover_text("Sign in under Friends to link this host.")
                                .clicked()
                            {
                                self.send(Command::LinkHost);
                            }
                        });
                    });
                }
                Some(Err(_)) => {
                    ui.horizontal_wrapped(|ui| {
                        if cfg!(windows) {
                            ui.label(
                                egui::RichText::new(
                                    "Hosting service unavailable or needs an update",
                                )
                                .color(design::MUTED),
                            );
                            ui.hyperlink_to(
                                "Secpar downloads",
                                "https://github.com/modnite/Secpar/releases",
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(
                                    "Hosting on this platform is still in development.",
                                )
                                .color(design::MUTED),
                            );
                        }
                    });
                }
                None => {
                    ui.label(egui::RichText::new("Checking Secpar Host…").color(design::MUTED));
                }
            }
            if !self.note.is_empty() {
                ui.label(&self.note);
            }
        });
    }
    fn requests_ui(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;
        let own = self.user.as_ref().map(|u| u.id.clone()).unwrap_or_default();
        let waiting: Vec<Incoming> = self.requests.iter().filter(|r| r.owner == own && r.state == "pending").cloned().collect();
        for request in waiting {
            ui.horizontal(|ui| {
                let computer = request.computer_name.clone().unwrap_or_else(|| "your computer".into());
                ui.add(egui::Label::new(format!("@{} wants to use {computer}", request.guest_handle)).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!self.busy, egui::Button::new("Deny")).clicked() {
                        self.send(Command::Answer(Answer::DenyConnection, request.id.clone()));
                    }
                    if ui.add_enabled(!self.busy, egui::Button::new("Allow")).clicked() {
                        self.send(Command::Answer(Answer::AllowConnection, request.id.clone()));
                    }
                });
            });
        }
    }
    /// The waiting line and the permission choice for a friend's computer. True when one is showing,
    /// and the list of computers stays hidden meanwhile.
    fn guest_flow_ui(&mut self, ui: &mut eframe::egui::Ui) -> bool {
        use eframe::egui;
        {
            if let Some(owner) = self.guest_waiting.clone() {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(format!("Waiting for @{owner}")).truncate());
                    ui.spinner();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cancel").clicked() {
                            let _ = self.tx.try_send(Command::CancelGuest);
                            self.guest_waiting = None;
                        }
                    });
                });
                return true;
            }
            if let Some((computer, mut flags)) = self.guest_choice.clone() {
                ui.add(egui::Label::new(format!("{} (@{})", computer.name, computer.owner_handle)).truncate());
                for (flag, label) in flags.iter_mut().zip(PERMISSION_LABELS) {
                    ui.checkbox(flag, label);
                }
                let permissions = guest_permissions(flags);
                let (mut ask, mut cancel) = (false, false);
                ui.horizontal(|ui| {
                    ask = ui.add_enabled(!permissions.is_empty(), egui::Button::new("Request access")).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if ask {
                    let owner = computer.owner_handle.clone();
                    if self.tx.try_send(Command::RequestGuest { computer, permissions }).is_ok() {
                        self.guest_waiting = Some(owner);
                    } else {
                        self.note = "An account request is already in progress. Please wait.".into();
                    }
                    self.guest_choice = None;
                } else if cancel {
                    self.guest_choice = None;
                } else {
                    self.guest_choice = Some((computer, flags));
                }
                return true;
            }
        }
        false
    }
    fn friend_computers_ui(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;
        egui::CollapsingHeader::new("Friends' computers").default_open(true).show(ui, |ui| {
            if self.guest_flow_ui(ui) {
                return;
            }
            if self.friend_computers.is_empty() {
                ui.weak("No friends' computers online");
            }
            for computer in self.friend_computers.clone() {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(&computer.name).truncate());
                    ui.label(egui::RichText::new(format!("@{}", computer.owner_handle)).size(12.).color(crate::design::MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(!self.session_open, egui::Button::new("Connect")).clicked() {
                            self.guest_choice = Some((computer.clone(), PERMISSION_DEFAULTS));
                        }
                    });
                });
            }
        });
    }
    fn people_ui(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;
        egui::CollapsingHeader::new("Find people").default_open(self.friends.is_empty()).show(ui, |ui| {
            let width = ui.available_width().max(80.);
            let edit = ui.add_sized(
                [width, crate::design::CONTROL],
                egui::TextEdit::singleline(&mut self.query).hint_text("Search by username").margin(egui::vec2(10., 7.)),
            );
            if edit.changed() {
                self.query_at = Some(Instant::now());
            }
            let wanted = self.query.trim().to_string();
            if self.asked.as_deref() != Some(&wanted) {
                if self.query_at.is_none_or(|t| t.elapsed() >= Duration::from_millis(300)) {
                    self.asked = Some(wanted.clone());
                    self.query_at = None;
                    let _ = self.tx.try_send(Command::Search(wanted));
                } else {
                    ui.ctx().request_repaint_after(Duration::from_millis(300));
                }
            }
            if self.people.is_empty() {
                ui.weak("No one found");
            }
            for person in self.people.clone() {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(format!("@{}", person.handle)).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match person.relation.as_str() {
                        "none" => {
                            if ui.add_enabled(!self.busy, egui::Button::new("Add")).clicked() {
                                self.send(Command::Action(Action::Invite(person.handle.clone())));
                            }
                        }
                        "friend" => { ui.label(egui::RichText::new("Friend").size(12.).color(crate::design::MUTED)); }
                        "requested" => { ui.label(egui::RichText::new("Request sent").size(12.).color(crate::design::MUTED)); }
                        _ => { ui.label(egui::RichText::new("Sent you a request").size(12.).color(crate::design::MUTED)); }
                    });
                });
            }
        });
    }
    pub fn ui(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;
        crate::design::page_header(ui, "Friends", |ui| {
            if let Some(user) = &self.user {
                let handle = user.handle.clone();
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Sign out"))
                    .clicked()
                {
                    self.send(Command::SignOut);
                }
                ui.add(egui::Label::new(handle).truncate());
            }
        });
        ui.add_space(8.);
        if self.user.is_none() {
            if let Some((code, url)) = self.waiting.clone() {
                ui.label(SIGN_IN_CONFIRM);
                ui.heading(code);
                ui.horizontal(|ui| {
                    if ui.button("Open browser").clicked() {
                        std::thread::spawn(move || {
                            let _ = open::that(url);
                        });
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Cancel"))
                        .clicked()
                    {
                        self.send(Command::Cancel);
                    }
                });
            } else {
                ui.label("Sign in to add friends.");
                ui.add_space(8.);
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Button::new("Sign in").min_size(egui::vec2(100., 32.)),
                    )
                    .clicked()
                {
                    self.send(Command::SignIn);
                }
                ui.hyperlink_to("Create an account", format!("{ORIGIN}/auth/register"));
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                let handle = self.user.as_ref().map(|u| u.handle.clone()).unwrap_or_default();
                ui.add(egui::Label::new(you_are(&handle)).truncate());
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Refresh"))
                    .clicked()
                {
                    self.send(Command::Refresh);
                }
            });
            ui.add_space(8.);
            let mut visible = self.user.as_ref().is_some_and(|u| u.visible);
            if ui.add_enabled(!self.busy, egui::Checkbox::new(&mut visible, "Show me in search")).changed() {
                self.send(Command::SetVisible(visible));
            }
            ui.add_space(4.);
            self.requests_ui(ui);
            self.friend_computers_ui(ui);
            self.people_ui(ui);
            ui.add_space(8.);
            if self.friends.is_empty() {
                ui.weak("No friends yet");
            }
            let own = self.user.as_ref().unwrap().id.clone();
            let friends = self.friends.clone();
            // Only the friend list scrolls; navigation and primary actions stay visible.
            egui::ScrollArea::vertical()
                .id_salt("friends-list")
                .max_height(
                    (ui.available_height()
                        - if self.blocked.is_empty() && self.note.is_empty() {
                            0.
                        } else {
                            48.
                        })
                    .max(60.),
                )
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.;
                    for (index, friend) in friends.into_iter().enumerate() {
                        if index > 0 {
                            ui.separator();
                        }
                        ui.horizontal(|ui| {
                            let (avatar, _) =
                                ui.allocate_exact_size(egui::vec2(32., 32.), egui::Sense::hover());
                            ui.painter().rect_filled(
                                avatar,
                                6,
                                egui::Color32::from_rgb(32, 48, 54),
                            );
                            ui.painter().text(
                                avatar.center(),
                                egui::Align2::CENTER_CENTER,
                                friend
                                    .handle
                                    .chars()
                                    .next()
                                    .unwrap_or('?')
                                    .to_uppercase()
                                    .to_string(),
                                egui::FontId::proportional(14.),
                                crate::design::ACCENT,
                            );
                            ui.add(egui::Label::new(&friend.handle).truncate());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.menu_button("…", |ui| {
                                        if friend.state == "accepted" {
                                            ui.label("Control");
                                            let (mut keyboard, mut mouse) = (friend.keyboard(), friend.mouse());
                                            if ui.checkbox(&mut keyboard, "Keyboard").changed() {
                                                self.send(Command::Action(Action::Input { id: friend.id.clone(), keyboard: Some(keyboard), mouse: None }));
                                                ui.close();
                                            }
                                            if ui.checkbox(&mut mouse, "Mouse").changed() {
                                                self.send(Command::Action(Action::Input { id: friend.id.clone(), keyboard: None, mouse: Some(mouse) }));
                                                ui.close();
                                            }
                                            ui.separator();
                                            ui.label("Connecting");
                                            for (value, label) in [("always", "Any time"), ("ask", "Ask me first")] {
                                                if ui.radio(friend.connect == value, label).clicked() {
                                                    self.send(Command::Action(Action::Setting { id: friend.id.clone(), input: None, connect: Some(value) }));
                                                    ui.close();
                                                }
                                            }
                                            ui.separator();
                                        }
                                        if ui.button("Block").clicked() {
                                            self.send(Command::Action(Action::Block(
                                                friend.id.clone(),
                                            )));
                                            ui.close();
                                        }
                                        if ui
                                            .button(if friend.state == "pending" {
                                                "Cancel request"
                                            } else {
                                                "Remove friend"
                                            })
                                            .clicked()
                                        {
                                            self.send(Command::Action(Action::Remove(
                                                friend.id.clone(),
                                            )));
                                            ui.close();
                                        }
                                    });
                                    if friend.state == "accepted" {
                                        ui.label(
                                            egui::RichText::new(friend_summary(&friend))
                                                .size(12.)
                                                .color(crate::design::MUTED),
                                        );
                                    }
                                    if friend.state == "pending" {
                                        if friend.requester != own {
                                            if ui
                                                .add_enabled(
                                                    !self.busy,
                                                    egui::Button::new("Accept"),
                                                )
                                                .clicked()
                                            {
                                                self.send(Command::Action(Action::Accept(
                                                    friend.id.clone(),
                                                )));
                                            }
                                        } else {
                                            ui.label(egui::RichText::new("Request sent").size(12.).color(crate::design::MUTED));
                                        }
                                    }
                                },
                            );
                        });
                    }
                });
        }
        if !self.blocked.is_empty() {
            eframe::egui::CollapsingHeader::new("Blocked people").show(ui, |ui| {
                for person in self.blocked.clone() {
                    ui.horizontal(|ui| {
                        ui.label(&person.handle);
                        if ui
                            .add_enabled(!self.busy, eframe::egui::Button::new("Unblock"))
                            .clicked()
                        {
                            self.send(Command::Action(Action::Unblock(person.id)));
                        }
                    });
                }
            });
        }
        if !self.note.is_empty() {
            ui.add_space(8.);
            ui.label(&self.note);
        }
        if self.busy {
            ui.spinner();
        }
    }
}
#[derive(Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}
struct Pending {
    secret: String,
    expires: Instant,
    next: Instant,
    interval: Duration,
}
pub(crate) struct Api {
    client: Client,
    token: Option<String>,
    base: String,
}
impl Api {
    async fn call(
        &self,
        method: Method,
        path: &str,
        value: Option<Value>,
    ) -> Result<Value, (StatusCode, String)> {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(value) = value {
            request = request.json(&value);
        }
        let result = request.send().await.map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Could not reach Secpar. Check your connection and try again.".into(),
            )
        })?;
        let status = result.status();
        // Responses are bounded, including servers that omit Content-Length.
        let mut result = result;
        let mut bytes = Vec::new();
        while let Some(chunk) = result
            .chunk()
            .await
            .map_err(|_| (status, "Incomplete account response".into()))?
        {
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err((status, "Account response was too large".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let data: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|_| (status, "Unexpected account response".into()))?
        };
        if !status.is_success() {
            let code = data["error"].as_str().unwrap_or("request_failed");
            // Map known error codes, never display raw HTTP bodies or credential-bearing URLs.
            let message = match code {
                "authorization_pending" => "authorization_pending",
                "slow_down" => "slow_down",
                "session_expired" | "sign_in_required" => "Your session expired. Sign in again.",
                "host_already_linked" => "This host is linked to another account. Unlink it first.",
                "host_challenge_expired_or_used" => "Host linking expired. Please try again.",
                "invalid_host_proof" => {
                    "The host identity could not be verified. Update Secpar and try again."
                }
                "account_unavailable" => "No one with that username can be added.",
                "rate_limited" => "Too many attempts. Please wait and try again.",
                "device_code_expired_or_used" => "Sign-in expired. Please start again.",
                "incoming_request_not_found" => "This friend request is no longer available.",
                "input_not_allowed" => "This friend may not use the keyboard or mouse.",
                "request_not_pending" => "This request was already answered or has expired.",
                "request_not_found" => "This request is no longer available.",
                "friend_access_required" => "You are not friends with this person any more.",
                "access_expired_or_revoked" => "Access was removed or the request ran out.",
                "host_not_sharing" => "This computer is offline or is not sharing with friends.",
                "grant_not_available" => "This request was already used. Ask again.",
                "intended_guest_required" => "This request belongs to someone else.",
                "invalid_permissions" => "Choose what you want to do first.",
                _ => "The account request failed. Please try again.",
            };
            return Err((status, message.into()));
        }
        Ok(data)
    }
    async fn link_host(&self) -> Result<local_host::State, (StatusCode, String)> {
        self.link_host_via(local_host::call).await
    }
    async fn link_host_via<F, Fut>(
        &self,
        mut host: F,
    ) -> Result<local_host::State, (StatusCode, String)>
    where
        F: FnMut(&'static str, Value) -> Fut,
        Fut: std::future::Future<Output = Result<Value, String>>,
    {
        if self.token.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Sign in under Friends first".into(),
            ));
        }
        let state = host("host_account_state", Value::Null)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
        serde_json::from_value::<local_host::State>(state).map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "Update Secpar to use account linking".into(),
            )
        })?;
        let challenge = self
            .call(Method::POST, "/v1/host-enrollment/start", None)
            .await?;
        let proof = host("host_enrollment_proof", json!({"challenge":challenge}))
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
        // No user session crosses the pipe. Only the one-use proof and resulting machine credential do.
        let registration = self
            .call(Method::POST, "/v1/host-enrollment/finish", Some(proof))
            .await?;
        match host("host_account_install", json!({"registration":registration})).await {
            Ok(state) => serde_json::from_value(state)
                .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid host link response".into())),
            Err(_) => {
                // Don't leave a usable cloud credential behind if the local service couldn't save it.
                if let Some(id) = registration["computer"]
                    .as_str()
                    .filter(|id| path_id(id).is_ok())
                {
                    let _ = self
                        .call(Method::DELETE, &format!("/v1/computers/{id}/link"), None)
                        .await;
                }
                Err((
                    StatusCode::BAD_GATEWAY,
                    "The host could not save its account link. Refresh and try linking again."
                        .into(),
                ))
            }
        }
    }
    async fn ensure_host_via<F, Fut>(&self, owner: &str, enroll: bool, mut host: F) -> Result<local_host::State, (StatusCode,String)>
    where F: FnMut(&'static str,Value) -> Fut, Fut: std::future::Future<Output=Result<Value,String>> {
        let state: local_host::State = serde_json::from_value(host("host_account_state",Value::Null).await.map_err(|note| (StatusCode::BAD_GATEWAY,note))?).map_err(|_| (StatusCode::BAD_GATEWAY,"Invalid local host state".into()))?;
        if state.owner.as_deref().is_some_and(|current| current != owner) {
            return Err((StatusCode::CONFLICT,"This host is linked to another account. Unlink it first.".into()));
        }
        if state.computer.is_some() || !enroll { return Ok(state); }
        self.link_host_via(host).await
    }
    async fn computers(&self) -> Result<Vec<OwnedComputer>, (StatusCode,String)> {
        let computers: Vec<OwnedComputer> = serde_json::from_value(self.call(Method::GET,"/v1/computers",None).await?).map_err(|_| (StatusCode::BAD_GATEWAY,"Invalid computer list".into()))?;
        if computers.len() > 256 || computers.iter().any(|c| path_id(&c.id).is_err() || c.name.len()>320 || c.platform.len()>16 || c.linked>1 || c.online>1 || c.certificate_fp.as_ref().is_some_and(|fp| secpar_client::pairing::fingerprint(fp).is_err())) {
            return Err((StatusCode::BAD_GATEWAY,"Invalid computer list".into()));
        }
        Ok(computers)
    }
    async fn friend_computers(&self) -> Result<Vec<FriendComputer>, (StatusCode, String)> {
        let bad = || (StatusCode::BAD_GATEWAY, "Invalid list of friends' computers".to_string());
        let list: Vec<FriendComputer> = serde_json::from_value(self.call(Method::GET, "/v1/friend-computers", None).await?).map_err(|_| bad())?;
        if list.len() > 256 || list.iter().any(|c| path_id(&c.id).is_err() || c.name.len() > 320 || c.platform.len() > 16 || !valid_handle(&c.owner_handle)) {
            return Err(bad());
        }
        Ok(list)
    }
    /// Ask a friend for access. Answers the request id, its state and the permissions the service kept.
    async fn request_access(&self, computer: &str, permissions: &[String]) -> Result<(String, String, u64, Vec<String>), (StatusCode, String)> {
        let computer = path_id(computer).map_err(|note| (StatusCode::BAD_REQUEST, note))?;
        if permissions.is_empty() || permissions.iter().any(|p| !PERMISSIONS.contains(&p.as_str())) {
            return Err((StatusCode::BAD_REQUEST, "Choose what you want to do first.".into()));
        }
        let value = self.call(Method::POST, "/v1/requests", Some(json!({"computer": computer, "permissions": permissions}))).await?;
        let bad = || (StatusCode::BAD_GATEWAY, "Invalid request response".to_string());
        let id = value["id"].as_str().filter(|id| path_id(id).is_ok()).ok_or_else(bad)?.to_string();
        let state = value["state"].as_str().ok_or_else(bad)?.to_string();
        let kept = permission_list(&value["permissions"]).unwrap_or_else(|| permissions.to_vec());
        Ok((id, state, value["expires_in"].as_u64().unwrap_or(300), kept))
    }
    /// The state of a request and the permissions the owner approved, once there are any.
    async fn request_state(&self, id: &str) -> Result<(String, Option<Vec<String>>), (StatusCode, String)> {
        let id = path_id(id).map_err(|note| (StatusCode::BAD_REQUEST, note))?;
        let row = self.call(Method::GET, &format!("/v1/requests/{id}"), None).await?;
        let state = row["state"].as_str().ok_or((StatusCode::BAD_GATEWAY, "Invalid request response".to_string()))?.to_string();
        Ok((state, permission_list(&row["approved"])))
    }
    async fn claim(&self, id: &str, computer: &FriendComputer, permissions: Vec<String>) -> Result<Grant, (StatusCode, String)> {
        let id = path_id(id).map_err(|note| (StatusCode::BAD_REQUEST, note))?;
        let value = self.call(Method::POST, &format!("/v1/requests/{id}/claim"), None).await?;
        let bad = || (StatusCode::BAD_GATEWAY, "Invalid connection grant".to_string());
        let grant = value["grant"].as_str().filter(|g| !g.is_empty() && g.len() <= 4096).ok_or_else(bad)?;
        let pc = value["host"]["pc"].as_str().ok_or_else(bad)?;
        let relay = value["host"]["relay"].as_str().ok_or_else(bad)?;
        // The relay and computer id are checked here, so a bad answer never reaches the session.
        secpar_client::control::guest_url(relay, pc).map_err(|note| (StatusCode::BAD_GATEWAY, note))?;
        Ok(Grant { relay: relay.into(), pc: pc.into(), grant: grant.into(), name: computer.name.clone(), owner_handle: computer.owner_handle.clone(), permissions })
    }
    async fn live(&self) -> Result<(Vec<Friend>, Vec<Incoming>), (StatusCode, String)> {
        let friends = serde_json::from_value(self.call(Method::GET, "/v1/friends", None).await?)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid friend list".into()))?;
        let requests = serde_json::from_value(self.call(Method::GET, "/v1/requests", None).await?)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid request list".into()))?;
        Ok((friends, requests))
    }
    async fn search(&self, query: &str) -> Result<Vec<Person>, (StatusCode, String)> {
        let name = query.trim().trim_start_matches('@').to_ascii_lowercase();
        if !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') || name.len() > 30 {
            return Ok(vec![]);
        }
        let path = if name.is_empty() { "/v1/directory".to_string() } else { format!("/v1/directory?q={name}") };
        serde_json::from_value(self.call(Method::GET, &path, None).await?)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid search response".into()))
    }
    /// Answer a friend request or a connection request. Ids come from the account service or a signed link.
    async fn answer(&self, answer: Answer, id: &str) -> Result<(), (StatusCode, String)> {
        let id = path_id(id).map_err(|note| (StatusCode::BAD_REQUEST, note))?;
        match answer {
            Answer::AcceptFriend => { self.call(Method::POST, &format!("/v1/friends/{id}/accept"), None).await?; }
            Answer::DeclineFriend => { self.call(Method::DELETE, &format!("/v1/friends/{id}"), None).await?; }
            Answer::DenyConnection => { self.call(Method::POST, &format!("/v1/requests/{id}/revoke"), None).await?; }
            Answer::AllowConnection => {
                let row = self.call(Method::GET, &format!("/v1/requests/{id}"), None).await?;
                let permissions: Vec<String> = row["requested"].as_str().and_then(|r| serde_json::from_str(r).ok())
                    .ok_or((StatusCode::BAD_GATEWAY, "Invalid request response".to_string()))?;
                self.call(Method::POST, &format!("/v1/requests/{id}/approve"), Some(json!({"permissions": permissions}))).await?;
            }
        }
        Ok(())
    }
    async fn snapshot(&self) -> Result<(User, Vec<Friend>, Vec<User>), (StatusCode, String)> {
        let user = serde_json::from_value(self.call(Method::GET, "/v1/me", None).await?)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid account profile".into()))?;
        let friends = serde_json::from_value(self.call(Method::GET, "/v1/friends", None).await?)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid friend list".into()))?;
        let blocked = serde_json::from_value(self.call(Method::GET, "/v1/blocks", None).await?)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "Invalid blocked list".into()))?;
        Ok((user, friends, blocked))
    }
}
fn path_id(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.len() != 36
        || !value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err(USERNAME_ERROR.into());
    }
    Ok(value)
}
pub const USERNAME_ERROR: &str = "Enter a username of 3 to 30 letters, numbers or underscores.";
/// The line that tells people which name to give their friends.
pub fn you_are(handle: &str) -> String { format!("You are @{handle}") }
/// A username as the server expects it: trimmed, one leading @ removed, lower case. Only letters,
/// digits and underscore survive, so the result is safe to place in a URL path.
fn clean_username(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    let name = name.strip_prefix('@').unwrap_or(name).to_ascii_lowercase();
    if (3..=30).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
        Ok(name)
    } else {
        Err(USERNAME_ERROR.into())
    }
}
/// The verb and path for one friend or block change. Invite takes a username, the rest take account ids.
fn action_request(action: &Action) -> Result<(Method, String), String> {
    Ok(match action {
        Action::Invite(name) => (Method::PUT, format!("/v1/friends/by-username/{}", clean_username(name)?)),
        Action::Accept(id) => (Method::POST, format!("/v1/friends/{}/accept", path_id(id)?)),
        Action::Remove(id) => (Method::DELETE, format!("/v1/friends/{}", path_id(id)?)),
        Action::Block(id) => (Method::PUT, format!("/v1/blocks/{}", path_id(id)?)),
        Action::Unblock(id) => (Method::DELETE, format!("/v1/blocks/{}", path_id(id)?)),
        Action::Setting { id, .. } | Action::Input { id, .. } => (Method::PUT, format!("/v1/friends/{}/settings", path_id(id)?)),
    })
}
fn action_body(action: &Action) -> Option<Value> {
    match action {
        Action::Setting { input, connect, .. } => Some(json!({"input": input, "connect": connect})),
        Action::Input { keyboard, mouse, .. } => Some(json!({"keyboard": keyboard, "mouse": mouse})),
        _ => None,
    }
}
fn owned_key(owner: &str, id: &str) -> Result<String,String> {
    path_id(owner)?; path_id(id)?;
    Ok(format!("owned-pairing:{owner}:{id}"))
}
struct PendingGuest {
    id: String,
    computer: FriendComputer,
    permissions: Vec<String>,
    expires: Instant,
    next: Instant,
    approved: bool,
}
/// The credential saved for an account computer by an earlier connection.
fn saved_owned(owner: &str, id: &str) -> Option<Computer> {
    let key = owned_key(owner, id).ok()?;
    serde_json::from_str(&vault::load(&key).ok().flatten()?).ok()
}
/// Opens an account computer: reuses a saved credential that still matches the identity the account
/// has on record, or asks the computer for a new one through the account.
async fn connect_owned(api: &Api, owner: &str, id: &str, saved: &impl Fn(&str, &str) -> Option<Computer>, pending_pair: &mut Option<PendingPair>, events: &mpsc::Sender<Event>) -> Result<String, (StatusCode, String)> {
    let computers = api.computers().await?;
    let Some(computer) = computers.into_iter().find(|c| c.id == id && c.linked == 1) else {
        return Err((StatusCode::NOT_FOUND, "This computer is no longer linked to your account".into()));
    };
    let Some(fingerprint) = computer.certificate_fp.as_deref().and_then(|fp| secpar_client::pairing::fingerprint(fp).ok()) else {
        return Err((StatusCode::BAD_GATEWAY, "This computer has no verified host identity".into()));
    };
    if let Some(credential) = saved(owner, id).filter(|saved| saved.fingerprint == fingerprint) {
        *pending_pair = None;
        let _ = events.send(Event::OwnedConnection(OwnedConnection::Cached { owner: owner.into(), id: id.into(), computer: credential })).await;
        return Ok(String::new());
    }
    let value = api.call(Method::POST, "/v1/owner-pairings", Some(json!({"computer": id, "device": crate::device_name()}))).await?;
    let Some(request) = value["id"].as_str().filter(|id| path_id(id).is_ok()) else {
        return Err((StatusCode::BAD_GATEWAY, "Invalid pairing response".into()));
    };
    *pending_pair = Some(PendingPair { id: request.into(), owner: owner.into(), computer: id.into(), fingerprint, expires: Instant::now() + Duration::from_secs(120), next: Instant::now() + Duration::from_secs(2) });
    let _ = events.send(Event::Message(pairing_with(&computer.name))).await;
    Ok(String::new())
}
#[cfg(test)]
impl Api {
    /// A client for a scripted account worker on a local port.
    pub(crate) fn for_test(base: String) -> Self {
        Api { client: Client::new(), token: Some("d".repeat(64)), base }
    }
}
/// `connect_owned` with the saved credential given directly, for tests.
#[cfg(test)]
pub(crate) async fn connect_owned_for_test(api: &Api, owner: &str, id: &str, saved: Option<Computer>, pending_pair: &mut Option<PendingPair>, events: &mpsc::Sender<Event>) -> Result<String, (StatusCode, String)> {
    connect_owned(api, owner, id, &|_: &str, _: &str| saved.clone(), pending_pair, events).await
}
pub(crate) struct PendingPair {
    id: String, owner: String, computer: String, fingerprint: [u8;32],
    expires: Instant, next: Instant,
}
async fn synchronize_owned(api: &Api, owner: &str, enroll: bool, events: &mpsc::Sender<Event>) -> bool {
    let _ = events.send(Event::Host(api.ensure_host_via(owner,enroll,local_host::call).await.map_err(|(_,note)|note))).await;
    match api.computers().await {
        Ok(computers) => { let _ = events.send(Event::Owned(computers)).await; }
        Err((StatusCode::UNAUTHORIZED,note)) => { let _=events.send(Event::SignedOut(note)).await; return false; }
        Err((_,note)) => { let _ = events.send(Event::Message(note)).await; }
    }
    true
}
#[derive(Default)]
struct Seen { friends: std::collections::HashSet<String>, requests: std::collections::HashSet<String>, primed: bool }
/// Alerts for requests this account has not been told about yet. Friend requests that were already
/// waiting when the app started are not announced again, because they carry no time.
fn new_alerts(me: &str, friends: &[Friend], requests: &[Incoming], seen: &mut Seen) -> Vec<crate::alerts::Alert> {
    let mut out = vec![];
    for f in friends.iter().filter(|f| f.state == "pending" && f.requester != me) {
        if seen.friends.insert(f.id.clone()) && seen.primed {
            out.push(crate::alerts::Alert {
                title: "Friend request".into(),
                body: format!("@{} wants to be your friend", f.handle),
                id: f.id.clone(),
                yes: Answer::AcceptFriend,
                no: Answer::DeclineFriend,
            });
        }
    }
    seen.primed = true;
    for r in requests.iter().filter(|r| r.owner == me && r.state == "pending") {
        if seen.requests.insert(r.id.clone()) {
            out.push(crate::alerts::Alert {
                title: "Connection request".into(),
                body: format!("@{} wants to use {}", r.guest_handle, r.computer_name.as_deref().unwrap_or("your computer")),
                id: r.id.clone(),
                yes: Answer::AllowConnection,
                no: Answer::DenyConnection,
            });
        }
    }
    out
}
/// Runs when a toast button starts this program with a signed link. It answers and exits.
pub fn answer_link(uri: &str) -> Result<String, String> {
    let key = crate::alerts::key().ok_or("Secure storage is unavailable")?;
    let (answer, id) = crate::alerts::verify(&key, uri).ok_or("This link is not valid")?;
    let token = vault::load(CREDENTIAL)?.ok_or("Sign in to Secpar first")?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "Could not start networking")?;
    runtime.block_on(async {
        let client = Client::builder().https_only(true).redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(15)).build()
            .map_err(|_| "Could not start secure account networking".to_string())?;
        let api = Api { client, token: Some(token), base: ORIGIN.into() };
        api.answer(answer, &id).await.map_err(|(_, note)| note)?;
        Ok(match answer { Answer::AllowConnection => "Allowed", Answer::DenyConnection => "Denied", Answer::AcceptFriend => "Friend added", Answer::DeclineFriend => "Declined" }.into())
    })
}
pub fn start(ctx: eframe::egui::Context) -> (mpsc::Sender<Command>, mpsc::Receiver<Event>) {
    let (tx, mut rx) = mpsc::channel(16);
    let tx_alerts = tx.clone();
    let (events, output) = mpsc::channel(32);
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(_) => return,
        };
        runtime.block_on(async move {
            // Keyring I/O runs on this dedicated worker, never the render thread.
            if let Ok(Some(saved)) = vault::load("paired-computer") {
                if let Ok(computer) = serde_json::from_str::<Computer>(&saved) {
                    let _ = events.send(Event::Restored(computer)).await;
                    ctx.request_repaint();
                }
            }
            let token=match vault::load(CREDENTIAL) { Ok(t)=>t,Err(note)=>{let _=events.send(Event::Message(note)).await;None} };
            let client=match Client::builder().https_only(true).redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(15)).connect_timeout(Duration::from_secs(8)).build() { Ok(c)=>c,Err(_)=>{let _=events.send(Event::SignedOut("Could not start secure account networking".into())).await;ctx.request_repaint();return} };
            let _=events.send(Event::Host(local_host::state().await)).await;
            ctx.request_repaint();
            let mut api=Api{client,token,base:ORIGIN.into()}; let mut pending: Option<Pending>=None;
            let mut pending_pair: Option<PendingPair> = None;
            let mut pending_guest: Option<PendingGuest> = None;
            let mut next_friend_computers = Instant::now();
            let mut signed_owner: Option<String> = None;
            let mut suppressed_owner = vault::load("host-auto-link-disabled").ok().flatten();
            let mut next_owned = Instant::now();
            let mut next_alerts = Instant::now();
            let mut seen = Seen::default();
            if api.token.is_some() {
                match api.snapshot().await {
                    Ok((user,friends,blocked))=>{signed_owner=Some(user.id.clone());next_owned=Instant::now();let _=events.send(Event::SignedIn{user,friends,blocked,note:String::new()}).await;}
                    Err((status,note))=>{
                        if status==StatusCode::UNAUTHORIZED {api.token=None;let _=vault::forget(CREDENTIAL);}
                        let _=events.send(Event::SignedOut(note)).await;
                    }
                }
                ctx.request_repaint();
            }
            let mut timer=tokio::time::interval(Duration::from_secs(1));
            loop {
                let command=tokio::select! { command=rx.recv()=>{let Some(command)=command else {break};Some(command)}, _=timer.tick()=>None };
                if let Some(Command::Search(query))=&command {
                    if api.token.is_some() { let _=events.send(Event::Directory(api.search(query).await.map_err(|(_,note)|note))).await; ctx.request_repaint(); }
                    continue;
                }
                if matches!(command,Some(Command::RequestGuest{..})|Some(Command::CancelGuest)) {
                    pending_guest=None;
                    if let Some(Command::RequestGuest{computer,permissions})=command {
                        let refused=match api.request_access(&computer.id,&permissions).await {
                            Ok((id,state,expires_in,kept))=>{
                                let mut p=PendingGuest{id,computer,permissions:kept,expires:Instant::now()+Duration::from_secs(expires_in.clamp(1,600)),next:Instant::now()+Duration::from_secs(2),approved:false};
                                if state=="approved" {
                                    p.approved=true;
                                    match api.claim(&p.id,&p.computer,p.permissions.clone()).await {
                                        Ok(grant)=>{let _=events.send(Event::Guest(Guest::Grant(grant))).await;None},
                                        Err(e)=>Some(e),
                                    }
                                } else {
                                    let _=events.send(Event::Guest(Guest::Waiting(p.computer.owner_handle.clone()))).await;
                                    pending_guest=Some(p);
                                    None
                                }
                            }
                            Err(e)=>Some(e),
                        };
                        if let Some((status,note))=refused {
                            if status==StatusCode::UNAUTHORIZED {api.token=None;signed_owner=None;let _=vault::forget(CREDENTIAL);let _=events.send(Event::SignedOut(note)).await;}
                            else {let _=events.send(Event::Guest(Guest::Refused(note))).await;}
                        }
                    }
                    ctx.request_repaint();
                    continue;
                }
                if let Some(command)=command {
                    let _=events.send(Event::Busy(true)).await;
                    let skip_refresh=matches!(&command,Command::Remember(_)|Command::ForgetComputer|Command::HostRefresh|Command::LinkHost|Command::UnlinkHost|Command::ConnectOwned(_)|Command::RememberOwned{..});
                    let result: Result<String,(StatusCode,String)>= match command {
                        Command::HostRefresh=>{let _=events.send(Event::Host(local_host::state().await)).await;let _=events.send(Event::Message(String::new())).await;Ok(String::new())},
                        Command::LinkHost=>match api.link_host().await {
                            Ok(state)=>{suppressed_owner=None;let _=vault::forget("host-auto-link-disabled");let _=events.send(Event::Host(Ok(state))).await;let _=events.send(Event::Message("Host linked. Friend access remains off.".into())).await;Ok(String::new())},
                            Err(e)=>Err(e)
                        },
                        Command::UnlinkHost=>{
                            match local_host::call("host_account_unlink",Value::Null).await {
                                Ok(value)=>{suppressed_owner=signed_owner.clone();if let Some(owner)=&suppressed_owner {let _=vault::save("host-auto-link-disabled",owner);}let state=serde_json::from_value(value).map_err(|_|"Invalid host response".to_string());let _=events.send(Event::Host(state)).await;let _=events.send(Event::Message("Host unlinked".into())).await;Ok(String::new())},
                                Err(e)=>Err((StatusCode::BAD_GATEWAY,e))
                            }
                        },
                        Command::SignIn=>{
                            pending=None;
                            api.token=None;
                            let _=vault::forget(CREDENTIAL);
                            match api.call(Method::POST,"/auth/device",Some(json!({"name":"Secpar Desktop"}))).await {
                                Ok(data)=>match serde_json::from_value::<Device>(data) {
                                    Ok(device)=>{
                                        // The approval destination is pinned, including path. Never open a server-supplied arbitrary URL.
                                        let expected=format!("{ORIGIN}/activate?code={}",device.user_code);
                                        if device.verification_uri_complete!=expected || device.user_code.len()!=8 || !device.user_code.bytes().all(|c|c.is_ascii_uppercase()||c.is_ascii_digit()) {
                                            Err((StatusCode::BAD_GATEWAY,"Invalid device approval response".into()))
                                        } else {
                                            let interval=Duration::from_secs(device.interval.clamp(5,60));
                                            pending=Some(Pending{secret:device.device_code,expires:Instant::now()+Duration::from_secs(device.expires_in.min(600)),next:Instant::now()+interval,interval});
                                            let _=events.send(Event::Waiting{code:device.user_code,url:expected.clone()}).await;
                                            let _=open::that(&expected);
                                            Ok(String::new())
                                        }
                                    },Err(_)=>Err((StatusCode::BAD_GATEWAY,"Invalid sign-in response".into()))
                                },Err(e)=>Err(e)
                            }
                        }
                        Command::Cancel=>{pending=None;let _=events.send(Event::SignedOut(String::new())).await;Ok(String::new())}
                        Command::SignOut=>{
                            pending=None;
                            pending_pair=None;
                            signed_owner=None;
                            // Revoke remotely first. Offline logout still forgets locally and reports the limit.
                            let revoked=if api.token.is_some(){api.call(Method::POST,"/auth/logout",None).await.is_ok()}else{true};
                            api.token=None;
                            let note=match vault::forget(CREDENTIAL){Err(note)=>note,Ok(()) if !revoked=>"Signed out here. Could not revoke the remote session while offline.".into(),_=>String::new()};
                            let _=events.send(Event::SignedOut(note)).await;Ok(String::new())
                        }
                        Command::Remember(computer)=>{
                            match serde_json::to_string(&computer) {
                                Ok(saved)=>{if let Err(note)=vault::save("paired-computer",&saved){let _=events.send(Event::StorageNote(note)).await;}Ok(String::new())},
                                Err(_)=>Err((StatusCode::INTERNAL_SERVER_ERROR,"Could not remember this computer".into()))
                            }
                        },
                        Command::ForgetComputer=>{match vault::forget("paired-computer"){Ok(())=>{let _=events.send(Event::ForgotComputer).await;},Err(note)=>{let _=events.send(Event::StorageNote(note)).await;}}Ok(String::new())},
                        Command::RememberOwned{owner,id,computer}=>{
                            if signed_owner.as_deref()!=Some(&owner) {Ok(String::new())} else {
                                match owned_key(&owner,&id).and_then(|key| serde_json::to_string(&computer).map_err(|_|"Could not remember this computer".into()).and_then(|value| vault::save(&key,&value))) {
                                    Ok(())=>Ok(String::new()),Err(note)=>{let _=events.send(Event::StorageNote(note)).await;Ok(String::new())}
                                }
                            }
                        }
                        Command::ForgetOwned{owner,id}=>{
                            if signed_owner.as_deref()==Some(&owner) { if let Ok(key)=owned_key(&owner,&id) { let _=vault::forget(&key); } }
                            Ok(String::new())
                        }
                        Command::ConnectOwned(id)=>{
                            if let Some(old)=pending_pair.take() { let _=api.call(Method::DELETE,&format!("/v1/owner-pairings/{}",old.id),None).await; }
                            match signed_owner.clone().filter(|_| api.token.is_some()) {
                                Some(owner)=>connect_owned(&api,&owner,&id,&saved_owned,&mut pending_pair,&events).await,
                                None=>Err((StatusCode::UNAUTHORIZED,"Sign in to connect to your computers".into())),
                            }
                        }
                        Command::Refresh=>{next_friend_computers=Instant::now();Ok(String::new())},
                        Command::RequestGuest{..}|Command::CancelGuest=>Ok(String::new()),
                        Command::Search(_)=>Ok(String::new()),
                        Command::SetVisible(visible)=>api.call(Method::PUT,"/v1/me/visibility",Some(json!({"visible":visible}))).await.map(|_| String::new()),
                        Command::Answer(answer,id)=>api.answer(answer,&id).await.map(|_| match answer {
                            Answer::AllowConnection=>"Request allowed".into(),Answer::DenyConnection=>"Request denied".into(),
                            Answer::AcceptFriend=>"Friend added".into(),Answer::DeclineFriend=>String::new(),
                        }),
                        Command::Action(action)=>{
                            match action_request(&action) {
                                Err(note)=>Err((StatusCode::BAD_REQUEST,note)),
                                Ok((method,path))=>api.call(method,&path,action_body(&action)).await.map(|_| if matches!(action,Action::Invite(_)){"Friend request sent".into()}else{String::new()})
                            }
                        }
                    };
                    match result {
                        Err((status,note))=>{
                            if status==StatusCode::UNAUTHORIZED {api.token=None;let _=vault::forget(CREDENTIAL);let _=events.send(Event::SignedOut(note)).await;}
                            else {let _=events.send(Event::Message(note)).await;}
                        }
                        Ok(note) if api.token.is_some()&&!skip_refresh=>match api.snapshot().await {
                            Ok((user,friends,blocked))=>{signed_owner=Some(user.id.clone());next_owned=Instant::now();let _=events.send(Event::SignedIn{user,friends,blocked,note}).await;}
                            Err((status,note))=>{if status==StatusCode::UNAUTHORIZED{api.token=None;let _=vault::forget(CREDENTIAL);let _=events.send(Event::SignedOut(note)).await;}else{let _=events.send(Event::Message(note)).await;}}
                        },_=>{}
                    }
                    let _=events.send(Event::Busy(false)).await;ctx.request_repaint();
                }
                if pending.as_ref().is_some_and(|p| Instant::now()>=p.next) {
                    let p=pending.as_mut().unwrap();
                    if Instant::now()>=p.expires {pending=None;let _=events.send(Event::SignedOut("Sign-in expired. Try again.".into())).await;ctx.request_repaint();continue;}
                    p.next=Instant::now()+p.interval;
                    match api.call(Method::POST,"/auth/device/token",Some(json!({"device_code":p.secret}))).await {
                        Ok(data)=>{
                            let token=data["access_token"].as_str().unwrap_or("");
                            if token.len()!=64 || !token.bytes().all(|b|b.is_ascii_hexdigit()) {pending=None;let _=events.send(Event::SignedOut("Invalid account session".into())).await;}
                            else {
                                let note=vault::save(CREDENTIAL,token).err().unwrap_or_default();api.token=Some(token.into());pending=None;
                                match api.snapshot().await {Ok((user,friends,blocked))=>{signed_owner=Some(user.id.clone());next_owned=Instant::now();let _=events.send(Event::SignedIn{user,friends,blocked,note}).await;},Err((status,note))=>{if status==StatusCode::UNAUTHORIZED{api.token=None;let _=vault::forget(CREDENTIAL);}let _=events.send(Event::SignedOut(note)).await;}}
                            }
                        }
                        Err((_,code)) if code=="authorization_pending"=>{},
                        Err((_,code)) if code=="slow_down"=>{p.interval+=Duration::from_secs(5);p.next=Instant::now()+p.interval;},
                        Err((status,note)) if status.is_server_error()=>{let _=events.send(Event::Message(note)).await;},
                        Err((_,note))=>{pending=None;let _=events.send(Event::SignedOut(note)).await;}
                    }
                    ctx.request_repaint();
                }
                if api.token.is_none() { pending_pair=None; pending_guest=None; signed_owner=None; next_friend_computers=Instant::now(); }
                if signed_owner.is_some() && api.token.is_some() && Instant::now()>=next_friend_computers {
                    next_friend_computers=Instant::now()+Duration::from_secs(20);
                    match api.friend_computers().await {
                        Ok(list)=>{let _=events.send(Event::FriendComputers(list)).await;}
                        Err((StatusCode::UNAUTHORIZED,note))=>{api.token=None;signed_owner=None;pending_pair=None;pending_guest=None;let _=vault::forget(CREDENTIAL);let _=events.send(Event::SignedOut(note)).await;}
                        Err(_)=>{}
                    }
                    ctx.request_repaint();
                }
                if pending_guest.as_ref().is_some_and(|p|Instant::now()>=p.next) {
                    let mut p=pending_guest.take().unwrap();
                    p.next=Instant::now()+Duration::from_secs(2);
                    let owner=p.computer.owner_handle.clone();
                    if Instant::now()>=p.expires {
                        let _=events.send(Event::Guest(Guest::Refused(format!("@{owner} did not answer.")))).await;
                    } else {
                        match api.request_state(&p.id).await {
                            Ok((state,_)) if state=="pending"=>{pending_guest=Some(p);},
                            Ok((state,approved)) if state=="approved"=>{
                                p.approved=true;
                                if let Some(list)=approved {p.permissions=list;}
                                match api.claim(&p.id,&p.computer,p.permissions.clone()).await {
                                    Ok(grant)=>{let _=events.send(Event::Guest(Guest::Grant(grant))).await;},
                                    Err((status,_)) if status.is_server_error() && status!=StatusCode::BAD_GATEWAY =>{pending_guest=Some(p);},
                                    Err((_,note))=>{let _=events.send(Event::Guest(Guest::Refused(note))).await;},
                                }
                            }
                            Ok((state,_))=>{let _=events.send(Event::Guest(Guest::Refused(guest_refusal(&state,&owner,p.approved)))).await;},
                            Err((status,_)) if status.is_server_error()=>{pending_guest=Some(p);},
                            Err((StatusCode::UNAUTHORIZED,note))=>{api.token=None;signed_owner=None;pending_pair=None;let _=vault::forget(CREDENTIAL);let _=events.send(Event::SignedOut(note)).await;},
                            Err((_,note))=>{let _=events.send(Event::Guest(Guest::Refused(note))).await;},
                        }
                    }
                    ctx.request_repaint();
                }
                if let Some(owner)=signed_owner.as_deref().filter(|_| api.token.is_some() && Instant::now()>=next_owned) {
                    next_owned=Instant::now()+Duration::from_secs(30);
                    if !synchronize_owned(&api,owner,suppressed_owner.as_deref()!=Some(owner),&events).await { api.token=None; signed_owner=None; pending_pair=None;let _=vault::forget(CREDENTIAL); }
                    ctx.request_repaint();
                }
                if let Some(me)=signed_owner.clone().filter(|_| api.token.is_some() && Instant::now()>=next_alerts) {
                    next_alerts=Instant::now()+Duration::from_secs(10);
                    match api.live().await {
                        Ok((friends,requests))=>{
                            for alert in new_alerts(&me,&friends,&requests,&mut seen) { crate::alerts::show(alert,tx_alerts.clone()); }
                            let _=events.send(Event::Live{friends,requests}).await;
                        }
                        Err((StatusCode::UNAUTHORIZED,note))=>{api.token=None;signed_owner=None;pending_pair=None;let _=vault::forget(CREDENTIAL);let _=events.send(Event::SignedOut(note)).await;}
                        Err(_)=>{}
                    }
                    ctx.request_repaint();
                }
                if api.token.is_none() { seen=Seen::default(); }
                if pending_pair.as_ref().is_some_and(|p|Instant::now()>=p.next) {
                    let p=pending_pair.as_mut().unwrap();
                    if Instant::now()>=p.expires {
                        pending_pair=None;let _=events.send(Event::Message("Computer did not respond. Check that Secpar is open there.".into())).await;
                    } else {
                        p.next=Instant::now()+Duration::from_secs(2);
                        match api.call(Method::GET,&format!("/v1/owner-pairings/{}",p.id),None).await {
                            Ok(value) if value["state"]=="pending"=>{},
                            Ok(value)=>{
                                let result=value["link"].as_str().and_then(|link|Pairing::parse(link).ok()).filter(|pairing|pairing.fingerprint==p.fingerprint && value["certificate_fp"].as_str().and_then(|fp|secpar_client::pairing::fingerprint(fp).ok())==Some(p.fingerprint));
                                if let Some(pairing)=result {
                                    let _=events.send(Event::OwnedConnection(OwnedConnection::Pair{owner:p.owner.clone(),id:p.computer.clone(),pairing})).await;
                                }else{let _=events.send(Event::Message("Computer identity did not match its account registration".into())).await;}
                                pending_pair=None;
                            }
                            Err((status,_)) if status.is_server_error()=>{},
                            Err((_,note))=>{pending_pair=None;let _=events.send(Event::Message(note)).await;}
                        }
                    }
                    ctx.request_repaint();
                }
            }
        });
    });
    (tx, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn automatic_enrollment_reuses_a_current_link_and_cannot_transfer_other_accounts() {
        let api=Api{base:ORIGIN.into(),client:Client::new(),token:Some("d".repeat(64))};
        let state=json!({"computer":"00000000-0000-4000-8000-000000000010","owner":"alice","online":true});
        let result=api.ensure_host_via("alice",true,|command,_| {
            assert_eq!(command,"host_account_state"); std::future::ready(Ok(state.clone()))
        }).await.unwrap();
        assert!(result.online);
        assert!(api.ensure_host_via("bob",true,|command,_| {
            assert_eq!(command,"host_account_state"); std::future::ready(Ok(state.clone()))
        }).await.is_err());
        let unlinked=api.ensure_host_via("alice",false,|command,_| {
            assert_eq!(command,"host_account_state"); std::future::ready(Ok(json!({"online":false})))
        }).await.unwrap();
        assert!(unlinked.owner.is_none());
    }
    #[test]
    fn saved_owner_access_is_namespaced_by_both_account_and_computer() {
        let alice="00000000-0000-4000-8000-000000000001";
        let bob="00000000-0000-4000-8000-000000000002";
        let pc="00000000-0000-4000-8000-000000000010";
        assert_ne!(owned_key(alice,pc).unwrap(),owned_key(bob,pc).unwrap());
        assert_ne!(owned_key(alice,pc).unwrap(),owned_key(alice,bob).unwrap());
        assert!(owned_key("../account",pc).is_err());
        assert!(owned_key(alice,"../../paired-computer").is_err());
    }
    #[test]
    fn account_ids_cannot_inject_account_paths() {
        assert!(path_id("d0d12c4d-13f1-4fbc-b113-a89d901ac34b").is_ok());
        for code in ["../me", "abcd/../../blocks", "", "a".repeat(36).as_str()] {
            assert!(path_id(code).is_err());
        }
    }
    #[test]
    fn usernames_are_cleaned_before_they_reach_a_path() {
        assert_eq!(clean_username("  @River_2\n").unwrap(), "river_2");
        assert_eq!(clean_username("RIVER").unwrap(), "river");
        assert_eq!(clean_username(&"A".repeat(30)).unwrap(), "a".repeat(30));
        for bad in ["", "  ", "@", "@@river", "ab", "ri ver", "ri-ver", "../me", "river/accept", "river?x=1",
            "river#x", "%2e%2e", "river%2f", "r\u{131}ver", "\u{ff52}iver", "river\u{0}", &"a".repeat(31),
            "d0d12c4d-13f1-4fbc-b113-a89d901ac34b"] {
            assert!(clean_username(bad).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn friend_actions_build_fixed_paths() {
        let id = "d0d12c4d-13f1-4fbc-b113-a89d901ac34b";
        let line = |a: Action| action_request(&a).map(|(m, p)| format!("{m} {p}"));
        assert_eq!(line(Action::Invite(" @Bob_Smith ".into())).unwrap(), "PUT /v1/friends/by-username/bob_smith");
        assert_eq!(line(Action::Accept(id.into())).unwrap(), format!("POST /v1/friends/{id}/accept"));
        assert_eq!(line(Action::Remove(id.into())).unwrap(), format!("DELETE /v1/friends/{id}"));
        assert_eq!(line(Action::Block(id.into())).unwrap(), format!("PUT /v1/blocks/{id}"));
        assert_eq!(line(Action::Unblock(id.into())).unwrap(), format!("DELETE /v1/blocks/{id}"));
        // A username is not an account id and an id is not a username.
        assert!(line(Action::Accept("river".into())).is_err());
        assert!(line(Action::Block("river".into())).is_err());
        assert!(line(Action::Invite(id.into())).is_err());
        assert!(line(Action::Invite("../me".into())).is_err());
        assert!(line(Action::Remove("../me".into())).is_err());
    }
    #[test]
    fn friend_settings_use_a_fixed_path_and_body() {
        let id = "d0d12c4d-13f1-4fbc-b113-a89d901ac34b";
        let action = Action::Setting { id: id.into(), input: Some("full"), connect: None };
        let (method, path) = action_request(&action).unwrap();
        assert_eq!((method, path.as_str()), (Method::PUT, format!("/v1/friends/{id}/settings").as_str()));
        assert_eq!(action_body(&action), Some(json!({"input": "full", "connect": null})));
        assert!(action_request(&Action::Setting { id: "../me".into(), input: None, connect: Some("ask") }).is_err());
        assert_eq!(action_body(&Action::Block(id.into())), None);
    }
    #[test]
    fn what_a_friend_may_do_is_shown_in_short_words() {
        let mut f = friend("a", "accepted", "a");
        assert_eq!(friend_summary(&f), "Ask first · View only");
        f.input = "full".into();
        f.connect = "always".into();
        assert_eq!(friend_summary(&f), "Any time · Keyboard and mouse");
        f.keyboard = Some(true);
        f.mouse = Some(false);
        assert_eq!(friend_summary(&f), "Any time · Keyboard only");
        assert_eq!(action_body(&Action::Input { id: "a".into(), keyboard: None, mouse: Some(true) }), Some(json!({"keyboard": null, "mouse": true})));
    }
    #[test]
    fn old_servers_without_settings_read_as_view_only_and_ask() {
        let f: Friend = serde_json::from_str(r#"{"id":"a","handle":"b","state":"accepted","requester":"a"}"#).unwrap();
        assert_eq!((f.input.as_str(), f.connect.as_str()), ("view", "ask"));
        let u: User = serde_json::from_str(r#"{"id":"a","handle":"b"}"#).unwrap();
        assert!(!u.visible);
    }
    fn friend(id: &str, state: &str, requester: &str) -> Friend {
        Friend { id: id.into(), handle: id.into(), state: state.into(), requester: requester.into(), input: "view".into(), connect: "ask".into(), keyboard: None, mouse: None }
    }
    fn incoming(id: &str, owner: &str, state: &str) -> Incoming {
        Incoming { id: id.into(), owner: owner.into(), guest_handle: "bob".into(), computer_name: Some("Studio".into()), requested: "[\"view\"]".into(), state: state.into() }
    }
    #[test]
    fn alerts_fire_once_and_skip_requests_that_were_waiting_at_start() {
        let mut seen = Seen::default();
        let waiting = [friend("old", "pending", "old")];
        assert!(new_alerts("me", &waiting, &[], &mut seen).is_empty());
        let both = [friend("old", "pending", "old"), friend("new", "pending", "new"), friend("mine", "pending", "me"), friend("pal", "accepted", "pal")];
        let got = new_alerts("me", &both, &[incoming("r1", "me", "pending"), incoming("r2", "me", "approved"), incoming("r3", "other", "pending")], &mut seen);
        let titles: Vec<_> = got.iter().map(|a| (a.title.as_str(), a.id.as_str(), a.yes)).collect();
        assert_eq!(titles, vec![("Friend request", "new", Answer::AcceptFriend), ("Connection request", "r1", Answer::AllowConnection)]);
        assert_eq!(got[1].body, "@bob wants to use Studio");
        assert!(new_alerts("me", &both, &[incoming("r1", "me", "pending")], &mut seen).is_empty());
    }
    #[test]
    fn a_connection_request_waiting_at_start_is_announced() {
        let mut seen = Seen::default();
        let got = new_alerts("me", &[], &[incoming("r1", "me", "pending")], &mut seen);
        assert_eq!(got.len(), 1);
    }
    fn api_for(base: String) -> Api {
        Api { base, client: Client::builder().timeout(Duration::from_secs(5)).build().unwrap(), token: Some("d".repeat(64)) }
    }
    #[test]
    fn search_asks_the_directory_with_a_clean_lower_case_name() {
        let (base, server) = server(vec![
            ("GET /v1/directory?q=bob_1 ", r#"[{"handle":"Bob_1","name":"x","cursor":"bob_1","relation":"none"}]"#.into()),
            ("GET /v1/directory ", "[]".into()),
        ]);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let api = api_for(base);
            let found = api.search(" @Bob_1 ").await.unwrap();
            assert_eq!((found[0].handle.as_str(), found[0].relation.as_str()), ("Bob_1", "none"));
            // Text that is not a username never reaches the URL.
            assert!(api.search("a/../b").await.unwrap().is_empty());
            assert!(api.search("a b").await.unwrap().is_empty());
            assert!(api.search(&"x".repeat(31)).await.unwrap().is_empty());
            assert!(api.search("").await.unwrap().is_empty());
        });
        server.join().unwrap();
    }
    #[test]
    fn answers_call_the_request_routes_in_order() {
        let id = "d0d12c4d-13f1-4fbc-b113-a89d901ac34b";
        let (base, server) = server(vec![
            (&*Box::leak(format!("GET /v1/requests/{id} ").into_boxed_str()), format!(r#"{{"id":"{id}","requested":"[\"view\",\"audio\"]"}}"#)),
            (&*Box::leak(format!("POST /v1/requests/{id}/approve ").into_boxed_str()), r#"{"state":"approved"}"#.into()),
            (&*Box::leak(format!("POST /v1/requests/{id}/revoke ").into_boxed_str()), r#"{"state":"revoked"}"#.into()),
            (&*Box::leak(format!("POST /v1/friends/{id}/accept ").into_boxed_str()), r#"{"state":"accepted"}"#.into()),
            (&*Box::leak(format!("DELETE /v1/friends/{id} ").into_boxed_str()), r#"{"removed":true}"#.into()),
        ]);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let api = api_for(base);
            for a in [Answer::AllowConnection, Answer::DenyConnection, Answer::AcceptFriend, Answer::DeclineFriend] {
                api.answer(a, id).await.unwrap();
            }
            assert!(api.answer(Answer::AllowConnection, "../../me").await.is_err());
        });
        server.join().unwrap();
    }
    #[test]
    fn the_five_checkboxes_become_permission_names() {
        assert_eq!(guest_permissions(PERMISSION_DEFAULTS), ["view", "audio", "keyboard", "mouse"]);
        assert_eq!(guest_permissions([true, false, false, true, true]), ["view", "gamepad", "clipboard"]);
        assert!(guest_permissions([false; 5]).is_empty());
        assert!(guest_permissions([true; 5]).iter().all(|p| PERMISSIONS.contains(&p.as_str())));
    }
    #[test]
    fn host_refusals_have_plain_words() {
        assert_eq!(guest_refusal("revoked", "sam", false), "@sam said no.");
        assert_eq!(guest_refusal("rejected", "sam", false), "@sam said no.");
        assert_eq!(guest_refusal("revoked", "sam", true), "Access was removed.");
        assert!(guest_refusal("claimed", "sam", true).contains("already used"));
    }
    #[test]
    fn permission_lists_come_as_text_or_arrays_and_unknown_names_are_dropped() {
        assert_eq!(permission_list(&json!("[\"audio\",\"view\"]")).unwrap(), ["audio", "view"]);
        assert_eq!(permission_list(&json!(["view"])).unwrap(), ["view"]);
        assert!(permission_list(&json!("[\"root\"]")).is_none());
        assert!(permission_list(&Value::Null).is_none());
        assert!(valid_handle("river_2") && !valid_handle("a/b") && !valid_handle("") && !valid_handle(&"x".repeat(31)));
    }
    #[test]
    fn the_friend_flow_calls_the_request_routes_in_order() {
        let id = "d0d12c4d-13f1-4fbc-b113-a89d901ac34b";
        let pc = "0f0e0d0c-1111-2222-3333-444455556666";
        let (base, server) = server(vec![
            ("GET /v1/friend-computers ", format!(r#"[{{"id":"{pc}","name":"Den PC","platform":"windows","owner":"{id}","owner_handle":"sam"}}]"#)),
            ("POST /v1/requests ", format!(r#"{{"id":"{id}","state":"pending","expires_in":300,"permissions":["view","audio"]}}"#)),
            (&*Box::leak(format!("GET /v1/requests/{id} ").into_boxed_str()), r#"{"state":"approved","approved":"[\"view\"]"}"#.into()),
            (&*Box::leak(format!("POST /v1/requests/{id}/claim ").into_boxed_str()), r#"{"grant":"g.r.a","expires_at":1,"host":{"pc":"pc-1","relay":"https://relay.example.com","fingerprint":"aa"}}"#.into()),
            (&*Box::leak(format!("POST /v1/requests/{id}/claim ").into_boxed_str()), r#"{"grant":"g.r.a","expires_at":1,"host":{"pc":"pc-1","relay":"http://evil.example.com","fingerprint":"aa"}}"#.into()),
        ]);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let api = api_for(base);
            let list = api.friend_computers().await.unwrap();
            assert_eq!((list[0].name.as_str(), list[0].owner_handle.as_str()), ("Den PC", "sam"));
            let (request, state, expires_in, kept) = api.request_access(pc, &guest_permissions([true, true, false, false, false])).await.unwrap();
            assert_eq!((request.as_str(), state.as_str(), expires_in, kept.len()), (id, "pending", 300, 2));
            let (state, approved) = api.request_state(&request).await.unwrap();
            assert_eq!((state.as_str(), approved.unwrap()), ("approved", vec!["view".to_string()]));
            let grant = api.claim(&request, &list[0], vec!["view".into()]).await.unwrap();
            assert_eq!((grant.pc.as_str(), grant.relay.as_str(), grant.grant.as_str()), ("pc-1", "https://relay.example.com", "g.r.a"));
            assert!(grant.allows("view") && !grant.allows("audio"));
            // A relay that is not https never becomes a grant.
            assert!(api.claim(&request, &list[0], vec![]).await.is_err());
            // Ids and permissions are checked before anything is sent.
            assert!(api.request_access("../x", &["view".into()]).await.is_err());
            assert!(api.request_access(pc, &[]).await.is_err());
            assert!(api.request_access(pc, &["root".into()]).await.is_err());
            assert!(api.request_state("a/b").await.is_err());
            assert!(api.claim("a/b", &list[0], vec![]).await.is_err());
        });
        server.join().unwrap();
    }
    #[test]
    fn friend_computer_lists_with_odd_rows_are_refused() {
        let pc = "0f0e0d0c-1111-2222-3333-444455556666";
        let (base, server) = server(vec![("GET /v1/friend-computers ", format!(r#"[{{"id":"{pc}","name":"x","platform":"windows","owner":"o","owner_handle":"a/b"}}]"#))]);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            assert!(api_for(base).friend_computers().await.is_err());
        });
        server.join().unwrap();
    }
    #[test]
    fn your_own_name_is_shown_with_an_at_sign() {
        assert_eq!(you_are("river"), "You are @river");
    }
    fn server(replies: Vec<(&'static str, String)>) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            for (expected, body) in replies {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 1024];
                    let n = socket.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    if bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes);
                assert!(request.starts_with(expected));
                assert!(request
                    .to_lowercase()
                    .contains(&format!("authorization: bearer {}", "d".repeat(64))));
                let status = if body.contains("session_expired") {
                    "401 Unauthorized"
                } else {
                    "200 OK"
                };
                write!(socket,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        (base, task)
    }
    #[test]
    fn real_http_profile_and_friend_refresh_use_the_account_session() {
        let (base, server) = server(vec![
            ("GET /v1/me ", r#"{"id":"me","handle":"player"}"#.into()),
            (
                "GET /v1/friends ",
                r#"[{"id":"friend","handle":"guest","state":"pending","requester":"friend"}]"#
                    .into(),
            ),
            ("GET /v1/blocks ", "[]".into()),
        ]);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let api = Api {
                base,
                client: Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .unwrap(),
                token: Some("d".repeat(64)),
            };
            let (user, friends, blocked) = api.snapshot().await.unwrap();
            assert!(blocked.is_empty());
            assert_eq!(user.handle, "player");
            assert_eq!(friends[0].state, "pending");
            assert_eq!(friends[0].requester, "friend");
        });
        server.join().unwrap();
    }
    #[test]
    fn real_http_expiry_maps_to_sign_in_without_exposing_response_data() {
        let (base, server) = server(vec![(
            "GET /v1/me ",
            r#"{"error":"session_expired","private":"do-not-display"}"#.into(),
        )]);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let api = Api {
                base,
                client: Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .unwrap(),
                token: Some("d".repeat(64)),
            };
            let (status, note) = api.call(Method::GET, "/v1/me", None).await.unwrap_err();
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(note.contains("Sign in again"));
            assert!(!note.contains("do-not-display"));
        });
        server.join().unwrap();
    }
    #[test]
    fn host_link_keeps_the_user_session_out_of_the_service() {
        let registration = json!({"computer":"00000000-0000-4000-8000-000000000001","owner":"me","certificate_fp":"ab".repeat(32),"credential":format!("h1_{}","a".repeat(64))});
        let (base, server) = server(vec![
            (
                "POST /v1/host-enrollment/start ",
                json!({"challenge":"1".repeat(64),"owner":"me"}).to_string(),
            ),
            ("POST /v1/host-enrollment/finish ", registration.to_string()),
        ]);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let api = Api {
                base,
                client: Client::new(),
                token: Some("d".repeat(64)),
            };
            let mut calls = Vec::new();
            let state = api
                .link_host_via(|command, args| {
                    assert!(!args.to_string().contains(&"d".repeat(64)));
                    calls.push(command);
                    std::future::ready(Ok(match command {
                        "host_account_state" => {
                            json!({"computer":null,"owner":null,"online":false})
                        }
                        "host_enrollment_proof" => {
                            assert_eq!(args["challenge"]["owner"], "me");
                            json!({"certificate":"public-fixture","proof":"one-use-proof"})
                        }
                        "host_account_install" => {
                            assert_eq!(args["registration"], registration);
                            json!({"computer":registration["computer"],"owner":"me","online":true})
                        }
                        _ => panic!("unexpected local command"),
                    }))
                })
                .await
                .unwrap();
            assert!(state.online);
            assert_eq!(
                calls,
                vec![
                    "host_account_state",
                    "host_enrollment_proof",
                    "host_account_install"
                ]
            );
        });
        server.join().unwrap();
    }
    #[test]
    fn failed_local_install_revokes_the_new_machine_link() {
        let computer = "00000000-0000-4000-8000-000000000001";
        let (base, server) = server(vec![
            (
                "POST /v1/host-enrollment/start ",
                json!({"challenge":"fixture","owner":"me"}).to_string(),
            ),
            (
                "POST /v1/host-enrollment/finish ",
                json!({"computer":computer,"credential":"machine-only"}).to_string(),
            ),
            (
                "DELETE /v1/computers/00000000-0000-4000-8000-000000000001/link ",
                "{}".into(),
            ),
        ]);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let api = Api {
                base,
                client: Client::new(),
                token: Some("d".repeat(64)),
            };
            let error = api
                .link_host_via(|command, _| {
                    std::future::ready(match command {
                        "host_account_state" => Ok(json!({"online":false})),
                        "host_enrollment_proof" => {
                            Ok(json!({"certificate":"fixture","proof":"fixture"}))
                        }
                        "host_account_install" => Err("local-disk-error".into()),
                        _ => panic!("unexpected command"),
                    })
                })
                .await
                .err()
                .unwrap();
            assert!(error.1.contains("could not save"));
        });
        server.join().unwrap();
    }
}

#[cfg(feature = "ui-review")]
fn review_textures(ctx: &eframe::egui::Context) -> previews::Cache<eframe::egui::TextureHandle> {
    use eframe::egui::{Color32, ColorImage, TextureOptions};
    let mut cache = previews::Cache::default();
    if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p.contains("preview")) {
        // Flat blocks standing in for a desktop, so edges and rounding are easy to judge.
        let (w, h) = (320usize, 180usize);
        let mut pixels = vec![Color32::from_rgb(30, 70, 110); w * h];
        for (x0, y0, x1, y1, color) in [(0, 152, 320, 180, Color32::from_rgb(20, 24, 30)), (24, 20, 190, 120, Color32::from_rgb(235, 238, 244)), (120, 56, 296, 140, Color32::from_rgb(60, 66, 80))] {
            for y in y0..y1 { for x in x0..x1 { pixels[y * w + x] = color; } }
        }
        let image = ColorImage { size: [w, h], source_size: eframe::egui::vec2(w as f32, h as f32), pixels };
        let ids: &[&str] = if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p.contains("long")) {
            &["00000000-0000-4000-8000-000000000011", "00000000-0000-4000-8000-000000000013", "00000000-0000-4000-8000-000000000015", "00000000-0000-4000-8000-000000000016"]
        } else {
            &["00000000-0000-4000-8000-000000000010", "00000000-0000-4000-8000-000000000011"]
        };
        for id in ids {
            cache.insert(id, ctx.load_texture(format!("preview {id}"), image.clone(), TextureOptions::LINEAR));
        }
    }
    cache
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use crate::design::{ACCENT, MUTED, OK, WARN};
    #[test]
    fn status_text_and_colour() {
        assert_eq!(computer_status(true, Presence::Online), ("Online", OK));
        assert_eq!(computer_status(true, Presence::Offline), ("Offline", MUTED));
        assert_eq!(computer_status(true, Presence::Streaming), ("Streaming", ACCENT));
        assert_eq!(computer_status(true, Presence::Locked), ("Locked", WARN));
        assert_eq!(computer_status(false, Presence::Online), ("Host update required", WARN));
    }
    fn computer(id: &str) -> OwnedComputer {
        OwnedComputer { id: id.into(), name: id.into(), platform: "linux".into(), certificate_fp: None, linked: 1, online: 1 }
    }
    #[test]
    fn this_computer_is_never_a_connect_target() {
        let all = vec![computer("a"), computer("b"), computer("c")];
        let ids = |local| remote_computers(&all, local).iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(Some("b")), ["a", "c"]);
        assert_eq!(ids(None), ["a", "b", "c"]);
        assert_eq!(ids(Some("elsewhere")), ["a", "b", "c"]);
        assert!(remote_computers(&all[1..2], Some("b")).is_empty());
        assert!(remote_computers(&[], Some("b")).is_empty());
    }
    #[test]
    fn strings() {
        assert_eq!(pairing_with("Office"), "Pairing with Office…");
        assert_eq!(NO_COMPUTERS, "No computers yet");
        assert_eq!(SIGN_IN_CONFIRM, "Confirm this code in your browser");
    }
}
