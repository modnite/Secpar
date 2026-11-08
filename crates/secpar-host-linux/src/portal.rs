//! One consented RemoteDesktop session supplies both screen capture and input.
use ashpd::desktop::{
    remote_desktop::{DeviceType, RemoteDesktop, SelectDevicesOptions},
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    PersistMode, Session,
};
use std::{
    os::fd::{AsRawFd, OwnedFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub struct Portal {
    pub node_id: u32,
    /// Compositor logical dimensions, also used for absolute input coordinates.
    pub width: u32,
    pub height: u32,
    pub restore_token: Option<String>,
    pub(crate) remote: RemoteDesktop,
    pub(crate) session: Arc<Session<RemoteDesktop>>,
    remote_fd: OwnedFd,
    closed: AtomicBool,
    runtime: tokio::runtime::Handle,
}
impl Portal {
    pub async fn open(restore_token: Option<&str>) -> Result<Arc<Self>, String> {
        let remote = RemoteDesktop::new().await.map_err(|_| {
            "KDE's remote desktop portal is unavailable. Check xdg-desktop-portal-kde."
        })?;
        let cast = Screencast::new()
            .await
            .map_err(|_| "The screen sharing portal is unavailable.")?;
        let session = remote
            .create_session(Default::default())
            .await
            .map_err(|_| "Could not create the desktop sharing session.")?;
        let result = async {
            let mut options = SelectDevicesOptions::default()
                .set_devices(DeviceType::Keyboard | DeviceType::Pointer);
            // Persistence belongs to SelectDevices for a combined remote-desktop session.
            if remote.version() >= 2 {
                options = options
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(restore_token);
            }
            remote
                .select_devices(&session, options)
                .await
                .map_err(|_| "Could not request keyboard and pointer access.")?
                .response()
                .map_err(|_| "Keyboard and pointer access was cancelled.")?;
            cast.select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_sources(ashpd::enumflags2::BitFlags::from(SourceType::Monitor))
                    .set_multiple(false)
                    .set_cursor_mode(CursorMode::Hidden),
            )
            .await
            .map_err(|_| "Could not request a monitor.")?
            .response()
            .map_err(|_| "Monitor selection was cancelled.")?;
            let selected = remote
                .start(&session, None, Default::default())
                .await
                .map_err(|_| "Could not start desktop sharing.")?
                .response()
                .map_err(|_| "Desktop sharing was cancelled or denied.")?;
            if !selected.devices().contains(DeviceType::Keyboard)
                || !selected.devices().contains(DeviceType::Pointer)
            {
                return Err(
                    "Allow both keyboard and pointer control in the desktop sharing dialog.",
                );
            }
            let stream = selected
                .streams()
                .first()
                .ok_or("No monitor was selected.")?;
            let (w, h) = stream
                .size()
                .filter(|(w, h)| *w > 0 && *h > 0)
                .ok_or("The portal did not report the selected monitor's logical size.")?;
            let node = stream.pipe_wire_node_id();
            let token = selected.restore_token().map(str::to_owned);
            let fd = cast
                .open_pipe_wire_remote(&session, Default::default())
                .await
                .map_err(|_| "Could not open the selected PipeWire stream.")?;
            Ok((node, w as u32, h as u32, token, fd))
        }
        .await;
        match result {
            Ok((node_id, width, height, restore_token, remote_fd)) => Ok(Arc::new(Self {
                node_id,
                width,
                height,
                restore_token,
                remote,
                session: Arc::new(session),
                remote_fd,
                closed: AtomicBool::new(false),
                runtime: tokio::runtime::Handle::current(),
            })),
            Err(error) => {
                let _ = session.close().await;
                Err(error.into())
            }
        }
    }
    pub(crate) fn fd(&self) -> i32 {
        self.remote_fd.as_raw_fd()
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    pub async fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            let _ = self.session.close().await;
        }
    }
}
impl Drop for Portal {
    fn drop(&mut self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            let session = self.session.clone();
            self.runtime.spawn(async move {
                let _ = session.close().await;
            });
        }
    }
}
