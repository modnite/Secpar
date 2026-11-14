//! The channel between the window and the background part. It never leaves the
//! computer: a named pipe on Windows, a socket file on Linux. One JSON object
//! per line in, one line back.
use crate::Core;
use secpar_hosting::Engine;
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// The longest line either side accepts.
const MAX_LINE: usize = 16 * 1024;

async fn converse<E: Engine, S: AsyncRead + AsyncWrite + Unpin>(core: Arc<Core<E>>, stream: S) {
    let (reading, mut writing) = tokio::io::split(stream);
    let mut lines = BufReader::new(tokio::io::AsyncReadExt::take(reading, u64::MAX));
    let mut line = Vec::new();
    loop {
        line.clear();
        // Stop at a line that is too long instead of buffering it.
        lines.get_mut().set_limit(MAX_LINE as u64 + 1);
        match lines.read_until(b'\n', &mut line).await {
            Ok(n) if n > 0 && n <= MAX_LINE && line.ends_with(b"\n") => {}
            _ => return,
        }
        let Ok(text) = std::str::from_utf8(&line) else { return };
        let mut answer = core.answer(text.trim()).await.to_string().into_bytes();
        answer.push(b'\n');
        if writing.write_all(&answer).await.is_err() {
            return;
        }
    }
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(stream: S, request: &Value) -> Option<Value> {
    let (reading, mut writing) = tokio::io::split(stream);
    let mut line = request.to_string().into_bytes();
    line.push(b'\n');
    writing.write_all(&line).await.ok()?;
    let mut answer = Vec::new();
    BufReader::new(tokio::io::AsyncReadExt::take(reading, MAX_LINE as u64)).read_until(b'\n', &mut answer).await.ok()?;
    serde_json::from_slice(&answer).ok()
}

#[cfg(windows)]
mod system {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES},
    };

    fn path(name: &str) -> String {
        format!(r"\\.\pipe\{name}")
    }
    /// Full use for the system and administrators, reading and writing for the
    /// person logged in at this computer, nothing for anyone over the network.
    fn create(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
        let rule: Vec<u16> = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)(A;;GA;;;OW)\0".encode_utf16().collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: `rule` is a terminated wide string; `descriptor` receives memory
        // that is freed below with LocalFree.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(rule.as_ptr(), 1, &mut descriptor, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut attributes = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        let mut options = ServerOptions::new();
        options.first_pipe_instance(first).reject_remote_clients(true);
        // SAFETY: `attributes` is valid for the call and points at the descriptor above.
        let pipe = unsafe { options.create_with_security_attributes_raw(path(name), &mut attributes as *mut _ as *mut _) };
        // SAFETY: allocated by the conversion above and not used after this.
        unsafe { LocalFree(descriptor) };
        pipe
    }
    pub async fn serve<E: Engine>(core: Arc<Core<E>>, name: &str) -> std::io::Result<()> {
        // Being the first to create the name means no other program is posing as this part.
        let mut waiting = create(name, true)?;
        loop {
            waiting.connect().await?;
            let connected = std::mem::replace(&mut waiting, create(name, false)?);
            tokio::spawn(converse(core.clone(), connected));
        }
    }
    pub async fn ask(name: &str, request: &Value) -> Option<Value> {
        exchange(ClientOptions::new().open(path(name)).ok()?, request).await
    }
}

#[cfg(unix)]
mod system {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::{UnixListener, UnixStream};

    /// `name` is the socket file's path, inside a folder only this person can use.
    pub async fn serve<E: Engine>(core: Arc<Core<E>>, name: &str) -> std::io::Result<()> {
        // A file left by an earlier run that nobody answers on is removed.
        if UnixStream::connect(name).await.is_ok() {
            return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "already running"));
        }
        let _ = std::fs::remove_file(name);
        let listener = UnixListener::bind(name)?;
        std::fs::set_permissions(name, std::fs::Permissions::from_mode(0o600))?;
        loop {
            let (connected, _) = listener.accept().await?;
            tokio::spawn(converse(core.clone(), connected));
        }
    }
    pub async fn ask(name: &str, request: &Value) -> Option<Value> {
        exchange(UnixStream::connect(name).await.ok()?, request).await
    }
}

/// Answers the window on the channel `name` until the process ends. An error
/// means the channel could not be opened, for example because this part is
/// already running.
pub use system::serve;
/// Sends one request to the background part and returns its answer.
pub use system::ask;
