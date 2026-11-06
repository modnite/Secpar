//! ScreenCaptureKit capture + VideoToolbox H.264 encoding + system audio.
//! macOS only. WRITTEN WITHOUT A MAC TO COMPILE ON: treat as unverified until the
//! Mac runner builds it and a person has watched a stream.
use crate::{
    annexb::avcc_to_annexb,
    convert::{fit_size, interleave_stereo},
    media::{now_us, AudioChunk, AudioSource, Frame, VideoSource},
};
use core_foundation::{
    base::{CFRelease, CFTypeRef, TCFType},
    boolean::CFBoolean,
    dictionary::{CFDictionary, CFDictionaryRef},
    number::CFNumber,
    string::{CFString, CFStringRef},
};
use screencapturekit::{
    cm::{CMSampleBuffer, CMSampleBufferExt, CMSampleBufferSCExt, SCFrameStatus},
    prelude::*,
};
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct Cmt {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}
type Callback = extern "C" fn(*mut c_void, *mut c_void, i32, u32, *mut c_void);
#[link(name = "VideoToolbox", kind = "framework")]
extern "C" {
    static kVTCompressionPropertyKey_RealTime: CFStringRef;
    static kVTCompressionPropertyKey_ProfileLevel: CFStringRef;
    static kVTCompressionPropertyKey_AllowFrameReordering: CFStringRef;
    static kVTCompressionPropertyKey_AverageBitRate: CFStringRef;
    static kVTCompressionPropertyKey_ExpectedFrameRate: CFStringRef;
    static kVTCompressionPropertyKey_MaxKeyFrameInterval: CFStringRef;
    static kVTProfileLevel_H264_High_AutoLevel: CFStringRef;
    static kVTEncodeFrameOptionKey_ForceKeyFrame: CFStringRef;
    static kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: CFStringRef;
    fn VTCompressionSessionCreate(
        allocator: *const c_void,
        width: i32,
        height: i32,
        codec: u32,
        encoder_spec: CFDictionaryRef,
        source_attrs: CFDictionaryRef,
        compressed_allocator: *const c_void,
        callback: Callback,
        refcon: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn VTSessionSetProperty(session: *mut c_void, key: CFStringRef, value: CFTypeRef) -> i32;
    fn VTCompressionSessionPrepareToEncodeFrames(session: *mut c_void) -> i32;
    fn VTCompressionSessionEncodeFrame(
        session: *mut c_void,
        image: *mut c_void,
        pts: Cmt,
        duration: Cmt,
        props: CFDictionaryRef,
        frame_refcon: *mut c_void,
        flags_out: *mut u32,
    ) -> i32;
    fn VTCompressionSessionCompleteFrames(session: *mut c_void, until: Cmt) -> i32;
    fn VTCompressionSessionInvalidate(session: *mut c_void);
}
#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    static kCMSampleAttachmentKey_NotSync: CFStringRef;
    fn CMSampleBufferGetDataBuffer(sbuf: *mut c_void) -> *mut c_void;
    fn CMSampleBufferGetFormatDescription(sbuf: *mut c_void) -> *mut c_void;
    fn CMSampleBufferGetSampleAttachmentsArray(sbuf: *mut c_void, create: bool) -> *const c_void;
    fn CMBlockBufferGetDataPointer(
        buffer: *mut c_void,
        offset: usize,
        length_at_offset: *mut usize,
        total_length: *mut usize,
        data: *mut *mut u8,
    ) -> i32;
    fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        desc: *mut c_void,
        index: usize,
        set: *mut *const u8,
        size: *mut usize,
        count: *mut usize,
        nal_header_len: *mut i32,
    ) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryContainsKey(dict: *const c_void, key: *const c_void) -> bool;
}
const AVC1: u32 = 0x6176_6331;

