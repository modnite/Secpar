//! PipeWire -> low-latency encoder -> complete Annex-B access units.
//! Only the raw-frame queue is leaky; encoded dependent frames are never discarded.
use crate::portal::Portal;
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::AppSink;
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct VideoOptions {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
    pub codecs: Vec<String>,
}
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub key: bool,
    pub width: u32,
    pub height: u32,
    pub capture_us: u64,
}
struct Encoder {
    factory: &'static str,
    codec: &'static str,
    raw: &'static str,
    properties: String,
}
fn candidates(options: &VideoOptions) -> Vec<Encoder> {
    let mut result = Vec::new();
    let kbps = options.kbps.clamp(500, 100_000);
    let gop = options.fps.clamp(1, 240) * 2;
    for codec in &options.codecs {
        match codec.as_str() {
            "hevc" => {
                result.push(Encoder {
                    factory: "nvh265enc",
                    codec: "hevc",
                    raw: "NV12",
                    properties: format!(
                        "bitrate={kbps} gop-size={gop} bframes=0 zerolatency=true rc-mode=cbr"
                    ),
                });
                result.push(Encoder {
                    factory: "vah265enc",
                    codec: "hevc",
                    raw: "NV12",
                    properties: format!("bitrate={kbps} key-int-max={gop} rate-control=cbr"),
                });
                result.push(Encoder {
                    factory: "vaapih265enc",
                    codec: "hevc",
                    raw: "NV12",
                    properties: format!("bitrate={kbps} keyframe-period={gop} rate-control=cbr"),
                });
            }
            "h264" => {
                result.push(Encoder {
                    factory: "nvh264enc",
                    codec: "h264",
                    raw: "NV12",
                    properties: format!(
                        "bitrate={kbps} gop-size={gop} bframes=0 zerolatency=true rc-mode=cbr"
                    ),
                });
                result.push(Encoder {
                    factory: "vah264enc",
                    codec: "h264",
                    raw: "NV12",
                    properties: format!("bitrate={kbps} key-int-max={gop} rate-control=cbr"),
                });
                result.push(Encoder {
                    factory: "vaapih264enc",
                    codec: "h264",
                    raw: "NV12",
                    properties: format!("bitrate={kbps} keyframe-period={gop} rate-control=cbr"),
                });
            }
            _ => {}
        }
    }
    // Hardware candidates are exhausted before choosing a CPU encoder.
    if options.codecs.iter().any(|c| c == "h264") {
        result.push(Encoder{factory:"x264enc",codec:"h264",raw:"I420",properties:format!("tune=zerolatency speed-preset=ultrafast bitrate={kbps} key-int-max={gop} bframes=0 byte-stream=true aud=true")});
        result.push(Encoder {
            factory: "openh264enc",
            codec: "h264",
            raw: "I420",
            properties: format!(
                "bitrate={} gop-size={gop} complexity=low enable-frame-skip=false",
                kbps * 1000
            ),
        });
    }
    // SECPAR_ENCODER=<gstreamer element> forces one encoder (for comparing them on a machine).
    if let Ok(only) = std::env::var("SECPAR_ENCODER") {
        result.retain(|e| e.factory == only);
    }
    // SECPAR_ENCODER_PROPS="name=value ..." is appended to the encoder's properties (for tuning on a machine).
    if let Ok(extra) = std::env::var("SECPAR_ENCODER_PROPS") {
        for e in &mut result {
            e.properties.push(' ');
            e.properties.push_str(extra.trim());
        }
    }
    result
}
/// Makes the encoder see a live pipeline. The VA encoders ask upstream whether the source is live when their
/// input caps arrive, and keep a four frame output delay when it says no. `pipewiresrc` only starts saying yes
/// after its stream has started, which is after it announced the caps, so the answer at that moment is no:
/// every frame then left the encoder four frames late, which is 130 ms on a still screen at 30 frames/s
/// and the whole reason a pointer painted into the video trailed the real one. The source is live by nature.
fn claim_live(pipeline: &gst::Pipeline) {
    let Some(pad) = pipeline.by_name("size").and_then(|e| e.static_pad("src")) else { return };
    pad.add_probe(gst::PadProbeType::QUERY_UPSTREAM, |_, info| {
        if let Some(gst::PadProbeData::Query(query)) = info.data.as_mut() {
            if let gst::QueryViewMut::Latency(latency) = query.view_mut() {
                latency.set(true, gst::ClockTime::ZERO, gst::ClockTime::NONE);
                return gst::PadProbeReturn::Handled;
            }
        }
        gst::PadProbeReturn::Ok
    });
}
/// Stops a size change downstream from reaching the PipeWire source. Changing the caps of the `size` filter makes
/// it send a reconfigure event upstream, which `pipewiresrc` answers by disconnecting its stream and connecting it
/// again. That throws away the buffers, makes the compositor renegotiate and has been seen to leave the stream
/// delivering nothing. Only the converters in between need to hear about it: they stop at the queue.
fn isolate_source(pipeline: &gst::Pipeline) {
    let Some(pad) = pipeline.by_name("isolate").and_then(|e| e.static_pad("src")) else { return };
    pad.add_probe(gst::PadProbeType::EVENT_UPSTREAM, |_, info| match info.data.as_ref() {
        Some(gst::PadProbeData::Event(event)) if event.type_() == gst::EventType::Reconfigure => gst::PadProbeReturn::Drop,
        _ => gst::PadProbeReturn::Ok,
    });
}
fn fit(native: (u32, u32), wanted: (u32, u32)) -> (u32, u32) {
    if wanted.0 == 0 || wanted.1 == 0 {
        return native;
    }
    let scale = (wanted.0 as f64 / native.0 as f64)
        .min(wanted.1 as f64 / native.1 as f64)
        .min(1.0);
    (
        ((native.0 as f64 * scale) as u32 & !1).max(2),
        ((native.1 as f64 * scale) as u32 & !1).max(2),
    )
}
fn description(
    source: &str,
    encoder: &Encoder,
    options: &VideoOptions,
    native: (u32, u32),
) -> String {
    let fps = options.fps.clamp(1, 240);
    let resolution = if options.width == 0 || options.height == 0 {
        String::new()
    } else {
        let (w, h) = fit(native, (options.width, options.height));
        format!(",width={w},height={h}")
    };
    let (parser, caps) = if encoder.codec == "hevc" {
        ("h265parse", "video/x-h265")
    } else {
        ("h264parse", "video/x-h264")
    };
    // GPU frames (DMA-BUF) are imported by OpenGL (EGL) and colour-converted on the GPU, then read back as
    // the encoder's format, for any encoder. System-memory frames convert on the CPU. The last caps filter
    // is named "size": changing its caps resizes the running stream (see `Capture::set_size`).
    let convert = if source.contains("memory:DMABuf") { "glupload ! glcolorconvert ! gldownload" } else { "videoconvert" };
    let convert = format!("{convert} ! videoscale ! videorate drop-only=true ! capsfilter name=size caps=video/x-raw,format={},framerate={fps}/1{resolution}",encoder.raw);
    format!("{source} ! queue name=isolate leaky=downstream max-size-buffers=1 max-size-bytes=0 max-size-time=0 ! {convert} ! {} name=encoder {} ! {parser} config-interval=-1 ! {caps},stream-format=byte-stream,alignment=au ! appsink name=output sync=false max-buffers=2 drop=false enable-last-sample=false",encoder.factory,encoder.properties)
}
/// The portal's stream as plain system-memory frames. KDE offers GPU buffers with tiling modifiers that
/// the GStreamer plugins can't always import ("error alloc buffers", "Invalid caps ... DMABuf"). On
/// an X250 the VA import of its DMA-BUF frames also fails on a driver size mismatch, and converting
/// system frames on the GPU measured slower than on the CPU, so frames are copied. `keepalive-time`
/// resends the last frame while the screen is still: Wayland only sends frames when something changes.
fn source_variants(base: &str, fps: u32, native: (u32, u32)) -> Vec<String> {
    let keepalive = (2000 / fps.max(1)).max(10);
    // The source is pinned to the display's own size. Without this the size a client asks for travels
    // upstream through the converters, pipewiresrc offers it to the compositor and the whole PipeWire
    // stream is renegotiated: new buffers, and on a virtual output KWin even resizes the screen itself.
    // The video then stalls or the encoder is torn down mid-frame. Scaling belongs to `videoscale`.
    // The portal reports a logical size, which on a scaled (HiDPI) display is not the pixel size of the stream.
    // Pinning to it would then fail to negotiate, so the same sources without a pin follow as a fallback.
    let mut all = Vec::new();
    let mut sizes = vec![pinned(native)];
    if !sizes[0].is_empty() {
        sizes.push(String::new());
    }
    for fixed in &sizes {
        all.push(("dmabuf", format!("{base} keepalive-time={keepalive} ! video/x-raw(memory:DMABuf),format=DMA_DRM{fixed}")));
        all.push(("copy", format!("{base} always-copy=true keepalive-time={keepalive} ! video/x-raw{fixed}")));
        all.push(("plain", format!("{base} keepalive-time={keepalive} ! video/x-raw(ANY){fixed}")));
    }
    // SECPAR_CAPTURE=dmabuf|copy|plain forces one way of taking the frames (for comparing them)
    if let Ok(only) = std::env::var("SECPAR_CAPTURE") {
        all.retain(|(name, _)| *name == only);
    }
    all.into_iter().map(|(_, v)| v).collect()
}
/// Caps fields that pin a source to one size; nothing when the size is not known.
pub(crate) fn pinned(size: (u32, u32)) -> String {
    if size.0 == 0 || size.1 == 0 {
        String::new()
    } else {
        format!(",width={},height={}", size.0, size.1)
    }
}
pub struct Capture {
    pipeline: gst::Pipeline,
    sink: AppSink,
    encoder_name: &'static str,
    codec: &'static str,
    raw: &'static str,
    native: (u32, u32),
    size: (u32, u32),
    fps: u32,
    first: Option<EncodedFrame>,
    /// What this capture was built from, so a size change can build the same chain again.
    source: String,
    options: VideoOptions,
    _portal: Option<Arc<Portal>>,
}
impl Capture {
    pub fn start(portal: Arc<Portal>, options: VideoOptions) -> Result<Self, String> {
        gst::init().map_err(|_| "Could not initialize GStreamer.")?;
        let source = format!(
            "pipewiresrc fd={} path={} do-timestamp=true",
            portal.fd(),
            portal.node_id
        );
        // PipeWire may offer DMA-BUF buffers (with GPU-specific modifiers) that this pipeline can't
        // allocate: "error alloc buffers: Invalid argument". The next variant asks for copies instead.
        let mut failures = Vec::new();
        for variant in source_variants(&source, options.fps.clamp(1, 240), (portal.width, portal.height)) {
            match Self::from_source(&variant, (portal.width, portal.height), options.clone(), Some(portal.clone())) {
                Ok(capture) => return Ok(capture),
                Err(error) => failures.push(error),
            }
        }
        Err(if failures.is_empty() { "Could not start desktop capture.".into() } else { failures.join(" / ") })
    }
    fn from_source(
        source: &str,
        native: (u32, u32),
        options: VideoOptions,
        portal: Option<Arc<Portal>>,
    ) -> Result<Self, String> {
        Self::build(source, native, options, portal, None)
    }
    /// `only` limits the encoders to one factory: a size change builds the chain that is already known to work.
    fn build(
        source: &str,
        native: (u32, u32),
        options: VideoOptions,
        portal: Option<Arc<Portal>>,
        only: Option<&str>,
    ) -> Result<Self, String> {
        let mut failures = Vec::new();
        for encoder in candidates(&options) {
            if only.is_some_and(|o| o != encoder.factory) || gst::ElementFactory::find(encoder.factory).is_none() {
                continue;
            }
            let pipeline = match gst::parse::launch(&description(source, &encoder, &options, native)) {
                Ok(p) => match p.downcast::<gst::Pipeline>() {
                    Ok(p) => p,
                    Err(_) => {failures.push(format!("{}: invalid capture pipeline",encoder.factory));continue;}
                },
                Err(error) => {failures.push(format!("{}: {error}",encoder.factory));continue;}
            };
            claim_live(&pipeline);
            isolate_source(&pipeline);
            let Some(sink) = pipeline
                .by_name("output")
                .and_then(|e| e.downcast::<AppSink>().ok())
            else {
                failures.push(format!("{}: missing encoded output",encoder.factory));
                discard(pipeline, encoder.factory);
                continue;
            };
            if pipeline.set_state(gst::State::Playing).is_err() {
                failures.push(format!("{}: {}",encoder.factory,pipeline_error(&pipeline).unwrap_or_else(||"could not start capture".into())));
                discard(pipeline, encoder.factory);
                continue;
            }
            let mut capture = Self {
                pipeline,
                sink,
                encoder_name: encoder.factory,
                codec: encoder.codec,
                raw: encoder.raw,
                native,
                size: fit(native, (options.width, options.height)),
                fps: options.fps.clamp(1, 240),
                first: None,
                source: source.to_string(),
                options: options.clone(),
                _portal: portal.clone(),
            };
            // Establish negotiated pixel dimensions before sending stream.ready. The
            // portal's logical size is not necessarily its pixel size on HiDPI.
            match capture.pull(Duration::from_secs(2)) {
                Ok(Some(frame)) => {
                    // A picture smaller than the display says nothing about the display's own size.
                    if fit(native, (options.width, options.height)) == native {
                        capture.native = (frame.width, frame.height);
                    }
                    eprintln!("secpar: capture {} {} asked {}x{} display {}x{} first frame {}x{}", encoder.factory, encoder.codec, options.width, options.height, native.0, native.1, frame.width, frame.height);
                    capture.size = (frame.width, frame.height);
                    capture.first = Some(frame);
                    return Ok(capture);
                }
                Ok(None) => failures.push(format!("{}: no encoded frame received within 2 seconds",encoder.factory)),
                Err(error) => failures.push(format!("{}: {error}",encoder.factory)),
            }
        }
        Err(if failures.is_empty() {"No compatible GStreamer encoder is installed. Install gst-plugins-ugly (x264) or the VA/NVENC plugins.".into()}else{format!("Could not start desktop capture: {}",failures.join("; "))})
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    pub fn fps(&self) -> u32 {
        self.fps
    }
    pub fn codec(&self) -> &'static str {
        self.codec
    }
    pub fn encoder_name(&self) -> &'static str {
        self.encoder_name
    }
    /// Resizes the running stream to fit `wanted` (0x0 = the display's own size) and starts it with a keyframe.
    /// Returns the new size, or None when the stream could not be resized (it then keeps running as it was).
    ///
    /// Software encoders renegotiate in place. A VA encoder is not asked to: changing its input size makes it
    /// destroy and recreate its VA context, and destroying that context aborts the whole process on Intel's
    /// iHD driver (the same crash `discard` works around). It gets a new chain beside the old one instead, and
    /// the old one is parked like on a disconnect.
    pub fn set_size(&mut self, wanted: (u32, u32)) -> Option<(u32, u32)> {
        if replaces_chain_on_resize(self.encoder_name, parked_chains()) {
            return self.set_size_by_new_chain(wanted);
        }
        self.set_size_in_place(wanted)
    }
    fn set_size_in_place(&mut self, wanted: (u32, u32)) -> Option<(u32, u32)> {
        let filter = self.pipeline.by_name("size")?;
        let (w, h) = fit(self.native, wanted);
        let mut caps = gst::Caps::builder("video/x-raw").field("format", self.raw).field("framerate", gst::Fraction::new(self.fps as i32, 1));
        if wanted.0 != 0 && wanted.1 != 0 {
            caps = caps.field("width", w as i32).field("height", h as i32);
        }
        filter.set_property("caps", caps.build());
        self.size = (w, h);
        self.options.width = wanted.0;
        self.options.height = wanted.1;
        Some((w, h))
    }
    fn set_size_by_new_chain(&mut self, wanted: (u32, u32)) -> Option<(u32, u32)> {
        let mut options = self.options.clone();
        options.width = wanted.0;
        options.height = wanted.1;
        // two captures of one stream would compete for its frames while the new one starts
        let _ = self.pipeline.set_state(gst::State::Paused);
        match Self::build(&self.source, self.native, options, self._portal.clone(), Some(self.encoder_name)) {
            Ok(mut fresh) => {
                std::mem::swap(self, &mut fresh);
                // `fresh` is the old chain now and is parked when it goes out of scope
                Some(self.size)
            }
            Err(_) => {
                let _ = self.pipeline.set_state(gst::State::Playing);
                None
            }
        }
    }
    pub fn request_keyframe(&self) {
        let event = gstreamer_video::UpstreamForceKeyUnitEvent::builder()
            .all_headers(true)
            .build();
        let _ = self.sink.send_event(event);
    }
    pub fn next_frame(&mut self, timeout: Duration) -> Result<Option<EncodedFrame>, String> {
        if let Some(frame) = self.first.take() {
            return Ok(Some(frame));
        }
        self.pull(timeout)
    }
    fn pull(&self, timeout: Duration) -> Result<Option<EncodedFrame>, String> {
        let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::from_nseconds(
            timeout.as_nanos().min(u64::MAX as u128) as u64,
        )) else {
            if let Some(error) = pipeline_error(&self.pipeline) {return Err(error);}
            return Ok(None);
        };
        let buffer = sample.buffer().ok_or("Encoder returned an empty sample.")?;
        let caps = sample
            .caps()
            .and_then(|c| c.structure(0))
            .ok_or("Encoder omitted stream dimensions.")?;
        let width = caps
            .get::<i32>("width")
            .ok()
            .filter(|n| *n > 0)
            .ok_or("Encoder omitted stream width.")? as u32;
        let height = caps
            .get::<i32>("height")
            .ok()
            .filter(|n| *n > 0)
            .ok_or("Encoder omitted stream height.")? as u32;
        let map = buffer
            .map_readable()
            .map_err(|_| "Could not read an encoded frame.")?;
        let running = self
            .pipeline
            .current_running_time()
            .map(|t| t.nseconds())
            .unwrap_or(0);
        let age = buffer
            .pts()
            .map(|p| running.saturating_sub(p.nseconds()) / 1000)
            .unwrap_or(0);
        Ok(Some(EncodedFrame {
            data: map.as_slice().to_vec(),
            key: !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT),
            width,
            height,
            capture_us: crate::now_us().saturating_sub(age),
        }))
    }
}
fn pipeline_error(pipeline: &gst::Pipeline) -> Option<String> {
    let message=pipeline.bus()?.pop_filtered(&[gst::MessageType::Error,gst::MessageType::Eos])?;
    Some(match message.view() {
        gst::MessageView::Error(error)=>format!("{}: {}",error.src().map(|s|s.name().to_string()).unwrap_or_else(||"capture".into()),error.error()),
        _=>"Desktop capture ended before another frame was available".into(),
    })
}
/// Pipelines of VA encoders that were stopped but not torn down; see `discard`.
static PARKED: std::sync::Mutex<Vec<gst::Pipeline>> = std::sync::Mutex::new(Vec::new());
/// Idle parked chains the host keeps before it resizes a VA stream in place again (about 10 MB each).
const MAX_PARKED: usize = 24;
fn parked_chains() -> usize {
    PARKED.lock().map_or(MAX_PARKED, |p| p.len())
}
/// Whether a size change builds a new chain instead of renegotiating the running encoder.
fn replaces_chain_on_resize(encoder: &str, parked: usize) -> bool {
    encoder.starts_with("va") && parked < MAX_PARKED
}
/// Stops a chain that is no longer used. Tearing down a VA encoder aborts the process on Intel's iHD driver
/// ("free(): invalid pointer" in vaDestroyContext), so one disconnect would end the whole app. A VA chain is
/// paused and parked instead, freed when the host exits; the others are torn down.
fn discard(pipeline: gst::Pipeline, encoder: &str) {
    if encoder.starts_with("va") {
        let _ = pipeline.set_state(gst::State::Paused);
        if let Ok(mut parked) = PARKED.lock() {
            parked.push(pipeline);
        }
        return;
    }
    let _ = pipeline.set_state(gst::State::Null);
}
impl Drop for Capture {
    fn drop(&mut self) {
        discard(self.pipeline.clone(), self.encoder_name);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample_options() -> VideoOptions {
        VideoOptions { width: 0, height: 0, fps: 60, kbps: 8000, codecs: vec!["h264".into()] }
    }
    #[test]
    fn dmabuf_frames_convert_on_the_gpu_and_system_frames_on_the_cpu() {
        let enc = &candidates(&sample_options())[1]; // vah264enc
        let gpu = description("pipewiresrc ! video/x-raw(memory:DMABuf),format=DMA_DRM", enc, &sample_options(), (1366, 768));
        assert!(gpu.contains("glupload ! glcolorconvert ! gldownload"), "{gpu}");
        assert!(!gpu.contains("videoconvert"), "{gpu}");
        let cpu = description("pipewiresrc ! video/x-raw", enc, &sample_options(), (1366, 768));
        assert!(cpu.contains("videoconvert ! videoscale"), "{cpu}");
        assert!(!cpu.contains("glupload"), "{cpu}");
    }
    #[test]
    fn every_chain_ends_in_a_named_size_filter_and_variants_try_the_gpu_first() {
        let enc = &candidates(&sample_options())[1];
        for source in ["pipewiresrc ! video/x-raw(memory:DMABuf),format=DMA_DRM", "pipewiresrc ! video/x-raw"] {
            let d = description(source, enc, &sample_options(), (1366, 768));
            assert!(d.contains("capsfilter name=size caps=video/x-raw,format=NV12,framerate=60/1"), "{d}");
        }
        let v = source_variants("pipewiresrc fd=3 path=7", 60, (1366, 768));
        assert_eq!(v.len(), 6);
        assert!(v[0].contains("memory:DMABuf") && v[1].contains("always-copy=true") && v[2].contains("video/x-raw(ANY)"));
        assert!(v.iter().all(|s| s.contains("keepalive-time=33")), "{v:?}");
    }
    #[test]
    fn a_requested_size_is_baked_into_the_filter_caps_and_stays_inside_the_request() {
        let mut options = sample_options();
        options.width = 1280;
        options.height = 720;
        let enc = &candidates(&options)[1];
        let d = description("pipewiresrc ! video/x-raw", enc, &options, (1366, 768));
        let (w, h) = fit((1366, 768), (1280, 720));
        assert!(w <= 1280 && h <= 720 && w % 2 == 0 && h % 2 == 0, "{w}x{h}");
        assert!(d.contains(&format!("width={w},height={h}")), "{d}");
        assert_eq!(fit((1366, 768), (854, 480)), (852, 480));
        assert_eq!(fit((1366, 768), (0, 0)), (1366, 768));
    }
    /// The test's own saved screen-share permission (portal tokens rotate; the app's stays untouched).
    /// The first run asks on screen once and saves it to ~/secpar-engine-test/portal-token.
    fn test_portal(rt: &tokio::runtime::Runtime) -> Arc<crate::portal::Portal> {
        let path = format!("{}/secpar-engine-test/portal-token", std::env::var("HOME").unwrap());
        let token = std::fs::read_to_string(&path).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let portal = rt.block_on(crate::portal::Portal::open(token.as_deref())).expect("portal");
        if let Some(new) = &portal.restore_token {
            let _ = std::fs::write(&path, new);
        }
        portal
    }
    /// By hand with something moving on screen: the running stream changes size and goes back.
    #[test]
    #[ignore = "needs a desktop session and a saved screen-share permission"]
    fn portal_resize_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let portal = test_portal(&rt);
        let options = VideoOptions { width: 0, height: 0, fps: 60, kbps: 12000, codecs: vec!["h264".into()] };
        let mut capture = Capture::start(portal.clone(), options).expect("capture");
        let native = capture.size();
        let wait = |capture: &mut Capture, ok: &dyn Fn(u32, u32) -> bool| {
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_secs(6) {
                if let Ok(Some(f)) = capture.next_frame(Duration::from_millis(200)) {
                    if ok(f.width, f.height) {
                        return Some((f.width, f.height, f.key));
                    }
                }
            }
            None
        };
        let small = capture.set_size((854, 480)).expect("size filter");
        let got_small = wait(&mut capture, &|w, h| w <= 854 && h <= 480);
        let back = capture.set_size((0, 0)).expect("size filter");
        let got_back = wait(&mut capture, &|w, h| (w, h) == native);
        eprintln!("RESIZE native {native:?} -> asked {small:?} got {got_small:?}; back to {back:?} got {got_back:?}");
        assert!(got_small.is_some() && got_back.is_some());
    }
    /// By hand with something moving on screen: how many frames each stage of the pipeline can pass,
    /// to find the one that limits the stream.
    #[test]
    #[ignore = "needs a desktop session and a saved screen-share permission"]
    fn portal_stage_rates() {
        use gstreamer::prelude::*;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let portal = test_portal(&rt);
        gst::init().unwrap();
        let base = format!("pipewiresrc fd={} path={} do-timestamp=true", portal.fd(), portal.node_id);
        let tail = |sink: &str| format!("! appsink name=output sync=false max-buffers=2 drop=true enable-last-sample=false {sink}");
        let stages = [
            ("capture only (copy)", format!("{base} always-copy=true keepalive-time=33 ! video/x-raw {}", tail(""))),
            ("capture only (plain)", format!("{base} keepalive-time=33 {}", tail(""))),
            ("capture + videoconvert NV12", format!("{base} always-copy=true keepalive-time=33 ! video/x-raw ! videoconvert ! video/x-raw,format=NV12 {}", tail(""))),
            ("dmabuf capture only", format!("{base} keepalive-time=33 ! video/x-raw(memory:DMABuf),format=DMA_DRM {}", tail(""))),
            ("dmabuf + vapostproc", format!("{base} keepalive-time=33 ! video/x-raw(memory:DMABuf),format=DMA_DRM ! vapostproc ! video/x-raw(memory:VAMemory),format=NV12 {}", tail(""))),
            ("dmabuf + glupload/glcolorconvert/gldownload", format!("{base} keepalive-time=33 ! video/x-raw(memory:DMABuf),format=DMA_DRM ! glupload ! glcolorconvert ! gldownload ! video/x-raw,format=NV12 {}", tail(""))),
            ("capture + convert + vah264enc", format!("{base} always-copy=true keepalive-time=33 ! video/x-raw ! videoconvert ! video/x-raw,format=NV12 ! vah264enc bitrate=12000 key-int-max=120 rate-control=cbr ! h264parse {}", tail(""))),
        ];
        // one stage per run (SECPAR_STAGE=index): a second pipeline on the same portal stream won't start
        let only = std::env::var("SECPAR_STAGE").ok().and_then(|v| v.parse::<usize>().ok());
        for (index, (name, text)) in stages.into_iter().enumerate() {
            if only.is_some_and(|o| o != index) {
                continue;
            }
            let Ok(p) = gst::parse::launch(&text) else { eprintln!("STAGE {name}: pipeline did not parse"); continue };
            let pipeline = p.downcast::<gst::Pipeline>().unwrap();
            let sink = pipeline.by_name("output").unwrap().downcast::<AppSink>().unwrap();
            if pipeline.set_state(gst::State::Playing).is_err() { eprintln!("STAGE {name}: would not start"); continue; }
            let (mut n, start) = (0u32, std::time::Instant::now());
            while start.elapsed() < Duration::from_secs(6) {
                if sink.try_pull_sample(gst::ClockTime::from_mseconds(200)).is_some() { n += 1; }
            }
            eprintln!("STAGE {name}: {:.1} fps", n as f64 / start.elapsed().as_secs_f64());
            let _ = pipeline.set_state(gst::State::Paused); // not Null: Intel's driver aborts on teardown
            std::mem::forget(pipeline);
        }
    }
    /// By hand with something moving on screen: frames per second and CPU use of the capture and
    /// encode pipeline (SECPAR_ENCODER picks the encoder).
    #[test]
    #[ignore = "needs a desktop session and a saved screen-share permission"]
    fn portal_stream_throughput() {
        fn cpu_ticks() -> u64 {
            let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
            let rest: Vec<&str> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
            rest[11].parse::<u64>().unwrap() + rest[12].parse::<u64>().unwrap()
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let portal = test_portal(&rt);
        let options = VideoOptions { width: 0, height: 0, fps: 60, kbps: 12000, codecs: vec!["h264".into()] };
        let mut capture = Capture::start(portal.clone(), options).expect("capture");
        let (name, size) = (capture.encoder_name(), capture.size());
        let (mut frames, mut bytes, mut worst) = (0u32, 0usize, Duration::ZERO);
        let (start, cpu0) = (std::time::Instant::now(), cpu_ticks());
        let mut last = start;
        while start.elapsed() < Duration::from_secs(8) {
            if let Ok(Some(f)) = capture.next_frame(Duration::from_millis(200)) {
                frames += 1;
                bytes += f.data.len();
                worst = worst.max(last.elapsed());
                last = std::time::Instant::now();
            }
        }
        let secs = start.elapsed().as_secs_f64();
        eprintln!("BENCH {name} {}x{}: {:.1} fps, {:.1} Mbps, worst gap {} ms, cpu {:.0}%", size.0, size.1, frames as f64 / secs, bytes as f64 * 8.0 / secs / 1e6, worst.as_millis(), (cpu_ticks() - cpu0) as f64 / 100.0 / secs * 100.0);
    }
    /// By hand on a Plasma session with a saved screen-share permission: which source variants
    /// capture here. Prints one line per variant.
    #[test]
    #[ignore = "needs a desktop session and a saved screen-share permission"]
    fn portal_stream_captures_with_each_source_variant() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let portal = test_portal(&rt);
        gst::init().unwrap();
        let base = format!("pipewiresrc fd={} path={} do-timestamp=true", portal.fd(), portal.node_id);
        let options = VideoOptions { width: 0, height: 0, fps: 30, kbps: 6000, codecs: vec!["h264".into()] };
        let mut worked = 0;
        for variant in source_variants(&base, options.fps, (portal.width, portal.height)) {
            let label = variant.split(" ! ").skip(1).collect::<Vec<_>>().join(" ! ");
            match Capture::from_source(&variant, (portal.width, portal.height), options.clone(), Some(portal.clone())) {
                Ok(c) => { worked += 1; eprintln!("OK   [{}] {}x{} via {}", label, c.size().0, c.size().1, c.encoder_name()); break; }
                Err(e) => eprintln!("FAIL [{label}] {e}"),
            }
        }
        assert!(worked > 0, "no variant captured");
    }
    #[test]
    fn a_va_encoder_is_never_renegotiated_in_place() {
        assert!(replaces_chain_on_resize("vah264enc", 0));
        assert!(replaces_chain_on_resize("vah265enc", MAX_PARKED - 1));
        assert!(!replaces_chain_on_resize("vah264enc", MAX_PARKED), "parked chains are bounded");
        assert!(!replaces_chain_on_resize("x264enc", 0));
        assert!(!replaces_chain_on_resize("nvh264enc", 0));
        assert!(!replaces_chain_on_resize("openh264enc", 0));
    }
    #[test]
    fn every_source_variant_is_pinned_to_the_display_size() {
        let all = source_variants("pipewiresrc fd=3 path=7", 60, (1366, 768));
        for variant in &all[..3] {
            assert!(variant.contains(",width=1366,height=768"), "{variant}");
        }
        // a scaled display reports a logical size that is not the stream's pixel size: unpinned fallbacks follow
        for variant in &all[3..] {
            assert!(!variant.contains("width="), "{variant}");
        }
        // an unknown size pins nothing instead of asking for 0x0
        for variant in source_variants("pipewiresrc fd=3 path=7", 60, (0, 0)) {
            assert!(!variant.contains("width="), "{variant}");
        }
        assert_eq!(pinned((1920, 1080)), ",width=1920,height=1080");
        assert_eq!(pinned((0, 1080)), "");
    }
    fn synthetic(options: VideoOptions) -> Capture {
        gst::init().unwrap();
        Capture::from_source("videotestsrc is-live=true pattern=ball ! video/x-raw,width=320,height=180", (320, 180), options, None)
            .expect("Synthetic H264 encoder requires x264 or OpenH264; no desktop capture is involved")
    }
    fn wait_for_size(capture: &mut Capture, w: u32, h: u32) -> bool {
        for _ in 0..60 {
            if let Ok(Some(f)) = capture.next_frame(Duration::from_millis(100)) {
                if (f.width, f.height) == (w, h) {
                    return true;
                }
            }
        }
        false
    }
    #[test]
    fn the_running_stream_changes_size_and_comes_back_to_the_display_size() {
        let mut capture = synthetic(VideoOptions { width: 0, height: 0, fps: 30, kbps: 2000, codecs: vec!["h264".into()] });
        assert_eq!(capture.size(), (320, 180));
        assert_eq!(capture.set_size_in_place((160, 90)), Some((160, 90)));
        assert!(wait_for_size(&mut capture, 160, 90), "frames at the smaller size");
        assert_eq!(capture.set_size_in_place((0, 0)), Some((320, 180)));
        assert!(wait_for_size(&mut capture, 320, 180), "frames at the display size again");
    }
    #[test]
    fn a_new_chain_takes_over_without_touching_the_display_size() {
        let mut capture = synthetic(VideoOptions { width: 0, height: 0, fps: 30, kbps: 2000, codecs: vec!["h264".into()] });
        let native = capture.native;
        assert_eq!(capture.set_size_by_new_chain((160, 90)), Some((160, 90)));
        assert_eq!(capture.native, native, "a small picture does not redefine the display");
        let first = capture.next_frame(Duration::from_secs(2)).unwrap().expect("the new chain's first frame");
        assert!(first.key && (first.width, first.height) == (160, 90));
        assert_eq!(capture.set_size_by_new_chain((0, 0)), Some((320, 180)));
        assert!(wait_for_size(&mut capture, 320, 180));
    }
    #[test]
    fn a_size_change_never_reaches_the_pipewire_source() {
        gst::init().unwrap();
        let options = VideoOptions { width: 0, height: 0, fps: 30, kbps: 2000, codecs: vec!["h264".into()] };
        let encoder = candidates(&options).pop().unwrap();
        let pipeline = gst::parse::launch(&description("videotestsrc name=fake_source is-live=true pattern=ball ! video/x-raw,width=320,height=180", &encoder, &options, (320, 180)))
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        isolate_source(&pipeline);
        let reached = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = reached.clone();
        // what pipewiresrc would see: upstream events leaving the first element
        pipeline.by_name("fake_source").unwrap().static_pad("src").unwrap().add_probe(gst::PadProbeType::EVENT_UPSTREAM, move |_, info| {
            if let Some(gst::PadProbeData::Event(e)) = info.data.as_ref() {
                if e.type_() == gst::EventType::Reconfigure {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            gst::PadProbeReturn::Ok
        });
        pipeline.set_state(gst::State::Playing).unwrap();
        let sink = pipeline.by_name("output").unwrap().downcast::<AppSink>().unwrap();
        assert!(sink.try_pull_sample(gst::ClockTime::from_seconds(3)).is_some());
        let before = reached.load(std::sync::atomic::Ordering::SeqCst);
        let filter = pipeline.by_name("size").unwrap();
        let caps = gst::Caps::builder("video/x-raw").field("format", "I420").field("framerate", gst::Fraction::new(30, 1)).field("width", 160i32).field("height", 90i32).build();
        filter.set_property("caps", caps);
        let mut got = false;
        for _ in 0..40 {
            if let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_mseconds(100)) {
                let s = sample.caps().and_then(|c| c.structure(0).map(|s| s.get::<i32>("width").unwrap_or(0))).unwrap_or(0);
                if s == 160 {
                    got = true;
                    break;
                }
            }
        }
        let _ = pipeline.set_state(gst::State::Null);
        assert!(got, "the picture changed size");
        assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), before, "no reconfigure reached the source");
    }
    #[test]
    fn the_encoder_is_told_the_source_is_live() {
        gst::init().unwrap();
        let options = VideoOptions { width: 0, height: 0, fps: 30, kbps: 2000, codecs: vec!["h264".into()] };
        let encoder = candidates(&options).pop().unwrap();
        // not a live source: the answer upstream is "not live", as pipewiresrc gives while it negotiates
        let pipeline = gst::parse::launch(&description("videotestsrc", &encoder, &options, (320, 180))).unwrap().downcast::<gst::Pipeline>().unwrap();
        let pad = pipeline.by_name("size").unwrap().static_pad("src").unwrap();
        let ask = |pad: &gst::Pad| {
            let mut query = gst::query::Latency::new();
            pad.query(&mut query);
            query.result().0
        };
        assert!(!ask(&pad), "without the claim this chain is not live");
        claim_live(&pipeline);
        assert!(ask(&pad), "the encoder must see a live source and so keep no output delay");
    }
    #[test]
    fn aspect_ratio_and_native_request_are_preserved() {
        assert_eq!(fit((1920, 1200), (1280, 720)), (1152, 720));
        assert_eq!(fit((1920, 1080), (0, 0)), (1920, 1080));
    }
    #[test]
    fn only_raw_frames_may_be_dropped() {
        let options = VideoOptions {
            width: 320,
            height: 180,
            fps: 60,
            kbps: 2000,
            codecs: vec!["h264".into()],
        };
        let candidate = candidates(&options).pop().unwrap();
        let text = description("videotestsrc", &candidate, &options, (320, 180));
        assert!(text.find("leaky=downstream").unwrap() < text.find("name=encoder").unwrap());
        assert!(text.contains("alignment=au"));
        assert!(text.contains("max-buffers=2 drop=false"));
    }
    #[test]
    fn synthetic_video_encodes_complete_annexb_frames_and_forces_idr() {
        gst::init().unwrap();
        let options = VideoOptions {
            width: 320,
            height: 180,
            fps: 60,
            kbps: 2000,
            codecs: vec!["h264".into()],
        };
        let mut capture = Capture::from_source(
            "videotestsrc is-live=true pattern=ball",
            (320, 180),
            options,
            None,
        )
        .expect("Synthetic H264 encoder requires x264 or OpenH264; no desktop capture is involved");
        let first = capture.next_frame(Duration::from_secs(1)).unwrap().unwrap();
        assert!(first.key);
        assert_eq!((first.width, first.height), (320, 180));
        assert!(first.data.windows(3).any(|b| b == [0, 0, 1]));
        capture.request_keyframe();
        let mut key = false;
        for _ in 0..10 {
            if let Some(frame) = capture.next_frame(Duration::from_millis(100)).unwrap() {
                assert!(frame.capture_us <= crate::now_us());
                key |= frame.key;
            }
        }
        assert!(key, "On-demand recovery keyframe");
    }
    #[test]
    fn startup_preserves_the_actual_capture_failure() {
        gst::init().unwrap();
        let options=VideoOptions {width:320,height:180,fps:30,kbps:2000,codecs:vec!["h264".into()]};
        let result=Capture::from_source("secpar_missing_capture_source",(320,180),options,None);
        let error=result.err().expect("Missing synthetic source must fail");
        assert!(error.contains("secpar_missing_capture_source"),"{error}");
        assert!(error.contains("264enc"),"Encoder attempt must be identified: {error}");
    }
}
