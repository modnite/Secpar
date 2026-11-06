//! One authenticated device's streaming session. Several run at once (limit: 3).
use crate::platform::{Platform, Sink, StreamHandle, StreamRequest, SCREEN_RECORDING_DENIED};
use secpar_host_core::{
    agent::Session,
    limits::{StreamSlots, LIMIT_MESSAGE},
    proto::{ClientMsg, ServerMsg},
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    stream: Option<Box<dyn StreamHandle>>,
    starting: bool,
    generation: u64,
}
pub struct OwnerSession {
    platform: Arc<dyn Platform>,
    active: Arc<StreamSlots>,
    state: Arc<Mutex<State>>,
    out: Arc<dyn Sink>,
}
impl OwnerSession {
    pub fn new(platform: Arc<dyn Platform>, active: Arc<StreamSlots>, out: Arc<dyn Sink>) -> Self {
        Self { platform, active, state: Arc::default(), out }
    }
    fn error(&self, code: &str, msg: &str) {
        self.out.send(ServerMsg::StreamError { code: code.into(), msg: msg.into() });
    }
    fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.generation += 1;
        state.starting = false;
        state.stream = None;
    }
    fn start(&self, mut request: StreamRequest) {
        {
            let state = self.state.lock().unwrap();
            if state.starting || state.stream.as_ref().is_some_and(|s| s.alive()) {
                return self.error("busy", "This device is already streaming");
            }
        }
        let permissions = self.platform.permissions();
        if !permissions.screen_recording {
            return self.error("permission_denied", SCREEN_RECORDING_DENIED);
        }
        request.input_allowed = permissions.accessibility;
        let Some(guard) = self.active.try_acquire() else {
            return self.error("connection_limit", LIMIT_MESSAGE);
        };
        let generation = {
            let mut s = self.state.lock().unwrap();
            s.generation += 1;
            s.starting = true;
            s.generation
        };
        let (state, out) = (self.state.clone(), self.out.clone());
        let started = self.platform.start_stream(request, out.clone(), guard);
        tokio::spawn(async move {
            let result = started.await;
            let mut s = state.lock().unwrap();
            if s.generation != generation {
                return; // stopped or replaced meanwhile; dropping `result` frees the slot
            }
            s.starting = false;
            match result {
                Ok((stream, ready)) => {
                    s.stream = Some(stream);
                    out.send(ready);
                }
                Err(msg) => {
                    out.send(ServerMsg::StreamError { code: "capture_failed".into(), msg });
                }
            }
        });
    }
}
impl Session for OwnerSession {
    fn on_msg(&mut self, msg: ClientMsg) -> Option<ServerMsg> {
        match msg {
            ClientMsg::StreamStart { w, h, fps, codecs, max_kbps, audio, cands, .. } => {
                self.start(StreamRequest {
                    width: w,
                    height: h,
                    fps: if fps == 0 { 60 } else { fps.clamp(1, 120) },
                    kbps: if max_kbps == 0 { 12000 } else { max_kbps.clamp(1000, 50000) },
                    codecs,
                    candidates: cands.iter().take(16).filter_map(|c| c.parse().ok()).collect(),
                    audio,
                    input_allowed: false,
                });
                None
            }
            ClientMsg::StreamStop => {
                self.stop();
                Some(ServerMsg::Ok { of: "stream.stop".into() })
            }
            ClientMsg::StreamCand { c } => {
                if let (Ok(peer), Some(stream)) = (c.parse(), self.state.lock().unwrap().stream.as_ref()) {
                    stream.add_candidate(peer);
                }
                None
            }
            m @ (ClientMsg::MouseMove { .. }
            | ClientMsg::MousePos { .. }
            | ClientMsg::MouseButton { .. }
            | ClientMsg::MouseScroll { .. }
            | ClientMsg::Key { .. }
            | ClientMsg::Text { .. }) => {
                // Without Accessibility consent the stream is view-only; input is dropped.
                if self.platform.permissions().accessibility {
                    if let Some(stream) = self.state.lock().unwrap().stream.as_ref() {
                        stream.input(m.sanitize());
                    }
                }
                None
            }
            other => Some(ServerMsg::error(
                "unsupported",
                "This action is not available on this Mac host",
                Some(other.t()),
            )),
        }
    }
}
impl Drop for OwnerSession {
    fn drop(&mut self) {
        self.stop();
    }
}
