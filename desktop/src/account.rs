//! Account credentials remain on this worker; UI events never contain a bearer token.
use crate::{local_host, previews, vault};
use reqwest::{Client, Method, StatusCode};
use secpar_client::control::{Computer, Connected};
use secpar_client::pairing::Pairing;
use secpar_client::connlog;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const ORIGIN: &str = "https://accounts.secpar.win";
const CREDENTIAL: &str = "account-session";

#[derive(Clone, Deserialize)]
pub struct User {
    pub id: String,
    pub handle: String,
}
#[derive(Clone, Deserialize)]
pub struct Friend {
    pub id: String,
    pub handle: String,
    pub state: String,
    pub requester: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct OwnedComputer {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub certificate_fp: Option<String>,
    pub linked: u8,
    pub online: u8,
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
    username: String,
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
                    }),
                    friends: if sample {
                        vec![
                            Friend {
                                id: "00000000-0000-4000-8000-000000000002".into(),
                                handle: "Jamie".into(),
                                state: "accepted".into(),
                                requester: own.clone(),
                            },
                            Friend {
                                id: "00000000-0000-4000-8000-000000000003".into(),
                                handle: "Morgan".into(),
                                state: "pending".into(),
                                requester: "00000000-0000-4000-8000-000000000003".into(),
                            },
                            Friend {
                                id: "00000000-0000-4000-8000-000000000004".into(),
                                handle: "Riley".into(),
                                state: "pending".into(),
                                requester: own.clone(),
                            },
                        ]
                    } else {
                        vec![]
                    },
                    blocked: vec![],
                    waiting: None,
                    note: String::new(),
                    username: String::new(),
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
            username: String::new(),
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
    pub fn sync_previews(&mut self, ctx: &eframe::egui::Context, streaming: Option<&str>) {
        let local = self.host.as_ref().and_then(|state| state.as_ref().ok()).and_then(|state| state.computer.as_deref());
        let owner = self.user.as_ref().map(|user| user.id.as_str());
        let shown = |c: &OwnedComputer| c.online == 1 && c.linked == 1 && Some(c.id.as_str()) != local && Some(c.id.as_str()) != streaming;
        let targets = owner.map(|owner| {
            self.computers.iter().filter(|c| shown(c)).filter_map(|c| {
                let fingerprint = secpar_client::pairing::fingerprint(c.certificate_fp.as_deref()?).ok()?;
                Some(previews::Target { id: c.id.clone(), key: owned_key(owner, &c.id).ok()?, fingerprint })
            }).collect()
        });
        self.previews.set(previews::Wanted { targets: targets.unwrap_or_default() });
        self.textures.apply(self.previews.take(), |id, image| ctx.load_texture(format!("preview {id}"), image, eframe::egui::TextureOptions::LINEAR));
        let computers = &self.computers;
        self.textures.retain(|id| computers.iter().any(|c| c.id == id && shown(c)));
    }
    /// The computer this app hosts from, when the local host is linked to the account.
    pub fn local_id(&self) -> Option<&str> {
        self.host.as_ref().and_then(|state| state.as_ref().ok()).and_then(|state| state.computer.as_deref())
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
            ui.spacing_mut().item_spacing.y=8.;
            if shown.is_empty() { ui.label(egui::RichText::new(if self.computers.is_empty() {NO_COMPUTERS}else{NO_OTHER_COMPUTERS}).color(design::MUTED)); }
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
                    self.username.clear();
                }
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
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 80.).max(80.);
                ui.add_sized(
                    [width, crate::design::CONTROL],
                    egui::TextEdit::singleline(&mut self.username)
                        .hint_text("Add by username")
                        .margin(egui::vec2(10., 7.)),
                );
                if ui
                    .add_enabled(
                        !self.busy && !self.username.trim().is_empty(),
                        egui::Button::new("Add").min_size(egui::vec2(72., crate::design::CONTROL)),
                    )
                    .clicked()
                {
                    self.send(Command::Action(Action::Invite(self.username.clone())));
                }
            });
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
                                            egui::RichText::new("Friend")
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
#[derive(Clone)]
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
    })
}
pub(crate) fn owned_key(owner: &str, id: &str) -> Result<String,String> {
    path_id(owner)?; path_id(id)?;
    Ok(format!("owned-pairing:{owner}:{id}"))
}
/// The credential saved for an account computer by an earlier connection.
fn saved_owned(owner: &str, id: &str) -> Option<Computer> {
    let key = owned_key(owner, id).ok()?;
    serde_json::from_str(&vault::load(&key).ok().flatten()?).ok()
}
fn fingerprint_of(c: &OwnedComputer) -> Option<[u8; 32]> {
    c.certificate_fp.as_deref().and_then(|fp| secpar_client::pairing::fingerprint(fp).ok())
}
/// How an account computer is opened.
#[derive(Debug, PartialEq)]
pub(crate) enum Plan {
    /// The saved credential goes straight to the connection. Its pinned fingerprint is checked by
    /// the TLS handshake itself, so the account is not asked first.
    UseSaved,
    /// No usable credential: ask the account to have the computer make one.
    AskAccount,
}
/// Chooses between the saved credential and the account. `known` is the computer list from the
/// last sync (empty before the first one). Only what the account has already said can rule the
/// credential out: the computer is no longer linked, or has another identity now.
pub(crate) fn plan_open(known: &[OwnedComputer], id: &str, saved: Option<&[u8; 32]>) -> Plan {
    let Some(saved) = saved else { return Plan::AskAccount };
    match known.iter().find(|c| c.id == id) {
        Some(c) if c.linked != 1 => Plan::AskAccount,
        Some(c) if fingerprint_of(c).is_some_and(|fp| fp != *saved) => Plan::AskAccount,
        _ => Plan::UseSaved,
    }
}
/// How long to wait before asking the account whether the computer has answered a pairing
/// request. Quick at first, because the computer usually answers within a few seconds.
pub(crate) fn pair_poll_delay(waited: Duration) -> Duration {
    if waited < Duration::from_secs(6) { Duration::from_millis(300) }
    else if waited < Duration::from_secs(20) { Duration::from_millis(800) }
    else { Duration::from_secs(2) }
}
const AUTO_PAIR_LIMIT: Duration = Duration::from_secs(45);
const AUTO_PAIR_BACKOFF: Duration = Duration::from_secs(300);
/// The next online computer of the account that has no credential on this device yet. Each one is
/// tried again only after the backoff, so a computer that cannot answer does not cause a request
/// every half minute.
pub(crate) fn plan_auto_pair<'a>(list: &'a [OwnedComputer], local: Option<&str>, saved: &dyn Fn(&str) -> Option<[u8; 32]>, tried: &HashMap<String, Instant>, now: Instant) -> Option<&'a OwnedComputer> {
    list.iter().find(|c| {
        c.online == 1
            && c.linked == 1
            && Some(c.id.as_str()) != local
            && fingerprint_of(c).is_some_and(|fp| saved(&c.id) != Some(fp))
            && tried.get(&c.id).map_or(true, |at| now.saturating_duration_since(*at) >= AUTO_PAIR_BACKOFF)
    })
}
fn ms(since: Instant) -> u64 { since.elapsed().as_millis() as u64 }
/// Opens an account computer: reuses a saved credential unless the account has said it is out of
/// date, or asks the computer for a new one through the account.
async fn connect_owned(api: &Api, owner: &str, id: &str, known: &[OwnedComputer], saved: &impl Fn(&str, &str) -> Option<Computer>, pending_pair: &mut Option<PendingPair>, events: &mpsc::Sender<Event>) -> Result<String, (StatusCode, String)> {
    let started = Instant::now();
    let credential = saved(owner, id);
    if plan_open(known, id, credential.as_ref().map(|c| &c.fingerprint)) == Plan::UseSaved {
        if let Some(credential) = credential {
            *pending_pair = None;
            connlog::note(id, "credential", &format!("saved host={}", connlog::short_label(&credential.id)), ms(started));
            let _ = events.send(Event::OwnedConnection(OwnedConnection::Cached { owner: owner.into(), id: id.into(), computer: credential })).await;
            return Ok(String::new());
        }
    }
    let fetched = Instant::now();
    let computers = api.computers().await?;
    connlog::note(id, "account_fetch", "computers", ms(fetched));
    let Some(computer) = computers.into_iter().find(|c| c.id == id && c.linked == 1) else {
        return Err((StatusCode::NOT_FOUND, "This computer is no longer linked to your account".into()));
    };
    let Some(fingerprint) = fingerprint_of(&computer) else {
        return Err((StatusCode::BAD_GATEWAY, "This computer has no verified host identity".into()));
    };
    if let Some(credential) = credential.filter(|saved| saved.fingerprint == fingerprint) {
        *pending_pair = None;
        let _ = events.send(Event::OwnedConnection(OwnedConnection::Cached { owner: owner.into(), id: id.into(), computer: credential })).await;
        return Ok(String::new());
    }
    let asked = Instant::now();
    let value = api.call(Method::POST, "/v1/owner-pairings", Some(json!({"computer": id, "device": crate::device_name()}))).await?;
    connlog::note(id, "pair_request", "posted", ms(asked));
    let Some(request) = value["id"].as_str().filter(|id| path_id(id).is_ok()) else {
        return Err((StatusCode::BAD_GATEWAY, "Invalid pairing response".into()));
    };
    *pending_pair = Some(PendingPair { id: request.into(), owner: owner.into(), computer: id.into(), fingerprint, expires: Instant::now() + Duration::from_secs(120), next: Instant::now() + pair_poll_delay(Duration::ZERO), since: Instant::now() });
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
pub(crate) async fn connect_owned_for_test(api: &Api, owner: &str, id: &str, known: &[OwnedComputer], saved: Option<Computer>, pending_pair: &mut Option<PendingPair>, events: &mpsc::Sender<Event>) -> Result<String, (StatusCode, String)> {
    connect_owned(api, owner, id, known, &|_: &str, _: &str| saved.clone(), pending_pair, events).await
}
/// Pairs with one of the account's computers without anyone asking: the account passes a request to
/// the computer, the computer answers with a one-use link, and this connects once to spend it.
/// The device credential that comes out is returned to be saved. Nothing is shown.
async fn auto_pair(api: Api, id: String, fingerprint: [u8; 32], limit: Duration) -> AutoOut {
    let begun = Instant::now();
    let result: Result<Computer, String> = async {
        let asked = Instant::now();
        let device = crate::device_name();
        let value = api.call(Method::POST, "/v1/owner-pairings", Some(json!({"computer": id, "device": device}))).await.map_err(|(_, note)| note)?;
        connlog::note(&id, "pair_request", "auto", ms(asked));
        let request = value["id"].as_str().filter(|r| path_id(r).is_ok()).ok_or("Invalid pairing response")?.to_string();
        let waiting = Instant::now();
        let answer = loop {
            tokio::time::sleep(pair_poll_delay(waiting.elapsed())).await;
            if waiting.elapsed() > limit {
                let _ = api.call(Method::DELETE, &format!("/v1/owner-pairings/{request}"), None).await;
                return Err("The computer did not answer".to_string());
            }
            match api.call(Method::GET, &format!("/v1/owner-pairings/{request}"), None).await {
                Ok(value) if value["state"] == "pending" => {}
                Ok(value) => break value,
                Err((status, _)) if status.is_server_error() => {}
                Err((_, note)) => return Err(note),
            }
        };
        connlog::note(&id, "pair_wait", "answered", ms(waiting));
        let pairing = answer["link"].as_str().and_then(|link| Pairing::parse(link).ok())
            .filter(|pairing| pairing.fingerprint == fingerprint && answer["certificate_fp"].as_str().and_then(|fp| secpar_client::pairing::fingerprint(fp).ok()) == Some(fingerprint))
            .ok_or("Computer identity did not match its account registration")?;
        let mut connected = Connected::pair_lan_first(pairing, &device).await?;
        let computer = connected.computer.clone();
        let _ = connected.socket.close(None).await;
        Ok(computer)
    }.await;
    connlog::note(&id, "auto_pair", if result.is_ok() { "saved" } else { "failed" }, ms(begun));
    AutoOut { id, result }
}
pub(crate) struct AutoOut {
    id: String,
    result: Result<Computer, String>,
}
pub(crate) struct PendingPair {
    id: String, owner: String, computer: String, fingerprint: [u8;32],
    expires: Instant, next: Instant, since: Instant,
}
/// What one sync of the account's computers found.
struct SyncOut {
    host: Result<local_host::State, String>,
    computers: Result<Vec<OwnedComputer>, (StatusCode, String)>,
}
/// The local host check and the computer list run side by side, so a slow local service does not
/// hold up the list, and the list does not wait for the host.
async fn synchronize_owned(api: Api, owner: String, enroll: bool) -> SyncOut {
    let started = Instant::now();
    let out = gather(
        async { api.ensure_host_via(&owner, enroll, local_host::call).await.map_err(|(_, note)| note) },
        api.computers(),
    ).await;
    connlog::note("account", "account_fetch", "sync", ms(started));
    out
}
async fn gather(host: impl Future<Output = Result<local_host::State, String>>, computers: impl Future<Output = Result<Vec<OwnedComputer>, (StatusCode, String)>>) -> SyncOut {
    let (host, computers) = tokio::join!(host, computers);
    SyncOut { host, computers }
}
type Background<T> = Option<Pin<Box<dyn Future<Output = T>>>>;
enum Wake {
    Command(Command),
    Tick,
    Sync(SyncOut),
    Auto(AutoOut),
}
/// What the worker does next: a command, the end of a background job, or just the clock. A job in
/// the background (the sync, an automatic pairing) never holds up a command. None when the app is
/// closing.
async fn next_wake(rx: &mut mpsc::Receiver<Command>, tick: Duration, sync_fut: &mut Background<SyncOut>, auto_fut: &mut Background<AutoOut>) -> Option<Wake> {
    Some(tokio::select! {
        command = rx.recv() => Wake::Command(command?),
        _ = tokio::time::sleep(tick) => Wake::Tick,
        out = async { sync_fut.as_mut().expect("guarded").await }, if sync_fut.is_some() => Wake::Sync(out),
        out = async { auto_fut.as_mut().expect("guarded").await }, if auto_fut.is_some() => Wake::Auto(out),
    })
}
pub fn start(ctx: eframe::egui::Context) -> (mpsc::Sender<Command>, mpsc::Receiver<Event>) {
    let (tx, mut rx) = mpsc::channel(16);
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
            // The local service may be slow to answer. Nothing else waits for it.
            { let host_events=events.clone(); let host_ctx=ctx.clone(); tokio::spawn(async move { let _=host_events.send(Event::Host(local_host::state().await)).await; host_ctx.request_repaint(); }); }
            ctx.request_repaint();
            let mut api=Api{client,token,base:ORIGIN.into()}; let mut pending: Option<Pending>=None;
            let mut pending_pair: Option<PendingPair> = None;
            let mut signed_owner: Option<String> = None;
            let mut suppressed_owner = vault::load("host-auto-link-disabled").ok().flatten();
            let mut next_owned = Instant::now();
            let mut known: Vec<OwnedComputer> = vec![];
            let mut local_id: Option<String> = None;
            let mut sync_fut: Background<SyncOut> = None;
            let mut auto_fut: Background<AutoOut> = None;
            let mut auto_for: Option<String> = None;
            let mut want_after_auto: Option<String> = None;
            let mut auto_tried: HashMap<String, Instant> = HashMap::new();
            if api.token.is_some() {
                // The profile and the computer list are independent: one round trip, not two.
                let started = Instant::now();
                let (snapshot, computers) = tokio::join!(api.snapshot(), api.computers());
                connlog::note("account", "account_fetch", "startup", ms(started));
                match snapshot {
                    Ok((user,friends,blocked))=>{
                        signed_owner=Some(user.id.clone());next_owned=Instant::now();
                        if let Ok(list)=computers { known=list.clone(); let _=events.send(Event::Owned(list)).await; }
                        let _=events.send(Event::SignedIn{user,friends,blocked,note:String::new()}).await;
                    }
                    Err((status,note))=>{
                        if status==StatusCode::UNAUTHORIZED {api.token=None;let _=vault::forget(CREDENTIAL);}
                        let _=events.send(Event::SignedOut(note)).await;
                    }
                }
                ctx.request_repaint();
            }
            loop {
                // While a pairing is under way the loop looks at it often. Otherwise once a second is plenty.
                let tick = if pending_pair.is_some() || auto_fut.is_some() { Duration::from_millis(150) } else { Duration::from_secs(1) };
                let Some(wake) = next_wake(&mut rx, tick, &mut sync_fut, &mut auto_fut).await else { break };
                let mut command=None;
                match wake {
                    Wake::Command(c)=>command=Some(c),
                    Wake::Tick=>{}
                    Wake::Sync(out)=>{
                        sync_fut=None;
                        if api.token.is_some() {
                            if let Ok(state)=&out.host { local_id=state.computer.clone(); }
                            let _ = events.send(Event::Host(out.host)).await;
                            match out.computers {
                                Ok(list)=>{ known=list.clone(); let _ = events.send(Event::Owned(list)).await; }
                                Err((StatusCode::UNAUTHORIZED,note))=>{
                                    let _=events.send(Event::SignedOut(note)).await;
                                    api.token=None; signed_owner=None; pending_pair=None; let _=vault::forget(CREDENTIAL);
                                }
                                Err((_,note))=>{ let _ = events.send(Event::Message(note)).await; }
                            }
                            ctx.request_repaint();
                        }
                    }
                    Wake::Auto(out)=>{
                        auto_fut=None; auto_for=None;
                        let wanted=want_after_auto.take().filter(|id| *id==out.id);
                        match (out.result, signed_owner.clone()) {
                            (Ok(computer), Some(owner))=>{
                                if let Ok(key)=owned_key(&owner,&out.id) {
                                    match serde_json::to_string(&computer).map_err(|_|"Could not remember this computer".to_string()).and_then(|v| vault::save(&key,&v)) {
                                        Ok(())=>{}, Err(note)=>{ let _=events.send(Event::StorageNote(note)).await; }
                                    }
                                }
                                if wanted.is_some() { let _=events.send(Event::OwnedConnection(OwnedConnection::Cached{owner,id:out.id,computer})).await; }
                            }
                            (Err(note),_)=>{ if wanted.is_some() { let _=events.send(Event::Message(note)).await; } }
                            (Ok(_),None)=>{}
                        }
                        let _=events.send(Event::Busy(false)).await;
                        ctx.request_repaint();
                    }
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
                            // Withdrawing an old request is not on the way to the new connection.
                            if let Some(old)=pending_pair.take() { let api=api.clone(); tokio::spawn(async move { let _=api.call(Method::DELETE,&format!("/v1/owner-pairings/{}",old.id),None).await; }); }
                            match signed_owner.clone().filter(|_| api.token.is_some()) {
                                Some(_) if auto_for.as_deref()==Some(id.as_str())=>{
                                    // The computer is being paired in the background already: wait for that.
                                    want_after_auto=Some(id.clone());
                                    let name=known.iter().find(|c| c.id==id).map(|c| c.name.clone()).unwrap_or_default();
                                    let _=events.send(Event::Message(pairing_with(&name))).await;
                                    Ok(String::new())
                                }
                                Some(owner)=>connect_owned(&api,&owner,&id,&known,&saved_owned,&mut pending_pair,&events).await,
                                None=>Err((StatusCode::UNAUTHORIZED,"Sign in to connect to your computers".into())),
                            }
                        }
                        Command::Refresh=>Ok(String::new()),
                        Command::Action(action)=>{
                            match action_request(&action) {
                                Err(note)=>Err((StatusCode::BAD_REQUEST,note)),
                                Ok((method,path))=>api.call(method,&path,None).await.map(|_| if matches!(action,Action::Invite(_)){"Friend request sent".into()}else{String::new()})
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
                if api.token.is_none() { pending_pair=None; signed_owner=None; sync_fut=None; auto_fut=None; auto_for=None; want_after_auto=None; known.clear(); }
                if sync_fut.is_none() {
                    if let Some(owner)=signed_owner.clone().filter(|_| api.token.is_some() && Instant::now()>=next_owned) {
                        next_owned=Instant::now()+Duration::from_secs(30);
                        let enroll=suppressed_owner.as_deref()!=Some(owner.as_str());
                        sync_fut=Some(Box::pin(synchronize_owned(api.clone(),owner,enroll)));
                    }
                }
                // Computers of the account that this device has no credential for are paired in the
                // background, so opening one later needs no pairing.
                if auto_fut.is_none() && pending_pair.is_none() {
                    if let Some(owner)=signed_owner.as_deref().filter(|_| api.token.is_some()) {
                        let saved=|id:&str| saved_owned(owner,id).map(|c| c.fingerprint);
                        if let Some(next)=plan_auto_pair(&known,local_id.as_deref(),&saved,&auto_tried,Instant::now()).cloned() {
                            if let Some(fingerprint)=fingerprint_of(&next) {
                                auto_tried.insert(next.id.clone(),Instant::now());
                                auto_for=Some(next.id.clone());
                                auto_fut=Some(Box::pin(auto_pair(api.clone(),next.id,fingerprint,AUTO_PAIR_LIMIT)));
                            }
                        }
                    }
                }
                if pending_pair.as_ref().is_some_and(|p|Instant::now()>=p.next) {
                    let p=pending_pair.as_mut().unwrap();
                    if Instant::now()>=p.expires {
                        pending_pair=None;let _=events.send(Event::Message("Computer did not respond. Check that Secpar is open there.".into())).await;
                    } else {
                        p.next=Instant::now()+pair_poll_delay(p.since.elapsed());
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
#[path = "connect_speed_tests.rs"]
mod speed_tests;

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
