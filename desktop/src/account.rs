//! Account credentials remain on this worker; UI events never contain a bearer token.
use crate::{local_host, vault};
use reqwest::{Client, Method, StatusCode};
use secpar_client::control::Computer;
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
}
#[derive(Clone, Deserialize)]
pub struct Friend {
    pub id: String,
    pub handle: String,
    pub state: String,
    pub requester: String,
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
}

pub struct Panel {
    host: Option<Result<local_host::State, String>>,
    pub restored_computer: Option<Computer>,
    pub forgot_computer: bool,
    pub storage_note: Option<String>,
    pub user: Option<User>,
    friends: Vec<Friend>,
    blocked: Vec<User>,
    waiting: Option<(String, String)>,
    note: String,
    code: String,
    busy: bool,
    tx: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
}
impl Panel {
    pub fn new(ctx: eframe::egui::Context) -> Self {
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
                        if std::env::var("SECPAR_REVIEW_PAGE").as_deref() == Ok("host-sample") {
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
                    code: String::new(),
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
            forgot_computer: false,
            storage_note: None,
            user: None,
            friends: vec![],
            blocked: vec![],
            waiting: None,
            note: String::new(),
            code: String::new(),
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
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Host(state) => self.host = Some(state),
                Event::SignedOut(note) => {
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
                    self.code.clear();
                }
                Event::Message(note) => self.note = note,
                Event::Busy(busy) => self.busy = busy,
                Event::Restored(computer) => self.restored_computer = Some(computer),
                Event::ForgotComputer => self.forgot_computer = true,
                Event::StorageNote(note) => self.storage_note = Some(note),
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
                ui.label("Confirm this code in your browser");
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
                if ui.button("Copy my friend code").clicked() {
                    ui.ctx().copy_text(self.user.as_ref().unwrap().id.clone());
                    self.note = "Friend code copied".into();
                }
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
                    egui::TextEdit::singleline(&mut self.code)
                        .hint_text("Paste a friend code")
                        .margin(egui::vec2(10., 7.)),
                );
                if ui
                    .add_enabled(
                        !self.busy && !self.code.trim().is_empty(),
                        egui::Button::new("Add").min_size(egui::vec2(72., crate::design::CONTROL)),
                    )
                    .clicked()
                {
                    self.send(Command::Action(Action::Invite(self.code.clone())));
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
                                            ui.weak("Request sent");
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
struct Api {
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
                "account_unavailable" => "Friend code was not found or is unavailable.",
                "cannot_friend_yourself" => "That is your own friend code.",
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
        return Err("Paste your friend's complete friend code.".into());
    }
    Ok(value)
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
            let _=events.send(Event::Host(local_host::state().await)).await;
            ctx.request_repaint();
            let mut api=Api{client,token,base:ORIGIN.into()}; let mut pending: Option<Pending>=None;
            if api.token.is_some() {
                match api.snapshot().await {
                    Ok((user,friends,blocked))=>{let _=events.send(Event::SignedIn{user,friends,blocked,note:String::new()}).await;}
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
                if let Some(command)=command {
                    let _=events.send(Event::Busy(true)).await;
                    let skip_refresh=matches!(&command,Command::Remember(_)|Command::ForgetComputer|Command::HostRefresh|Command::LinkHost|Command::UnlinkHost);
                    let result: Result<String,(StatusCode,String)>= match command {
                        Command::HostRefresh=>{let _=events.send(Event::Host(local_host::state().await)).await;let _=events.send(Event::Message(String::new())).await;Ok(String::new())},
                        Command::LinkHost=>match api.link_host().await {
                            Ok(state)=>{let _=events.send(Event::Host(Ok(state))).await;let _=events.send(Event::Message("Host linked. Friend access remains off.".into())).await;Ok(String::new())},
                            Err(e)=>Err(e)
                        },
                        Command::UnlinkHost=>{
                            match local_host::call("host_account_unlink",Value::Null).await {
                                Ok(value)=>{let state=serde_json::from_value(value).map_err(|_|"Invalid host response".to_string());let _=events.send(Event::Host(state)).await;let _=events.send(Event::Message("Host unlinked".into())).await;Ok(String::new())},
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
                        Command::Refresh=>Ok(String::new()),
                        Command::Action(action)=>{
                            let (method,id,suffix)=match &action {
                                Action::Invite(id)=>(Method::PUT,id,""),Action::Accept(id)=>(Method::POST,id,"/accept"),Action::Remove(id)=>(Method::DELETE,id,""),Action::Block(id)=>(Method::PUT,id,""),Action::Unblock(id)=>(Method::DELETE,id,"")
                            };
                            match path_id(id) {
                                Err(note)=>Err((StatusCode::BAD_REQUEST,note)),
                                Ok(id)=>{let collection=if matches!(action,Action::Block(_)|Action::Unblock(_)){"blocks"}else{"friends"};api.call(method,&format!("/v1/{collection}/{id}{suffix}"),None).await.map(|_| if matches!(action,Action::Invite(_)){"Friend request sent".into()}else{String::new()})}
                            }
                        }
                    };
                    match result {
                        Err((status,note))=>{
                            if status==StatusCode::UNAUTHORIZED {api.token=None;let _=vault::forget(CREDENTIAL);let _=events.send(Event::SignedOut(note)).await;}
                            else {let _=events.send(Event::Message(note)).await;}
                        }
                        Ok(note) if api.token.is_some()&&!skip_refresh=>match api.snapshot().await {
                            Ok((user,friends,blocked))=>{let _=events.send(Event::SignedIn{user,friends,blocked,note}).await;}
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
                                match api.snapshot().await {Ok((user,friends,blocked))=>{let _=events.send(Event::SignedIn{user,friends,blocked,note}).await;},Err((status,note))=>{if status==StatusCode::UNAUTHORIZED{api.token=None;let _=vault::forget(CREDENTIAL);}let _=events.send(Event::SignedOut(note)).await;}}
                            }
                        }
                        Err((_,code)) if code=="authorization_pending"=>{},
                        Err((_,code)) if code=="slow_down"=>{p.interval+=Duration::from_secs(5);p.next=Instant::now()+p.interval;},
                        Err((status,note)) if status.is_server_error()=>{let _=events.send(Event::Message(note)).await;},
                        Err((_,note))=>{pending=None;let _=events.send(Event::SignedOut(note)).await;}
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
    #[test]
    fn friend_codes_cannot_inject_account_paths() {
        assert!(path_id("d0d12c4d-13f1-4fbc-b113-a89d901ac34b").is_ok());
        for code in ["../me", "abcd/../../blocks", "", "a".repeat(36).as_str()] {
            assert!(path_id(code).is_err());
        }
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