/// Output callback: runs on a VideoToolbox thread. Never panics across the FFI boundary.
extern "C" fn on_encoded(refcon: *mut c_void, capture_us: *mut c_void, status: i32, _flags: u32, sbuf: *mut c_void) {
    if status != 0 || sbuf.is_null() || refcon.is_null() {
        return;
    }
    let tx = unsafe { &*(refcon as *const mpsc::SyncSender<Frame>) };
    let frame = std::panic::catch_unwind(|| unsafe { frame_from(sbuf, capture_us as usize as u64) });
    if let Ok(Some(frame)) = frame {
        let _ = tx.send(frame); // blocks when the network side is slow; SCK then drops raw frames
    }
}
unsafe fn frame_from(sbuf: *mut c_void, capture_us: u64) -> Option<Frame> {
    let block = CMSampleBufferGetDataBuffer(sbuf);
    let (mut at_offset, mut total, mut ptr) = (0usize, 0usize, std::ptr::null_mut::<u8>());
    if block.is_null() || CMBlockBufferGetDataPointer(block, 0, &mut at_offset, &mut total, &mut ptr) != 0 || ptr.is_null() || at_offset != total {
        return None; // non-contiguous buffers are not produced by VideoToolbox H.264
    }
    let attachments = CMSampleBufferGetSampleAttachmentsArray(sbuf, false);
    let key = attachments.is_null()
        || CFArrayGetCount(attachments) == 0
        || !CFDictionaryContainsKey(CFArrayGetValueAtIndex(attachments, 0), kCMSampleAttachmentKey_NotSync as *const c_void);
    let desc = CMSampleBufferGetFormatDescription(sbuf);
    let (mut sets, mut count, mut nal_len) = (Vec::new(), 0usize, 4i32);
    if key && !desc.is_null() {
        let (mut p, mut n) = (std::ptr::null(), 0usize);
        CMVideoFormatDescriptionGetH264ParameterSetAtIndex(desc, 0, &mut p, &mut n, &mut count, &mut nal_len);
        for i in 0..count {
            if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(desc, i, &mut p, &mut n, &mut count, &mut nal_len) == 0 && !p.is_null() {
                sets.push(std::slice::from_raw_parts(p, n).to_vec());
            }
        }
    }
    let data = std::slice::from_raw_parts(ptr, total);
    Some(Frame { data: avcc_to_annexb(data, nal_len as usize, &sets, key)?, key, capture_us })
}

struct Encoder {
    session: *mut c_void,
    refcon: *mut mpsc::SyncSender<Frame>,
    force_key: AtomicBool,
    fps: i32,
}
// SAFETY: VTCompressionSession calls are documented as thread-safe; the raw pointers are
// owned exclusively by this struct and freed once in Drop.
unsafe impl Send for Encoder {}
unsafe impl Sync for Encoder {}
impl Encoder {
    fn new(w: u32, h: u32, fps: u32, kbps: u32, tx: mpsc::SyncSender<Frame>) -> Result<Self, String> {
        let refcon = Box::into_raw(Box::new(tx));
        let mut session = std::ptr::null_mut();
        let spec = unsafe {
            CFDictionary::from_CFType_pairs(&[(
                CFString::wrap_under_get_rule(kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder),
                CFBoolean::true_value(),
            )])
        };
        let status = unsafe {
            VTCompressionSessionCreate(std::ptr::null(), w as i32, h as i32, AVC1, spec.as_concrete_TypeRef(), std::ptr::null(), std::ptr::null(), on_encoded, refcon.cast(), &mut session)
        };
        if status != 0 || session.is_null() {
            drop(unsafe { Box::from_raw(refcon) });
            return Err("This Mac could not start the H.264 video encoder".into());
        }
        let this = Self { session, refcon, force_key: AtomicBool::new(true), fps: fps as i32 };
        unsafe {
            let set = |key: CFStringRef, value: CFTypeRef| VTSessionSetProperty(session, key, value);
            set(kVTCompressionPropertyKey_RealTime, CFBoolean::true_value().as_CFTypeRef());
            set(kVTCompressionPropertyKey_AllowFrameReordering, CFBoolean::false_value().as_CFTypeRef());
            set(kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel as CFTypeRef);
            set(kVTCompressionPropertyKey_AverageBitRate, CFNumber::from((kbps.clamp(500, 100_000) * 1000) as i32).as_CFTypeRef());
            set(kVTCompressionPropertyKey_ExpectedFrameRate, CFNumber::from(fps as i32).as_CFTypeRef());
            set(kVTCompressionPropertyKey_MaxKeyFrameInterval, CFNumber::from((fps * 2) as i32).as_CFTypeRef());
            if VTCompressionSessionPrepareToEncodeFrames(session) != 0 {
                return Err("This Mac could not prepare the H.264 video encoder".into());
            }
        }
        Ok(this)
    }
    fn encode(&self, pixel_buffer: *mut c_void, pts_us: u64) {
        let props = self.force_key.swap(false, Ordering::Relaxed).then(|| unsafe {
            CFDictionary::from_CFType_pairs(&[(CFString::wrap_under_get_rule(kVTEncodeFrameOptionKey_ForceKeyFrame), CFBoolean::true_value())])
        });
        let at = Cmt { value: pts_us as i64, timescale: 1_000_000, flags: 1, epoch: 0 };
        let dur = Cmt { value: 1, timescale: self.fps.max(1), flags: 1, epoch: 0 };
        let mut flags = 0u32;
        unsafe {
            VTCompressionSessionEncodeFrame(
                self.session,
                pixel_buffer,
                at,
                dur,
                props.as_ref().map_or(std::ptr::null(), |p| p.as_concrete_TypeRef()),
                pts_us as usize as *mut c_void,
                &mut flags,
            );
        }
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            VTCompressionSessionCompleteFrames(self.session, Cmt { value: 0, timescale: 0, flags: 0, epoch: 0 });
            VTCompressionSessionInvalidate(self.session);
            CFRelease(self.session as CFTypeRef);
            drop(Box::from_raw(self.refcon));
        }
    }
}

