//! Another display of the connected computer in its own window. The main window starts this
//! program again with `--view` and passes the computer on standard input, never on the command
//! line. This window streams one display, takes mouse and keyboard, and closes when the stream
//! ends. Sound, controllers and the clipboard stay with the main window.

use crate::{gpu, input, picture, video};
use eframe::egui::{self, Color32, TextureOptions};
use secpar_client::{
    control::{Computer, Connected},
    session::{self, Command, Event, Options},
};
use std::io::{Read, Write};
use tokio::sync::mpsc;

#[derive(serde::Serialize, serde::Deserialize)]
struct Request {
    computer: Computer,
    display: String,
    name: String,
    width: u32,
    height: u32,
    fps: u32,
    max_kbps: u32,
    #[serde(default)]
    chroma444: bool,
}

/// Opens `display` (an id from `stream.displays`) of `computer` in a new window.
pub fn open(computer: &Computer, display: &str, name: &str, options: &Options) -> std::io::Result<()> {
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("--view")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    let request = Request {
        computer: computer.clone(), display: display.into(), name: name.into(),
        width: options.width, height: options.height, fps: options.fps, max_kbps: options.max_kbps,
        chroma444: options.chroma444,
    };
    let mut stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no input pipe"))?;
    stdin.write_all(&serde_json::to_vec(&request).map_err(std::io::Error::other)?)?;
    Ok(())
}

pub fn run() -> eframe::Result {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut text = String::new();
    let Some(request) = std::io::stdin().read_to_string(&mut text).ok().and_then(|_| serde_json::from_str::<Request>(&text).ok()) else {
        return Ok(());
    };
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{} · {}", request.computer.name, request.name))
            .with_app_id("com.secpar.desktop")
            .with_inner_size([960., 600.])
            .with_min_inner_size([320., 200.]),
        ..Default::default()
    };
    #[cfg(windows)]
    gpu::choose_renderer(&mut options);
    eframe::run_native("Secpar display", options, Box::new(|cc| Ok(Box::new(View::new(cc, request)))))
}

struct View {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    route: video::Route,
    decoder: video::Worker,
    gpu: Option<gpu::Handle>,
    texture: Option<egui::TextureHandle>,
    size: [usize; 2],
    input: input::Input,
    status: String,
    full_colour: bool,
}

impl View {
    fn new(cc: &eframe::CreationContext<'_>, r: Request) -> Self {
        crate::design::configure(&cc.egui_ctx);
        let route = video::Route::default();
        let decoder = video::Worker::with_frame_rate(cc.egui_ctx.clone(), video::Preference::default(), r.fps);
        route.output(Some(&decoder));
        let options = Options {
            width: r.width, height: r.height, fps: r.fps, max_kbps: r.max_kbps,
            audio: false, clipboard: false, display: r.display, chroma444: r.chroma444,
            video_sink: Some(route.sink()),
            ..Options::default()
        };
        let (commands, rx) = mpsc::channel(256);
        let (ev_tx, events) = mpsc::channel(8);
        let ctx = cc.egui_ctx.clone();
        let computer = r.computer;
        std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().enable_all().build() else { return };
            runtime.block_on(async move {
                let result = match Connected::reconnect(computer, &crate::device_name()).await {
                    Ok(c) => session::run(c, options, rx, ev_tx.clone()).await,
                    Err(e) => Err(e),
                };
                let _ = ev_tx.send(Event::Ended(result.err().unwrap_or_else(|| "Disconnected".into()))).await;
                ctx.request_repaint();
            });
        });
        let mut input = input::Input::default();
        input.free();
        View { commands, events, route, decoder, gpu: gpu::new(cc), texture: None, size: [0, 0], input, status: "Connecting".into(), full_colour: false }
    }

    fn send(&self, m: serde_json::Value) {
        let _ = self.commands.try_send(Command::Input(m));
    }
}

impl eframe::App for View {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Status(s) => self.status = s,
                Event::FullColour(on) => self.full_colour = on,
                Event::Ended(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                _ => {}
            }
        }
        let output = self.decoder.output();
        if output.request_key { let _ = self.commands.try_send(Command::Keyframe); }
        if let Some(error) = output.error { self.status = error; }
        if let Some(mut p) = output.image {
            p.packed = self.full_colour && p.size[1] % 2 == 0;
            self.size = p.shown_size();
            let image = match self.gpu.clone().filter(|h| !gpu::failed(h)) {
                Some(handle) => { gpu::submit(&handle, p); egui::ColorImage::new([1, 1], vec![Color32::BLACK]) }
                None => p.to_color_image(picture::Matrix::Bt709),
            };
            match self.texture.as_mut() {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("Remote display", image, TextureOptions::LINEAR)),
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ctx, |ui| {
            let Some(texture) = &self.texture else {
                ui.centered_and_justified(|ui| ui.label(&self.status));
                return;
            };
            let available = ui.available_size();
            let size = egui::vec2(self.size[0].max(1) as f32, self.size[1].max(1) as f32);
            let shown = size * (available.x / size.x).min(available.y / size.y);
            let (_, rect) = ui.allocate_space(available);
            let picture = egui::Rect::from_center_size(rect.center(), shown);
            match self.gpu.clone().filter(|h| !gpu::failed(h)) {
                Some(handle) => { ui.painter().add(gpu::callback(handle, picture)); }
                None => { ui.painter().image(texture.id(), picture, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), Color32::WHITE); }
            }
            // Keys and buttons go to the computer while this window is in front; the pointer
            // follows the mouse over the picture.
            let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
            let events = ctx.input(|i| i.events.clone());
            if focused {
                for e in &events {
                    if let egui::Event::PointerMoved(pos) = e {
                        if let Some(m) = input::Input::absolute(*pos, picture) { self.send(m); }
                    } else {
                        for m in self.input.event(e) { self.send(m); }
                    }
                }
            } else {
                for m in self.input.release() { self.send(m); }
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        for m in self.input.release() { self.send(m); }
        let _ = self.commands.try_send(Command::Stop);
        self.route.output(None);
    }
}
