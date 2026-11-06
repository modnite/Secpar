//! The only surface the host logic needs from macOS. Native code implements it.
use secpar_host_core::{agent::BoxFut, proto::{ClientMsg, ServerMsg}, Display};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Permissions {
    /// Screen & System Audio Recording (TCC). Required before any capture starts.
    pub screen_recording: bool,
    /// Accessibility (TCC). Required to inject remote keyboard and pointer input.
    pub accessibility: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    ScreenRecording,
    Accessibility,
}
pub const SCREEN_RECORDING_DENIED: &str = "Allow Secpar under System Settings > Privacy & Security > Screen & System Audio Recording, then reopen Secpar.";

pub struct StreamRequest {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
    pub codecs: Vec<String>,
    pub candidates: Vec<SocketAddr>,
    pub audio: bool,
    pub input_allowed: bool,
}
/// Where a session reports to its device. `Outbox` implements it for the agent.
pub trait Sink: Send + Sync {
    fn send(&self, message: ServerMsg) -> bool;
}
impl Sink for secpar_host_core::agent::Outbox {
    fn send(&self, message: ServerMsg) -> bool {
        secpar_host_core::agent::Outbox::send(self, message)
    }
}
/// One running device stream. Dropping it must stop capture, encoding and sockets.
pub trait StreamHandle: Send {
    fn alive(&self) -> bool;
    fn input(&self, message: ClientMsg);
    fn add_candidate(&self, peer: SocketAddr);
}
pub trait Platform: Send + Sync + 'static {
    fn machine_name(&self) -> String;
    fn permissions(&self) -> Permissions;
    /// Asks macOS to show its consent flow; consent itself stays with the system.
    fn request_permission(&self, which: Permission);
    fn displays(&self) -> Vec<Display>;
    /// Starts one independent stream. `guard` is held until the stream is dropped.
    fn start_stream(
        &self,
        request: StreamRequest,
        sink: Arc<dyn Sink>,
        guard: secpar_host_core::limits::StreamPermit,
    ) -> BoxFut<Result<(Box<dyn StreamHandle>, ServerMsg), String>>;
}
