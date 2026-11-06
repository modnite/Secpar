//! Exactly one DXGI duplication per display/process. Viewers consume the newest
//! frame independently, with bounded GPU texture leases and no frame queues.
use super::*;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Condvar, Mutex, OnceLock, Weak,
    },
    thread::JoinHandle,
};

type Registry = Mutex<HashMap<String, Weak<Source>>>;
fn registry() -> &'static Registry {
    static SOURCES: OnceLock<Registry> = OnceLock::new();
    SOURCES.get_or_init(Default::default)
}

#[derive(Default)]
struct Latest {
    event: u64,
    image: u64,
    shape_version: u64,
    epoch: u64,
    size: (u32, u32),
    texture: Option<Arc<FrameTexture>>,
    capture_us: u64,
    pointer: Option<Pointer>,
    shape: Option<Shape>,
}
struct Source {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    luid: i64,
    display: Display,
    latest: Mutex<Latest>,
    changed: Condvar,
    stopped: AtomicBool,
    subscribers: AtomicUsize,
    thread: Mutex<Option<JoinHandle<()>>>,
}
// SAFETY: D3D11's device is free-threaded and we require protection on its
// immediate context. DXGI duplication itself stays on its single producer
// thread. CPU publication and texture lease ownership are synchronized.
unsafe impl Send for Source {}
unsafe impl Sync for Source {}

