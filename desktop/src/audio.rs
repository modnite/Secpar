use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(target_os = "linux")]
mod pulse;
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
    audible_frames: u64,
    pushed_frames: u64,
    dropped_frames: u64,
    rendered_frames: u64,
}
impl Buffer {
    fn push(&mut self, samples: &[f32]) {
        {self.pushed_frames+=(samples.len()/2) as u64;}
        self.pcm.extend(samples.iter().copied());
        while self.pcm.len() > MAX_SAMPLES {
            {self.dropped_frames+=1;}
            self.pcm.pop_front();
            self.pcm.pop_front();
        }
    }
    fn frame(&mut self, rate: u32) -> [f32; 2] {
        {self.rendered_frames+=1;}
        if self.pcm.len() < 4 {
            return [0.; 2];
        }
        let t = self.position as f32;
        let a = [self.pcm[0], self.pcm[1]];
        let b = [self.pcm[2], self.pcm[3]];
        let out = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
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
    _stream: Option<cpal::Stream>,
    #[cfg(target_os = "linux")]
    pulse: Option<pulse::Output>,
    failed: Arc<AtomicBool>,
    output: String,
    received_packets: u64,
    decode_errors: u64,
}
impl Audio {
    pub fn new() -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        return Self::open_desktop_service().or_else(|pulse| {
            Self::open_cpal().map_err(|alsa| format!("{pulse}. {alsa}"))
        });
        #[cfg(not(target_os = "linux"))]
        Self::open_cpal()
    }
    #[cfg(target_os = "linux")]
    fn open_desktop_service() -> Result<Self, String> {
        let pulse = pulse::Output::open()?;
        Ok(Self {
                decoder: opus::Decoder::new(48_000, opus::Channels::Stereo)
                    .map_err(|_| "Could not start Opus decoder")?,
                buffer: Arc::default(), _stream: None, pulse: Some(pulse),
                failed: Arc::new(AtomicBool::new(false)), output: String::new(),
                received_packets: 0, decode_errors: 0,
        })
    }
    fn open_cpal() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device")?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("No usable audio output format: {e}"))?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let output = format!("{} · {} Hz · {} channels", device.description()
            .map(|d| d.name().to_owned()).unwrap_or_else(|_| "System default".into()),
            config.sample_rate, config.channels);
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
            _stream: Some(stream),
            #[cfg(target_os = "linux")]
            pulse: None,
            failed,
            output,
            received_packets: 0,
            decode_errors: 0,
        })
    }
    pub fn failed(&self) -> bool {
        #[cfg(target_os = "linux")]
        if self.pulse.as_ref().is_some_and(pulse::Output::failed) { return true; }
        self.failed.load(Ordering::Relaxed)
    }
    /// Counts only; never includes captured audio, credentials, or remote addresses.
    pub fn details(&self) -> String {
        #[cfg(target_os = "linux")]
        if let Some(pulse) = &self.pulse {
            return format!("{}\nPackets received: {}\nDecode errors: {}",
                pulse.details(), self.received_packets, self.decode_errors);
        }
        let b = self.buffer.lock().unwrap();
        format!("Output: {}\nPackets received: {}\nDecode errors: {}\nFrames decoded: {}\nFrames rendered: {}\nAudible frames rendered: {}\nFrames dropped: {}\nFrames queued: {}",
            self.output, self.received_packets, self.decode_errors, b.pushed_frames,
            b.rendered_frames, b.audible_frames, b.dropped_frames, b.pcm.len()/2)
    }
    pub fn packet(&mut self, packet: &[u8]) {
        self.received_packets += 1;
        let mut pcm = [0f32; 5760 * 2];
        if let Ok(n) = self.decoder.decode_float(packet, &mut pcm, false) {
            #[cfg(target_os = "linux")]
            if let Some(pulse) = self.pulse.as_mut() { pulse.packet(&pcm[..n*2]); return; }
            self.buffer.lock().unwrap().push(&pcm[..n * 2]);
        } else {
            self.decode_errors += 1;
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
    #[test]
    #[ignore = "Requires isolated unavailable Pulse service and synthetic ALSA null PCM"]
    fn missing_desktop_service_can_fall_back_to_alsa() {
        assert_eq!(std::env::var("SECPAR_AUDIO_REVIEW").as_deref(), Ok("1"));
        let player = Audio::new().expect("ALSA compatibility output must remain available");
        assert!(player.pulse.is_none(), "Unavailable desktop service must not mask ALSA fallback");
        assert!(player._stream.is_some());
    }
    #[test]
    #[ignore = "Requires isolated Pulse service and deliberately broken ALSA configuration"]
    fn desktop_service_plays_when_default_alsa_format_is_unavailable() {
        assert_eq!(std::env::var("SECPAR_AUDIO_REVIEW").as_deref(), Ok("1"));
        assert!(Audio::open_cpal().is_err(), "Fixture must reproduce ALSA output initialization failure");
        let mut player = Audio::new().expect("Desktop sound service must bypass broken ALSA default");
        assert!(player.pulse.is_some(), "Must actually exercise desktop-service output");
        let mut encoder = opus::Encoder::new(48000, opus::Channels::Stereo, opus::Application::LowDelay).unwrap();
        let mut packet = [0; 1100];
        for block in 0..200 {
            if block == 100 { std::thread::sleep(std::time::Duration::from_millis(600)); }
            let pcm: Vec<f32> = (0..480).flat_map(|i| {
                let value = ((block*480+i) as f32 * std::f32::consts::TAU * 880. / 48000.).sin()*0.2;
                [value,value]
            }).collect();
            let n = encoder.encode_float(&pcm, &mut packet).unwrap();
            player.packet(&packet[..n]);
            std::thread::sleep(std::time::Duration::from_millis(10));
            assert!(!player.failed(), "Desktop sound service failed");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(player.decode_errors, 0);
        assert_eq!(player.pulse.as_ref().unwrap().submitted_frames(), 96000);
        eprintln!("{}", player.details());
    }
    /// Explicit isolated-container gate: no physical audio devices or microphone are mounted.
    #[test]
    #[ignore = "Requires the isolated virtual-output audio review container"]
    fn renders_opus_through_system_audio_output() {
        assert_eq!(std::env::var("SECPAR_AUDIO_REVIEW").as_deref(),Ok("1"));
        let mut player=Audio::open_cpal().expect("ALSA compatibility output must open");
        player.packet(&[3]); // Opus code-3 TOC without its required frame-count byte.
        assert_eq!(player.decode_errors, 1, "Malformed Opus must be visible in diagnostics");
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
        drop(buffer);
        let details = player.details();
        assert!(details.contains("Packets received: 201"));
        assert!(details.contains("Decode errors: 1"));
        assert!(details.contains("Frames decoded: 96000"));
    }
}
