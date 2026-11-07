//! Native NV12 decode stage. Feed and drain are independent: no per-frame wait
//! for a decoder that may need more compressed input before producing output.
use super::{dmabuf::{Exporter, ExportedFrame}, yuv_shader::{Color, Geometry}};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{AppSink, AppSinkCallbacks, AppSrc};
use gstreamer_video::VideoInfo;
use std::sync::{Arc, Mutex};

pub struct Frame {
    pub storage: Arc<ExportedFrame>,
    pub color: Color,
    pub geometry: Geometry,
}

pub struct NativeDecoder {
    decoder: Pipeline,
    exporter: Exporter,
}
impl NativeDecoder {
    pub fn open(factory: &str, fps: u32, ready: Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        // The legacy vaapi plugin uses a different memory ABI. Never feed its
        // buffers into the modern GstVa export functions.
        if factory != "vah264dec" { return Err("Native NV12 requires the GstVa decoder".into()); }
        let exporter = Exporter::open().map_err(|e| format!("VA export unavailable: {e:?}"))?;
        let decoder = Pipeline::open(factory, "video/x-raw(memory:VAMemory),format=NV12", fps, ready)?;
        Ok(Self { decoder, exporter })
    }
    pub fn submit(&mut self, bytes: &[u8], capture_us: u64) -> Result<(), String> {
        self.decoder.submit(bytes, capture_us)
    }
    pub fn take_latest(&self, modifiers: &[u64]) -> Result<Option<Frame>, String> {
        let Some(sample) = self.decoder.take_latest()? else { return Ok(None); };
        let info = VideoInfo::from_caps(sample.caps().ok_or("Native video has no format")?)
            .map_err(|_| "Invalid native video format")?;
        let color = Color::from_info(&info);
        let storage = Arc::new(self.exporter.export(sample, modifiers)
            .map_err(|e| format!("Native video export failed: {e:?}"))?);
        let geometry = Geometry::new(storage.dimensions(), storage.storage_dimensions(), info.chroma_site())?;
        Ok(Some(Frame { storage, color, geometry }))
    }
}

