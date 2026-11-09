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
}
enum Event {
    State(Result<(State, Value), String>),
    Pair(Result<String, String>),
    Error(String),
}
pub struct Panel {
    tx: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    state: Option<State>,
    settings: Value,
    error: String,
    busy: bool,
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
                            Command::Refresh => Event::State(snapshot().await),
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
            settings: json!({"config":{"autostart":true,"display":"","max_quality":"auto","allow_files":true,"allow_clip":true,"friends":true,"anywhere":false},"displays":[],"pad":{"ok":true,"text":"Gamepad ready"}}),
            error: String::new(),
            busy: false,
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
    fn send(&mut self, command: Command) {
        if self.sample {
            return;
        }
        if self.tx.try_send(command).is_ok() {
            self.busy = true;
        }
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        account: &mut crate::account::Panel,
    ) {
        while let Ok(event) = self.events.try_recv() {
            self.busy = false;
            match event {
                Event::State(Ok((state, config))) => {
                    // A friend who just connected gets a notice. The first read after launch does not.
                    if let Some(old) = &self.state {
                        for f in state.friends.iter().filter(|f| !old.friends.iter().any(|o| o.id == f.id)) {
                            crate::alerts::info("Friend connected", &format!("{} is using this PC.", f.name));
                        }
                    }
                    self.state = Some(state);
                    self.settings = config;
                    self.error.clear();
                }
                Event::State(Err(e)) => {
                    self.state = None;
                    self.error = e;
                }
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
        if !self.busy && self.last_refresh.elapsed() > Duration::from_secs(5) {
            self.last_refresh = Instant::now();
            self.send(Command::Refresh);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        ui.heading("Hosting");
        ui.add_space(if self.settings_open { 0. } else { 8. });
        if !self.settings_open {
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
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.settings_open, false, "Paired devices");
            ui.selectable_value(&mut self.settings_open, true, "Settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Account").clicked() {
                    self.account_open = true;
                }
            });
        });
        ui.add_space(2.);
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.settings_open {
                self.settings_ui(ui);
            } else if let Some(state) = self.state.clone() {
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
            }
        });
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
    }
    /// Linux hosting settings: what paired devices may do, and whether a gamepad can be offered.
    /// Display choice, quality limit, router ports and tray start are not available on Linux yet.
    fn linux_settings_ui(&mut self, ui: &mut egui::Ui) {
        let config = self.settings["config"].clone();
        let pad_ok = self.settings["pad"]["ok"].as_bool().unwrap_or(false);
        let pad_text = self.settings["pad"]["text"].as_str().unwrap_or("Gamepad status unknown").to_string();
        ui.add_enabled_ui(self.state.is_some() && !self.busy, |ui| {
            ui.spacing_mut().item_spacing.y = 4.;
            design::surface().inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new("Device access").strong());
                self.setting_toggle(ui, &config, "allow_files", "Receive and browse files", "Allow paired devices to send files to this computer and to browse it.");
                self.setting_toggle(ui, &config, "allow_clip", "Allow clipboard sharing", "Allow clipboard exchange with paired devices.");
                self.setting_toggle(ui, &config, "friends", "Let friends connect", "Friends you added still need your approval unless you set them to connect at any time.");
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    design::dot(ui, if pad_ok { design::ACCENT } else { design::MUTED });
                    ui.label(egui::RichText::new(pad_text).size(12.).color(design::MUTED));
                });
            });
        });
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        if cfg!(target_os = "linux") {
            return self.linux_settings_ui(ui);
        }
        let config = self.settings["config"].clone();
        ui.add_enabled_ui(self.state.is_some() && !self.busy, |ui| {
            ui.spacing_mut().item_spacing.y = 4.;
            design::surface().inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new("Streaming").strong());
                ui.add_space(6.);
                ui.columns(2, |columns| {
                    let left = &mut columns[0];
                    left.label(egui::RichText::new("Display").size(12.).color(design::MUTED));
                    let mut display = config["display"].as_str().unwrap_or("").to_string();
                    let old = display.clone();
                    let selected = if display.is_empty() { "Primary display" } else {
                        self.settings["displays"].as_array().into_iter().flatten()
                            .find(|d| d["id"].as_str() == Some(&display))
                            .and_then(|d| d["name"].as_str()).unwrap_or("Display unavailable")
                    };
                    egui::ComboBox::from_id_salt("host-display").width(left.available_width())
                        .selected_text(selected).show_ui(left, |ui| {
                            ui.selectable_value(&mut display, String::new(), "Primary display");
                            for d in self.settings["displays"].as_array().into_iter().flatten() {
                                if let (Some(id), Some(name)) = (d["id"].as_str(), d["name"].as_str()) {
                                    ui.selectable_value(&mut display, id.into(), name);
                                }
                            }
                        });
                    if display != old { self.send(Command::Patch(json!({"display":display}))); }
                    let right = &mut columns[1];
                    right.label(egui::RichText::new("Quality limit").size(12.).color(design::MUTED));
                    let mut quality = config["max_quality"].as_str().unwrap_or("auto").to_string();
                    let old = quality.clone();
                    let choices = [("auto", "No limit"), ("battery", "1280 × 720 · 30 fps"), ("smooth", "1920 × 1080 · 60 fps"), ("sharp", "2560 × 1440 · 60 fps"), ("2160p60", "3840 × 2160 · 60 fps")];
                    let label = choices.iter().find(|(value,_)| *value == quality).map(|(_,label)| *label).unwrap_or("Custom");
                    egui::ComboBox::from_id_salt("host-quality").width(right.available_width())
                        .selected_text(label).show_ui(right, |ui| {
                            for (value,label) in choices { ui.selectable_value(&mut quality,value.into(),label); }
                        });
                    if quality != old { self.send(Command::Patch(json!({"max_quality":quality}))); }
                });
            });
            ui.add_space(4.);
            ui.columns(2, |columns| {
                design::surface().inner_margin(10).show(&mut columns[0], |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new("Device access").strong());
                    self.setting_toggle(ui,&config,"allow_files","Receive files","Allow paired devices to send files to this computer.");
                    self.setting_toggle(ui,&config,"allow_clip","Allow clipboard sharing","Allow clipboard exchange with paired devices.");
                    self.setting_toggle(ui,&config,"friends","Let friends connect","Friends you added still need your approval unless you set them to connect at any time.");
                });
                design::surface().inner_margin(10).show(&mut columns[1], |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new("Connections & startup").strong());
                    self.setting_toggle(ui,&config,"anywhere","Router ports","Use UPnP to request a router port for direct remote connections. Relay connections are separate.");
                    self.setting_toggle(ui,&config,"autostart","Start in tray","Open Secpar in the system tray when you sign in. The hosting service runs independently.");
                });
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
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 28.),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(label).on_hover_text(help);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, mut response) =
                        ui.allocate_exact_size(egui::vec2(34., 28.), egui::Sense::click());
                    if response.clicked() {
                        value = !value;
                        response.mark_changed();
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Checkbox,
                            ui.is_enabled(),
                            value,
                            label,
                        )
                    });
                    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(32., 18.));
                    let fill = if value {
                        design::ACCENT
                    } else {
                        egui::Color32::from_rgb(62, 73, 87)
                    };
                    ui.painter().rect_filled(track, 9, fill);
                    let x = if value {
                        track.right() - 9.
                    } else {
                        track.left() + 9.
                    };
                    ui.painter().circle_filled(
                        egui::pos2(x, track.center().y),
                        6.,
                        if value {
                            egui::Color32::from_rgb(7, 38, 30)
                        } else {
                            egui::Color32::from_rgb(205, 216, 226)
                        },
                    );
                    if response.has_focus() {
                        ui.painter().rect_stroke(
                            rect.expand(2.),
                            5,
                            egui::Stroke::new(1_f32, design::ACCENT),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if response.on_hover_text(help).changed() {
                        self.send(Command::Patch(json!({key:value})));
                    }
                });
            },
        );
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
    let config = local_host::call("get_config", Value::Null).await?;
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
