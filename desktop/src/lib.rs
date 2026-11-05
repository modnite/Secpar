mod account;
mod audio;
mod design;
mod input;
mod local_host;
mod vault;
use eframe::egui::{
    self, Color32, CursorGrab, Event as UiEvent, Key, TextureHandle, TextureOptions,
    ViewportCommand,
};
use openh264::{decoder::Decoder, formats::YUVSource};
use secpar_client::{
    control::{Computer, Connected},
    pairing::Pairing,
    session::{self, Command, Event, Options},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

pub fn run() -> eframe::Result {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rgba = image::load_from_memory(include_bytes!("../../assets/branding/secpar-icon.png"))
        .expect("Secpar icon")
        .to_rgba8();
    let icon = egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Secpar Desktop · Development")
            .with_app_id("com.secpar.desktop")
            .with_inner_size(review_size())
            .with_min_inner_size([480., 360.])
            .with_clamp_size_to_monitor_size(true)
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    eframe::run_native("Secpar", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
fn review_size() -> [f32; 2] {
    #[cfg(feature = "ui-review")]
    if let Ok(size) = std::env::var("SECPAR_REVIEW_SIZE") {
        if let Some((w, h)) = size.split_once('x') {
            if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                if (480.0..=1920.0).contains(&w) && (360.0..=1200.0).contains(&h) {
                    return [w, h];
                }
            }
        }
    }
    [720., 440.]
}
fn initial_status() -> String {
    #[cfg(feature = "ui-review")]
    if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p.ends_with("-sample")) {
        return "Layout sample".into();
    }
    "Ready".into()
}
fn initial_page() -> Page {
    #[cfg(feature = "ui-review")]
    if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|page| page.starts_with("friends")) {
        return Page::Friends;
    }
    Page::Computers
}
#[derive(PartialEq)]
enum Page {
    Computers,
    Friends,
}
struct App {
    account: account::Panel,
    page: Page,
    pair_link: String,
    status: String,
    command: Option<mpsc::Sender<Command>>,
    events: Option<mpsc::Receiver<Event>>,
    computer: Option<Computer>,
    decoder: Option<Decoder>,
    audio: Option<audio::Audio>,
    texture: Option<TextureHandle>,
    brand: TextureHandle,
    input: input::Input,
    dimensions: [usize; 2],
    options: Options,
    displays: Vec<(String, String)>,
    display: String,
    cursor: (i16, i16, bool),
    cursor_shape: Option<(TextureHandle, [i32; 2])>,
    fullscreen: bool,
    focused: bool,
    auto_capture_pending: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        design::configure(&cc.egui_ctx);
        let image = image::load_from_memory(include_bytes!(
            "../../assets/branding/secpar-wordmark-light.png"
        ))
        .expect("Secpar brand")
        .to_rgba8();
        let brand = cc.egui_ctx.load_texture(
            "Secpar brand",
            egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
            TextureOptions::LINEAR,
        );
        Self {
            account: account::Panel::new(cc.egui_ctx.clone()),
            page: initial_page(),
            pair_link: String::new(),
            status: initial_status(),
            command: None,
            events: None,
            computer: None,
            decoder: None,
            audio: None,
            texture: None,
            brand,
            input: Default::default(),
            dimensions: [0, 0],
            options: Options {
                width: 1280,
                height: 720,
                ..Default::default()
            },
            displays: vec![],
            display: String::new(),
            cursor: (0, 0, false),
            cursor_shape: None,
            fullscreen: false,
            focused: true,
            auto_capture_pending: false,
        }
    }
    fn pairing_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        design::surface().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Pair a computer").size(15.).strong());
            ui.add_sized(
                [ui.available_width(), 34.],
                egui::TextEdit::singleline(&mut self.pair_link)
                    .hint_text("Paste a Secpar Host pairing link")
                    .password(true)
                    .margin(egui::vec2(10., 8.)),
            );
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Resolution")
                        .size(12.)
                        .color(design::MUTED),
                );
                egui::ComboBox::from_id_salt("resolution")
                    .width(142.)
                    .selected_text(if self.options.width == 0 {
                        "Host native".into()
                    } else {
                        format!("{} × {}", self.options.width, self.options.height)
                    })
                    .show_ui(ui, |ui| {
                        for (w, h) in [
                            (0, 0),
                            (1280, 720),
                            (1920, 1080),
                            (2560, 1440),
                            (3840, 2160),
                        ] {
                            let label = if w == 0 {
                                "Host native".into()
                            } else {
                                format!("{w} × {h}")
                            };
                            if ui
                                .selectable_label(self.options.width == w, label)
                                .clicked()
                            {
                                self.options.width = w;
                                self.options.height = h;
                            }
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !self.pair_link.trim().is_empty(),
                            design::primary("Pair and connect"),
                        )
                        .clicked()
                    {
                        self.start(ctx, false);
                    }
                });
            });
        });
    }
    fn start(&mut self, ctx: &egui::Context, reconnect: bool) {
        if self.command.is_some() {
            return;
        }
        let pairing = if reconnect {
            None
        } else {
            match Pairing::parse(&self.pair_link) {
                Ok(p) => Some(p),
                Err(e) => {
                    self.status = e;
                    return;
                }
            }
        };
        let computer = self.computer.clone();
        self.audio = audio::Audio::new().ok();
        let options = Options {
            audio: self.audio.is_some(),
            ..self.options.clone()
        };
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel(256);
        let (ev_tx, ev_rx) = mpsc::channel(8);
        self.command = Some(tx);
        self.events = Some(ev_rx);
        self.status = "Connecting".into();
        self.auto_capture_pending = true;
        self.decoder = None;
        self.texture = None;
        self.displays.clear();
        self.cursor_shape = None;
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(_) => {
                    let _ = ev_tx.blocking_send(Event::Ended(
                        "Could not start the connection worker".into(),
                    ));
                    ctx.request_repaint();
                    return;
                }
            };
            runtime.block_on(async move {
                let connection = if let Some(p) = pairing {
                    Connected::pair(p, "Secpar Desktop").await
                } else if let Some(c) = computer {
                    Connected::reconnect(c, "Secpar Desktop").await
                } else {
                    Err("Pair a computer first".into())
                };
                let result = match connection {
                    Ok(c) => session::run(c, options, rx, ev_tx.clone()).await,
                    Err(e) => Err(e),
                };
                let _ = ev_tx
                    .send(Event::Ended(result.err().unwrap_or("Disconnected".into())))
                    .await;
                ctx.request_repaint();
            });
        });
    }
    fn send(&mut self, command: Command) {
        if self
            .command
            .as_ref()
            .is_some_and(|tx| tx.try_send(command).is_err())
        {
            // Never silently drop a key-up: close the session so the host releases its held inputs.
            self.command = None;
            self.status = "Input queue stopped; disconnected to release remote keys".into();
        }
    }
    fn capture(&mut self, ctx: &egui::Context, on: bool) {
        self.auto_capture_pending = false;
        if on && self.texture.is_some() && self.command.is_some() {
            self.input.capture();
            ctx.send_viewport_cmd(ViewportCommand::CursorGrab(CursorGrab::Locked));
            ctx.send_viewport_cmd(ViewportCommand::CursorVisible(false));
        } else {
            for event in self.input.release() {
                self.send(Command::Input(event));
            }
            ctx.send_viewport_cmd(ViewportCommand::CursorGrab(CursorGrab::None));
            ctx.send_viewport_cmd(ViewportCommand::CursorVisible(true));
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let mut messages = vec![];
        if let Some(rx) = self.events.as_mut() {
            for _ in 0..8 {
                let Ok(event) = rx.try_recv() else {
                    break;
                };
                messages.push(event);
            }
        }
        for event in messages {
            match event {
                Event::Paired(c) => {
                    self.account.remember(c.clone());
                    self.computer = Some(c);
                    self.pair_link.clear();
                }
                Event::Status(s) => {
                    self.status = if self.audio.is_some() {
                        s
                    } else {
                        format!("{s} · no audio output")
                    }
                }
                Event::Audio(packet) => {
                    if let Some(audio) = self.audio.as_mut() {
                        audio.packet(&packet);
                    }
                }
                Event::Displays(v) => {
                    self.display = v["current"].as_str().unwrap_or("").into();
                    self.displays = v["list"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|d| {
                                    Some((d["id"].as_str()?.into(), d["name"].as_str()?.into()))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                }
                Event::CursorShape { png, hotspot } => {
                    if let Ok(mut reader) =
                        image::ImageReader::new(std::io::Cursor::new(png)).with_guessed_format()
                    {
                        let mut limits = image::Limits::default();
                        limits.max_image_width = Some(512);
                        limits.max_image_height = Some(512);
                        limits.max_alloc = Some(4 * 1024 * 1024);
                        reader.limits(limits);
                        if let Ok(image) = reader.decode() {
                            let rgba = image.to_rgba8();
                            let (w, h) = (rgba.width(), rgba.height());
                            let texture = ctx.load_texture(
                                "Remote cursor",
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    rgba.as_raw(),
                                ),
                                TextureOptions::LINEAR,
                            );
                            self.cursor_shape = Some((
                                texture,
                                [hotspot[0].clamp(0, w as i32), hotspot[1].clamp(0, h as i32)],
                            ));
                        }
                    }
                }
                Event::Cursor { x, y, visible } => self.cursor = (x, y, visible),
                Event::Ended(s) => {
                    self.capture(ctx, false);
                    self.command = None;
                    self.events = None;
                    self.status = s;
                    self.decoder = None;
                    self.audio = None;
                    self.texture = None;
                    self.cursor = (0, 0, false);
                    self.cursor_shape = None;
                }
                Event::Video(frame) => {
                    if self.decoder.is_none() {
                        self.decoder = Decoder::new().ok();
                    }
                    let Some(decoder) = self.decoder.as_mut() else {
                        self.status = "H.264 decoder could not start".into();
                        self.send(Command::Stop);
                        continue;
                    };
                    match decoder.decode(&frame.data) {
                        Ok(Some(yuv)) => {
                            let (w, h) = yuv.dimensions();
                            if w == 0 || h == 0 || w > 8192 || h > 8192 || w * h > 4096 * 2160 {
                                self.status =
                                    "Video dimensions exceed this preview's decoder limit".into();
                                self.send(Command::Stop);
                                continue;
                            }
                            let mut pixels = vec![0; w * h * 3];
                            yuv.write_rgb8(&mut pixels);
                            let image = egui::ColorImage::from_rgb([w, h], &pixels);
                            self.dimensions = [w, h];
                            if let Some(texture) = self.texture.as_mut() {
                                texture.set(image, TextureOptions::LINEAR);
                            } else {
                                self.texture = Some(ctx.load_texture(
                                    "Remote desktop",
                                    image,
                                    TextureOptions::LINEAR,
                                ));
                            }
                        }
                        Ok(None) => {}
                        Err(_) => {
                            self.send(Command::Keyframe);
                        }
                    }
                    if self.auto_capture_pending && self.focused && self.texture.is_some() {
                        self.capture(ctx, true);
                    }
                }
            }
        }
    }
}
impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.focused = raw.focused;
        if !raw.focused {
            self.capture(ctx, false);
            return;
        }
        let toggle=raw.events.iter().any(|event|matches!(event,UiEvent::Key{key:Key::F12,pressed:true,repeat:false,modifiers,..} if modifiers.ctrl));
        if toggle {
            self.capture(ctx, !self.input.captured);
            raw.events
                .retain(|e| !matches!(e, UiEvent::Key { key: Key::F12, .. }));
        }
        for m in self.input.modifiers(raw.modifiers) {
            self.send(Command::Input(m));
        }
        if self.input.captured {
            for event in std::mem::take(&mut raw.events) {
                for m in self.input.event(&event) {
                    self.send(Command::Input(m));
                }
            }
        }
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll(ctx);
        self.account.poll();
        if std::mem::take(&mut self.account.forgot_computer) {
            self.computer = None;
        }
        if let Some(note) = self.account.storage_note.take() {
            self.status = note;
        }
        if let Some(computer) = self.account.restored_computer.take() {
            if self.computer.is_none() {
                self.computer = Some(computer);
            }
        }
        if self.command.is_none() && self.input.captured {
            self.capture(ctx, false);
        }
        if !(self.fullscreen && self.input.captured) {
            egui::TopBottomPanel::top("toolbar")
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(22, 27, 35))
                        .inner_margin(egui::Margin::symmetric(20, 8)),
                )
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.image((self.brand.id(), egui::vec2(112., 27.)));
                        if self.command.is_none() {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(&self.status)
                                            .size(12.)
                                            .color(design::MUTED),
                                    );
                                    design::dot(ui, design::ACCENT);
                                },
                            );
                        } else {
                            ui.label(&self.status);
                        }
                        if self.command.is_some() {
                            if ui.button("Disconnect").clicked() {
                                self.capture(ctx, false);
                                self.send(Command::Stop);
                            }
                            if self.texture.is_some()
                                && ui
                                    .button(if self.input.captured {
                                        "Release · Ctrl+F12"
                                    } else {
                                        "Capture · Ctrl+F12"
                                    })
                                    .clicked()
                            {
                                self.capture(ctx, !self.input.captured);
                            }
                            if ui
                                .button(if self.fullscreen {
                                    "Window"
                                } else {
                                    "Fullscreen"
                                })
                                .clicked()
                            {
                                self.fullscreen = !self.fullscreen;
                                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
                            }
                            if self.displays.len() > 1 {
                                let mut selected = self.display.clone();
                                egui::ComboBox::from_id_salt("display")
                                    .selected_text("Display")
                                    .show_ui(ui, |ui| {
                                        for (id, name) in &self.displays {
                                            ui.selectable_value(&mut selected, id.clone(), name);
                                        }
                                    });
                                if selected != self.display {
                                    self.send(Command::Display(selected.clone()));
                                    self.display = selected;
                                }
                            }
                        }
                    });
                });
        }
        if self.command.is_none() {
            egui::TopBottomPanel::top("navigation")
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(22, 27, 35))
                        .inner_margin(egui::Margin::symmetric(20, 0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for (page, label) in
                            [(Page::Computers, "Computers"), (Page::Friends, "Friends")]
                        {
                            let selected = self.page == page;
                            let response = ui.add_sized(
                                [100., 36.],
                                egui::Button::new(egui::RichText::new(label).color(if selected {
                                    Color32::WHITE
                                } else {
                                    design::MUTED
                                }))
                                .frame(false),
                            );
                            if selected {
                                let r = response.rect;
                                ui.painter().line_segment(
                                    [egui::pos2(r.left(), r.bottom()), r.right_bottom()],
                                    egui::Stroke::new(2_f32, design::ACCENT),
                                );
                            }
                            if response.clicked() {
                                self.page = page;
                            }
                        }
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(17, 21, 28))
                    .inner_margin(if self.command.is_none() {
                        egui::Margin::symmetric(20, 16)
                    } else {
                        egui::Margin::same(0)
                    }),
            )
            .show(ctx, |ui| {
                if let Some(texture) = &self.texture {
                    let available = ui.available_size();
                    let size = egui::vec2(self.dimensions[0] as f32, self.dimensions[1] as f32);
                    let scale = (available.x / size.x).min(available.y / size.y);
                    let display = size * scale;
                    let (_, rect) = ui.allocate_space(available);
                    let picture = egui::Rect::from_center_size(rect.center(), display);
                    ui.painter().rect_filled(rect, 0., Color32::BLACK);
                    ui.painter().image(
                        texture.id(),
                        picture,
                        egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                    if self.cursor.2 {
                        let p = picture.min
                            + egui::vec2(self.cursor.0 as f32, self.cursor.1 as f32) * scale;
                        if picture.contains(p) {
                            if let Some((texture, hotspot)) = &self.cursor_shape {
                                let min =
                                    p - egui::vec2(hotspot[0] as f32, hotspot[1] as f32) * scale;
                                let rect =
                                    egui::Rect::from_min_size(min, texture.size_vec2() * scale);
                                ui.painter().with_clip_rect(picture).image(
                                    texture.id(),
                                    rect,
                                    egui::Rect::from_min_max(
                                        egui::pos2(0., 0.),
                                        egui::pos2(1., 1.),
                                    ),
                                    Color32::WHITE,
                                );
                            } else {
                                ui.painter().circle_filled(p, 4., Color32::WHITE);
                                ui.painter().circle_stroke(
                                    p,
                                    4.,
                                    egui::Stroke::new(1_f32, Color32::BLACK),
                                );
                            }
                        }
                    }
                } else if self.command.is_none() {
                    if self.page == Page::Friends {
                        self.account.ui(ui);
                    } else {
                        ui.heading("Computers");
                        ui.add_space(8.);
                        if let Some(computer) = &self.computer {
                            let name = computer.name.clone();
                            design::surface().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    design::dot(ui, design::ACCENT);
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(name).strong())
                                            .truncate(),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.menu_button("…", |ui| {
                                                if ui.button("Forget on this device").clicked() {
                                                    self.account.forget_computer();
                                                    ui.close();
                                                }
                                            });
                                            if ui.add(design::primary("Connect")).clicked() {
                                                self.start(ctx, true);
                                            }
                                        },
                                    );
                                });
                            });
                            ui.add_space(8.);
                            egui::CollapsingHeader::new("Pair another computer")
                                .show(ui, |ui| self.pairing_ui(ui, ctx));
                        } else {
                            self.pairing_ui(ui, ctx);
                        }
                        ui.add_space(12.);
                        self.account.host_ui(ui);
                    }
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label("Connecting to the remote desktop…");
                    });
                }
            });
        if self.command.is_some() {
            ctx.request_repaint_after(Duration::from_millis(8));
        }
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        for m in self.input.release() {
            self.send(Command::Input(m));
        }
        self.send(Command::Stop);
        self.command = None;
    }
}

#[cfg(test)]
mod video_tests {
    use super::*;
    #[test]
    fn decodes_real_h264_to_the_rgb_texture_format() {
        use openh264::{
            encoder::Encoder,
            formats::{RgbSliceU8, YUVBuffer},
        };
        let rgb: Vec<u8> = (0..128 * 128).flat_map(|_| [40, 200, 20]).collect();
        let source = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (128, 128)));
        let mut encoder = Encoder::new().unwrap();
        let packet = encoder.encode(&source).unwrap().to_vec();
        let mut decoder = Decoder::new().unwrap();
        let yuv = decoder.decode(&packet).unwrap().unwrap();
        assert_eq!(yuv.dimensions(), (128, 128));
        let mut decoded = vec![0; 128 * 128 * 3];
        yuv.write_rgb8(&mut decoded);
        for (expected, actual) in [40i16, 200, 20].into_iter().zip(decoded[..3].iter()) {
            assert!((expected - *actual as i16).abs() < 16);
        }
    }
}