struct Pipeline {
    pipeline: gst::Pipeline,
    input: AppSrc,
    latest: Arc<Mutex<Option<gst::Sample>>>,
    clock: Clock,
    duration: gst::ClockTime,
}
impl Pipeline {
    fn open(decoder: &str, caps: &str, fps: u32, ready: Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        gst::init().map_err(|e| e.to_string())?;
        // All strings are internal constants, not remote/user-supplied pipeline text.
        let fps = fps.clamp(1,240);
        let description = format!("appsrc name=input is-live=true format=time block=false max-buffers=2 max-bytes=16777216 caps=video/x-h264,stream-format=byte-stream,alignment=au,framerate={fps}/1 ! h264parse ! {decoder} ! {caps} ! appsink name=output sync=false max-buffers=1 drop=true enable-last-sample=false");
        let pipeline = gst::parse::launch(&description).map_err(|e| e.to_string())?
            .downcast::<gst::Pipeline>().map_err(|_| "Invalid video pipeline")?;
        let input = pipeline.by_name("input").ok_or("Missing video input")?
            .downcast::<AppSrc>().map_err(|_| "Invalid video input")?;
        let output = pipeline.by_name("output").ok_or("Missing video output")?
            .downcast::<AppSink>().map_err(|_| "Invalid video output")?;
        let latest = Arc::new(Mutex::new(None));
        let target = latest.clone();
        output.set_callbacks(AppSinkCallbacks::builder().new_sample(move |sink| {
            let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
            // Only raw decoded output may be replaced. Compressed dependencies
            // remain ordered; overflowing input returns an error for IDR recovery.
            *target.lock().unwrap() = Some(sample);
            ready();
            Ok(gst::FlowSuccess::Ok)
        }).build());
        let this = Self { pipeline, input, latest, clock: Clock::default(),
            duration: gst::ClockTime::from_nseconds(1_000_000_000 / fps.clamp(1,240) as u64) };
        this.pipeline.set_state(gst::State::Playing).map_err(|_| "Native decoder could not start")?;
        Ok(this)
    }
    fn check(&self) -> Result<(), String> {
        if let Some(message) = self.pipeline.bus().and_then(|b| b.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos])) {
            return Err(match message.view() {
                gst::MessageView::Error(e) => format!("Native decoder: {}", e.error()),
                _ => "Native decoder stopped".into(),
            });
        }
        Ok(())
    }
    fn submit(&mut self, bytes: &[u8], capture_us: u64) -> Result<(), String> {
        self.check()?;
        if bytes.is_empty() || bytes.len() > 16*1024*1024 { return Err("Invalid compressed video size".into()); }
        if self.input.property::<u64>("current-level-buffers") >= 2 ||
            self.input.property::<u64>("current-level-bytes").saturating_add(bytes.len() as u64) > 16*1024*1024 {
            return Err("Native decoder input is full; a new keyframe is required".into());
        }
        let pts = self.clock.timestamp(capture_us)?;
        let mut buffer = gst::Buffer::from_mut_slice(bytes.to_vec());
        let writable = buffer.get_mut().ok_or("Video buffer is not writable")?;
        writable.set_pts(gst::ClockTime::from_nseconds(pts));
        writable.set_duration(self.duration);
        self.input.push_buffer(buffer).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn take_latest(&self) -> Result<Option<gst::Sample>, String> {
        self.check()?;
        Ok(self.latest.lock().unwrap().take())
    }
}
impl Drop for Pipeline {
    fn drop(&mut self) { let _ = self.pipeline.set_state(gst::State::Null); }
}
#[derive(Default)]
struct Clock { origin: Option<u64>, last: Option<u64> }
impl Clock {
    fn timestamp(&mut self, capture_us: u64) -> Result<u64, String> {
        if self.last.is_some_and(|last| capture_us < last) { return Err("Video timestamp moved backwards".into()); }
        let origin = self.origin.unwrap_or(capture_us);
        let pts = capture_us.checked_sub(origin).and_then(|us| us.checked_mul(1000))
            .filter(|ns| *ns < u64::MAX).ok_or("Video timestamp exceeds decoder limits")?;
        self.origin = Some(origin); self.last = Some(capture_us);
        Ok(pts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::atomic::{AtomicUsize, Ordering}, time::{Duration, Instant}};
    #[test]
    fn timestamps_follow_capture_spacing_and_reject_clock_reset_and_overflow() {
        let mut clock = Clock::default();
        assert_eq!(clock.timestamp(50_000_000).unwrap(), 0);
        assert_eq!(clock.timestamp(50_016_667).unwrap(), 16_667_000);
        assert!(clock.timestamp(49_000_000).is_err());
        assert_eq!(clock.timestamp(50_033_333).unwrap(), 33_333_000);
        assert!(clock.timestamp(u64::MAX).is_err());
    }
    #[test]
    fn asynchronous_nv12_stage_decodes_without_waiting_for_output_per_input() {
        use openh264::{encoder::Encoder, formats::{RgbSliceU8, YUVBuffer}};
        let callbacks = Arc::new(AtomicUsize::new(0));
        let signal = callbacks.clone();
        // CPU decoder is deliberately used only to test asynchronous plumbing.
        // This is not VA export, DMA import or hardware throughput evidence.
        let mut decoder = Pipeline::open("openh264dec ! videoconvert", "video/x-raw,format=NV12", 60,
            Arc::new(move || { signal.fetch_add(1, Ordering::Relaxed); })).unwrap();
        let rgb: Vec<u8> = (0..128*128).flat_map(|_| [40,200,20]).collect();
        let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb,(128,128)));
        let mut encoder = Encoder::new().unwrap();
        for sequence in 0..8 {
            let packet = encoder.encode(&yuv).unwrap().to_vec();
            decoder.submit(&packet, 50_000_000 + sequence*16_667).unwrap();
            // The caller's cadence supplies more input; submit never waits for
            // output, unlike the previous100ms pull after each compressed frame.
            std::thread::sleep(Duration::from_millis(20));
        }
        let deadline = Instant::now()+Duration::from_secs(2);
        while callbacks.load(Ordering::Relaxed)<8 && Instant::now()<deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(callbacks.load(Ordering::Relaxed),8,"All dependent frames decoded without draining output");
        let sample = decoder.take_latest().unwrap().expect("Latest NV12 fixture");
        assert!(decoder.take_latest().unwrap().is_none(),"Output retains one decoded sample, not a backlog");
        assert_eq!(sample.buffer().unwrap().pts(),Some(gst::ClockTime::from_nseconds(116_669_000)));
        let info = VideoInfo::from_caps(sample.caps().unwrap()).unwrap();
        assert_eq!((info.width(),info.height(),info.format()),(128,128,gstreamer_video::VideoFormat::Nv12));
        assert!(callbacks.load(Ordering::Relaxed)>0);
        assert_eq!(sample.buffer().unwrap().duration(),Some(decoder.duration));
        // Never misidentify a decoded system-memory fixture as a VA surface.
        assert!(Exporter::open().unwrap().export(sample,&[0]).is_err());
    }
}
