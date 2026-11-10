//! Decode away from the interface thread. Bound compressed work and publish only
//! the latest decoded image; dropping dependent frames requires a new keyframe.
use crate::picture::Picture;
use eframe::egui::Context;
use openh264::{decoder::Decoder, formats::YUVSource};
use secpar_client::receiver::VideoFrame;
use std::{collections::VecDeque, sync::{Arc, Condvar, Mutex}, thread::JoinHandle, time::{Duration,Instant}};
#[cfg(target_os="linux")]
mod linux;
#[cfg(windows)]
mod dxva;
mod nal;
use secpar_stream::frame::Codec;
#[derive(Clone,Copy,Default,PartialEq,serde::Serialize,serde::Deserialize)]
pub enum Preference { #[default] Automatic, Software, Vaapi, D3d11 }
impl Preference {
    /// The hardware choice this operating system offers.
    pub const HARDWARE:Preference=if cfg!(windows) {Preference::D3d11}else{Preference::Vaapi};
    pub fn label(self)->&'static str {
        match self {Self::Automatic=>"Automatic",Self::Software=>"Software",Self::Vaapi=>"VA-API",Self::D3d11=>"Hardware (D3D11)"}
    }
    /// Whether hardware may be tried. A choice for the other OS counts as automatic.
    fn wants_hardware(self)->bool {self!=Self::Software}
}
static FACTORY: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
static PROBING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn detected_factory() -> Option<&'static str> {
    *FACTORY.get_or_init(|| {
        #[cfg(target_os="linux")] { linux::probe() }
        #[cfg(windows)] { probe_windows() }
        #[cfg(not(any(target_os="linux",windows)))] { None }
    })
}
/// A real GPU decode of a tiny H.264 stream. Registry or driver presence is not enough.
#[cfg(windows)]
fn probe_windows()->Option<&'static str> {
    use openh264::encoder::Encoder;
    let rgb:Vec<u8>=(0..128*128).flat_map(|_|[40,200,20]).collect();
    let yuv=openh264::formats::YUVBuffer::from_rgb_source(openh264::formats::RgbSliceU8::new(&rgb,(128,128)));
    let packet=Encoder::new().ok()?.encode(&yuv).ok()?.to_vec();
    dxva::probe(Codec::H264,&packet).then_some("d3d11")
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
    #[cfg(windows)]
    Hardware(dxva::Hardware),
}
/// `Invalid`: this data could not be decoded, ask for a keyframe. `Device`: the decoder itself
/// is unusable (GPU removed, MFT fell back to system memory), so switch to software.
#[derive(Debug,PartialEq)]
enum DecodeError { Invalid, Dimensions, Device }
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
        if preference.wants_hardware() {
            if let Some(factory)=detected_factory() {
                if let Ok(hardware)=linux::Hardware::open(factory) {return Ok(Self::Hardware(hardware));}
            }
        }
        #[cfg(windows)]
        if preference.wants_hardware() && detected_factory().is_some() {
            if let Ok(mut hardware)=dxva::Hardware::open() {
                // The stream is H.264 today; a failure here means software from the first frame.
                if hardware.prepare(Codec::H264).is_ok() {return Ok(Self::Hardware(hardware));}
            }
        }
        let _=preference;
        Decoder::new().map(Self::Software).map_err(|_|())
    }
    fn is_hardware(&self)->bool {
        match self {Self::Software(_)=>false,#[cfg(any(target_os="linux",windows))] Self::Hardware(_)=>true}
    }
    fn label(&self,codec:Codec)->&'static str {
        match (codec,self.is_hardware(),cfg!(windows)) {
            (Codec::H264,false,_)=>"H.264 · software",
            (Codec::H264,true,true)=>"H.264 · Hardware (D3D11)",
            (Codec::H264,true,false)=>"H.264 · VA-API",
            (Codec::Hevc,_,true)=>"HEVC · Hardware (D3D11)",
            (Codec::Av1,_,true)=>"AV1 · Hardware (D3D11)",
            (Codec::Hevc,_,false)=>"HEVC · VA-API",
            (Codec::Av1,_,false)=>"AV1 · VA-API",
        }
    }
    fn decode(&mut self,bytes:&[u8],codec:Codec)->Result<Option<Picture>,DecodeError> {
        match self {
            Self::Software(_) if codec!=Codec::H264 => Err(DecodeError::Device),
            Self::Software(decoder)=> {
                let Some(yuv)=decoder.decode(bytes).map_err(|_|DecodeError::Invalid)? else {return Ok(None)};
                let (w,h)=yuv.dimensions();
                let (sy,su,sv)=yuv.strides();
                Picture::from_i420([w,h],(yuv.y(),sy),(yuv.u(),su),(yuv.v(),sv)).map(Some).ok_or(DecodeError::Dimensions)
            }
            #[cfg(target_os="linux")]
            Self::Hardware(decoder)=>decoder.decode(bytes).map_err(|_|DecodeError::Device),
            #[cfg(windows)]
            Self::Hardware(decoder)=>decoder.decode(codec,bytes),
        }
    }
}

