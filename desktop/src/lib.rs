mod account;
mod signin;
mod friend_previews;
mod alerts;
pub use account::answer_link;
mod card;
mod awake;
use std::time::Instant;
mod client_settings;
mod video;
mod picture;
mod previews;
mod gpu;
mod audio;
mod audio_worker;
mod design;
mod dialogs;
mod files;
pub mod gamepad;
mod hosting;
mod input;
mod keys;
#[cfg(windows)]
mod keyhook;
mod stream_menu;
mod icons;
mod local_host;
#[cfg(windows)]
mod native_window;
#[cfg(all(windows, feature="tray-review"))]
mod tray_review;
#[cfg(all(windows, feature="tray-review"))]
pub use tray_review::run as run_tray_review;
#[cfg(windows)]
mod tray;
mod updates;
mod vault;
use eframe::egui::{
    self, Color32, CursorGrab, Event as UiEvent, Key, TextureHandle, TextureOptions,
    ViewportCommand,
};
#[cfg(test)]
use openh264::{decoder::Decoder, formats::YUVSource};
use secpar_client::{
    control::{Computer, Connected},
    pairing::Pairing,
    session::{self, Command, Event, Options},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

pub(crate) fn device_name() -> String {
    let name=std::env::var("COMPUTERNAME").or_else(|_|std::env::var("HOSTNAME")).ok();
    #[cfg(unix)]
    let name=name.or_else(||std::fs::read_to_string("/etc/hostname").ok());
    let name=name.unwrap_or_else(||"Secpar Desktop".into());
    let name:String=name.trim().chars().filter(|c|!c.is_control()).take(40).collect();
    if name.is_empty() { "Secpar Desktop".into() } else { name }
}

pub(crate) const START_FAILED_TITLE: &str = "Could not start the screen";

/// Plain-language text for the host's stream errors. The desktop only receives the host's message text,
/// so this matches on it. Unknown messages stay as they are.
pub(crate) fn friendly_error(raw: &str) -> Option<&'static str> {
    let text = raw.to_ascii_lowercase();
    if text.contains("connection limit") {
        Some("Connection limit reached. Three devices are already streaming. Disconnect a device and try again.")
    } else if text.contains("encoder") {
        Some("This computer has no hardware video encoder available. Update its graphics driver and try again.")
    } else if text.contains("capture") {
        Some("This computer could not capture its screen. If a login or admin prompt is showing, answer it there and retry.")
    } else if text.contains("busy") || text.contains("already streaming") {
        Some("The computer is busy. Try again or update Secpar on it.")
    } else if text.contains("decoder") {
        Some("This computer could not decode the video from the other computer.")
    } else {
        None
    }
}

fn connecting_text(name: Option<&str>) -> String {
    format!("Connecting to {}…", name.filter(|n| !n.is_empty()).unwrap_or("the computer"))
}

#[cfg(test)]
mod text_tests {
    use super::*;
    #[test]
    fn maps_host_errors_to_plain_text() {
        assert!(friendly_error("Connection limit reached: 3 devices are already streaming. Disconnect a device and try again.").unwrap().starts_with("Connection limit reached."));
        assert!(friendly_error("No hardware encoder").unwrap().contains("video encoder"));
        assert!(friendly_error("Screen capture failed").unwrap().contains("capture its screen"));
        assert!(friendly_error("This device is already streaming").unwrap().contains("busy"));
        assert!(friendly_error("no decoder").unwrap().contains("decode"));
        assert_eq!(friendly_error("Host ended the stream"), None);
        for t in ["Connection limit","encoder","capture","busy","decoder"] {
            let m = friendly_error(t).unwrap();
            assert!(!m.contains("PC") && !m.contains("Couldn't"));
        }
    }
    #[test]
    fn connecting_names_the_computer() {
        assert_eq!(connecting_text(Some("Office")), "Connecting to Office…");
        assert_eq!(connecting_text(None), "Connecting to the computer…");
        assert_eq!(connecting_text(Some("")), "Connecting to the computer…");
    }
}

