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
    result
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
    format!("{source} ! queue leaky=downstream max-size-buffers=1 max-size-bytes=0 max-size-time=0 ! videoconvert ! videoscale ! videorate drop-only=true ! video/x-raw,format={},framerate={fps}/1{resolution} ! {} name=encoder {} ! {parser} config-interval=-1 ! {caps},stream-format=byte-stream,alignment=au ! appsink name=output sync=false max-buffers=2 drop=false enable-last-sample=false",encoder.raw,encoder.factory,encoder.properties)
}
/// The portal's stream as plain system-memory frames. KDE offers GPU buffers with tiling modifiers that
/// the GStreamer plugins can't always import ("error alloc buffers", "Invalid caps ... DMABuf"). On
/// an X250 the VA import of its DMA-BUF frames also fails on a driver size mismatch, and converting
/// system frames on the GPU measured slower than on the CPU, so frames are copied. `keepalive-time`
/// resends the last frame while the screen is still: Wayland only sends frames when something changes.
fn source_variants(base: &str, fps: u32) -> [String; 2] {
    let keepalive = (2000 / fps.max(1)).max(10);
    [format!("{base} always-copy=true keepalive-time={keepalive} ! video/x-raw"), format!("{base} keepalive-time={keepalive}")]
}
pub struct Capture {
    pipeline: gst::Pipeline,
    sink: AppSink,
    encoder_name: &'static str,
    codec: &'static str,
    size: (u32, u32),
    fps: u32,
    first: Option<EncodedFrame>,
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
        for variant in source_variants(&source, options.fps.clamp(1, 240)) {
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
        let mut failures = Vec::new();
        for encoder in candidates(&options) {
            if gst::ElementFactory::find(encoder.factory).is_none() {
                continue;
            }
            let pipeline = match gst::parse::launch(&description(source, &encoder, &options, native)) {
                Ok(p) => match p.downcast::<gst::Pipeline>() {
                    Ok(p) => p,
                    Err(_) => {failures.push(format!("{}: invalid capture pipeline",encoder.factory));continue;}
                },
                Err(error) => {failures.push(format!("{}: {error}",encoder.factory));continue;}
            };
            let Some(sink) = pipeline
                .by_name("output")
                .and_then(|e| e.downcast::<AppSink>().ok())
            else {
                failures.push(format!("{}: missing encoded output",encoder.factory));
                let _ = pipeline.set_state(gst::State::Null);
                continue;
            };
            if pipeline.set_state(gst::State::Playing).is_err() {
                failures.push(format!("{}: {}",encoder.factory,pipeline_error(&pipeline).unwrap_or_else(||"could not start capture".into())));
                let _ = pipeline.set_state(gst::State::Null);
                continue;
            }
            let mut capture = Self {
                pipeline,
                sink,
                encoder_name: encoder.factory,
                codec: encoder.codec,
                size: fit(native, (options.width, options.height)),
                fps: options.fps.clamp(1, 240),
                first: None,
                _portal: portal.clone(),
            };
            // Establish negotiated pixel dimensions before sending stream.ready. The
            // portal's logical size is not necessarily its pixel size on HiDPI.
            match capture.pull(Duration::from_secs(2)) {
                Ok(Some(frame)) => {
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
/// Pipelines of VA encoders that were stopped but not torn down; see `Drop for Capture`.
static PARKED: std::sync::Mutex<Vec<gst::Pipeline>> = std::sync::Mutex::new(Vec::new());
impl Drop for Capture {
    fn drop(&mut self) {
        if self.encoder_name.starts_with("va") {
            // Tearing down a VA encoder aborts the process on Intel's iHD driver ("free(): invalid
            // pointer" in vaDestroyContext), so one disconnect would end the whole app. Pause it and
            // park it instead: one idle pipeline per connection, freed when the host exits.
            let _ = self.pipeline.set_state(gst::State::Paused);
            if let Ok(mut parked) = PARKED.lock() {
                parked.push(self.pipeline.clone());
            }
            return;
        }
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
        for variant in source_variants(&base, options.fps) {
            let label = variant.split(" ! ").skip(1).collect::<Vec<_>>().join(" ! ");
            match Capture::from_source(&variant, (portal.width, portal.height), options.clone(), Some(portal.clone())) {
                Ok(c) => { worked += 1; eprintln!("OK   [{}] {}x{} via {}", label, c.size().0, c.size().1, c.encoder_name()); break; }
                Err(e) => eprintln!("FAIL [{label}] {e}"),
            }
        }
        assert!(worked > 0, "no variant captured");
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
