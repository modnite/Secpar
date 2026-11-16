//! The Moonlight streaming protocol inside Secpar's own apps.
//!
//! `moonlight-common-c` carries video, sound and input between this device and
//! a computer's engine; Secpar's window shows the picture with its own decoder
//! and plays the sound with its own output. Nothing of Moonlight's applications
//! is used. The core keeps one connection per process.
mod crypto;
pub mod http;

use std::{
    os::raw::{c_char, c_int, c_short, c_uchar, c_void},
    sync::{Arc, Mutex},
};

/// What the connection produces.
pub enum Output {
    Video { data: Vec<u8>, keyframe: bool, hevc: bool },
    Audio(Vec<u8>),
    Started,
    Poor(bool),
    Ended(i32),
}

type Sink = Arc<dyn Fn(Output) + Send + Sync>;
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

fn emit(output: Output) {
    let sink = SINK.lock().unwrap().clone();
    if let Some(sink) = sink {
        sink(output);
    }
}

extern "C" fn on_video(_: *mut c_void, data: *const c_uchar, length: c_int, keyframe: c_int, format: c_int) {
    let data = unsafe { std::slice::from_raw_parts(data, length.max(0) as usize) }.to_vec();
    // VIDEO_FORMAT_MASK_H265 is 0x0F00.
    emit(Output::Video { data, keyframe: keyframe != 0, hevc: format & 0x0F00 != 0 });
}
extern "C" fn on_audio(_: *mut c_void, data: *const c_char, length: c_int) {
    emit(Output::Audio(unsafe { std::slice::from_raw_parts(data as *const u8, length.max(0) as usize) }.to_vec()));
}
extern "C" fn on_event(_: *mut c_void, kind: c_int, code: c_int) {
    emit(match kind {
        1 => Output::Started,
        4 => Output::Poor(true),
        5 => Output::Poor(false),
        _ => Output::Ended(code),
    });
}

type VideoFn = extern "C" fn(*mut c_void, *const c_uchar, c_int, c_int, c_int);
type AudioFn = extern "C" fn(*mut c_void, *const c_char, c_int);
type EventFn = extern "C" fn(*mut c_void, c_int, c_int);

extern "C" {
    fn secpar_start(
        address: *const c_char, app_version: *const c_char, gfe_version: *const c_char, rtsp_url: *const c_char,
        codec_support: c_int, width: c_int, height: c_int, fps: c_int, kbps: c_int, remote: c_int,
        input_key: *const c_uchar, input_key_id: c_int, formats: c_int, video: VideoFn, audio: AudioFn,
        event: EventFn, user: *mut c_void,
    ) -> c_int;
    fn LiStopConnection();
    fn LiRequestIdrFrame();
    fn LiSendMousePositionEvent(x: c_short, y: c_short, w: c_short, h: c_short) -> c_int;
    fn LiSendMouseMoveEvent(dx: c_short, dy: c_short) -> c_int;
    fn LiSendMouseButtonEvent(action: c_char, button: c_int) -> c_int;
    fn LiSendKeyboardEvent(key: c_short, action: c_char, modifiers: c_char) -> c_int;
    fn LiSendHighResScrollEvent(amount: c_short) -> c_int;
    fn LiSendHighResHScrollEvent(amount: c_short) -> c_int;
}

/// Everything one connection needs.
pub struct Start {
    pub address: String,
    pub info: http::ServerInfo,
    pub rtsp_url: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
    pub remote: bool,
    pub hevc: bool,
    pub input_key: [u8; 16],
    pub input_key_id: i32,
}

/// Opens the connection. Blocks until it is up or has failed; run it off the
/// async threads. Output keeps arriving on `sink` until `stop`.
pub fn connect(start: Start, sink: Sink) -> Result<(), String> {
    *SINK.lock().unwrap() = Some(sink);
    let text = |s: &str| std::ffi::CString::new(s).map_err(|_| "bad_text".to_string());
    let (address, app, gfe, url) = (text(&start.address)?, text(&start.info.app_version)?, text(&start.info.gfe_version)?, text(&start.rtsp_url)?);
    let formats = 0x0001 | if start.hevc { 0x0100 } else { 0 };
    let code = unsafe {
        secpar_start(
            address.as_ptr(), app.as_ptr(), gfe.as_ptr(), url.as_ptr(), start.info.codec_support,
            start.width as c_int, start.height as c_int, start.fps as c_int, start.kbps as c_int, start.remote as c_int,
            start.input_key.as_ptr(), start.input_key_id, formats, on_video, on_audio, on_event, std::ptr::null_mut(),
        )
    };
    if code == 0 { Ok(()) } else { Err(format!("connection_failed_{code}")) }
}

pub fn stop() {
    unsafe { LiStopConnection() };
    *SINK.lock().unwrap() = None;
}

pub fn keyframe() {
    unsafe { LiRequestIdrFrame() };
}

/// Secpar's input messages (the same ones its own stream used) in Moonlight's form.
pub fn input(message: &serde_like::Message) {
    unsafe {
        match message {
            serde_like::Message::Key { vk, down } => {
                LiSendKeyboardEvent((0x8000 | (*vk as u16 & 0xff)) as c_short, if *down { 0x03 } else { 0x04 }, 0);
            }
            serde_like::Message::Position { x, y } => {
                LiSendMousePositionEvent((x * 4095.0) as c_short, (y * 4095.0) as c_short, 4096, 4096);
            }
            serde_like::Message::Move { dx, dy } => {
                LiSendMouseMoveEvent(*dx as c_short, *dy as c_short);
            }
            serde_like::Message::Button { button, down } => {
                LiSendMouseButtonEvent(if *down { 0x07 } else { 0x08 }, *button);
            }
            serde_like::Message::Scroll { dx, dy } => {
                if *dy != 0 {
                    LiSendHighResScrollEvent((*dy).clamp(-32000, 32000) as c_short);
                }
                if *dx != 0 {
                    LiSendHighResHScrollEvent((*dx).clamp(-32000, 32000) as c_short);
                }
            }
        }
    }
}

/// Input in a form that does not tie this crate to the window's JSON.
pub mod serde_like {
    pub enum Message {
        Key { vk: u32, down: bool },
        Position { x: f32, y: f32 },
        Move { dx: i32, dy: i32 },
        /// 1 left, 2 middle, 3 right, 4 and 5 the side buttons.
        Button { button: i32, down: bool },
        Scroll { dx: i32, dy: i32 },
    }
}
