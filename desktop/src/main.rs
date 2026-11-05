#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod audio;
mod input;
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

fn main() -> eframe::Result {
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
            .with_inner_size([1100., 720.])
            .with_min_inner_size([640., 400.])
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    eframe::run_native("Secpar", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
struct App {
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
    fullscreen: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::from_rgb(21, 22, 26);
        style.visuals.window_fill = Color32::from_rgb(28, 29, 33);
        style.visuals.selection.bg_fill = Color32::from_rgb(0, 120, 90);
        cc.egui_ctx.set_style(style);
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
            pair_link: String::new(),
            status: "Ready".into(),
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
            fullscreen: false,
        }
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
        self.decoder = None;
        self.texture = None;
        self.displays.clear();
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
                            if w == 0 || h == 0 || w > 4096 || h > 4096 || w * h > 4096 * 2160 {
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
                }
            }
        }
    }
}
impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
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
        if self.command.is_none() && self.input.captured {
            self.capture(ctx, false);
        }
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.image((self.brand.id(), egui::vec2(112., 27.)));
                ui.separator();
                ui.label(&self.status);
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
        egui::CentralPanel::default().show(ctx, |ui| {
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
                        ui.painter().circle_filled(p, 4., Color32::WHITE);
                        ui.painter()
                            .circle_stroke(p, 4., egui::Stroke::new(1_f32, Color32::BLACK));
                    }
                }
            } else if self.command.is_none() {
                ui.heading("Connect to a computer");
                ui.add_space(8.);
                ui.label("Paste the pairing link from Secpar Host.");
                ui.add(
                    egui::TextEdit::multiline(&mut self.pair_link)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("secpar://pair?…")
                        .password(true),
                );
                ui.horizontal(|ui| {
                    if ui.button("Pair and connect").clicked() {
                        self.start(ctx, false);
                    }
                    if self.computer.is_some() && ui.button("Reconnect").clicked() {
                        self.start(ctx, true);
                    }
                });
                ui.add_space(12.);
                egui::ComboBox::from_label("Resolution")
                    .selected_text(format!("{} × {}", self.options.width, self.options.height))
                    .show_ui(ui, |ui| {
                        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440)] {
                            let selected = self.options.width == w;
                            if ui
                                .selectable_label(selected, format!("{w} × {h}"))
                                .clicked()
                            {
                                self.options.width = w;
                                self.options.height = h;
                            }
                        }
                    });
                ui.add_space(12.);
                ui.small("Development client · software H.264 decoding");
                ui.small(
                    "Pairings are kept in memory for this preview. Close the app to forget them.",
                );
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
