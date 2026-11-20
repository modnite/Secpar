//! One desktop UI for the local hosting engine and remote clients, including Android.
use crate::{design, local_host};
use eframe::egui::{self, Color32};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Clone, Default, Deserialize)]
struct Device {
    id: String,
    name: String,
    connected: bool,
}
#[derive(Clone, Default, Deserialize)]
struct FriendRow {
    id: String,
    handle: String,
    connect: String,
    keyboard: bool,
    mouse: bool,
}
#[derive(Clone, Default, Deserialize)]
struct State {
    name: String,
    devices: Vec<Device>,
    /// Friends with a running session. Not paired devices.
    #[serde(default)]
    friends: Vec<Device>,
    locked: bool,
}
enum Command {
    Refresh,
    Pair,
    Patch(Value),
    Revoke(String),
    Disconnect(String),
    Friend { id: String, keyboard: Option<bool>, mouse: Option<bool>, connect: Option<&'static str> },
}
enum Event {
    Refresh(Result<(State, Value), String>),
    State(Result<(State, Value), String>),
    Friends(Result<Vec<FriendRow>, String>),
    Pair(Result<String, String>),
    Error(String),
}
pub struct Panel {
    tx: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    state: Option<State>,
    settings: Value,
    /// Every friend of the account and what each may do. Empty when the host is not linked.
    friend_rows: Vec<FriendRow>,
    error: String,
    busy: bool,
    refreshing: bool,
    pairing: bool,
    settings_open: bool,
    account_open: bool,
    link: String,
    expires: Instant,
    qr: Option<qrcode::QrCode>,
    revoke: Option<Device>,
    last_refresh: Instant,
    sample: bool,
}
impl Panel {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, mut commands) = mpsc::channel(8);
        let (events, rx) = mpsc::channel(8);
        let sample = cfg!(feature = "ui-review")
            && std::env::var_os("SECPAR_REVIEW_SCREENSHOT_TO").is_some();
        if !sample {
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async move {
                    while let Some(command) = commands.recv().await {
                        let event = match command {
                            Command::Refresh => Event::Refresh(snapshot().await),
                            Command::Pair => Event::Pair(
                                local_host::call("new_qr", Value::Null).await.and_then(|v| {
                                    let link = v["link"]
                                        .as_str()
                                        .filter(|s| s.starts_with("secpar://pair?"))
                                        .ok_or("Host returned an invalid pairing link")?;
                                    if link.len() > 8192 {
                                        return Err("Host pairing link is too large".into());
                                    }
                                    Ok(link.into())
                                }),
                            ),
                            Command::Patch(patch) => {
                                // Read fresh settings, preserving unrelated/unknown fields.
                                match patch_config(patch).await {
                                    Ok(()) => Event::State(snapshot().await),
                                    Err(e) => Event::Error(e),
                                }
                            }
                            Command::Revoke(id) => {
                                match local_host::call("revoke", json!({"id":id})).await {
                                    Ok(_) => Event::State(snapshot().await),
                                    Err(e) => Event::Error(e),
                                }
                            }
                            Command::Friend { id, keyboard, mouse, connect } => Event::Friends(
                                local_host::call("host_friend_set", json!({"id":id,"keyboard":keyboard,"mouse":mouse,"connect":connect}))
                                    .await
                                    .and_then(|v| serde_json::from_value(v).map_err(|_| "Invalid friends response".to_string())),
                            ),
                            Command::Disconnect(id) => {
                                match local_host::call("disconnect_friend", json!({"id":id})).await {
                                    Ok(_) => Event::State(snapshot().await),
                                    Err(e) => Event::Error(e),
                                }
                            }
                        };
                        if events.send(event).await.is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                });
            });
        }
        #[cfg(target_os = "linux")]
        if !sample {
            let _ = tx.try_send(Command::Refresh);
        }
        Self {
            tx,
            events: rx,
            state: sample.then(|| State {
                name: "Studio PC".into(),
                devices: vec![Device {
                    id: "sample".into(),
                    name: "Pixel 8".into(),
                    connected: false,
                }],
                friends: Vec::new(),
                locked: false,
            }),
            friend_rows: if sample {
                vec![
                    FriendRow { id: "a".into(), handle: "Jamie".into(), connect: "always".into(), keyboard: true, mouse: true },
                    FriendRow { id: "b".into(), handle: "Riley".into(), connect: "ask".into(), keyboard: true, mouse: false },
                ]
            } else {
                Vec::new()
            },
            settings: json!({"config":{"autostart":true,"display":"","max_quality":"auto","encoder":{"backend":"auto","preset":"fast"},"allow_files":true,"allow_clip":true,"friends":true,"anywhere":false},"displays":[],"encoders":[{"id":"nvenc","name":"NVIDIA NVENC","available":true},{"id":"ffmpeg-nvenc","name":"NVIDIA NVENC (FFmpeg)","available":true},{"id":"ffmpeg-amf","name":"AMD AMF","available":false,"reason":"Not available on this GPU"},{"id":"ffmpeg-qsv","name":"Intel Quick Sync","available":false,"reason":"Not available on this GPU"}],"pad":{"ok":true,"text":"Gamepad ready"}}),
            error: String::new(),
            busy: false,
            refreshing: false,
            pairing: sample
                && std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p.contains("pair")),
            settings_open: sample
                && std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p.contains("settings")),
            account_open: false,
            link: String::new(),
            expires: Instant::now(),
            qr: None,
            revoke: None,
            last_refresh: Instant::now() - Duration::from_secs(10),
            sample,
        }
    }
    /// Takes in the host's answers and refreshes its state every few seconds.
    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events.try_recv() {
            let refresh=matches!(&event,Event::Refresh(_));
            if refresh {self.refreshing=false;} else {self.busy=false;}
            match event {
                Event::State(Ok((state, config))) | Event::Refresh(Ok((state, config))) => {
                    self.state = Some(state);
                    self.friend_rows = config["friend_rows"].as_array().map(|rows| rows.iter().filter_map(|r| serde_json::from_value(r.clone()).ok()).collect()).unwrap_or_default();
                    self.settings = config;
                    self.error.clear();
                }
                Event::State(Err(e)) | Event::Refresh(Err(e)) => {
                    self.error = e;
                }
                Event::Friends(Ok(rows)) => self.friend_rows = rows,
                Event::Friends(Err(e)) => self.error = e,
                Event::Pair(Ok(link)) => {
                    match qrcode::QrCode::with_error_correction_level(
                        link.as_bytes(),
                        qrcode::EcLevel::M,
                    ) {
                        Ok(qr) => {
                            self.qr = Some(qr);
                            self.link = link;
                            self.expires = Instant::now() + Duration::from_secs(110);
                            self.error.clear();
                        }
                        Err(_) => self.error = "Could not render the pairing code".into(),
                    }
                }
                Event::Pair(Err(e)) | Event::Error(e) => self.error = e,
            }
        }
        if !self.busy && !self.refreshing && self.last_refresh.elapsed() > Duration::from_secs(5) {
            self.last_refresh = Instant::now();
            self.send(Command::Refresh);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    pub fn open_settings(&mut self) {self.settings_open=true;}
    fn send(&mut self, command: Command) {
        if self.sample {
            return;
        }
        let refresh=matches!(&command,Command::Refresh);
        if self.tx.try_send(command).is_ok() {
            if refresh {self.refreshing=true;} else {self.busy=true;}
        }
    }
    /// Settings content shares the parent's scroll area and has no back navigation.
    pub fn inline_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, account: &mut crate::account::Panel) {
        self.ui_inner(ui, ctx, account, true);
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, account: &mut crate::account::Panel) -> bool {
        self.ui_inner(ui, ctx, account, false)
    }
    fn ui_inner(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        account: &mut crate::account::Panel,
        inline: bool,
    ) -> bool {
        let mut back=false;
        self.pump(ctx);
        if account.user.is_some() {
            self.pairing=false;
            self.link.clear();
            self.qr=None;
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        if inline {
            // Settings, This computer: what it is and whether it hosts, as rows.
            ui.spacing_mut().item_spacing.y = 8.;
            design::group(ui, "This computer", |ui| {
                let name = self.state.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| "This computer".into());
                design::row(ui, "Name", |ui| { ui.label(egui::RichText::new(name).color(design::MUTED)); });
                let status = match &self.state {
                    Some(state) if state.locked => "Locked",
                    Some(_) => "Hosting",
                    None if cfg!(windows) => "Hosting service unavailable",
                    None => "Not hosting",
                };
                design::row(ui, "Status", |ui| { ui.label(egui::RichText::new(status).color(design::MUTED)); });
                design::row(ui, "Hosting account", |ui| {
                    if ui.add(egui::Button::new("Manage").min_size(egui::vec2(96., design::CONTROL))).clicked() { self.account_open = true; }
                });
            });
        } else {
        ui.horizontal(|ui| {
            back=design::back_button(ui);
            ui.heading(if self.settings_open {"Host settings"}else{"Paired devices"});
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                ui.menu_button("Manage",|ui| {
                    if ui.button(if self.settings_open {"Paired devices"}else{"Host settings"}).clicked(){self.settings_open=!self.settings_open;ui.close();}
                    if ui.button("Account").clicked(){self.account_open=true;ui.close();}
                });
            });
        });
        }
        ui.add_space(if self.settings_open { 0. } else { 8. });
        if !inline && !self.settings_open {
            design::surface().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    design::dot(
                        ui,
                        if self.state.is_some() {
                            design::ACCENT
                        } else {
                            design::MUTED
                        },
                    );
                    ui.label(
                        egui::RichText::new(
                            self.state
                                .as_ref()
                                .map(|s| s.name.as_str())
                                .unwrap_or("This computer"),
                        )
                        .strong(),
                    );
                    if account.user.is_none() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    self.state.is_some() && !self.busy,
                                    design::primary("Pair a device"),
                                )
                                .clicked()
                            {
                                self.pairing = true;
                                self.link.clear();
                                self.qr = None;
                                self.send(Command::Pair);
                            }
                        });
                    }
                });
                if let Some(state) = &self.state {
                    ui.label(
                        egui::RichText::new(if state.locked {
                            "Locked"
                        } else {
                            "Ready for paired devices"
                        })
                        .color(design::MUTED),
                    );
                } else if cfg!(windows) {
                    ui.label("Hosting service unavailable. Reinstall Secpar.");
                } else {
                    ui.label("Hosting on this platform is still in development.");
                }
            });
        }
        if !self.error.is_empty() {
            ui.colored_label(Color32::from_rgb(255, 170, 120), &self.error);
        }
        ui.add_space(if self.settings_open { 0. } else { 10. });
        let mut contents = |ui: &mut egui::Ui| {
            if inline || self.settings_open { self.settings_ui(ui); }
            if !inline && !self.settings_open { if let Some(state) = self.state.clone() {
                if state.devices.is_empty() {
                    ui.label(egui::RichText::new("No paired devices").color(design::MUTED));
                }
                for friend in &state.friends {
                    ui.horizontal(|ui| {
                        design::dot(ui, design::ACCENT);
                        ui.label(format!("{} (friend)", friend.name));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(!self.busy, egui::Button::new("Disconnect")).clicked() {
                                self.send(Command::Disconnect(friend.id.clone()));
                            }
                        });
                    });
                }
                for device in state.devices {
                    ui.horizontal(|ui| {
                        design::dot(
                            ui,
                            if device.connected {
                                design::ACCENT
                            } else {
                                design::MUTED
                            },
                        );
                        ui.label(&device.name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(!self.busy, egui::Button::new("Remove"))
                                .clicked()
                            {
                                self.revoke = Some(device.clone());
                            }
                            ui.label(
                                egui::RichText::new(if device.connected {
                                    "Connected"
                                } else {
                                    "Paired"
                                })
                                .size(12.)
                                .color(design::MUTED),
                            );
                        });
                    });
                }
            } else { ui.label(egui::RichText::new("Hosting unavailable").color(design::MUTED)); }
            }
        };
        if inline { contents(ui); } else { egui::ScrollArea::vertical().show(ui, contents); }
        if self.account_open {
            egui::Modal::new(egui::Id::new("hosting-account")).show(ctx, |ui| {
                ui.set_width(420_f32.min(ctx.content_rect().width() - 48.));
                account.host_ui(ui);
                ui.add_space(8.);
                if ui.button("Close").clicked() {
                    self.account_open = false;
                }
            });
        }
        if self.pairing {
            egui::Modal::new(egui::Id::new("host-pairing")).show(ctx, |ui| {
                let size = (ctx.content_rect().height() - 100.).clamp(160., 320.);
                ui.set_width((size + 196.).min(ctx.content_rect().width() - 48.));
                ui.horizontal(|ui| {
                    ui.heading("Pair a device");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            self.pairing = false;
                            self.link.clear();
                            self.qr = None;
                        }
                    });
                });
                ui.horizontal_top(|ui| {
                    ui.allocate_ui(egui::vec2(size, size), |ui| {
                        if let Some(qr) = &self.qr {
                            paint_qr(ui, qr);
                        } else {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 4., design::BG);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                if self.sample {
                                    "Pairing code hidden"
                                } else {
                                    "Creating code…"
                                },
                                egui::FontId::proportional(14.),
                                design::MUTED,
                            );
                        }
                    });
                    ui.vertical(|ui| {
                        ui.set_width(176.);
                        if ui
                            .add_enabled(
                                !self.link.is_empty(),
                                egui::Button::new("Copy pairing link"),
                            )
                            .clicked()
                        {
                            ctx.copy_text(self.link.clone());
                        }
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("New code"))
                            .clicked()
                        {
                            self.send(Command::Pair);
                        }
                        if !self.link.is_empty() {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Refreshes in {}s",
                                    self.expires
                                        .saturating_duration_since(Instant::now())
                                        .as_secs()
                                ))
                                .size(12.)
                                .color(design::MUTED),
                            );
                        }
                    });
                });
                if !self.error.is_empty() {
                    ui.label(&self.error);
                }
            });
            if !self.link.is_empty() && Instant::now() >= self.expires && !self.busy {
                self.send(Command::Pair);
            }
        }
        if let Some(device) = self.revoke.clone() {
            egui::Modal::new(egui::Id::new("remove-host-device")).show(ctx, |ui| {
                ui.heading("Remove paired device?");
                ui.label(&device.name);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.revoke = None;
                    }
                    if ui.button("Remove").clicked() {
                        self.revoke = None;
                        self.send(Command::Revoke(device.id.clone()));
                    }
                });
            });
        }
        back
    }
    /// Linux hosting settings: the encoder, what paired devices may do, and whether a gamepad can
    /// be offered. The screen is picked in the system's sharing prompt, so there is no display row.
    fn linux_settings_ui(&mut self, ui: &mut egui::Ui) {
        let config = self.settings["config"].clone();
        let pad_ok = self.settings["pad"]["ok"].as_bool().unwrap_or(false);
        let pad_text = self.settings["pad"]["text"].as_str().unwrap_or("Gamepad status unknown").to_string();
        ui.add_enabled_ui(self.state.is_some() && !self.busy, |ui| {
            ui.spacing_mut().item_spacing.y = 8.;
            design::group(ui, "Picture", |ui| {
                self.quality_ui(ui, &config);
                self.encoder_ui(ui, &config);
                self.setting_toggle(ui, &config, "pointer_in_picture", "Show host cursor", "");
                ui.label(egui::RichText::new("Changes apply to the next connection.").size(12.).color(design::MUTED));
            });
            design::group(ui, "Who can use it", |ui| {
                self.setting_toggle(ui, &config, "allow_files", "Receive and browse files", "Allow paired devices to send files to this computer and to browse it.");
                self.setting_toggle(ui, &config, "allow_clip", "Share clipboard", "Allow clipboard exchange with paired devices.");
                self.setting_toggle(ui, &config, "previews", "Show desktop preview thumbnail", "Your other devices and your friends see a small still of this screen. Off keeps it private.");

                if !pad_ok {ui.horizontal(|ui| {
                    design::dot(ui,design::MUTED);
                    ui.label(egui::RichText::new(pad_text).size(12.).color(design::MUTED));
                });}
            });
            design::group(ui, "Startup", |ui| {
                self.setting_toggle(ui, &config, "autostart", "Start in tray", "Open Secpar in the tray when you sign in, so this computer can be reached.");
            });
        });
    }
    /// The most a connecting device may ask for. Both hosts apply it to their own stream engine.
    fn quality_ui(&mut self, ui: &mut egui::Ui, config: &Value) {
        let width = design::value_width(ui);
        let mut quality = config["max_quality"].as_str().unwrap_or("auto").to_string();
        let old = quality.clone();
        let choices = [("auto", "No limit"), ("battery", "1280 × 720 · 60 fps"), ("smooth", "1920 × 1080 · 60 fps"), ("sharp", "2560 × 1440 · 60 fps"), ("2160p60", "3840 × 2160 · 60 fps")];
        let label = choices.iter().find(|(value, _)| *value == quality).map(|(_, label)| *label).unwrap_or("Custom");
        design::row(ui, "Quality limit", |ui| {
            egui::ComboBox::from_id_salt("host-quality").width(width).selected_text(label).show_ui(ui, |ui| {
                for (value, label) in choices { ui.selectable_value(&mut quality, value.into(), label); }
            });
        });
        if quality != old { self.send(Command::Patch(json!({"max_quality": quality}))); }
    }
    /// Computers › Paired devices: who may use this computer and who is on it now. Each row is the
    /// name, its state, then its buttons on the same line.
    pub fn paired_devices_ui(&mut self, ui: &mut egui::Ui, signed_out: bool) {
        self.pump(&ui.ctx().clone());
        let row = |ui: &mut egui::Ui, name: &str, state: &str, on: bool, buttons: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                ui.set_min_height(design::CONTROL);
                design::dot(ui, if on { design::ACCENT } else { design::MUTED });
                ui.add(egui::Label::new(name).truncate());
                ui.label(egui::RichText::new(state).size(12.).color(design::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    buttons(ui)
                });
            });
        };
        // Who is using this computer now, friends and the owner's own devices, always in view.
        if let Some(state) = self.state.clone() {
            if !state.friends.is_empty() || state.devices.iter().any(|d| d.connected) {
                design::group(ui, "Connected to this computer", |ui| {
                    ui.spacing_mut().item_spacing.y = 10.;
                    for friend in &state.friends {
                        let mut disconnect = false;
                        row(ui, &friend.name, "Friend", true, &mut |ui| {
                            disconnect = ui.add_enabled(!self.busy, egui::Button::new("Disconnect")).clicked();
                        });
                        if disconnect { self.send(Command::Disconnect(friend.id.clone())); }
                    }
                    for device in state.devices.iter().filter(|d| d.connected) {
                        let mut disconnect = false;
                        row(ui, &device.name, "Your device", true, &mut |ui| {
                            disconnect = ui.add_enabled(!self.busy, egui::Button::new("Disconnect")).clicked();
                        });
                        if disconnect { self.send(Command::Disconnect(device.id.clone())); }
                    }
                });
                ui.add_space(8.);
            }
        }
        let open_id = egui::Id::new("paired-devices-open");
        let mut open = ui.ctx().data_mut(|d| *d.get_persisted_mut_or(open_id, true));
        let count = self.state.as_ref().map_or(0, |s| s.devices.len());
        design::surface().inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 6 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.;
            let (rect, header) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.), egui::Sense::click());
            crate::icons::paint(ui, if open { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::Chevron },
                egui::Rect::from_min_size(rect.min, egui::vec2(16., 16.)), design::MUTED);
            ui.painter().text(egui::pos2(rect.min.x + 20., rect.center().y), egui::Align2::LEFT_CENTER,
                if count > 0 { format!("Paired devices ({count})") } else { "Paired devices".into() },
                egui::FontId::proportional(12.), design::MUTED);
            header.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, "Paired devices"));
            if header.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                open = !open;
                ui.ctx().data_mut(|d| d.insert_persisted(open_id, open));
            }
            ui.add_space(4.);
            if !open { return; }
            // Room between rows, so one row's buttons do not sit on the next row's.
            ui.spacing_mut().item_spacing.y = 10.;
            match self.state.clone() {
                Some(state) => {
                    if state.devices.is_empty() {
                        ui.label(egui::RichText::new("No paired devices").color(design::MUTED));
                    }
                    for device in state.devices {
                        let mut remove = false;
                        row(ui, &device.name, if device.connected { "Connected" } else { "Paired" }, device.connected, &mut |ui| {
                            remove = ui.add_enabled(!self.busy, egui::Button::new("Remove")).clicked();
                        });
                        if remove { self.revoke = Some(device.clone()); }
                    }
                }
                None => { ui.label(egui::RichText::new("Hosting unavailable").color(design::MUTED)); }
            }
            if signed_out {
                let mut pair = false;
                row(ui, "New device", "", false, &mut |ui| {
                    pair = ui.add_enabled(self.state.is_some() && !self.busy, egui::Button::new("Pair a device")).clicked();
                });
                if pair { self.pairing = true; self.link.clear(); self.qr = None; self.send(Command::Pair); }
            }
        });
    }
    fn encoder_ui(&mut self, ui:&mut egui::Ui, config:&Value) {
        let choices=self.settings["encoders"].as_array().cloned().unwrap_or_default();
        if choices.is_empty() {return;}
        let mut backend=config["encoder"]["backend"].as_str().unwrap_or("auto").to_string();
        let mut preset=config["encoder"]["preset"].as_str().unwrap_or("fast").to_string();
        let old=(backend.clone(),preset.clone());
        let width=design::value_width(ui);
        let selected=if backend=="auto" {"Automatic"} else {
            choices.iter().find(|c|c["id"].as_str()==Some(&backend)).and_then(|c|c["name"].as_str()).unwrap_or("Encoder unavailable")
        };
        design::row(ui,"Encoder",|ui| {
            egui::ComboBox::from_id_salt("host-encoder").width(width).selected_text(selected).show_ui(ui,|ui| {
                ui.selectable_value(&mut backend,"auto".into(),"Automatic");
                for choice in &choices {
                    let id=choice["id"].as_str().unwrap_or("");
                    let name=choice["name"].as_str().unwrap_or(id);
                    let enabled=choice["available"].as_bool().unwrap_or(false);
                    let response=ui.add_enabled(enabled,egui::Button::new(name).selected(backend==id));
                    if response.clicked() {backend=id.into();}
                    if !enabled {response.on_hover_text(choice["reason"].as_str().unwrap_or("Unavailable on this GPU"));}
                }
            }).response.on_hover_text("Applies to the next connection.");
        });
        let effective=if backend=="auto" {choices.iter().find(|c|c["available"].as_bool()==Some(true)).and_then(|c|c["id"].as_str()).unwrap_or("")}else{&backend};
        if cfg!(windows) && effective=="nvenc" {
            // NVIDIA's own encoder offers its whole preset range.
            let current=match preset.as_str(){"balanced"=>"p4".to_string(),"quality"=>"p7".to_string(),"fast"=>"p1".to_string(),p=>p.to_string()};
            let label=|id:&str| match id {"p1"=>"P1 · fastest".to_string(),"p7"=>"P7 · best quality".to_string(),p=>p.to_uppercase()};
            let mut chosen=current.clone();
            design::row(ui,"Encoder preset",|ui| {
                egui::ComboBox::from_id_salt("host-encoder-preset").width(width).selected_text(label(&current)).show_ui(ui,|ui| {
                    for id in ["p1","p2","p3","p4","p5","p6","p7"] {ui.selectable_value(&mut chosen,id.to_string(),label(id));}
                });
            });
            if chosen!=current {preset=chosen;}
        } else if effective!="media-foundation" {
            let labels=if effective.contains("nvenc") {["P1 · fastest","P4 · balanced","P7 · best quality"]}
                else if effective.contains("qsv") {["Very fast","Medium","Slow"]}else{["Speed","Balanced","Quality"]};
            let index=match preset.as_str(){"balanced"|"p3"|"p4"|"p5"=>1,"quality"|"p6"|"p7"=>2,_=>0};
            design::row(ui,"Encoder preset",|ui| {
                egui::ComboBox::from_id_salt("host-encoder-preset").width(width).selected_text(labels[index]).show_ui(ui,|ui| {
                    for (id,label) in ["fast","balanced","quality"].into_iter().zip(labels) {ui.selectable_value(&mut preset,id.into(),label);}
                });
            });
        }
        if old!=(backend.clone(),preset.clone()) {self.send(Command::Patch(json!({"encoder":{"backend":backend,"preset":preset}})));}
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        if cfg!(target_os = "linux") {
            return self.linux_settings_ui(ui);
        }
        let config = self.settings["config"].clone();
        ui.add_enabled_ui(self.state.is_some() && !self.busy, |ui| {
            ui.spacing_mut().item_spacing.y = 4.;
            ui.spacing_mut().item_spacing.y = 8.;
            design::group(ui, "Picture", |ui| {
                let width = design::value_width(ui);
                let mut display = config["display"].as_str().unwrap_or("").to_string();
                let old = display.clone();
                let selected = if display.is_empty() { "Primary display".to_string() } else {
                    self.settings["displays"].as_array().into_iter().flatten()
                        .find(|d| d["id"].as_str() == Some(&display))
                        .and_then(|d| d["name"].as_str()).unwrap_or("Display unavailable").to_string()
                };
                let displays = self.settings["displays"].clone();
                design::row(ui, "Display", |ui| {
                    egui::ComboBox::from_id_salt("host-display").width(width).selected_text(selected).show_ui(ui, |ui| {
                        ui.selectable_value(&mut display, String::new(), "Primary display");
                        for d in displays.as_array().into_iter().flatten() {
                            if let (Some(id), Some(name)) = (d["id"].as_str(), d["name"].as_str()) {
                                ui.selectable_value(&mut display, id.into(), name);
                            }
                        }
                    });
                });
                if display != old { self.send(Command::Patch(json!({"display":display}))); }
                self.quality_ui(ui, &config);
                self.encoder_ui(ui, &config);
                ui.label(egui::RichText::new("Changes apply to the next connection.").size(12.).color(design::MUTED));
            });
            design::group(ui, "Who can use it", |ui| {
                self.setting_toggle(ui,&config,"allow_files","Receive files","Allow paired devices to send files to this computer.");
                self.setting_toggle(ui,&config,"allow_clip","Share clipboard","Allow clipboard exchange with paired devices.");
                self.setting_toggle(ui,&config,"previews","Show desktop preview thumbnail","Your other devices and your friends see a small still of this screen. Off keeps it private.");
                if let Some(ok @ false) = self.settings["pad"]["ok"].as_bool() {
                    ui.horizontal(|ui| {
                        design::dot(ui, if ok { design::ACCENT } else { design::MUTED });
                        ui.label(egui::RichText::new(self.settings["pad"]["text"].as_str().unwrap_or("")).size(12.).color(design::MUTED));
                    });
                }
            });
            design::group(ui, "Connections and startup", |ui| {
                self.setting_toggle(ui,&config,"anywhere","Router ports","Use UPnP to request a router port for direct remote connections. Relay connections are separate.");
                self.setting_toggle(ui,&config,"autostart","Start in tray","Open Secpar in the system tray when you sign in. The hosting service runs independently.");
            });
        });
    }
    fn setting_toggle(
        &mut self,
        ui: &mut egui::Ui,
        config: &Value,
        key: &str,
        label: &str,
        help: &str,
    ) {
        let mut value = config[key].as_bool().unwrap_or(false);
        let changed = design::row(ui, label, |ui| design::switch(ui, &mut value, label));
        if changed {
            self.send(Command::Patch(json!({ key: value })));
        }
        let _ = help;
    }
}
fn paint_qr(ui: &mut egui::Ui, qr: &qrcode::QrCode) {
    // Integer-sized modules and a four-module quiet zone; never interpolate a QR.
    let modules = qr.width() + 8;
    let scale = (ui.available_width() / modules as f32).floor().max(1.);
    let size = modules as f32 * scale;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0., Color32::WHITE);
    for y in 0..qr.width() {
        for x in 0..qr.width() {
            if qr[(x, y)] == qrcode::Color::Dark {
                let min = rect.min + egui::vec2((x + 4) as f32 * scale, (y + 4) as f32 * scale);
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(min, egui::vec2(scale, scale)),
                    0.,
                    Color32::BLACK,
                );
            }
        }
    }
}
async fn snapshot() -> Result<(State, Value), String> {
    let state = serde_json::from_value(local_host::call("get_state", Value::Null).await?)
        .map_err(|_| "Invalid hosting state")?;
    let mut config = local_host::call("get_config", Value::Null).await?;
    // Friends come from the account. A PC that is not linked, or is offline, simply has none to show.
    if let Ok(rows) = local_host::call("host_friends", Value::Null).await {
        config["friend_rows"] = rows;
    }
    Ok((state, config))
}
async fn patch_config(patch: Value) -> Result<(), String> {
    let mut current = local_host::call("get_config", Value::Null).await?;
    let config = current["config"]
        .as_object_mut()
        .ok_or("Invalid hosting settings")?;
    for (key, value) in patch.as_object().ok_or("Invalid settings change")? {
        config.insert(key.clone(), value.clone());
    }
    local_host::call("set_config", json!({"config":config})).await?;
    Ok(())
}