pub struct SharedCapture {
    source: Arc<Source>,
    event: u64,
    image: u64,
    shape: u64,
    epoch: u64,
    pub reinit: bool,
}
impl SharedCapture {
    pub fn open(display: &str) -> Option<Self> {
        let mut sources = registry().lock().unwrap();
        let (dev, ctx, output, luid) = device(display)?;
        let display = describe(&output)?;
        if let Some(source) = sources
            .get(&display.id)
            .and_then(Weak::upgrade)
            .filter(|s| !s.stopped.load(Ordering::Acquire))
        {
            source.subscribers.fetch_add(1, Ordering::Relaxed);
            return Some(Self::subscribe(source));
        }
        // Every client and MF encoder shares this same protected D3D device.
        let protected: ID3D11Multithread = dev.cast().ok()?;
        unsafe {
            let _ = protected.SetMultithreadProtected(true);
            if !protected.GetMultithreadProtected().as_bool() {
                return None;
            }
        }
        let source = Arc::new(Source {
            dev,
            ctx,
            luid,
            display,
            latest: Mutex::new(Latest::default()),
            changed: Condvar::new(),
            stopped: AtomicBool::new(false),
            subscribers: AtomicUsize::new(1),
            thread: Mutex::new(None),
        });
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let weak = Arc::downgrade(&source);
        let thread = std::thread::spawn(move || {
            let _com = crate::engine::Com::init();
            let Some(source) = weak.upgrade() else { return };
            let first = Dxgi::open(&source.dev, &source.ctx, &output);
            let Some(first) = first else {
                let _ = ready_tx.send(false);
                return;
            };
            let size = first.size();
            {
                let mut latest = source.latest.lock().unwrap();
                latest.size = size;
                latest.epoch = 1;
            }
            let (dev, ctx) = (source.dev.clone(), source.ctx.clone());
            drop(source);
            let mut capture = Capture::new(Some(first), move || Dxgi::open(&dev, &ctx, &output));
            let mut buffers: Vec<Arc<FrameTexture>> = Vec::new();
            #[cfg(test)]
            let acquire_wait = Duration::from_millis(std::env::var("SECPAR_TEST_DXGI_WAIT_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(0));
            #[cfg(not(test))]
            let acquire_wait = Duration::ZERO;
            let _ = ready_tx.send(true);
            while let Some(source) = weak.upgrade() {
                if source.stopped.load(Ordering::Acquire) {
                    break;
                }
                if let Some(dup) = capture.dup.as_mut() {
                    dup.copy = None;
                    dup.copy = buffers
                        .iter()
                        .find(|buffer| Arc::strong_count(buffer) == 1)
                        .cloned();
                    // At most three viewers can hold an image, plus publication
                    // and a producer. Bound transient allocation during churn.
                    if dup.copy.is_none() && buffers.len() >= 8 {
                        drop(source);
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                }
                // Never wait inside DXGI while the encoders use this device.
                // A coarse DXGI timeout both delays publication and can hold up
                // work submitted through the shared immediate context.
                let frame = capture.next(acquire_wait);
                let reinit = std::mem::take(&mut capture.reinit);
                if reinit || frame.is_some() {
                    let mut latest = source.latest.lock().unwrap();
                    if reinit {
                        latest.epoch += 1;
                        latest.size = capture.size().unwrap_or(latest.size);
                        latest.texture = None;
                        // Old-dimension buffers cannot be recycled into a new
                        // duplication. Outstanding viewer leases remain valid.
                        buffers.clear();
                    }
                    if let Some(frame) = frame {
                        if let Some(texture) = frame.tex {
                            if !buffers.iter().any(|buffer| Arc::ptr_eq(buffer, &texture)) {
                                buffers.push(texture.clone());
                            }
                            latest.texture = Some(texture);
                            latest.capture_us = frame.capture_us;
                            latest.image += 1;
                        }
                        if frame.pointer.is_some() {
                            latest.pointer = frame.pointer;
                        }
                        if frame.shape.is_some() {
                            latest.shape = frame.shape;
                            latest.shape_version += 1;
                        }
                    }
                    latest.event += 1;
                    drop(latest);
                    source.changed.notify_all();
                } else if acquire_wait.is_zero() {
                    // Bound idle CPU usage without the blocking DXGI wait.
                    drop(source);
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
        *source.thread.lock().unwrap() = Some(thread);
        if ready_rx.recv_timeout(Duration::from_secs(5)).ok() != Some(true) {
            source.stopped.store(true, Ordering::Release);
            if let Some(thread) = source.thread.lock().unwrap().take() {
                let _ = thread.join();
            }
            return None;
        }
        sources.insert(source.display.id.clone(), Arc::downgrade(&source));
        Some(Self::subscribe(source))
    }
    fn subscribe(source: Arc<Source>) -> Self {
        Self {
            source,
            event: 0,
            image: 0,
            shape: 0,
            epoch: 0,
            reinit: true,
        }
    }
    pub fn device(&self) -> (ID3D11Device, ID3D11DeviceContext, i64, Display) {
        (
            self.source.dev.clone(),
            self.source.ctx.clone(),
            self.source.luid,
            self.source.display.clone(),
        )
    }
    pub fn size(&self) -> Option<(u32, u32)> {
        Some(self.source.latest.lock().unwrap().size)
    }
    #[cfg(test)]
    pub fn active_source_count() -> usize {
        registry()
            .lock()
            .unwrap()
            .values()
            .filter_map(Weak::upgrade)
            .filter(|s| !s.stopped.load(Ordering::Acquire))
            .count()
    }
    pub fn next(&mut self, timeout: Duration) -> Option<GpuFrame> {
        let mut latest = self.source.latest.lock().unwrap();
        if latest.event == self.event && !timeout.is_zero() {
            latest = self
                .source
                .changed
                .wait_timeout_while(latest, timeout, |latest| latest.event == self.event)
                .unwrap()
                .0;
        }
        if latest.event == self.event {
            return None;
        }
        self.event = latest.event;
        if self.epoch != latest.epoch {
            self.epoch = latest.epoch;
            self.reinit = true;
        }
        let tex = if latest.image != self.image {
            self.image = latest.image;
            latest.texture.clone()
        } else {
            None
        };
        let shape = if latest.shape_version != self.shape {
            self.shape = latest.shape_version;
            latest.shape.clone()
        } else {
            None
        };
        Some(GpuFrame {
            tex,
            capture_us: latest.capture_us,
            pointer: latest.pointer.clone(),
            shape,
        })
    }
}
impl Drop for SharedCapture {
    fn drop(&mut self) {
        // Serialize last-subscriber shutdown with the next subscribe: Windows
        // must release the old duplication before another one can be opened.
        let _sources = registry().lock().unwrap();
        if self.source.subscribers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.source.stopped.store(true, Ordering::Release);
            self.source.changed.notify_all();
            if let Some(thread) = self.source.thread.lock().unwrap().take() {
                let _ = thread.join();
            }
        }
    }
}
