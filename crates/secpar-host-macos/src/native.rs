//! macOS bindings. Compiled on macOS only; not exercised by any non-Mac check.
use crate::{
    backend::local_addresses,
    media::{AudioSource, InputSink, MediaStream, Sources},
    native_input::MacInput,
    native_media::Capture,
    platform::{Permission, Permissions, Platform, Sink, StreamHandle, StreamRequest},
};
use secpar_host_core::{account::CredentialStorage, agent::BoxFut, limits::StreamPermit, proto::ServerMsg, Display};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool; // macOS 10.15+
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub fn permissions() -> Permissions {
    // Both calls only read the current TCC state; they never show a prompt.
    unsafe {
        Permissions {
            screen_recording: CGPreflightScreenCaptureAccess(),
            accessibility: AXIsProcessTrusted(),
        }
    }
}
pub fn request_permission(which: Permission) {
    match which {
        // Shows the system consent dialog the first time and lists Secpar in Settings.
        Permission::ScreenRecording => unsafe {
            CGRequestScreenCaptureAccess();
        },
        // Accessibility has no prompt-free API here; take the user to the system pane.
        Permission::Accessibility => {
            let _ = std::process::Command::new("open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
                .status();
        }
    }
}
pub fn displays() -> Vec<Display> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    if unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } != 0 {
        return vec![];
    }
    let main = unsafe { CGMainDisplayID() };
    ids[..(count as usize).min(ids.len())]
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let r = unsafe { CGDisplayBounds(id) };
            Display {
                id: id.to_string(),
                name: format!("Display {}", i + 1),
                primary: id == main,
                rect: [r.origin.x as i32, r.origin.y as i32, r.size.width as i32, r.size.height as i32],
            }
        })
        .collect()
}
pub fn computer_name() -> String {
    std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Mac".into())
}

/// Machine-enrollment credential kept in the user's login Keychain, not in a file.
/// Not wired to a command yet: account enrollment waits until streaming works.
pub struct KeychainStorage;
impl KeychainStorage {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new("com.secpar.host", "host-account").map_err(|_| "Keychain is unavailable".into())
    }
}
impl CredentialStorage for KeychainStorage {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        match Self::entry()?.get_secret() {
            Ok(bytes) if bytes.len() <= 8192 => Ok(Some(bytes)),
            Ok(_) => Err("Saved host link is invalid".into()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("Could not read the saved host link from Keychain".into()),
        }
    }
    fn save(&self, plain: &[u8]) -> Result<(), String> {
        Self::entry()?
            .set_secret(plain)
            .map_err(|_| "Could not save the host link to Keychain".into())
    }
    fn remove(&self) -> Result<(), String> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Could not remove the saved host link from Keychain".into()),
        }
    }
}

/// Real permissions, displays, ScreenCaptureKit capture, VideoToolbox H.264, system audio
/// and CGEvent input. H.264 only: a request without "h264" is refused, not silently changed.
pub struct MacPlatform;
impl Platform for MacPlatform {
    fn machine_name(&self) -> String {
        computer_name()
    }
    fn permissions(&self) -> Permissions {
        permissions()
    }
    fn request_permission(&self, which: Permission) {
        request_permission(which)
    }
    fn displays(&self) -> Vec<Display> {
        displays()
    }
    fn start_stream(
        &self,
        request: StreamRequest,
        sink: Arc<dyn Sink>,
        guard: StreamPermit,
    ) -> BoxFut<Result<(Box<dyn StreamHandle>, ServerMsg), String>> {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                if !request.codecs.iter().any(|c| c == "h264") {
                    return Err("This Mac host streams H.264; the device did not offer it".to_string());
                }
                let shown = displays();
                let display = shown.iter().find(|d| d.primary).or(shown.first()).cloned().ok_or("No display is available")?;
                let id: u32 = display.id.parse().map_err(|_| "Invalid display")?;
                let (capture, audio) = Capture::start(id, (request.width, request.height), request.fps, request.kbps, request.audio)?;
                let input: Option<Box<dyn InputSink>> = request.input_allowed.then(|| Box::new(MacInput::new(display.rect)) as Box<dyn InputSink>);
                let sources = Sources {
                    video: Box::new(capture),
                    audio: audio.map(|a| Box::new(a) as Box<dyn AudioSource>),
                    input,
                    display,
                };
                let (stream, ready) = MediaStream::start(sources, request.candidates, local_addresses(), sink, guard)?;
                Ok((Box::new(stream) as Box<dyn StreamHandle>, ready))
            })
            .await
            .map_err(|_| "Video initialization failed".to_string())?
        })
    }
}
