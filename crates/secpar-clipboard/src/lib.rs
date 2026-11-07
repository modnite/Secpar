//! Text clipboard bridge. OS calls run off the UI/stream thread and only while
//! the caller owns an active sharing session. Initial contents are never sent.
use std::{sync::{Arc, atomic::{AtomicBool,Ordering},mpsc},time::Duration};
pub const MAX_TEXT: usize = 1_000_000;
pub fn valid(text:&str)->bool {text.len()<=MAX_TEXT}

pub enum Event { Text(String), Unavailable }
pub struct Bridge {
    write: mpsc::SyncSender<String>,
    read: mpsc::Receiver<Event>,
    live: Arc<AtomicBool>,
}
trait Backend: Send+'static {
    fn read(&mut self)->Option<String>;
    fn write(&mut self,text:&str)->bool;
}
impl Backend for arboard::Clipboard {
    fn read(&mut self)->Option<String> {self.get_text().ok()}
    fn write(&mut self,text:&str)->bool {self.set_text(text).is_ok()}
}
impl Bridge {
    pub fn start()->Self {Self::spawn(||arboard::Clipboard::new().map_err(|_|()))}
    fn spawn<B:Backend>(open:impl FnOnce()->Result<B,()>+Send+'static)->Self {
        let (write,commands)=mpsc::sync_channel::<String>(4);
        let (events,read)=mpsc::sync_channel(2);
        let live=Arc::new(AtomicBool::new(true));let running=live.clone();
        std::thread::spawn(move||{
            let Ok(mut backend)=open()else{let _=events.try_send(Event::Unavailable);return};
            let mut last=backend.read().filter(|s|valid(s));
            let mut seeded=last.is_some();
            while running.load(Ordering::Acquire) {
                match commands.recv_timeout(Duration::from_millis(150)) {
                    Ok(text)=>{
                        if !running.load(Ordering::Acquire){break;}
                        if backend.write(&text){last=Some(text);seeded=true;}
                        else{let _=events.try_send(Event::Unavailable);}
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected)=>break,
                    Err(mpsc::RecvTimeoutError::Timeout)=>{},
                }
                if !running.load(Ordering::Acquire){break;}
                if let Some(text)=backend.read().filter(|s|valid(s)) {
                    if !seeded {last=Some(text);seeded=true;continue;}
                    if last.as_ref()!=Some(&text) && events.try_send(Event::Text(text.clone())).is_ok(){last=Some(text);}
                }
            }
        });
        Self {write,read,live}
    }
    pub fn set(&self,text:String)->bool {valid(&text)&&self.write.try_send(text).is_ok()}
    pub fn poll(&self)->Option<Event> {self.read.try_recv().ok()}
}
impl Drop for Bridge {fn drop(&mut self){self.live.store(false,Ordering::Release);}}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Fake(Arc<Mutex<String>>);
    impl Backend for Fake {
        fn read(&mut self)->Option<String>{Some(self.0.lock().unwrap().clone())}
        fn write(&mut self,text:&str)->bool{*self.0.lock().unwrap()=text.into();true}
    }
    fn wait(mut f:impl FnMut()->bool){let end=std::time::Instant::now()+Duration::from_secs(2);while !f(){assert!(std::time::Instant::now()<end);std::thread::sleep(Duration::from_millis(10));}}
    #[test] fn local_changes_remote_writes_and_echo_suppression() {
        let os=Arc::new(Mutex::new("Initial".into()));let other=os.clone();
        let (started,ready)=mpsc::channel();
        let bridge=Bridge::spawn(move||{started.send(()).unwrap();Ok(Fake(other))});
        ready.recv().unwrap();std::thread::sleep(Duration::from_millis(180));
        assert!(bridge.poll().is_none(),"Connecting must not export pre-existing clipboard");
        *os.lock().unwrap()="Local copy".into();
        wait(||matches!(bridge.poll(),Some(Event::Text(t)) if t=="Local copy"));
        assert!(bridge.set("Remote copy".into()));
        wait(||*os.lock().unwrap()=="Remote copy");
        std::thread::sleep(Duration::from_millis(180));
        assert!(bridge.poll().is_none(),"Remote write must not echo back");
        assert!(!bridge.set("x".repeat(MAX_TEXT+1)));
    }
}
