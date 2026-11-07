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
        Self::from_source(
            &source,
            (portal.width, portal.height),
            options,
            Some(portal),
        )
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
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
