use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    sync::atomic::{AtomicBool, Ordering},
};
const MAX_SAMPLES: usize = 48_000 / 5 * 2; // 200 ms maximum; discard old audio instead of adding latency.
#[derive(Default)]
struct Buffer {
    pcm: VecDeque<f32>,
    position: f64,
    #[cfg(test)]
    audible_frames: u64,
    #[cfg(test)]
    pushed_frames: u64,
    #[cfg(test)]
    dropped_frames: u64,
    #[cfg(test)]
    rendered_frames: u64,
}
impl Buffer {
    fn push(&mut self, samples: &[f32]) {
        #[cfg(test)]
        {self.pushed_frames+=(samples.len()/2) as u64;}
        self.pcm.extend(samples.iter().copied());
        while self.pcm.len() > MAX_SAMPLES {
            #[cfg(test)]
            {self.dropped_frames+=1;}
            self.pcm.pop_front();
            self.pcm.pop_front();
        }
    }
    fn frame(&mut self, rate: u32) -> [f32; 2] {
        #[cfg(test)]
        {self.rendered_frames+=1;}
        if self.pcm.len() < 4 {
            return [0.; 2];
        }
        let t = self.position as f32;
        let a = [self.pcm[0], self.pcm[1]];
        let b = [self.pcm[2], self.pcm[3]];
        let out = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        #[cfg(test)]
        if out[0].abs().max(out[1].abs())>0.01 {self.audible_frames+=1;}
        self.position += 48_000.0 / rate as f64;
        while self.position >= 1.0 && self.pcm.len() >= 2 {
            self.pcm.pop_front();
            self.pcm.pop_front();
            self.position -= 1.;
        }
        out
    }
}
pub struct Audio {
    decoder: opus::Decoder,
    buffer: Arc<Mutex<Buffer>>,
    _stream: cpal::Stream,
    failed: Arc<AtomicBool>,
}
impl Audio {
    pub fn new() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device")?;
        let supported = device
            .default_output_config()
            .map_err(|_| "No usable audio output format")?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        #[cfg(test)]
        eprintln!("Output format: {format}, channels {}, rate {}, buffer {:?}",config.channels,config.sample_rate,config.buffer_size);
        if config.channels == 0 || config.sample_rate == 0 {
            return Err("Invalid audio output format".into());
        }
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        let failed = Arc::new(AtomicBool::new(false));
        let stream = match format {
            cpal::SampleFormat::I8 => build::<i8>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::I24 => build::<cpal::I24>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::I64 => build::<i64>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::U8 => build::<u8>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::U24 => build::<cpal::U24>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::U32 => build::<u32>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::U64 => build::<u64>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, buffer.clone(), failed.clone()),
            cpal::SampleFormat::F64 => build::<f64>(&device, &config, buffer.clone(), failed.clone()),
            _ => return Err(format!("Unsupported audio output format: {format}")),
        }
        .map_err(|e| format!("Could not open audio output: {e}"))?;
        stream.play().map_err(|_| "Could not start audio output")?;
        let decoder = opus::Decoder::new(48_000, opus::Channels::Stereo)
            .map_err(|_| "Could not start Opus decoder")?;
        Ok(Self {
            decoder,
            buffer,
            _stream: stream,
            failed,
        })
    }
    pub fn failed(&self) -> bool { self.failed.load(Ordering::Relaxed) }
    pub fn packet(&mut self, packet: &[u8]) {
        let mut pcm = [0f32; 5760 * 2];
        if let Ok(n) = self.decoder.decode_float(packet, &mut pcm, false) {
            self.buffer.lock().unwrap().push(&pcm[..n * 2]);
        }
    }
}
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<Buffer>>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut buffer = buffer.lock().unwrap();
            for frame in data.chunks_mut(channels) {
                let stereo = buffer.frame(rate);
                for (ch, sample) in frame.iter_mut().enumerate() {
                    let value = if channels == 1 {
                        (stereo[0] + stereo[1]) * 0.5
                    } else {
                        match ch {
                            0 => stereo[0],
                            1 => stereo[1],
                            _ => 0.0,
                        }
                    };
                    *sample = T::from_sample(value);
                }
            }
        },
        move |_| { failed.store(true, Ordering::Relaxed); },
        None,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_is_bounded_and_underruns_are_silent() {
        let mut b = Buffer::default();
        assert_eq!(b.frame(48000), [0., 0.]);
        b.push(&vec![0.5; MAX_SAMPLES * 3]);
        assert_eq!(b.pcm.len(), MAX_SAMPLES);
        assert_eq!(b.frame(48000), [0.5, 0.5]);
    }
    #[test]
    fn resamples_stereo_and_downstream_can_mix_to_mono() {
        let mut b = Buffer::default();
        b.push(&[0., 1., 1., 0., 0., 1., 1., 0.]);
        assert_eq!(b.frame(96000), [0., 1.]);
        assert_eq!(b.frame(96000), [0.5, 0.5]);
        assert_eq!(b.frame(96000), [1., 0.]);
    }
}

#[cfg(test)]
mod codec_tests {
    #[test]
    fn decodes_the_hosts_ten_millisecond_opus_format() {
        let mut encoder =
            opus::Encoder::new(48000, opus::Channels::Stereo, opus::Application::LowDelay).unwrap();
        let input: Vec<f32> = (0..480)
            .flat_map(|i| {
                let s = (i as f32 * std::f32::consts::TAU * 1000. / 48000.).sin() * 0.2;
                [s, s]
            })
            .collect();
        let mut packet = [0; 1100];
        let n = encoder.encode_float(&input, &mut packet).unwrap();
        let mut decoder = opus::Decoder::new(48000, opus::Channels::Stereo).unwrap();
        let mut output = [0.; 960];
        assert_eq!(
            decoder
                .decode_float(&packet[..n], &mut output, false)
                .unwrap(),
            480
        );
        assert!(output.iter().any(|s| s.abs() > 0.01));
    }
}

#[cfg(all(test,target_os="linux"))]
mod device_tests {
    use super::*;
    /// Explicit isolated-container gate: no physical audio devices or microphone are mounted.
    #[test]
    #[ignore = "Requires the isolated virtual-output audio review container"]
    fn renders_opus_through_system_audio_output() {
        assert_eq!(std::env::var("SECPAR_AUDIO_REVIEW").as_deref(),Ok("1"));
        let mut player=Audio::new().expect("System audio output must open");
        let mut encoder=opus::Encoder::new(48000,opus::Channels::Stereo,opus::Application::LowDelay).unwrap();
        let mut packet=[0u8;1100];
        for block in 0..200 {
            let samples:Vec<f32>=(0..480).flat_map(|i| {
                let value=((block*480+i) as f32*std::f32::consts::TAU*880./48000.).sin()*0.2;
                [value,value]
            }).collect();
            let size=encoder.encode_float(&samples,&mut packet).unwrap();
            player.packet(&packet[..size]);
            std::thread::sleep(std::time::Duration::from_millis(10));
            assert!(!player.failed(),"Audio callback failed");
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!player.failed());
        let buffer=player.buffer.lock().unwrap();
        eprintln!("Generated audio: pushed {}, dropped {}, rendered {}, audible {}, queued {}",buffer.pushed_frames,buffer.dropped_frames,buffer.rendered_frames,buffer.audible_frames,buffer.pcm.len());
        assert!(buffer.audible_frames>48000,"Output callback did not consume the generated audio");
    }
}
