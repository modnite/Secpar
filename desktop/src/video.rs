//! Decode away from the interface thread. Bound compressed work and publish only
//! the latest decoded image; dropping dependent frames requires a new keyframe.
#[cfg(all(test, target_os = "linux"))]
mod dmabuf;
use eframe::egui::{ColorImage, Context};
use openh264::{decoder::Decoder, formats::YUVSource};
use secpar_client::receiver::VideoFrame;
use std::{collections::VecDeque, sync::{Arc, Condvar, Mutex}, thread::JoinHandle, time::{Duration,Instant}};
#[cfg(target_os="linux")]
mod linux;
#[derive(Clone,Copy,Default,PartialEq,serde::Serialize,serde::Deserialize)]
pub enum Preference { #[default] Automatic, Software, Vaapi }
static FACTORY: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
static PROBING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn detected_factory() -> Option<&'static str> {
    *FACTORY.get_or_init(|| {
        #[cfg(target_os="linux")] { linux::probe() }
        #[cfg(not(target_os="linux"))] { None }
    })
}
/// None means still checking; Some(false) means no working hardware decoder.
pub fn hardware_available(ctx:&Context) -> Option<bool> {
    if let Some(value)=FACTORY.get() { return Some(value.is_some()); }
    if !PROBING.swap(true,std::sync::atomic::Ordering::Relaxed) {
        let ctx=ctx.clone(); std::thread::spawn(move|| {detected_factory();ctx.request_repaint();});
    }
    None
}
enum Backend {
    Software(Decoder),
    #[cfg(target_os="linux")]
    Hardware(linux::Hardware),
}
enum DecodeError { Invalid, Dimensions }
struct DecodeBudget { limit:Duration, slow:u8 }
impl DecodeBudget {
    fn new(fps:u32)->Self {Self {limit:Duration::from_secs_f64(1.5/fps.clamp(1,240) as f64),slow:0}}
    fn exceeded(&mut self,elapsed:Duration)->bool {
        self.slow=if elapsed>self.limit {self.slow.saturating_add(1)}else{0};
        self.slow>=3
    }
}
impl Backend {
    fn new(preference:Preference) -> Result<Self,()> {
        #[cfg(target_os="linux")]
        if preference!=Preference::Software {
            if let Some(factory)=detected_factory() {
                if let Ok(hardware)=linux::Hardware::open(factory) {return Ok(Self::Hardware(hardware));}
            }
        }
        let _=preference;
        Decoder::new().map(Self::Software).map_err(|_|())
    }
    fn is_hardware(&self)->bool {
        match self {Self::Software(_)=>false,#[cfg(target_os="linux")] Self::Hardware(_)=>true}
    }
    fn label(&self)->&'static str {if self.is_hardware(){"H.264 · VA-API"}else{"H.264 · software"}}
    fn decode(&mut self,bytes:&[u8])->Result<Option<ColorImage>,DecodeError> {
        match self {
            Self::Software(decoder)=> {
                let Some(yuv)=decoder.decode(bytes).map_err(|_|DecodeError::Invalid)? else {return Ok(None)};
                let (w,h)=yuv.dimensions();
                if w==0 || h==0 || w>8192 || h>8192 || w*h>4096*2160 {return Err(DecodeError::Dimensions)}
                let mut pixels=vec![0;w*h*3];yuv.write_rgb8(&mut pixels);
                Ok(Some(ColorImage::from_rgb([w,h],&pixels)))
            }
            #[cfg(target_os="linux")]
            Self::Hardware(decoder)=>decoder.decode(bytes).map_err(|_|DecodeError::Invalid),
        }
    }
}

#[derive(Default)]
struct State {
    queue: VecDeque<VideoFrame>,
    image: Option<ColorImage>,
    waiting_key: bool,
    request_key: bool,
    key_requested: Option<Instant>,
    generation: u64,
    error: Option<String>,
    backend: Option<&'static str>,
    stopped: bool,
}
impl State {
    fn submit(&mut self, frame: VideoFrame) {
        if self.queue.len() >= 3 {
            self.queue.clear();
            self.waiting_key = true;
            self.generation += 1;
        }
        if self.waiting_key && !frame.header.keyframe {
            self.request_key = true;
            return;
        }
        self.waiting_key = false;
        if frame.header.keyframe { self.request_key = false; }
        self.queue.push_back(frame);
    }
}
pub struct Output {
    pub image: Option<ColorImage>,
    pub request_key: bool,
    pub error: Option<String>,
    pub backend: Option<&'static str>,
}
pub struct Worker {
    state: Arc<(Mutex<State>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    #[cfg(test)]
    pub fn new(ctx: Context, preference:Preference) -> Self {
        Self::with_frame_rate(ctx,preference,60)
    }
    pub fn with_frame_rate(ctx:Context,preference:Preference,fps:u32) -> Self {
        Self::spawn_at_rate(ctx,fps,move||Backend::new(preference))
    }
    #[cfg(all(test,target_os="linux"))]
    fn spawn(ctx:Context, create:impl FnOnce()->Result<Backend,()>+Send+'static) -> Self {
        Self::spawn_at_rate(ctx,60,create)
    }
    fn spawn_at_rate(ctx:Context,fps:u32,create:impl FnOnce()->Result<Backend,()>+Send+'static)->Self {
        let state = Arc::new((Mutex::new(State { waiting_key: true, ..Default::default() }), Condvar::new()));
        let shared = state.clone();
        let thread = std::thread::spawn(move || {
            let (lock, ready) = &*shared;
            let mut decoder = match create() {
                Ok(d) => d,
                Err(_) => {
                    lock.lock().unwrap().error = Some("H.264 decoder could not start".into());
                    ctx.request_repaint();
                    return;
                }
            };
            let mut missing_output = 0;
            let mut budget=DecodeBudget::new(fps);
            let mut slow_fallback=false;
            loop {
                let (frame, generation) = {
                    let state = lock.lock().unwrap();
                    let mut state = ready.wait_while(state, |s| s.queue.is_empty() && !s.stopped).unwrap();
                    if state.stopped { break; }
                    (state.queue.pop_front().unwrap(), state.generation)
                };
                let started=Instant::now();
                let mut decoded = decoder.decode(&frame.data);
                if decoder.is_hardware() && budget.exceeded(started.elapsed()) {
                    // A small capability sample proves compatibility, not real
                    // stream throughput. Include native decode and GPU readback.
                    slow_fallback=true;decoded=Err(DecodeError::Invalid);
                }
                if decoder.is_hardware() && matches!(decoded,Ok(None)) {
                    missing_output += 1;
                    if missing_output >= 4 { decoded=Err(DecodeError::Invalid); }
                } else { missing_output=0; }
                let image = match decoded {
                    Ok(Some(image)) => Some(image),
                    Ok(None) => None,
                    Err(DecodeError::Dimensions) => {
                        lock.lock().unwrap().error=Some("Video dimensions exceed the decoder limit".into());
                        ctx.request_repaint();break;
                    }
                    Err(_) => {
                        if decoder.is_hardware() {
                            match Backend::new(Preference::Software) {
                                Ok(software)=>decoder=software,
                                Err(_)=> {lock.lock().unwrap().error=Some("H.264 decoder could not start".into());ctx.request_repaint();break;}
                            }
                        }
                        let mut state = lock.lock().unwrap();
                        state.queue.clear();
                        state.waiting_key = true;
                        state.request_key = true;
                        None
                    }
                };
                let mut state = lock.lock().unwrap();
                if generation == state.generation && !state.stopped {
                    if let Some(image) = image { state.image = Some(image); state.backend=Some(if slow_fallback {"H.264 · software (VA-API too slow)"}else{decoder.label()}); }
                }
                drop(state);
                ctx.request_repaint();
            }
        });
        Self { state, thread: Some(thread) }
    }
    pub fn submit(&self, frame: VideoFrame) {
        let (lock, ready) = &*self.state;
        lock.lock().unwrap().submit(frame);
        ready.notify_one();
    }
    pub fn output(&self) -> Output {
        let mut state = self.state.0.lock().unwrap();
        let request_key = state.request_key && state.key_requested.is_none_or(|at| at.elapsed() >= Duration::from_millis(250));
        if request_key { state.request_key = false; state.key_requested = Some(Instant::now()); }
        Output { image: state.image.take(), request_key, error: state.error.take(), backend:state.backend.take() }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, ready) = &*self.state;
        lock.lock().unwrap().stopped = true;
        ready.notify_one();
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secpar_stream::frame::{Codec, FrameHeader};
    #[test]
    fn hardware_budget_ignores_a_single_spike_but_rejects_sustained_slow_decode() {
        let mut budget=DecodeBudget::new(60);
        assert!(!budget.exceeded(Duration::from_millis(500)));
        assert!(!budget.exceeded(Duration::from_millis(5)));
        assert!(!budget.exceeded(Duration::from_millis(100)));
        assert!(!budget.exceeded(Duration::from_millis(100)));
        assert!(budget.exceeded(Duration::from_millis(100)));
        let mut lower_rate=DecodeBudget::new(30);
        for _ in 0..10 {assert!(!lower_rate.exceeded(Duration::from_millis(30)));}
    }
    fn frame(id:u32,key:bool) -> VideoFrame {
        VideoFrame { id, header:FrameHeader {len:1,capture_us:0,keyframe:key,codec:Codec::H264},data:vec![0] }
    }
    #[test]
    fn overflow_discards_dependent_frames_and_waits_for_a_keyframe() {
        let mut state = State::default();
        for id in 0..3 { state.submit(frame(id,id==0)); }
        state.submit(frame(3,false));
        assert!(state.queue.is_empty());
        assert!(state.request_key);
        state.submit(frame(4,false));
        assert!(state.queue.is_empty());
        state.submit(frame(5,true));
        state.submit(frame(6,false));
        assert_eq!(state.queue.iter().map(|f|f.id).collect::<Vec<_>>(),[5,6]);
        assert_eq!(state.generation,1);
    }
    #[test]
    fn worker_decodes_real_video_and_stops_while_idle() {
        use openh264::{encoder::Encoder, formats::{RgbSliceU8,YUVBuffer}};
        let rgb:Vec<u8> = (0..128*128).flat_map(|_|[40,200,20]).collect();
        let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb,(128,128)));
        let packet = Encoder::new().unwrap().encode(&yuv).unwrap().to_vec();
        let mut input = frame(0,true); input.header.len=packet.len() as u32; input.data=packet;
        let worker = Worker::new(Context::default(),Preference::Software);
        worker.submit(input);
        let until=Instant::now()+Duration::from_secs(3);
        let image=loop {
            let output=worker.output(); assert!(output.error.is_none(),"{:?}",output.error);
            if let Some(image)=output.image { break image; }
            assert!(Instant::now()<until,"Worker did not publish decoded video");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(image.size,[128,128]);
        let pixel=image.pixels[0];
        for (expected,actual) in [40i16,200,20].into_iter().zip([pixel.r(),pixel.g(),pixel.b()]) {
            assert!((expected-actual as i16).abs()<16);
        }
        let start=Instant::now(); drop(worker);
        assert!(start.elapsed()<Duration::from_secs(1),"Idle decoder failed to stop");
    }
    #[cfg(target_os="linux")]
    #[test]
    fn slow_native_decoder_recovers_with_software_at_a_fresh_keyframe() {
        use openh264::{encoder::Encoder,formats::{RgbSliceU8,YUVBuffer}};
        let rgb:Vec<u8>=(0..128*128).flat_map(|_|[40,200,20]).collect();
        let yuv=YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb,(128,128)));
        let packet=Encoder::new().unwrap().encode(&yuv).unwrap().to_vec();
        // Real native decode with a 50ms downstream delay, rather than a GPU claim.
        let worker=Worker::spawn_at_rate(Context::default(),60,||linux::Hardware::open("openh264dec ! identity sleep-time=50000").map(Backend::Hardware));
        let until=Instant::now()+Duration::from_secs(5);
        for id in 0..3 {
            let mut f=frame(id,true);f.data=packet.clone();f.header.len=f.data.len() as u32;worker.submit(f);
            loop {
                let output=worker.output();assert!(output.error.is_none());
                if output.request_key || output.image.is_some(){break;}
                assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(5));
            }
        }
        let mut f=frame(4,true);f.data=packet;f.header.len=f.data.len() as u32;worker.submit(f);
        loop {
            let output=worker.output();assert!(output.error.is_none());
            if let Some(image)=output.image {
                assert_eq!(image.size,[128,128]);
                assert_eq!(output.backend,Some("H.264 · software (VA-API too slow)"));break;
            }
            assert!(Instant::now()<until,"Slow native decode did not recover");std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[cfg(target_os="linux")]
    #[test]
    fn failed_native_pipeline_recovers_with_software_at_a_fresh_keyframe() {
        use openh264::{encoder::Encoder,formats::{RgbSliceU8,YUVBuffer}};
        let rgb:Vec<u8>=(0..128*128).flat_map(|_|[40,200,20]).collect();
        let yuv=YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb,(128,128)));
        let packet=Encoder::new().unwrap().encode(&yuv).unwrap().to_vec();
        // A real GStreamer error exercises failover without requiring a GPU or
        // lying to capability detection about an installed hardware decoder.
        let worker=Worker::spawn(Context::default(),||linux::Hardware::open("openh264dec ! identity error-after=1").map(Backend::Hardware));
        let make_frame=|id| {let mut f=frame(id,true);f.data=packet.clone();f.header.len=f.data.len() as u32;f};
        worker.submit(make_frame(0));
        let until=Instant::now()+Duration::from_secs(3);
        loop {
            let output=worker.output();assert!(output.error.is_none(),"{:?}",output.error);
            if output.request_key {break}
            assert!(Instant::now()<until,"Failed pipeline did not request recovery");
            std::thread::sleep(Duration::from_millis(5));
        }
        worker.submit(make_frame(1));
        loop {
            let output=worker.output();assert!(output.error.is_none(),"{:?}",output.error);
            if let Some(image)=output.image {
                assert_eq!(image.size,[128,128]);assert_eq!(output.backend,Some("H.264 · software"));break;
            }
            assert!(Instant::now()<until,"Software fallback did not resume");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
