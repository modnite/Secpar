//! Linux desktop sound-service fallback when ALSA's default PCM cannot open.
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::AppSrc;

pub struct Output {
    pipeline: gst::Pipeline,
    input: AppSrc,
    submitted: u64,
    error: Option<String>,
}
impl Output {
    pub fn open() -> Result<Self, String> {
        gst::init().map_err(|e| format!("Could not initialize desktop audio: {e}"))?;
        // Timestamp against the live pipeline clock: silence/background gaps must
        // not accumulate an old playback timeline. Bound queued PCM to 200 ms.
        let pipeline = gst::parse::launch(concat!(
            "appsrc name=input is-live=true format=time do-timestamp=true block=false ",
            "max-bytes=76800 leaky-type=downstream ",
            "caps=audio/x-raw,format=F32LE,layout=interleaved,rate=48000,channels=2 ",
            "! audioconvert ! audioresample ! pulsesink sync=false ",
            "buffer-time=40000 latency-time=10000 client-name=Secpar"
        ))
        .map_err(|e| format!("Could not open desktop sound service: {e}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "Invalid audio pipeline")?;
        let input = pipeline
            .by_name("input")
            .ok_or("Missing audio input")?
            .downcast::<AppSrc>()
            .map_err(|_| "Invalid audio input")?;
        let output = Self {
            pipeline,
            input,
            submitted: 0,
            error: None,
        };
        // Opening the sink happens in NULL -> READY, before waiting for PCM.
        // Detect an unavailable desktop service here so ALSA can be tried.
        output
            .pipeline
            .set_state(gst::State::Ready)
            .map_err(|e| format!("Could not connect to desktop sound service: {e}"))?;
        output
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Could not start desktop sound service: {e}"))?;
        Ok(output)
    }
    pub fn packet(&mut self, pcm: &[f32]) {
        let mut bytes = Vec::with_capacity(pcm.len() * 4);
        for sample in pcm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let mut buffer = gst::Buffer::from_mut_slice(bytes);
        buffer
            .get_mut()
            .unwrap()
            .set_duration(gst::ClockTime::from_nseconds(
                (pcm.len() / 2) as u64 * 1_000_000_000 / 48_000,
            ));
        if self.input.push_buffer(buffer).is_err() {
            self.error = Some("Desktop audio input stopped".into());
        } else {
            self.submitted += (pcm.len() / 2) as u64;
        }
    }
    pub fn failed(&self) -> bool {
        self.error.is_some()
            || self
                .pipeline
                .bus()
                .is_some_and(|bus| bus.pop_filtered(&[gst::MessageType::Error]).is_some())
    }
    pub fn details(&self) -> String {
        format!("Output: PulseAudio / PipeWire desktop default\nFrames submitted: {}\nFrames queued: {}",
            self.submitted, self.input.property::<u64>("current-level-bytes")/8)
    }
    #[cfg(test)]
    pub fn submitted_frames(&self) -> u64 {
        self.submitted
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
