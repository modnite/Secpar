//! Keep audio decoding/output independent of eframe repainting and occlusion.
//! The accepted Audio engine and its output/silence-recovery policy are unchanged.
use crate::audio::Audio;
use std::{sync::{Arc,Mutex,mpsc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};
type Packet=(Instant,Vec<u8>);
type Sender=mpsc::SyncSender<Packet>;
pub type Sink=Arc<dyn Fn(Vec<u8>)+Send+Sync>;
#[derive(Clone,Default)]
pub struct Route(Arc<Mutex<Option<Sender>>>);
impl Route {
    pub fn sink(&self)->Sink {
        let route=self.clone();Arc::new(move|packet|{
            if let Some(sender)=route.0.lock().unwrap().as_ref(){let _=sender.try_send((Instant::now(),packet));}
        })
    }
    pub fn output(&self,worker:Option<&Worker>){*self.0.lock().unwrap()=worker.map(|w|w.sender.clone());}
}
#[derive(Default)]
struct State {details:String,failed:bool}
pub struct Worker {sender:Sender,state:Arc<Mutex<State>>,live:Arc<AtomicBool>}
impl Worker {
    pub fn start()->Result<Self,String> {
        let(sender,packets)=mpsc::sync_channel::<Packet>(16);
        let(started,ready)=mpsc::sync_channel(1);
        let state=Arc::new(Mutex::new(State::default()));let status=state.clone();
        let live=Arc::new(AtomicBool::new(true));let running=live.clone();
        std::thread::spawn(move||{
            let mut player=match Audio::new(){Ok(player)=>player,Err(error)=>{let _=started.send(Err(error));return;}};
            status.lock().unwrap().details=player.details();
            if started.send(Ok(())).is_err(){return;}
            let mut report=Instant::now();
            while running.load(Ordering::Acquire) {
                match packets.recv_timeout(Duration::from_millis(20)) {
                    Ok((received,packet))=>if running.load(Ordering::Acquire) && received.elapsed()<=Duration::from_millis(200){player.packet(&packet);},
                    Err(mpsc::RecvTimeoutError::Disconnected)=>break,
                    Err(mpsc::RecvTimeoutError::Timeout)=>{},
                }
                if player.failed(){status.lock().unwrap().failed=true;break;}
                if report.elapsed()>=Duration::from_millis(200){status.lock().unwrap().details=player.details();report=Instant::now();}
            }
            // Output and decoder are destroyed on the thread that created them.
        });
        match ready.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(()))=>Ok(Self{sender,state,live}),
            Ok(Err(error))=>{live.store(false,Ordering::Release);Err(error)},
            Err(_)=>{live.store(false,Ordering::Release);Err("Audio output did not start".into())},
        }
    }
    pub fn failed(&self)->bool {self.state.lock().unwrap().failed}
    pub fn details(&self)->String {self.state.lock().unwrap().details.clone()}
    pub fn packet(&self,packet:&[u8]){let _=self.sender.try_send((Instant::now(),packet.to_vec()));}
}
impl Drop for Worker {fn drop(&mut self){self.live.store(false,Ordering::Release);}}

#[cfg(all(test,target_os="linux"))]
mod tests {
    use super::*;
    #[test]
    #[ignore="requires isolated desktop audio service; SECPAR_AUDIO_REVIEW=1"]
    fn audio_plays_and_recovers_without_any_ui_polling() {
        assert_eq!(std::env::var("SECPAR_AUDIO_REVIEW").as_deref(),Ok("1"));
        let worker=Worker::start().expect("Real desktop sound output must open");
        let route=Route::default();route.output(Some(&worker));let sink=route.sink();
        let mut encoder=opus::Encoder::new(48000,opus::Channels::Stereo,opus::Application::Audio).unwrap();
        let start=Instant::now();let mut packet=vec![0u8;4096];
        for n in 0..240 {
            let samples:Vec<f32>=(0..480).flat_map(|i|{
                let phase=(n*480+i) as f32*440.*std::f32::consts::TAU/48000.;let s=phase.sin()*0.1;[s,s]
            }).collect();
            let bytes=encoder.encode_float(&samples,&mut packet).unwrap();sink(packet[..bytes].to_vec());
            std::thread::sleep(Duration::from_millis(10));
            if n==100 {std::thread::sleep(Duration::from_millis(600));}
        }
        std::thread::sleep(Duration::from_millis(250));
        let details=worker.details();println!("{details}");
        assert!(details.contains("Packets received: 240"));assert!(details.contains("Decode errors: 0"));
        assert!(!worker.failed());assert!(start.elapsed()>Duration::from_secs(3));
        route.output(None);let stop=Instant::now();drop(worker);
        assert!(stop.elapsed()<Duration::from_millis(100),"Closing the UI must not wait for audio");
    }
}

#[cfg(all(test,windows))]
mod windows_tests {
    use super::*;
    #[test]
    #[ignore="opens the shared default output, with silence only; SECPAR_AUDIO_REVIEW=1"]
    fn default_output_opens_on_audio_thread_without_playing_content() {
        assert_eq!(std::env::var("SECPAR_AUDIO_REVIEW").as_deref(),Ok("1"));
        let worker=Worker::start().expect("Shared default output must initialize on worker thread");
        std::thread::sleep(Duration::from_millis(80));
        assert!(!worker.failed());assert!(worker.details().contains("Packets received: 0"));
        let stop=Instant::now();drop(worker);assert!(stop.elapsed()<Duration::from_millis(100));
    }
}
