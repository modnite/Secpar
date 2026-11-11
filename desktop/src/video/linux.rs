use crate::picture::Picture;
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{AppSink, AppSinkCallbacks, AppSrc};
use gstreamer_video::{VideoFrameRef, VideoInfo};
use gstreamer_video::prelude::*;
use std::{collections::VecDeque, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

struct Pending { pts: u64, generation: u64, submitted: Instant }

pub struct Hardware {
    pipeline: gst::Pipeline,
    input: AppSrc,
    output: AppSink,
    sequence: u64,
    epoch: Option<u64>,
    last_pts: Option<u64>,
    asynchronous: bool,
    pending: Arc<Mutex<VecDeque<Pending>>>,
    failed: Arc<AtomicBool>,
}
impl Hardware {
    pub fn open(factory: &str) -> Result<Self, ()> {
        gst::init().map_err(|_| ())?;
        let description = format!("appsrc name=input is-live=true format=time block=false max-buffers=8 caps=video/x-h264,stream-format=byte-stream,alignment=au ! h264parse ! {factory} ! videoconvert ! video/x-raw,format=NV12 ! appsink name=output sync=false max-buffers=1 drop=true enable-last-sample=false");
        let pipeline = gst::parse::launch(&description).map_err(|_| ())?.downcast::<gst::Pipeline>().map_err(|_| ())?;
        let input = pipeline.by_name("input").ok_or(())?.downcast::<AppSrc>().map_err(|_| ())?;
        let output = pipeline.by_name("output").ok_or(())?.downcast::<AppSink>().map_err(|_| ())?;
        let hardware = Self { pipeline, input, output, sequence:0, epoch:None, last_pts:None,
            asynchronous:false, pending:Arc::new(Mutex::new(VecDeque::new())), failed:Arc::new(AtomicBool::new(false)) };
        hardware.pipeline.set_state(gst::State::Playing).map_err(|_| ())?;
        Ok(hardware)
    }
    /// Deliver pictures from the pipeline's output thread. Input never waits for a
    /// particular frame, and a final picture is delivered even if input goes idle.
    pub fn enable_async_output(&mut self, deliver: impl Fn(u64, Picture)+Send+Sync+'static) {
        let pending=self.pending.clone();
        let failed=self.failed.clone();
        self.output.set_callbacks(AppSinkCallbacks::builder().new_sample(move |sink| {
            let result=(|| {
                let sample=sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let pts=sample.buffer().and_then(|b|b.pts()).ok_or(gst::FlowError::Error)?.nseconds();
                let image=picture(&sample).map_err(|_| gst::FlowError::Error)?;
                let generation={
                    let mut queued=pending.lock().unwrap();
                    let at=queued.iter().position(|p|p.pts==pts).ok_or(gst::FlowError::Error)?;
                    let generation=queued[at].generation;
                    queued.drain(..=at);
                    generation
                };
                deliver(generation,image);
                Ok(gst::FlowSuccess::Ok)
            })();
            if result.is_err() {failed.store(true,Ordering::Relaxed);}
            result
        }).build());
        self.asynchronous=true;
    }
    // The small capability probe uses the same pipeline with a bounded wait.
    pub fn decode(&mut self, bytes:&[u8]) -> Result<Option<Picture>, ()> {
        let capture=self.sequence*16_667;
        self.sequence+=1;
        self.decode_timed(bytes,capture,60,0)
    }
    pub fn decode_timed(&mut self, bytes:&[u8], capture_us:u64, fps:u32, generation:u64) -> Result<Option<Picture>, ()> {
        if self.failed.load(Ordering::Relaxed) {return Err(());}
        if let Some(bus)=self.pipeline.bus() {
            if bus.pop_filtered(&[gst::MessageType::Error]).is_some() {return Err(());}
        }
        // Bound both encoded input and frames waiting for the GPU/readback. A
        // stalled driver still triggers software fallback instead of growing RAM.
        if self.input.property::<u64>("current-level-buffers")>=8 {return Err(());}
        if self.asynchronous {
            let queued=self.pending.lock().unwrap();
            if queued.len()>=8 || queued.front().is_some_and(|p|p.submitted.elapsed()>Duration::from_millis(250)) {return Err(());}
        }
        let epoch=*self.epoch.get_or_insert(capture_us);
        let pts=capture_us.saturating_sub(epoch).saturating_mul(1000)
            .max(self.last_pts.map_or(0,|p|p.saturating_add(1)));
        self.last_pts=Some(pts);
        let mut buffer=gst::Buffer::from_mut_slice(bytes.to_vec());
        let data=buffer.get_mut().ok_or(())?;
        data.set_pts(gst::ClockTime::from_nseconds(pts));
        data.set_duration(gst::ClockTime::from_nseconds(1_000_000_000/fps.clamp(1,240) as u64));
        if self.asynchronous {
            // Publish metadata before push_buffer: output may run immediately.
            self.pending.lock().unwrap().push_back(Pending{pts,generation,submitted:Instant::now()});
        }
        self.input.push_buffer(buffer).map_err(|_| ())?;
        if self.asynchronous {return Ok(None);}
        self.output.try_pull_sample(gst::ClockTime::from_mseconds(100))
            .map(|sample|picture(&sample)).transpose()
    }
}
fn picture(sample:&gst::Sample)->Result<Picture,()> {
    let info=VideoInfo::from_caps(sample.caps().ok_or(())?).map_err(|_| ())?;
    let (w,h)=(info.width() as usize,info.height() as usize);
    if w==0 || h==0 || w>8192 || h>8192 || w*h>4096*2160 {return Err(());}
    let frame=VideoFrameRef::from_buffer_ref_readable(sample.buffer().ok_or(())?,&info).map_err(|_| ())?;
    let strides=frame.plane_stride();
    if strides[0]<0 || strides[1]<0 {return Err(());}
    let luma=frame.plane_data(0).map_err(|_| ())?;
    let chroma=frame.plane_data(1).map_err(|_| ())?;
    Picture::from_nv12([w,h],(luma,strides[0] as usize),(chroma,strides[1] as usize)).ok_or(())
}
impl Drop for Hardware {
    fn drop(&mut self) {let _=self.pipeline.set_state(gst::State::Null);}
}

/// A real decoded frame is required. Registry/plugin presence is insufficient.
pub fn probe() -> Option<&'static str> {
    gst::init().ok()?;
    let fixture = fixture();
    ["vah264dec","vaapih264dec"].into_iter().find(|factory| {
        gst::ElementFactory::find(factory).is_some() && verify(factory,&fixture)
    })
}
fn fixture() -> Vec<u8> {
    use openh264::{encoder::Encoder, formats::{RgbSliceU8,YUVBuffer}};
    let rgb:Vec<u8>=(0..128*128).flat_map(|_|[40,200,20]).collect();
    let yuv=YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb,(128,128)));
    Encoder::new().and_then(|mut e|e.encode(&yuv).map(|p|p.to_vec())).unwrap_or_default()
}
fn verify(factory:&str,fixture:&[u8]) -> bool {
    if fixture.is_empty() { return false; }
    let Ok(mut decoder)=Hardware::open(factory) else { return false; };
    for _ in 0..4 {
        match decoder.decode(fixture) {
            Ok(Some(image)) => {
                let pixel=image.rgb_at(0,0,crate::picture::Matrix::Bt601);
                return image.size==[128,128] && [40i16,200,20].into_iter().zip(pixel).all(|(a,b)|(a-b as i16).abs()<20);
            }
            Ok(None) => {}, Err(_) => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_driver_or_factory_is_not_reported_as_acceleration() {
        assert!(!verify("secpar_nonexistent_decoder",&fixture()));
    }
    #[test]
    fn gstreamer_path_decodes_real_fixture_without_a_gpu() {
        gst::init().unwrap();
        assert!(gst::ElementFactory::find("openh264dec").is_some(),"Test requires bundled OpenH264 plugin");
        assert!(verify("openh264dec",&fixture()));
    }
}