#[derive(Default)]
struct State {
    queue: VecDeque<VideoFrame>,
    image: Option<Picture>,
    waiting_key: bool,
    request_key: bool,
    key_requested: Option<Instant>,
    generation: u64,
    error: Option<String>,
    backend: Option<&'static str>,
    stopped: bool,
    /// The decoder is still being created (a hardware decoder takes tens of milliseconds).
    warming: bool,
}
/// Frames kept while the decoder starts: about two seconds, the newest keyframe onward.
const WARMING_LIMIT: usize = 120;
impl State {
    fn submit(&mut self, frame: VideoFrame) {
        if self.warming {
            // Everything before the newest keyframe is useless to a decoder that has seen nothing.
            if frame.header.keyframe {
                self.queue.clear();
                self.waiting_key = false;
                self.request_key = false;
            }
            if !self.waiting_key && self.queue.len() < WARMING_LIMIT {
                self.queue.push_back(frame);
                return;
            }
            if self.waiting_key {self.request_key = true; return;}
        }
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
    pub image: Option<Picture>,
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
    #[cfg(all(test,any(target_os="linux",windows)))]
    fn spawn(ctx:Context, create:impl FnOnce()->Result<Backend,()>+Send+'static) -> Self {
        Self::spawn_at_rate(ctx,60,create)
    }
    fn spawn_at_rate(ctx:Context,fps:u32,create:impl FnOnce()->Result<Backend,()>+Send+'static)->Self {
        let state = Arc::new((Mutex::new(State { waiting_key: true, warming: true, ..Default::default() }), Condvar::new()));
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
            {
                // Start from the newest keyframe that arrived while the decoder was being made.
                let mut state=lock.lock().unwrap();
                state.warming=false;
                if let Some(at)=state.queue.iter().rposition(|f|f.header.keyframe) {state.queue.drain(..at);}
            }
            let mut missing_output = 0;
            let mut budget=DecodeBudget::new(fps);
            let mut slow_fallback=false;
            // Consecutive hardware failures on bad data; a lone glitch only costs a keyframe.
            let mut bad_run=0u32;
            // Until the decoder produced a picture it can only start on parameter sets.
            let mut started=false;
            loop {
                let (frame, generation) = {
                    let state = lock.lock().unwrap();
                    let mut state = ready.wait_while(state, |s| s.queue.is_empty() && !s.stopped).unwrap();
                    if state.stopped { break; }
                    (state.queue.pop_front().unwrap(), state.generation)
                };
                let began=Instant::now();
                let codec=frame.header.codec;
                let mut decoded = if cfg!(windows) && decoder.is_hardware() && !started && !nal::has_parameter_sets(codec,&frame.data) {
                    Err(DecodeError::Invalid)
                } else {decoder.decode(&frame.data,codec)};
                if decoder.is_hardware() && budget.exceeded(began.elapsed()) {
                    // A small capability sample proves compatibility, not real
                    // stream throughput. Include native decode and GPU readback.
                    slow_fallback=true;decoded=Err(DecodeError::Device);
                }
                if decoder.is_hardware() && matches!(decoded,Ok(None)) {
                    missing_output += 1;
                    if missing_output >= 4 { missing_output=0;decoded=Err(DecodeError::Invalid); }
                } else { missing_output=0; }
                if matches!(decoded,Ok(Some(_))) {started=true;bad_run=0;}
                let image = match decoded {
                    Ok(Some(image)) => Some(image),
                    Ok(None) => None,
                    Err(DecodeError::Dimensions) => {
                        lock.lock().unwrap().error=Some("Video dimensions exceed the decoder limit".into());
                        ctx.request_repaint();break;
                    }
                    Err(DecodeError::Device) if !decoder.is_hardware() => {
                        lock.lock().unwrap().error=Some("This computer can't decode that video format".into());
                        ctx.request_repaint();break;
                    }
                    Err(error) => {
                        started=false;
                        if error==DecodeError::Invalid {bad_run+=1;}
                        // Linux keeps its old rule: any hardware failure switches to software.
                        if decoder.is_hardware() && (error==DecodeError::Device || bad_run>=3 || !cfg!(windows)) {
                            bad_run=0;
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
                    if let Some(image) = image { state.image = Some(image); state.backend=Some(if slow_fallback {"H.264 · software (hardware too slow)"}else{decoder.label(codec)}); }
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
    fn frames_arriving_while_the_decoder_starts_are_kept_from_the_newest_keyframe() {
        let mut state = State { waiting_key: true, warming: true, ..Default::default() };
        state.submit(frame(0,false));
        assert!(state.queue.is_empty() && state.request_key, "a delta frame cannot start decoding");
        for id in 1..30 { state.submit(frame(id,id==1||id==20)); }
        assert_eq!(state.queue.front().map(|f|f.id),Some(20));
        assert_eq!(state.queue.len(),10);
        assert!(!state.request_key);
        // Far more than the warm-up window falls back to the normal overflow rule.
        for id in 30..400 { state.submit(frame(id,false)); }
        assert!(state.queue.len()<=WARMING_LIMIT);
        assert!(state.waiting_key && state.request_key);
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
        let pixel=image.rgb_at(0,0,crate::picture::Matrix::Bt601);
        for (expected,actual) in [40i16,200,20].into_iter().zip(pixel) {
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
                assert_eq!(output.backend,Some("H.264 · software (hardware too slow)"));break;
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

    #[cfg(windows)]
    mod windows_hardware {
        use super::*;
        use openh264::{encoder::{Encoder,EncoderConfig},formats::{RgbSliceU8,YUVBuffer}};
        /// A moving test card, so P frames carry real motion and chroma varies.
        fn card(w:usize,h:usize,t:usize)->Vec<u8> {
            let mut rgb=Vec::with_capacity(w*h*3);
            for y in 0..h {for x in 0..w {
                let bar=((x+t*4)*8/w)%8;
                let c=[[235u8,235,235],[235,235,16],[16,235,235],[16,235,16],[235,16,235],[235,16,16],[16,16,235],[60,60,60]][bar];
                let g=(y*255/h) as u8;
                rgb.extend_from_slice(&[c[0].min(g.saturating_add(40)),c[1],c[2]]);
            }}
            rgb
        }
        /// Real OpenH264 output: key frame first, then `n-1` predicted frames.
        fn stream(w:usize,h:usize,n:usize)->Vec<Vec<u8>> {
            let mut encoder=Encoder::with_api_config(openh264::OpenH264API::from_source(),EncoderConfig::new().skip_frames(false)).unwrap();
            (0..n).map(|t|{
                let rgb=card(w,h,t);
                let yuv=YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb,(w,h)));
                encoder.encode(&yuv).unwrap().to_vec()
            }).collect()
        }
        fn gpu()->Option<dxva::Hardware> {
            let hardware=dxva::Hardware::open().ok();
            if hardware.is_none() {eprintln!("no GPU decoder here, skipping");}
            hardware
        }
        fn decode_all(hw:&mut dxva::Hardware,frames:&[Vec<u8>])->Vec<Picture> {
            frames.iter().filter_map(|f|hw.decode(Codec::H264,f).expect("hardware decode")).collect()
        }
        fn mean_abs_diff(a:&[u8],b:&[u8])->f64 {
            assert_eq!(a.len(),b.len());
            a.iter().zip(b).map(|(x,y)|(*x as i32-*y as i32).unsigned_abs() as u64).sum::<u64>() as f64/a.len() as f64
        }
        fn from_software(y:&openh264::decoder::DecodedYUV<'_>)->Picture {
            let (w,h)=y.dimensions();let (sy,su,sv)=y.strides();
            Picture::from_i420([w,h],(y.y(),sy),(y.u(),su),(y.v(),sv)).unwrap()
        }
        #[test]
        fn hardware_matches_openh264_at_1080p_without_crop_or_chroma_errors() {
            let Some(mut hw)=gpu() else {return};
            let frames=stream(1920,1080,8);
            let mut soft=Decoder::new().unwrap();
            let mut compared=0;
            for f in &frames {
                let picture=hw.decode(Codec::H264,f).expect("hardware decode");
                let reference=from_software(&soft.decode(f).unwrap().expect("software frame"));
                let Some(picture)=picture else {continue};
                assert_eq!(picture.size,[1920,1080],"visible size, not the 1088 coded size");
                assert_eq!(picture.y.len(),1920*1080);
                assert_eq!(picture.uv.len(),960*540*2);
                assert!(mean_abs_diff(&picture.y,&reference.y)<1.5,"luma differs");
                assert!(mean_abs_diff(&picture.uv,&reference.uv)<1.5,"chroma differs (swapped or shifted planes)");
                // Bottom rows are where a stride or crop error shows first.
                let last=1079*1920;
                assert!(mean_abs_diff(&picture.y[last..],&reference.y[last..])<2.5);
                compared+=1;
            }
            assert!(compared>=6,"decoder produced too few pictures: {compared}");
        }
        #[test]
        fn odd_sizes_keep_exact_dimensions() {
            let Some(mut hw)=gpu() else {return};
            for (w,h) in [(854usize,480usize),(1366,768),(1280,720)] {
                let pictures=decode_all(&mut hw,&stream(w,h,4));
                assert!(!pictures.is_empty(),"{w}x{h} produced nothing");
                assert!(pictures.iter().all(|p|p.size==[w,h]),"{w}x{h}: {:?}",pictures[0].size);
            }
        }
        #[test]
        fn mid_stream_resolution_change_keeps_decoding() {
            let Some(mut hw)=gpu() else {return};
            let mut sizes=Vec::new();
            for (w,h) in [(1280usize,720usize),(1920,1080),(854,480),(1920,1080)] {
                let pictures=decode_all(&mut hw,&stream(w,h,5));
                assert!(!pictures.is_empty(),"no output after switching to {w}x{h}");
                sizes.push(pictures.last().unwrap().size);
            }
            assert_eq!(sizes,[[1280,720],[1920,1080],[854,480],[1920,1080]]);
        }
        #[test]
        fn corrupt_data_is_an_error_or_nothing_and_the_next_keyframe_recovers() {
            let Some(mut hw)=gpu() else {return};
            let good=stream(1280,720,6);
            assert!(!decode_all(&mut hw,&good).is_empty());
            let mut damaged=good[3].clone();
            for (i,b) in damaged.iter_mut().enumerate().skip(40) {if i%7==0 {*b^=0xA5;}}
            // Neither a panic nor a hang: Ok or an Invalid error are both acceptable.
            for bad in [damaged,vec![0xFF;300],vec![0,0,0,1,0x65,1,2,3],Vec::new()] {
                if let Err(e)=hw.decode(Codec::H264,&bad) {assert_eq!(e,DecodeError::Invalid);}
            }
            let fresh=stream(1280,720,4);
            let pictures=decode_all(&mut hw,&fresh);
            assert!(!pictures.is_empty(),"no picture after recovering at a keyframe");
            assert_eq!(pictures[0].size,[1280,720]);
        }
        /// Feeds each frame and waits until it is handled. Returns the label of the last picture
        /// and how many keyframe requests the worker raised.
        fn feed(worker:&Worker,frames:Vec<Vec<u8>>,first_id:u32)->(Option<&'static str>,u32) {
            let (mut label,mut requests)=(None,0);
            for (n,data) in frames.into_iter().enumerate() {
                let mut f=frame(first_id+n as u32,true);f.header.len=data.len() as u32;f.data=data;
                worker.submit(f);
                let until=Instant::now()+Duration::from_secs(10);
                loop {
                    std::thread::sleep(Duration::from_millis(10));
                    let output=worker.output();assert!(output.error.is_none(),"{:?}",output.error);
                    if output.request_key {requests+=1;break;}
                    if output.image.is_some() {label=output.backend.or(label);break;}
                    assert!(Instant::now()<until,"frame {n} produced neither a picture nor a keyframe request");
                }
            }
            (label,requests)
        }
        /// Every OpenH264 IDR carries its own parameter sets, so each can restart decoding.
        fn keyframes(n:usize)->Vec<Vec<u8>> {
            (0..n).map(|_|stream(1280,720,1).remove(0)).collect()
        }
        #[test]
        fn worker_uses_hardware_and_labels_it() {
            if gpu().is_none() {return}
            let worker=Worker::with_frame_rate(Context::default(),Preference::Automatic,60);
            let (label,requests)=feed(&worker,stream(1280,720,6),0);
            assert_eq!(requests,0);
            assert_eq!(label,Some("H.264 · Hardware (D3D11)"));
        }
        #[test]
        fn one_bad_frame_asks_for_a_keyframe_and_stays_on_hardware() {
            if gpu().is_none() {return}
            let worker=Worker::spawn(Context::default(),||{let mut h=dxva::Hardware::open().map_err(|_|())?;h.inject_invalid=1;Ok(Backend::Hardware(h))});
            let (label,requests)=feed(&worker,keyframes(4),0);
            assert_eq!(requests,1);
            assert_eq!(label,Some("H.264 · Hardware (D3D11)"));
        }
        #[test]
        fn repeated_bad_frames_fall_back_to_software_at_the_next_keyframe() {
            if gpu().is_none() {return}
            let worker=Worker::spawn(Context::default(),||{let mut h=dxva::Hardware::open().map_err(|_|())?;h.inject_invalid=3;Ok(Backend::Hardware(h))});
            let (label,requests)=feed(&worker,keyframes(5),0);
            assert_eq!(requests,3,"each failure asks for a keyframe");
            assert_eq!(label,Some("H.264 · software"));
        }
        #[test]
        fn the_software_choice_never_touches_the_gpu() {
            let worker=Worker::spawn(Context::default(),||Backend::new(Preference::Software));
            let (label,requests)=feed(&worker,keyframes(2),0);
            assert_eq!((label,requests),(Some("H.264 · software"),0));
        }
        #[test]
        fn unsupported_formats_are_reported_not_crashed_on() {
            // The software backend only knows H.264.
            let mut soft=Backend::new(Preference::Software).unwrap();
            assert_eq!(soft.decode(&[0,0,0,1,0x40,1],Codec::Hevc).err(),Some(DecodeError::Device));
        }
        #[test]
        fn a_codec_without_an_installed_decoder_is_a_clean_error() {
            let Some(mut hw)=gpu() else {return};
            // HEVC and AV1 decoders are optional Windows extensions: absent is Device, never a panic.
            for (codec,data) in [(Codec::Hevc,vec![0,0,0,1,0x40,1,0x0c,1,0xff,0xff]),(Codec::Av1,vec![0x12,0,0x0a,0x0b,0,0,0,0])] {
                match hw.decode(codec,&data) {
                    Ok(_)|Err(DecodeError::Device)|Err(DecodeError::Invalid)=>{}
                    Err(other)=>panic!("{codec:?}: {other:?}"),
                }
            }
            // And the H.264 decoder still works afterwards.
            assert!(!decode_all(&mut hw,&stream(640,360,3)).is_empty());
        }
        #[test]
        fn probe_runs_a_real_decode() {
            let Some(hw)=gpu() else {return};
            eprintln!("decoder adapter: {}, all adapters: {:?}",hw.adapter,dxva::adapter_names());
            assert!(detected_factory().is_some(),"GPU opened but the H.264 probe failed");
        }
        /// Recorded host streams: see tests/capture_stream.rs for the file format.
        fn read_capture(name:&str)->Vec<(bool,Vec<u8>)> {
            let path=std::path::Path::new(&std::env::var("SECPAR_CAPTURE_DIR").expect("SECPAR_CAPTURE_DIR")).join(name);
            let bytes=std::fs::read(path).unwrap();
            let (mut at,mut out)=(0,Vec::new());
            while at+5<=bytes.len() {
                let len=u32::from_be_bytes(bytes[at..at+4].try_into().unwrap()) as usize;
                out.push((bytes[at+4]==1,bytes[at+5..at+5+len].to_vec()));at+=5+len;
            }
            out
        }
        fn process_cpu_ms()->f64 {
            use windows_sys::Win32::{Foundation::FILETIME,System::Threading::{GetCurrentProcess,GetProcessTimes}};
            let zero=FILETIME{dwLowDateTime:0,dwHighDateTime:0};
            let (mut c,mut e,mut k,mut u)=(zero,zero,zero,zero);
            // SAFETY: valid out pointers for the current process.
            unsafe {GetProcessTimes(GetCurrentProcess(),&mut c,&mut e,&mut k,&mut u);}
            let t=|f:FILETIME|(((f.dwHighDateTime as u64)<<32)|f.dwLowDateTime as u64) as f64/10_000.0;
            t(k)+t(u)
        }
        fn percentile(sorted:&[f64],p:f64)->f64 {sorted[((sorted.len()-1) as f64*p).round() as usize]}
        /// Paced at the stream rate like a live client, so CPU use is comparable.
        fn bench(label:&str,frames:&[(bool,Vec<u8>)],mut decode:impl FnMut(&[u8])->bool,rounds:usize) {
            let mut times=Vec::new();
            let (cpu0,wall0)=(process_cpu_ms(),Instant::now());
            let mut shown=0;
            let mut next=Instant::now();
            for _ in 0..rounds {
                for (_,data) in frames {
                    let start=Instant::now();
                    if decode(data) {shown+=1;}
                    times.push(start.elapsed().as_secs_f64()*1000.);
                    next+=Duration::from_micros(16_667);
                    if let Some(wait)=next.checked_duration_since(Instant::now()) {std::thread::sleep(wait);}
                }
            }
            let wall=wall0.elapsed().as_secs_f64()*1000.;let cpu=process_cpu_ms()-cpu0;
            times.sort_by(|a,b|a.partial_cmp(b).unwrap());
            eprintln!("BENCH {label}: frames {} pictures {shown} decode ms median {:.2} p95 {:.2} max {:.2}; process CPU {:.1}% of one core over {:.1}s",
                times.len(),percentile(&times,0.5),percentile(&times,0.95),times.last().unwrap(),cpu/wall*100.,wall/1000.);
        }
        #[test]
        #[ignore = "Needs recorded streams: see tests/capture_stream.rs"]
        fn measure_recorded_host_streams() {
            let rounds:usize=std::env::var("SECPAR_BENCH_ROUNDS").ok().and_then(|s|s.parse().ok()).unwrap_or(1);
            let only=std::env::var("SECPAR_BENCH_ONLY").ok();
            for (file,codec) in [("h264_1080.bin",Codec::H264),("h264_720.bin",Codec::H264),("hevc_1080.bin",Codec::Hevc)] {
                if only.as_deref().is_some_and(|o|o!="both"&&!file.contains(o.split(':').next().unwrap())) {continue;}
                let frames=read_capture(file);
                eprintln!("{file}: {} access units",frames.len());
                let mode=only.as_deref().and_then(|o|o.split(':').nth(1)).unwrap_or("both");
                if codec==Codec::H264 && mode!="hw" {
                    let mut soft=Decoder::new().unwrap();
                    bench(&format!("{file} software(openh264)"),&frames,|d|soft.decode(d).ok().flatten().map(|y|from_software(&y)).is_some(),rounds);
                }
                if mode=="sw" {continue;}
                let Some(mut hw)=gpu() else {return};
                let mut first=None;
                let name=hw.adapter.clone();
                bench(&format!("{file} hardware(D3D11 {name})"),&frames,|d|{
                    match hw.decode(codec,d) {Ok(Some(p))=>{first.get_or_insert(p.size);true},Ok(None)=>false,Err(e)=>panic!("{file}: {e:?}")}
                },rounds);
                eprintln!("{file}: hardware picture size {first:?}");
            }
        }
        /// The last decoded picture of a recorded stream as PNG, from each path, for looking at.
        #[test]
        #[ignore = "Needs recorded streams: see tests/capture_stream.rs"]
        fn dump_recorded_pictures_as_png() {
            let out=std::path::PathBuf::from(std::env::var("SECPAR_DUMP_DIR").expect("SECPAR_DUMP_DIR"));
            for file in ["h264_1080.bin","h264_720.bin"] {
                let frames=read_capture(file);
                let mut hw=gpu().expect("GPU");
                let mut soft=Decoder::new().unwrap();
                let (mut last,mut last_soft)=(None,None);
                for (_,d) in frames.iter().take(200) {
                    if let Ok(Some(p))=hw.decode(Codec::H264,d) {last=Some(p);}
                    if let Ok(Some(y))=soft.decode(d) {last_soft=Some(from_software(&y));}
                }
                let (hw_last,sw_last)=(last.unwrap(),last_soft.unwrap());
                assert_eq!(hw_last.size,sw_last.size);
                let (dy,duv)=(mean_abs_diff(&hw_last.y,&sw_last.y),mean_abs_diff(&hw_last.uv,&sw_last.uv));
                eprintln!("DIFF {file}: hardware vs OpenH264 same frame, mean abs luma {dy:.3} chroma {duv:.3}");
                assert!(dy<1.5 && duv<1.5,"hardware and software pictures differ");
                for (tag,picture) in [("hw",hw_last),("sw",sw_last)] {
                    let image=picture.to_color_image(crate::picture::Matrix::Bt709);
                    let rgb:Vec<u8>=image.pixels.iter().flat_map(|c|[c.r(),c.g(),c.b()]).collect();
                    image::save_buffer(out.join(format!("{}_{tag}.png",file.trim_end_matches(".bin"))),&rgb,image.size[0] as u32,image.size[1] as u32,image::ExtendedColorType::Rgb8).unwrap();
                }
            }
        }
    }
}
