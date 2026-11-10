//! Opt-in, content-free cadence measurements. No frame data or device identity is logged.
//! Set SECPAR_FRAME_TIMING=1 before launching a local review build.
use std::{cell::RefCell, sync::{Mutex, OnceLock}, time::{Duration, Instant}};

#[derive(Clone, Copy)]
pub enum Stage { Arrive, Submit, Decode, Decoded, Upload, Draw, Capture, Selected, Convert, Encode, Encoded, Packetize, Send, Sent, CaptureTextureWait, EncoderTextureWait, NvencMap, NvencSubmit, NvencWait, NvencLock, NvencCopy, NvencUnmap }
impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::Arrive=>"client.arrive.interval", Self::Submit=>"client.submit.interval",
            Self::Decode=>"client.decode.duration", Self::Decoded=>"client.decoded.interval",
            Self::Upload=>"client.upload.duration", Self::Draw=>"client.draw.interval",
            Self::Capture=>"host.capture.interval", Self::Selected=>"host.selected.interval",
            Self::Convert=>"host.convert.duration", Self::Encode=>"host.encode.duration",
            Self::Encoded=>"host.encoded.interval",
            Self::Packetize=>"host.packetize.duration", Self::Send=>"host.send.duration",
            Self::Sent=>"host.sent.interval",
            Self::CaptureTextureWait=>"host.capture_texture.wait", Self::EncoderTextureWait=>"host.encoder_texture.wait",
            Self::NvencMap=>"host.nvenc.map", Self::NvencSubmit=>"host.nvenc.submit", Self::NvencWait=>"host.nvenc.wait",
            Self::NvencLock=>"host.nvenc.lock", Self::NvencCopy=>"host.nvenc.copy", Self::NvencUnmap=>"host.nvenc.unmap",
        }
    }
}
const BINS: usize = 512;
const WINDOW: u64 = 300;
struct Series { bins: [u64; BINS], count: u64, total_us: u128, max_us: u128, last: Option<Instant> }
impl Default for Series {
    fn default()->Self { Self { bins:[0;BINS],count:0,total_us:0,max_us:0,last:None } }
}
impl Series {
    fn add(&mut self, duration:Duration) {
        let us=duration.as_micros();
        self.bins[(us/500).min((BINS-1) as u128) as usize]+=1;
        self.count+=1; self.total_us+=us; self.max_us=self.max_us.max(us);
    }
    fn percentile(&self, percent:u64)->u64 {
        let rank=(self.count*percent).div_ceil(100).max(1);
        let mut seen=0;
        for (bin,count) in self.bins.iter().enumerate() {
            seen+=count; if seen>=rank {return bin as u64*500;}
        }
        0
    }
    fn interval(&mut self,now:Instant)->Option<Duration> {
        self.last.replace(now).map(|last|now.saturating_duration_since(last))
    }
}
static ENABLED: OnceLock<bool> = OnceLock::new();
// A network task can migrate between Tokio threads. Its arrival clock must not.
static ARRIVAL: OnceLock<Mutex<Series>> = OnceLock::new();
const STAGES: usize = Stage::NvencUnmap as usize + 1;
thread_local! {static SERIES: RefCell<[Series;STAGES]> = RefCell::new(std::array::from_fn(|_|Series::default()));}
pub fn enabled()->bool {*ENABLED.get_or_init(|| std::env::var("SECPAR_FRAME_TIMING").is_ok_and(|v|v=="1"))}
/// The first event establishes the origin; it is not a zero-length sample.
pub fn mark(stage:Stage) {
    if !enabled() {return;}
    if matches!(stage,Stage::Arrive) {
        let mut series=ARRIVAL.get_or_init(||Mutex::new(Series::default())).lock().unwrap();
        if let Some(duration)=series.interval(Instant::now()) {record(&mut series,stage,duration);}
        return;
    }
    SERIES.with(|all| {
        let mut all=all.borrow_mut(); let series=&mut all[stage as usize];
        if let Some(duration)=series.interval(Instant::now()) {record(series,stage,duration);}
    });
}
pub fn elapsed(stage:Stage, began:Option<Instant>) {
    if let Some(began)=began {sample(stage,began.elapsed());}
}
pub fn begin()->Option<Instant> {enabled().then(Instant::now)}
pub fn duration(stage:Stage,duration:Duration) {
    if enabled() {sample(stage,duration);}
}
fn sample(stage:Stage,duration:Duration) {
    SERIES.with(|all|record(&mut all.borrow_mut()[stage as usize],stage,duration));
}
fn record(series:&mut Series,stage:Stage,duration:Duration) {
    series.add(duration);
    if series.count==WINDOW {
        eprintln!("frame_timing stage={} n={} mean_us={} p50_us={} p95_us={} p99_us={} max_us={} hist_500us={:?}",
            stage.label(),series.count,series.total_us/series.count as u128,
            series.percentile(50),series.percentile(95),series.percentile(99),series.max_us,
            series.bins.iter().enumerate().filter(|(_,n)|**n>0).map(|(i,n)|(i*500,*n)).collect::<Vec<_>>());
        let last=series.last; *series=Series::default();series.last=last;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn histogram_distinguishes_even_frames_from_alternating_short_long_frames() {
        let mut even=Series::default();let mut judder=Series::default();
        for _ in 0..150 {even.add(Duration::from_micros(16667));even.add(Duration::from_micros(16667));
            judder.add(Duration::from_micros(8333));judder.add(Duration::from_micros(25001));}
        assert!(even.total_us.abs_diff(judder.total_us)<300);
        assert_eq!(even.percentile(95),16500);assert_eq!(judder.percentile(95),25000);
        assert_eq!(judder.bins[16],150);assert_eq!(judder.bins[50],150);
    }
    #[test]
    fn first_event_is_not_counted_and_long_hitches_remain_visible() {
        let mut s=Series::default();let now=Instant::now();
        assert!(s.interval(now).is_none());
        let interval=s.interval(now+Duration::from_secs(2)).unwrap();s.add(interval);
        assert_eq!(s.count,1);assert_eq!(s.max_us,2_000_000);assert_eq!(s.bins[BINS-1],1);
    }
}