struct Handler {
    encoder: Arc<Encoder>,
    audio: Option<mpsc::SyncSender<AudioChunk>>,
}
impl SCStreamOutputTrait for Handler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, kind: SCStreamOutputType) {
        match kind {
            SCStreamOutputType::Screen => {
                if sample.frame_status() != Some(SCFrameStatus::Complete) {
                    return;
                }
                if let Some(pixels) = sample.pixel_buffer() {
                    self.encoder.encode(pixels.as_ptr(), now_us());
                }
            }
            SCStreamOutputType::Audio => {
                let (Some(tx), Ok(list)) = (&self.audio, sample.audio_buffer_list()) else { return };
                let buffers: Vec<&[u8]> = list.iter().map(|b| b.data()).collect();
                if let Some(pcm) = interleave_stereo(&buffers) {
                    let _ = tx.try_send(AudioChunk { pcm, capture_us: now_us() }); // live audio: drop when behind
                }
            }
            _ => {}
        }
    }
}

pub struct Capture {
    stream: SCStream,
    encoder: Arc<Encoder>,
    frames: mpsc::Receiver<Frame>,
    size: (u32, u32),
    fps: u32,
}
impl Capture {
    /// Blocking: shows no UI itself, but fails if Screen Recording is not allowed.
    pub fn start(display_id: u32, want: (u32, u32), fps: u32, kbps: u32, audio: bool) -> Result<(Self, Option<Audio>), String> {
        let content = SCShareableContent::get().map_err(|_| "Screen Recording is not allowed for Secpar")?;
        let display = content
            .displays()
            .into_iter()
            .find(|d| d.display_id() == display_id)
            .ok_or("The selected display is no longer available")?;
        let size = fit_size(display.width(), display.height(), want.0, want.1);
        let (tx, frames) = mpsc::sync_channel(8);
        let encoder = Arc::new(Encoder::new(size.0, size.1, fps, kbps, tx)?);
        let filter = SCContentFilter::create().with_display(&display).with_excluding_windows(&[]).build().map_err(|_| "Could not select the display")?;
        let mut config = SCStreamConfiguration::new()
            .with_width(size.0)
            .with_height(size.1)
            .with_scales_to_fit(true)
            .with_shows_cursor(true)
            .with_pixel_format(PixelFormat::YCbCr_420v)
            .with_queue_depth(4)
            .with_minimum_frame_interval(&screencapturekit::cm::CMTime::new(1, fps as i32));
        let (audio_tx, audio_rx) = mpsc::sync_channel(64);
        if audio {
            config = config.with_captures_audio(true).with_sample_rate(48000).with_channel_count(2).with_excludes_current_process_audio(true);
        }
        let mut stream = SCStream::new(&filter, &config).map_err(|_| "Could not create the capture stream")?;
        let handler = |tx| Handler { encoder: encoder.clone(), audio: tx };
        stream.add_output_handler(handler(audio.then(|| audio_tx.clone())), SCStreamOutputType::Screen).map_err(|_| "Could not attach video capture")?;
        if audio {
            stream.add_output_handler(handler(Some(audio_tx)), SCStreamOutputType::Audio).map_err(|_| "Could not attach audio capture")?;
        }
        stream.start_capture().map_err(|_| "Could not start screen capture. Check Screen Recording permission.")?;
        Ok((Self { stream, encoder, frames, size, fps }, audio.then_some(Audio(audio_rx))))
    }
}
impl VideoSource for Capture {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn fps(&self) -> u32 {
        self.fps
    }
    fn codec(&self) -> &'static str {
        "h264"
    }
    fn request_keyframe(&mut self) {
        self.encoder.force_key.store(true, Ordering::Relaxed);
    }
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>, String> {
        match self.frames.recv_timeout(timeout) {
            Ok(f) => Ok(Some(f)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err("Video capture ended".into()),
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stream.stop_capture();
    }
}
pub struct Audio(mpsc::Receiver<AudioChunk>);
impl AudioSource for Audio {
    fn next(&mut self, timeout: Duration) -> Result<Option<AudioChunk>, String> {
        match self.0.recv_timeout(timeout) {
            Ok(c) => Ok(Some(c)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err("System audio capture ended".into()),
        }
    }
}