/// Explorer file offers use the same hosting service without opening a second UI.
pub fn offer_file(path: String) -> Result<(), String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start the file request")?
        .block_on(local_host::call("offer", serde_json::json!({"path":path})))?;
    Ok(())
}

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
            .with_title("Secpar")
            .with_visible(!std::env::args().any(|arg| arg == "--hidden"))
            .with_app_id("com.secpar.desktop")
            .with_inner_size(review_size())
            .with_min_inner_size([480., 360.])
            .with_clamp_size_to_monitor_size(true)
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    eframe::run_native("Secpar", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
fn new_files(ctx: egui::Context, dir: &str) -> files::Panel {
    #[cfg(feature = "ui-review")]
    if let Ok(page) = std::env::var("SECPAR_REVIEW_PAGE") {
        if std::env::var_os("SECPAR_REVIEW_SCREENSHOT_TO").is_some() && page.starts_with("files") {
            if let Ok(link) = std::env::var("SECPAR_REVIEW_FILES_LINK") {
                let script = std::env::var("SECPAR_REVIEW_FILES_SCRIPT").unwrap_or_default();
                return files::Panel::live(ctx, &link, &script, &std::env::var("SECPAR_REVIEW_FILES_SAVE").unwrap_or_default());
            }
            return files::Panel::sample(ctx, &page);
        }
    }
    files::Panel::new(ctx, Some(dir))
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
    [DEFAULT_SIZE.0, DEFAULT_SIZE.1]
}
/// Big enough for six other computers, some with previews, without a scrollbar.
const DEFAULT_SIZE: (f32, f32) = (720., 660.);
fn initial_status() -> String {
    #[cfg(feature = "ui-review")]
    if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p.ends_with("-sample")) {
        return "Layout sample".into();
    }
    "Ready".into()
}
fn initial_page() -> Page {
    #[cfg(feature = "ui-review")]
    if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|page| page.starts_with("hosting")) {
        return Page::Hosting;
    }
    #[cfg(feature = "ui-review")]
    if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|page| page.starts_with("friends")) {
        return Page::Friends;
    }
    #[cfg(feature="ui-review")]
    if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.starts_with("client-settings")) {return Page::Settings;}
    #[cfg(feature="ui-review")]
    if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.starts_with("files")) {return Page::Files;}
    Page::Computers
}
#[derive(PartialEq)]
enum Page {
    Computers,
    Files,
    Hosting,
    Friends,
    Settings,
}
struct App {
    #[cfg(feature = "ui-review")]
    review_frames: u32,
    #[cfg(feature = "ui-review")]
    review_shot: bool,
    #[cfg(feature="ui-review")]
    review_commands: Option<mpsc::Receiver<Command>>,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    updates: updates::Panel,
    client: client_settings::Settings,
    shortcut_key: Option<Key>,
    hosting: hosting::Panel,
    account: account::Panel,
    files: files::Panel,
    page: Page,
    status: String,
    command: Option<mpsc::Sender<Command>>,
    events: Option<mpsc::Receiver<Event>>,
    computer: Option<Computer>,
    computer_account: Option<String>,
    session_account: Option<String>,
    owned_connecting: Option<(String,String)>,
    /// A friend's computer is open. Nothing about it is remembered.
    guest_session: bool,
    guest_error: bool,
    decoder: Option<video::Worker>,
    audio: Option<audio_worker::Worker>,
    audio_route:audio_worker::Route,
    audio_error: Option<String>,
    clipboard: Option<secpar_clipboard::Bridge>,
    notice: Option<String>,
    connection_error: Option<String>,
    texture: Option<TextureHandle>,
    gpu: Option<gpu::Handle>,
    gpu_live: bool,
    brand: TextureHandle,
    mark: TextureHandle,
    stream_menu_open: bool,
    menu_page: u8,
    show_stream_stats: bool,
    /// Held while a stream is connected: the screen must not dim or sleep.
    awake: Option<awake::KeepAwake>,
    awake_tried: bool,
    menu_button_hidden: bool,
    /// A live resize was asked for and the host has not answered with a new size yet.
    resize_wait: Option<(Instant, (u32, u32))>,
    stream_menu_bounds: Option<egui::Rect>,
    stream_menu_button_bounds: Option<egui::Rect>,
    brand_scale: f32,
    input: input::Input,
    dimensions: [usize; 2],
    stream_fps: u32,
    stream_stats: Option<(Option<u16>,f32,f32)>,
    options: Options,
    displays: Vec<(String, String)>,
    display: String,
    cursor: (i16, i16, bool),
    cursor_shape: Option<(TextureHandle, [i32; 2])>,
    fs: keys::Fullscreen,
    focused: bool,
    pads: gamepad::Runner,
    /// A pairing link being typed, and whether its box is open. Only used without an account.
    pair_link: String,
    pair_open: bool,
    pad_unsupported: bool,
    auto_capture_pending: bool,
    video_rect: Option<egui::Rect>,
    video_layer: Option<egui::LayerId>,
    remote_keyboard: bool,
    pointer_in_video: bool,
    observed_fullscreen: Option<bool>,
    #[cfg(windows)]
    hook: Option<keyhook::KeyHook>,
    #[cfg(windows)]
    native: Option<native_window::NativeWindow>,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        design::configure(&cc.egui_ctx);
        let _ = video::hardware_available(&cc.egui_ctx);
        let brand_scale = cc.egui_ctx.pixels_per_point();
        let image = image::load_from_memory(include_bytes!(
            "../../assets/branding/secpar-wordmark-light.png"
        ))
        .expect("Secpar brand")
        .resize_exact(
            (112. * brand_scale).round().max(1.) as u32,
            (27. * brand_scale).round().max(1.) as u32,
            image::imageops::FilterType::Lanczos3,
        ).to_rgba8();
        let brand = cc.egui_ctx.load_texture(
            "Secpar brand",
            egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
            TextureOptions::LINEAR,
        );
        let client=client_settings::Settings::load();
        let client_files_dir=client.files_dir.clone();
        let options=client.options();
        let icon=image::load_from_memory(include_bytes!("../../assets/branding/secpar-icon.png")).expect("Secpar icon")
            .resize_exact((24.*brand_scale).round().max(1.) as u32,(24.*brand_scale).round().max(1.) as u32,image::imageops::FilterType::Lanczos3).to_rgba8();
        let mark=cc.egui_ctx.load_texture("Secpar menu",egui::ColorImage::from_rgba_unmultiplied(
            [icon.width() as usize,icon.height() as usize],icon.as_raw()),TextureOptions::LINEAR);
        #[allow(unused_mut)]
        let mut app=Self {
            client,
            shortcut_key: None,
            #[cfg(feature = "ui-review")]
            review_frames: 0,
            #[cfg(feature = "ui-review")]
            review_shot: false,
            #[cfg(feature="ui-review")]
            review_commands: None,
            #[cfg(windows)]
            tray: tray::Tray::new(cc),
            updates: updates::Panel::new(),
            hosting: hosting::Panel::new(cc.egui_ctx.clone()),
            account: account::Panel::new(cc.egui_ctx.clone()),
            files: new_files(cc.egui_ctx.clone(), &client_files_dir),
            page: initial_page(),
            status: initial_status(),
            command: None,
            events: None,
            computer: None,
            computer_account: None,
            session_account: None,
            owned_connecting: None,
            guest_session: false,
            guest_error: false,
            decoder: None,
            audio: None,
            audio_route:audio_worker::Route::default(),
            audio_error: None,
            clipboard: None,
            notice: None,
            connection_error: None,
            texture: None,
            gpu: cc.gl.as_ref().and_then(gpu::new),
            gpu_live: false,
            brand,
            mark,
            stream_menu_open: false,
            menu_page: 0,
            show_stream_stats: false,
            awake: None,
            awake_tried: false,
            menu_button_hidden: false,
            resize_wait: None,
            stream_menu_bounds: None,
            stream_menu_button_bounds: None,
            brand_scale,
            input: Default::default(),
            dimensions: [0, 0],
            stream_fps: 60,
            stream_stats: None,
            options,
            displays: vec![],
            display: String::new(),
            cursor: (0, 0, false),
            cursor_shape: None,
            fs: Default::default(),
            focused: true,
            pads: Default::default(),
            pair_link: String::new(),
            pair_open: false,
            pad_unsupported: false,
            auto_capture_pending: false,
            video_rect: None, video_layer: None, remote_keyboard: false, pointer_in_video: false,
            observed_fullscreen: None,
            #[cfg(windows)]
            hook: None,
            #[cfg(windows)]
            native: native_window::NativeWindow::from_context(cc),
        };
        #[cfg(feature="ui-review")]
        if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.starts_with("stream")) {
            let (tx,rx)=mpsc::channel(256);app.command=Some(tx);app.review_commands=Some(rx);
            let image=egui::ColorImage::new([640,360],vec![Color32::from_rgb(37,48,56);640*360]);
            app.texture=Some(cc.egui_ctx.load_texture("Stream sample",image,TextureOptions::LINEAR));
            app.dimensions=[1280,720];app.stream_fps=60;app.status="Direct".into();
            app.client.active_decoder=Some("H.264 · software".into());app.stream_stats=Some((Some(2),18.4,60.));
            if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("bars")) {
                if let Some(handle)=&app.gpu {gpu::submit(handle,picture::Picture::bars(1280,720));app.gpu_live=true;}
            }
            app.stream_menu_open=std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("menu"));
            app.show_stream_stats=std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("stats"));
            if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("resolution")) {app.menu_page=4;}
            if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("audio")) {app.menu_page=3;}
            if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p|p.contains("waiting")) {app.stream_stats=None;}
            if let Some(path)=std::env::var_os("SECPAR_REVIEW_STREAM") {
                // Recorded host frames through the real decode worker, GL shader and stats strip.
                let (tx,rx)=mpsc::channel(256);app.events=Some(rx);
                let ctx=cc.egui_ctx.clone();
                std::thread::spawn(move||{
                    let bytes=std::fs::read(path).unwrap_or_default();
                    let mut id=0u32;
                    // Loops: a keyframe only comes again at the start of the file.
                    if bytes.is_empty() {return}
                    loop {let mut at=0;
                    while at+5<=bytes.len() {
                        let len=u32::from_be_bytes(bytes[at..at+4].try_into().unwrap()) as usize;
                        let key=bytes[at+4]==1;
                        let Some(data)=bytes.get(at+5..at+5+len) else {break};
                        at+=5+len;
                        let header=secpar_stream::frame::FrameHeader{len:len as u32,capture_us:0,keyframe:key,codec:secpar_stream::frame::Codec::H264};
                        if tx.blocking_send(Event::Video(secpar_client::receiver::VideoFrame{id,header,data:data.to_vec()})).is_err() {break}
                        id+=1;ctx.request_repaint();std::thread::sleep(Duration::from_micros(16_667));
                    }
                    }
                });
            }
        }
        app
    }
    /// Reconnects to the computer this device remembers. Account computers pair themselves.
    /// Pairing by link, for a computer with no account behind it.
    fn pairing_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        design::surface().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Pair a computer").size(14.).strong());
            ui.add_sized(
                [ui.available_width(), 34.],
                egui::TextEdit::singleline(&mut self.pair_link)
                    .hint_text("Paste a Secpar Host pairing link")
                    .password(true)
                    .margin(egui::vec2(10., 8.)),
            );
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), design::CONTROL), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!self.pair_link.trim().is_empty(), design::primary("Pair and connect")).clicked() {
                    self.pair_and_connect(ctx);
                }
            });
        });
    }
    fn pair_and_connect(&mut self, ctx: &egui::Context) {
        if self.command.is_some() {
            return;
        }
        match Pairing::parse(&self.pair_link) {
            Ok(p) => {
                self.pair_link.clear();
                self.computer_account = None;
                self.owned_connecting = None;
                self.connect(ctx, Some(p));
            }
            Err(e) => self.status = e,
        }
    }
    fn start(&mut self, ctx: &egui::Context) {
        if self.command.is_some() {
            return;
        }
        self.connect(ctx,None);
    }
    fn connect(&mut self, ctx: &egui::Context, pairing: Option<Pairing>) {
        self.connect_with(ctx, pairing, None);
    }
    fn connect_with(&mut self, ctx: &egui::Context, pairing: Option<Pairing>, guest: Option<account::Grant>) {
        if self.command.is_some() { return; }
        self.session_account=self.computer_account.clone();
        self.guest_session=guest.is_some();
        self.guest_error=false;
        if guest.is_some() { self.session_account=self.account.user.as_ref().map(|u|u.id.clone()); }
        let lan_first = self.computer_account.is_some();
        let computer = self.computer.clone();
        self.audio_route.output(None);
        self.audio_route=audio_worker::Route::default();
        self.open_audio();
        let clipboard=self.client.clipboard && guest.as_ref().is_none_or(|g|g.allows("clipboard"));
        self.clipboard=clipboard.then(secpar_clipboard::Bridge::start);
        self.notice=None;self.connection_error=None;self.pad_unsupported=false;
        self.stream_stats=None;
        let options = Options {
            audio: self.client.audio && guest.as_ref().is_none_or(|g|g.allows("audio")),
            audio_sink:Some(self.audio_route.sink()),
            clipboard,
            ..self.options.clone()
        };
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel(256);
        let (ev_tx, ev_rx) = mpsc::channel(8);
        self.command = Some(tx);
        self.events = Some(ev_rx);
        self.status = match &guest { Some(g) => format!("Connecting to @{}", g.owner_handle), None => "Connecting".into() };
        self.auto_capture_pending = true;
        self.remote_keyboard = true;
        self.video_rect = None;
        self.decoder = None;
        self.client.active_decoder = None;
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
                let connection = if let Some(g) = guest {
                    Connected::guest(&g.relay, &g.pc, &g.grant, &g.name).await
                } else if let Some(p) = pairing {
                    if lan_first { Connected::pair_lan_first(p, &device_name()).await }
                    else { Connected::pair(p, &device_name()).await }
                } else if let Some(c) = computer {
                    Connected::reconnect(c, &device_name()).await
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
    fn open_audio(&mut self) {
        self.audio_error=None;
        self.audio_route.output(None);
        self.audio=if self.client.audio {match audio_worker::Worker::start() {
            Ok(output)=>Some(output), Err(error)=>{self.audio_error=Some(error);None}
        }} else {None};
        self.audio_route.output(self.audio.as_ref());
    }
    fn capture(&mut self, ctx: &egui::Context, on: bool) {
        self.auto_capture_pending = false;
        if on && self.fs.on && self.texture.is_some() && self.command.is_some() {
            self.input.capture();
            ctx.send_viewport_cmd(ViewportCommand::CursorGrab(CursorGrab::Confined));
            ctx.send_viewport_cmd(ViewportCommand::CursorVisible(false));
        } else {
            for event in self.input.release() {
                self.send(Command::Input(event));
            }
            ctx.send_viewport_cmd(ViewportCommand::CursorGrab(CursorGrab::None));
            ctx.send_viewport_cmd(ViewportCommand::CursorVisible(true));
        }
    }
    /// A stream is on screen: connected and showing a picture.
    fn stream_active(&self) -> bool {
        self.command.is_some() && self.texture.is_some()
    }
    /// Per-session keyboard permission. The protocol has no view-only state yet, so this is
    /// always true. When it grows one, this is the single place to read it.
    fn keyboard_allowed(&self) -> bool {
        true
    }
    fn window_geometry(ctx: &egui::Context) -> Option<keys::Geometry> {
        ctx.input(|i| {
            let v = i.viewport();
            if v.maximized == Some(true) || v.fullscreen == Some(true) {
                return None;
            }
            let (outer, inner) = (v.outer_rect?, v.inner_rect?);
            Some(keys::Geometry { pos: [outer.min.x, outer.min.y], size: [inner.width(), inner.height()] })
        })
    }
    /// Fullscreen belongs to the stream. The app window never goes fullscreen.
    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        let step = self.fs.request(on, self.stream_active(), Self::window_geometry(ctx));
        self.apply_fullscreen(ctx, step);
    }
    fn apply_fullscreen(&mut self, ctx: &egui::Context, step: keys::Step) {
        match step {
            keys::Step::Nothing => return,
            keys::Step::Enter => ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true)),
            keys::Step::Leave(saved) => {
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                if let Some(g) = saved {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(g.pos[0], g.pos[1])));
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(g.size[0], g.size[1])));
                }
            }
        }
        self.fullscreen_changed(ctx);
    }
    fn fullscreen_changed(&mut self, ctx: &egui::Context) {
        self.stream_menu_open = false;
        self.capture(ctx, false);
        self.remote_keyboard = true;
        self.video_rect = None;
        if self.fs.on && self.focused {
            self.capture(ctx, true);
        } else if self.focused && self.stream_active() {
            self.input.free();
        }
    }
    /// The stream is gone: the window returns to normal size and position.
    fn leave_fullscreen_if_idle(&mut self, ctx: &egui::Context) {
        if self.fs.on && !self.stream_active() {
            self.set_fullscreen(ctx, false);
        }
    }
    /// What the escape chord does. It is the same from the window path and from the hook.
    fn escape(&mut self, ctx: &egui::Context, action: keys::Escape) {
        match action {
            keys::Escape::Fullscreen => self.set_fullscreen(ctx, !self.fs.on),
            keys::Escape::Disconnect => {
                if self.command.is_some() {
                    self.capture(ctx, false);
                    self.send(Command::Stop);
                    self.stream_menu_open = false;
                }
            }
            keys::Escape::MenuButton => {
                if self.stream_active() {
                    self.menu_button_hidden = !self.menu_button_hidden;
                    self.stream_menu_open = false;
                }
            }
        }
    }
    /// Keeps the Windows keyboard hook up exactly while the host should get every key.
    #[cfg(windows)]
    fn manage_key_hook(&mut self, ctx: &egui::Context) {
        let wanted = keys::HookWanted {
            setting: self.client.system_keys,
            stream_on_screen: self.stream_active(),
            window_focused: self.focused,
            keys_go_to_host: self.remote_keyboard && !self.stream_menu_open,
            keyboard_allowed: self.keyboard_allowed(),
        }
        .yes();
        // Review captures run on the user's desktop: never take their keyboard for a fixture.
        #[cfg(feature = "ui-review")]
        let wanted = wanted && std::env::var_os("SECPAR_REVIEW_HOOK").is_some();
        if let Some(hook) = &self.hook {
            keyhook::heartbeat();
            hook.flush();
            let mut remove = !wanted;
            for outcome in hook.take_outcomes() {
                match outcome {
                    keyhook::Outcome::Escape(action) => self.escape(ctx, action),
                    keyhook::Outcome::Stalled | keyhook::Outcome::FocusLost => remove = true,
                }
            }
            if remove || !self.stream_active() {
                self.hook = None;
                for m in self.input.keys_to_hook(false) {
                    self.send(Command::Input(m));
                }
            } else {
                ctx.request_repaint_after(keyhook::BEAT_EVERY);
            }
        } else if wanted {
            let (Some(native), Some(tx)) = (self.native, self.command.clone()) else { return };
            let wake = ctx.clone();
            let sink = Box::new(move |v| !matches!(tx.try_send(Command::Input(v)), Err(mpsc::error::TrySendError::Full(_))));
            self.hook = keyhook::KeyHook::install(native.raw(), self.client.escape, true, sink, move || wake.request_repaint());
            if self.hook.is_some() {
                for m in self.input.keys_to_hook(true) {
                    self.send(Command::Input(m));
                }
                ctx.request_repaint_after(keyhook::BEAT_EVERY);
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(bridge)=&self.clipboard {
            match bridge.poll() {
                Some(secpar_clipboard::Event::Text(text))=>self.send(Command::Clipboard(text)),
                Some(secpar_clipboard::Event::Unavailable)=>self.notice=Some("Local clipboard is unavailable".into()),
                None=>{},
            }
        }
        if self.audio.as_ref().is_some_and(|audio|audio.failed()) {
            self.audio_route.output(None);self.audio=None;self.audio_error=Some("Audio output stopped. Check the selected system output and retry.".into());
        }
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
                    // A response may arrive after logout while the worker drains.
                    // Never save that account credential as a manual/global pair.
                    if self.session_account.as_ref().is_some_and(|owner|self.account.user.as_ref().is_none_or(|user|user.id!=*owner)) {
                        self.send(Command::Stop); continue;
                    }
                    if let Some((owner,id))=self.owned_connecting.take() {
                        self.account.remember_owned(owner,id,c.clone());
                    } else if self.computer_account.is_none() && !self.guest_session {
                        self.account.remember(c.clone());
                    }
                    if !self.guest_session {
                        // Files the computer offers while we stream arrive through a second, quiet connection.
                        self.files.background(&c.id, &c.name, self.session_account.clone(), files::Source::Saved(c.clone()));
                        self.computer = Some(c);
                    }
                }
                Event::Status(s) => { self.status=s; }
                Event::Caps(_) => {}
                Event::Notice(text) => self.notice=Some(text),
                Event::Audio{packet,received_at} => {
                    // A covered window may resume with a few old events queued.
                    // Skip stale compressed audio instead of replaying it late.
                    if received_at.elapsed()<=Duration::from_millis(200) {
                        if let Some(audio)=self.audio.as_mut(){audio.packet(&packet);}
                    }
                }
                Event::AudioUnavailable(error)=>self.notice=Some(format!("Host audio: {error}")),
                Event::ClipboardUnavailable(error)=>{self.clipboard=None;self.notice=Some(error);},
                Event::Clipboard(text)=>{if let Some(bridge)=&self.clipboard {bridge.set(text);}},
                Event::StreamInfo{width,height,fps}=>{self.dimensions=[width as usize,height as usize];self.stream_fps=fps;self.resize_wait=None;},
                Event::Stats{rtt_ms,mbps,received_fps}=>self.stream_stats=Some((rtt_ms,mbps,received_fps)),
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
                Event::PadSupport(ok) => self.pad_unsupported = !ok,
                Event::Rumble(levels) => for (id, (lo, hi)) in levels.into_iter().enumerate() { self.pads.rumble(id as u8, lo, hi); },
                Event::Ended(s) => {
                    self.guest_error=self.guest_session && s!="Disconnected";
                    self.guest_session=false;
                    self.files.end_background();
                    self.connection_error=if s=="Disconnected" {None}else{Some(s.clone())};
                    self.clipboard=None;
                    self.session_account=None;
                    self.capture(ctx, false);
                    self.command = None;
                    self.events = None;
                    self.status = s;
                    self.decoder = None;
                    self.audio_route.output(None);
                    self.audio = None;
                    self.texture = None;
                    self.leave_fullscreen_if_idle(ctx);
                    self.video_rect = None;self.pointer_in_video=false;self.remote_keyboard=false;
                    self.cursor = (0, 0, false);
                    self.cursor_shape = None;
                }
                Event::Video(frame) => {
                    let preference = self.client.decoder;
                    let fps=self.stream_fps;
                    let decoder = self.decoder.get_or_insert_with(|| video::Worker::with_frame_rate(ctx.clone(),preference,fps));
                    decoder.submit(frame);
                }
            }
        }
        if let Some(decoder) = &self.decoder {
            let output = decoder.output();
            if let Some(backend)=output.backend {
                if self.client.active_decoder.as_deref()!=Some(backend) {self.client.active_decoder=Some(backend.into());}
            }
            if let Some(error) = output.error {
                self.status = error;
                self.send(Command::Stop);
            }
            if output.request_key { self.send(Command::Keyframe); }
            if let Some(picture) = output.image {
                self.dimensions = picture.size;
                let gpu = self.gpu.clone().filter(|handle| !gpu::failed(handle));
                self.gpu_live = gpu.is_some();
                let image = match gpu {
                    Some(handle) => {
                        gpu::submit(&handle, picture);
                        // The texture only marks a live stream; the shader draws the picture.
                        egui::ColorImage::new([1, 1], vec![Color32::BLACK])
                    }
                    None => picture.to_color_image(picture::Matrix::Bt709),
                };
                if let Some(texture) = self.texture.as_mut() {
                    texture.set(image, TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture("Remote desktop", image, TextureOptions::LINEAR));
                }
                ctx.request_repaint();
            }
        }
        if self.auto_capture_pending && self.focused && self.texture.is_some() {
            self.auto_capture_pending=false;
            if self.fs.on {self.capture(ctx,true);} else {self.input.free();}
        }
    }
}
impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.focused = raw.focused;
        #[cfg(windows)]
        {
            keyhook::heartbeat();
            self.input.hooked = self.hook.is_some();
        }
        if let Some(actual) = raw.viewport().fullscreen {
            let changed = self.observed_fullscreen.is_some_and(|old| old != actual);
            self.observed_fullscreen = Some(actual);
            // The app window never stays fullscreen. Only a stream adopts an OS change.
            if changed || (actual && !self.fs.on) {
                let before = self.fs.on;
                let step = self.fs.observed(actual, self.stream_active());
                if let keys::Step::Leave(_) = step {
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
                if before != self.fs.on {
                    self.fullscreen_changed(ctx);
                }
            }
        }
        if !raw.focused {
            self.capture(ctx, false);
            self.pointer_in_video = false;
            self.shortcut_key = None;
            return;
        }
        if let Some((shortcut, key)) = raw.events.iter().find_map(|e| input::shortcut(e, self.client.escape)) {
            self.shortcut_key = Some(key);
            self.escape(ctx, match shortcut {
                input::Shortcut::Fullscreen => keys::Escape::Fullscreen,
                input::Shortcut::Disconnect => keys::Escape::Disconnect,
                input::Shortcut::MenuButton => keys::Escape::MenuButton,
            });
        }
        if let Some(key) = self.shortcut_key {
            let released = raw.events.iter().any(|e| matches!(e,UiEvent::Key{key:k,pressed:false,..} if *k==key));
            raw.events.retain(|e| !matches!(e,UiEvent::Key{key:k,..} if *k==key));
            if released { self.shortcut_key=None; }
        }
        let ready = self.texture.is_some() && self.command.is_some();
        if ready && self.stream_menu_open && !self.fs.on {
            if self.input.enabled {self.capture(ctx,false);}
            self.remote_keyboard=false;
            let button=self.stream_menu_button_bounds.unwrap_or_else(||egui::Rect::from_min_size(egui::pos2(12.,12.),egui::vec2(40.,40.)));
            if raw.events.iter().any(|e|matches!(e,UiEvent::PointerButton{pos,pressed:true,..}
                if !button.contains(*pos) && !self.stream_menu_bounds.is_some_and(|r|r.contains(*pos)))) {
                self.stream_menu_open=false;
            }
            return;
        }
        if ready && self.fs.on {
            let button=self.stream_menu_button_bounds.unwrap_or_else(||egui::Rect::from_min_size(egui::pos2(12.,12.),egui::vec2(40.,40.)));
            let pointer=raw.events.iter().rev().find_map(|e|match e {
                UiEvent::PointerMoved(p)|UiEvent::PointerButton{pos:p,..}=>Some(*p),_=>None
            }).or_else(||ctx.pointer_latest_pos());
            let local=self.stream_menu_open || pointer.is_some_and(|p|button.contains(p));
            if local {
                if self.input.enabled {self.capture(ctx,false);}
                self.remote_keyboard=false;
                if self.stream_menu_open && raw.events.iter().any(|e|matches!(e,UiEvent::PointerButton{pos,pressed:true,..}
                    if !button.contains(*pos) && !self.stream_menu_bounds.is_some_and(|r|r.contains(*pos)))) {
                    self.stream_menu_open=false;
                }
                return;
            }
            // Fullscreen maps the pointer exactly as the window does: absolute positions on the
            // picture, so it lands where the host cursor is. Relative deltas drift from the host's
            // own pointer acceleration. The keyboard goes to the host at once.
            self.remote_keyboard = true;
            if !self.input.enabled { self.input.free(); }
        }
        if !ready { return; }
        if self.input.captured { self.capture(ctx, false); }
        if self.remote_keyboard && !self.input.enabled { self.input.free(); }
        let mut local = Vec::new();
        for event in std::mem::take(&mut raw.events) {
            let hit = |pos:egui::Pos2| self.video_rect.is_some_and(|r| r.contains(pos)) &&
                ctx.layer_id_at(pos)==self.video_layer;
            match &event {
                UiEvent::PointerMoved(pos) => {
                    self.pointer_in_video=hit(*pos);
                    if self.pointer_in_video {
                        if let Some(message)=input::Input::absolute(*pos,self.video_rect.unwrap()) {self.send(Command::Input(message));}
                    } else { for m in self.input.release_buttons() {self.send(Command::Input(m));} }
                    local.push(event);
                }
                UiEvent::PointerGone => {
                    self.pointer_in_video=false;
                    for m in self.input.release_buttons() {self.send(Command::Input(m));}
                    local.push(event);
                }
                UiEvent::PointerButton{pos,pressed,..} => {
                    self.pointer_in_video=hit(*pos);
                    if self.pointer_in_video {
                        if *pressed {self.remote_keyboard=true;if !self.input.enabled {self.input.free();}}
                        if let Some(m)=input::Input::absolute(*pos,self.video_rect.unwrap()){self.send(Command::Input(m));}
                        for m in self.input.modifiers(raw.modifiers){self.send(Command::Input(m));}
                        for m in self.input.event(&event){self.send(Command::Input(m));}
                    } else {
                        if *pressed { self.remote_keyboard=false;for m in self.input.release(){self.send(Command::Input(m));} }
                        else {for m in self.input.release_buttons(){self.send(Command::Input(m));}}
                        local.push(event);
                    }
                }
                UiEvent::MouseWheel{..} if self.pointer_in_video => {
                    if !self.input.enabled {self.input.free();}
                    for m in self.input.event(&event){self.send(Command::Input(m));}
                }
                UiEvent::Key{..} if self.remote_keyboard => {
                    for m in self.input.modifiers(raw.modifiers){self.send(Command::Input(m));}
                    for m in self.input.event(&event){self.send(Command::Input(m));}
                }
                UiEvent::Copy|UiEvent::Cut|UiEvent::Paste(_) if self.remote_keyboard=>{
                    for m in self.input.modifiers(raw.modifiers){self.send(Command::Input(m));}
                    for m in self.input.event(&event){self.send(Command::Input(m));}
                }
                UiEvent::MouseMoved(_) => {}, // free mode uses absolute positions, never raw deltas
                _ => local.push(event),
            }
        }
        if self.remote_keyboard {for m in self.input.modifiers(raw.modifiers){self.send(Command::Input(m));}}
        raw.events=local;
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.leave_fullscreen_if_idle(ctx);
        #[cfg(windows)]
        self.manage_key_hook(ctx);
        #[cfg(windows)]
        if let Some(tray) = &self.tray {
            tray.set_connected(self.command.is_some());
            if tray.take_disconnect() && self.command.is_some() {
                self.capture(ctx, false);
                self.send(Command::Stop);
                self.stream_menu_open = false;
            }
        }
        // Controllers are read only while a stream is showing and this window has focus.
        let pads_on = self.command.is_some() && self.texture.is_some() && self.focused;
        self.pads.set_active(pads_on, { let ctx = ctx.clone(); move || ctx.request_repaint() });
        if self.pad_unsupported && self.notice.is_none() && self.pads.connected() > 0 {
            self.notice = Some("This computer has no virtual gamepad, so your controller is not passed. See Hosting on that computer.".into());
        }
        for message in self.pads.take() {
            if pads_on || message["t"] == "pad.off" { self.send(Command::Input(message)); }
        }
        if self.command.is_some() {
            if self.awake.is_none() && !self.awake_tried {
                self.awake_tried = true;
                self.awake = awake::KeepAwake::new();
            }
        } else {
            self.awake = None;
            self.awake_tried = false;
        }
        let scale = ctx.pixels_per_point();
        if self.brand_scale != scale {
            // Filter to physical pixels before upload. GPU bilinear minification
            // of the full wordmark aliases its fine edges, particularly at 1x.
            let image = image::load_from_memory(include_bytes!("../../assets/branding/secpar-wordmark-light.png"))
                .expect("Secpar brand").resize_exact(
                    (112. * scale).round().max(1.) as u32,
                    (27. * scale).round().max(1.) as u32,
                    image::imageops::FilterType::Lanczos3,
                ).to_rgba8();
            self.brand.set(egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize], image.as_raw()), TextureOptions::LINEAR);
            self.brand_scale = scale;
        }
        self.updates.tick(ctx);
        self.updates.window(ctx, self.command.is_some());
        #[cfg(windows)]
        if self
            .tray
            .as_ref()
            .is_some_and(|tray| tray.hide_on_close(ctx))
        {
            self.capture(ctx, false);
        }
        self.poll(ctx);
        self.account.poll();
        self.files.poll();
        if let Some(connection) = self.account.files_connection.take() {
            let signed_in = self.account.user.as_ref().map(|user| user.id.clone());
            let computers = &self.account.computers;
            self.files.receive(connection, signed_in.as_deref(), |id| computers.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default());
        }
        if let Some((owner, id)) = self.files.take_stale() {
            self.account.forget_owned(owner, id.clone());
            self.account.connect_owned_for_files(id);
        }
        if let Some((owner, id, computer)) = self.files.take_paired() {
            self.account.remember_owned(owner, id, computer);
        }
        if self.account.user.is_none() {
            self.files.account_gone();
        }
        let streaming = if self.command.is_some() { self.owned_connecting.as_ref().map(|(_, id)| id.clone()).or_else(|| self.computer.as_ref().map(|c| c.id.clone())) } else { None };
        let manual = self.computer.clone().filter(|_| self.computer_account.is_none());
        self.account.sync_previews(ctx, streaming.as_deref(), manual.as_ref());
        let signed_out = std::mem::take(&mut self.account.signed_out);
        if signed_out && self.computer_account.is_some() {
            self.clipboard=None;self.audio_route.output(None);self.audio=None;self.capture(ctx,false); self.send(Command::Stop);
            self.computer=None; self.computer_account=None; self.owned_connecting=None;
        }
        self.account.session_open = self.command.is_some();
        if signed_out && self.guest_session {
            self.clipboard=None;self.capture(ctx,false);self.send(Command::Stop);
        }
        if self.command.is_none() {
            if let Some(grant)=self.account.guest_grant.take() {
                if grant.permissions == ["files".to_string()] {
                    // A request made from the card's Files entry: open the Files page, not a stream.
                    let (id, name) = (grant.pc.clone(), grant.name.clone());
                    self.files.open(&id, &name, None, files::Source::Guest(grant));
                    self.page = Page::Files;
                } else {
                    self.computer_account=None;self.owned_connecting=None;
                    self.connect_with(ctx,None,Some(grant));
                }
            }
        } else { self.account.guest_grant=None; }
        if self.command.is_none() {
            if let Some(connection)=self.account.owned_connection.take() {
                match connection {
                    account::OwnedConnection::Cached{owner,computer,..} if self.account.user.as_ref().is_some_and(|user|user.id==owner) => {
                        self.computer_account=Some(owner);self.owned_connecting=None;self.computer=Some(computer);self.connect(ctx,None);
                    }
                    account::OwnedConnection::Pair{owner,id,pairing} if self.account.user.as_ref().is_some_and(|user|user.id==owner) => {
                        self.computer_account=Some(owner.clone());self.owned_connecting=Some((owner,id));self.connect(ctx,Some(pairing));
                    }
                    _=>{}
                }
            }
        } else { self.account.owned_connection=None; }
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
        if (self.options.width,self.options.height)!=(self.client.width,self.client.height) {
            self.client.width=self.options.width;self.client.height=self.options.height;self.client.save();
        }
        if self.command.is_none() && self.input.enabled {
            self.capture(ctx, false);
        }
        if self.command.is_none() {
            egui::TopBottomPanel::top("toolbar")
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(22, 27, 35))
                        .inner_margin(egui::Margin::symmetric(20, 8)),
                )
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.image((self.brand.id(), egui::vec2(112., 27.)));
                        ui.label(egui::RichText::new(env!("CARGO_PKG_VERSION")).size(11.).color(design::MUTED));
                        if self.command.is_none() {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    self.updates.button(ui);
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
                            if let Some(error)=self.audio_error.clone() {
                                if ui.button("Retry audio").on_hover_text(error).clicked(){self.open_audio();}
                            }
                            if self.client.audio {
                                ui.menu_button("Audio", |ui| {
                                    if let Some(audio) = self.audio.as_ref() {
                                        let details = audio.details();
                                        ui.label(&details);
                                        if ui.button("Copy details").clicked() { ui.ctx().copy_text(details); }
                                    } else if let Some(error) = &self.audio_error { ui.label(error); }
                                    if ui.button("Reopen system output").clicked() { self.open_audio(); ui.close(); }
                                });
                            }
                        }
                        if self.command.is_some() {
                            if ui.button("Disconnect").clicked() {
                                self.capture(ctx, false);
                                self.send(Command::Stop);
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
                        // Five tabs still fit the smallest window.
                        let tab = ((ui.available_width() - 4. * ui.spacing().item_spacing.x) / 5.).clamp(60., 100.);
                        for (page, label) in [
                            (Page::Computers, "Computers"),
                            (Page::Files, "Files"),
                            (Page::Hosting, "Hosting"),
                            (Page::Friends, "Friends"),
                            (Page::Settings, "Settings"),
                        ] {
                            if page == Page::Hosting && !cfg!(any(windows, target_os = "linux")) {
                                continue;
                            }
                            let selected = self.page == page;
                            let response = ui.add_sized(
                                [tab, 36.],
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
        if self.command.is_none() {
            if let Some(error)=self.connection_error.clone() {
                egui::TopBottomPanel::top("connection-error").show(ctx,|ui|{
                    ui.horizontal_wrapped(|ui|{
                        let friendly=if self.guest_error {None} else {friendly_error(&error)};
                        if friendly.is_some() {ui.label(egui::RichText::new(START_FAILED_TITLE).strong().color(design::BAD));}
                        ui.colored_label(design::BAD,friendly.unwrap_or(&error));
                        if !self.guest_error && ui.small_button("Retry").clicked() {
                            self.connection_error=None;
                            if self.computer.is_some() {self.start(ctx);}
                        }
                        if ui.small_button("Copy error").clicked(){ui.ctx().copy_text(error);}
                        if ui.small_button("Dismiss").clicked(){self.connection_error=None;}
                    });
                });
            }
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
                    self.video_rect=Some(picture);self.video_layer=Some(ui.layer_id());
                    ui.painter().rect_filled(rect, 0., Color32::BLACK);
                    match self.gpu.clone().filter(|_| self.gpu_live) {
                        Some(handle) => { ui.painter().add(gpu::callback(handle, picture)); }
                        None => {
                            ui.painter().image(
                                texture.id(),
                                picture,
                                egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                                Color32::WHITE,
                            );
                        }
                    }
                    #[cfg(feature="ui-review")]
                    if self.review_commands.is_some() {ui.painter().text(picture.center(),egui::Align2::CENTER_CENTER,"Stream layout sample",egui::FontId::proportional(16.),design::MUTED);}
                    if self.cursor.2 && (self.input.captured || !self.pointer_in_video) {
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
                    if self.page == Page::Hosting {
                        self.hosting.ui(ui, ctx, &mut self.account);
                    } else if self.page == Page::Files {
                        let manual = self.computer.clone().filter(|_| self.computer_account.is_none());
                        let picks = self.account.picks(manual.as_ref());
                        let note = self.account.note().to_string();
                        if let Some(files::Action::Owned(id)) = self.files.ui(ui, &picks, &note) {
                            self.account.connect_owned_for_files(id);
                        }
                        if let Some(dir) = self.files.take_save_dir() {
                            self.client.files_dir = dir;
                            self.client.save();
                        }
                    } else if self.page == Page::Friends {
                        self.account.ui(ui);
                    } else if self.page == Page::Settings {
                        egui::ScrollArea::vertical().show(ui,|ui| {
                            if self.client.ui(ui) {self.options=self.client.options();self.client.save();}
                        });
                    } else {
                        // Everything fits at the default size. Only a smaller window scrolls, as a whole.
                        let shown = egui::ScrollArea::vertical().id_salt("computers-page").auto_shrink([false, false]).show(ui, |ui| {
                            self.account.computers_ui(ui);
                            if let Some(computer) = self.computer.as_ref().filter(|_|self.computer_account.is_none()) {
                                let name = computer.name.clone();
                                let mut forget = false;
                                let mut menu = |ui: &mut egui::Ui| {
                                    ui.menu_button("…", |ui| {
                                        if ui.button("Forget on this device").clicked() {
                                            forget = true;
                                            ui.close();
                                        }
                                    });
                                };
                                let preview = self.account.preview(&computer.id).cloned();
                                let connect = card::show(ui, card::Card { name: &name, status: None, platform: "", preview: preview.as_ref(), connect: Some(true), menu: Some(&mut menu) });
                                if forget { self.account.forget_computer(); }
                                if connect { self.start(ctx); }
                            }
                            // Without an account a computer is still paired by its link, kept out of the way.
                            if self.account.user.is_none() {
                                ui.add_space(8.);
                                let text = if self.pair_open { "Hide" } else { "Use without an account" };
                                if ui.add(egui::Label::new(egui::RichText::new(text).color(design::ACCENT)).sense(egui::Sense::click())).clicked() {
                                    self.pair_open = !self.pair_open;
                                }
                                if self.pair_open { self.pairing_ui(ui, ctx); }
                            }
                        });
                        #[cfg(feature = "ui-review")]
                        if self.review_frames == 11 {
                            eprintln!("FIT content={:.1} viewport={:.1}", shown.content_size.y, shown.inner_rect.height());
                        }
                        let _ = shown;
                    }
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(connecting_text(self.computer.as_ref().map(|c|c.name.as_str())));
                    });
                }
            });
        if self.command.is_some() {
            self.stream_menu(ctx);
            ctx.request_repaint_after(Duration::from_millis(8));
        }
        // Drives the real window through the fullscreen rules and prints what the OS reports.
        #[cfg(feature = "ui-review")]
        if std::env::var_os("SECPAR_REVIEW_FS").is_some() {
            self.review_frames += 1;
            let report = |label: &str, ctx: &egui::Context, this: &Self| {
                let (outer, inner, full, max) = ctx.input(|i| {
                    let v = i.viewport();
                    (v.outer_rect, v.inner_rect, v.fullscreen, v.maximized)
                });
                eprintln!("FS {label} state_on={} os_fullscreen={full:?} maximized={max:?} stream={} outer={outer:?} inner={inner:?}", this.fs.on, this.stream_active());
            };
            match self.review_frames {
                10 => report("start", ctx, self),
                20 => {
                    if std::env::var_os("SECPAR_REVIEW_FS_OS").is_some() {
                        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
                    } else {
                        self.escape(ctx, keys::Escape::Fullscreen);
                    }
                }
                60 => {
                    report("after fullscreen request", ctx, self);
                    if self.stream_active() {
                        self.escape(ctx, keys::Escape::Fullscreen);
                    }
                }
                100 => report("after leaving", ctx, self),
                110 if self.stream_active() => {
                    // The stream ends while fullscreen.
                    self.escape(ctx, keys::Escape::Fullscreen);
                }
                150 if self.stream_active() => {
                    report("fullscreen again", ctx, self);
                    self.command = None;
                    self.texture = None;
                }
                200 => {
                    report("after stream ended", ctx, self);
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                _ => {}
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        #[cfg(feature = "ui-review")]
        if let Some(path) = std::env::var_os("SECPAR_REVIEW_SCREENSHOT_TO") {
            self.review_frames += 1;
            if self.review_frames >= if std::env::var_os("SECPAR_REVIEW_STREAM").is_some() {150} else {12} && !self.review_shot && (self.files.review_ready() || self.review_frames > 3000) {
                self.review_shot = true;
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let UiEvent::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    image::save_buffer(
                        &path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .expect("Review screenshot");
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        #[cfg(windows)]
        {
            self.hook = None;
        }
        self.files.close();
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
