use eframe::egui::ColorImage;
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{AppSink, AppSrc};
use gstreamer_video::{VideoFrameRef, VideoInfo};
use gstreamer_video::prelude::*;

pub struct Hardware {
    pipeline: gst::Pipeline,
    input: AppSrc,
    output: AppSink,
    sequence: u64,
}
impl Hardware {
    pub fn open(factory: &str) -> Result<Self, ()> {
        gst::init().map_err(|_| ())?;
        let description = format!("appsrc name=input is-live=true format=time block=false max-buffers=2 caps=video/x-h264,stream-format=byte-stream,alignment=au ! h264parse ! {factory} ! videoconvert ! video/x-raw,format=RGB ! appsink name=output sync=false max-buffers=1 drop=true enable-last-sample=false");
        let pipeline = gst::parse::launch(&description).map_err(|_| ())?.downcast::<gst::Pipeline>().map_err(|_| ())?;
        let input = pipeline.by_name("input").ok_or(())?.downcast::<AppSrc>().map_err(|_| ())?;
        let output = pipeline.by_name("output").ok_or(())?.downcast::<AppSink>().map_err(|_| ())?;
        let hardware = Self { pipeline, input, output, sequence:0 };
        hardware.pipeline.set_state(gst::State::Playing).map_err(|_| ())?;
        Ok(hardware)
    }
    pub fn decode(&mut self, bytes:&[u8]) -> Result<Option<ColorImage>, ()> {
        // appsrc's advertised maximum alone doesn't bound a nonblocking source.
        if self.input.property::<u64>("current-level-buffers") >= 2 { return Err(()); }
        let mut buffer = gst::Buffer::from_mut_slice(bytes.to_vec());
        buffer.get_mut().ok_or(())?.set_pts(gst::ClockTime::from_mseconds(self.sequence*10));
        self.sequence += 1;
        self.input.push_buffer(buffer).map_err(|_| ())?;
        let sample = self.output.try_pull_sample(gst::ClockTime::from_mseconds(100));
        if let Some(bus) = self.pipeline.bus() {
            if bus.pop_filtered(&[gst::MessageType::Error]).is_some() { return Err(()); }
        }
        let Some(sample) = sample else { return Ok(None); };
        let info = VideoInfo::from_caps(sample.caps().ok_or(())?).map_err(|_| ())?;
        let (w,h) = (info.width() as usize,info.height() as usize);
        if w==0 || h==0 || w>8192 || h>8192 || w*h>4096*2160 { return Err(()); }
        let frame = VideoFrameRef::from_buffer_ref_readable(sample.buffer().ok_or(())?,&info).map_err(|_| ())?;
        let stride = frame.plane_stride()[0];
        if stride < (w*3) as i32 { return Err(()); }
        let data = frame.plane_data(0).map_err(|_| ())?;
        let mut rgb = Vec::with_capacity(w*h*3);
        for row in data.chunks(stride as usize).take(h) {
            if row.len()<w*3 { return Err(()); }
            rgb.extend_from_slice(&row[..w*3]);
        }
        if rgb.len()!=w*h*3 { return Err(()); }
        Ok(Some(ColorImage::from_rgb([w,h],&rgb)))
    }
}
impl Drop for Hardware {
    fn drop(&mut self) { let _=self.pipeline.set_state(gst::State::Null); }
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
                let pixel=image.pixels[0];
                return image.size==[128,128] && [40i16,200,20].into_iter().zip([pixel.r(),pixel.g(),pixel.b()]).all(|(a,b)|(a-b as i16).abs()<20);
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
