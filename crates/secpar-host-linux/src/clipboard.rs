use crate::store::Store;
use secpar_host_core::{agent::Outbox,proto::ServerMsg};
use std::sync::{Arc,Mutex};
use tokio::sync::mpsc;

pub struct Sharing {write:mpsc::Sender<String>,task:tokio::task::JoinHandle<()>}
impl Sharing {
    pub fn start(store:Arc<Mutex<Store>>,out:Outbox)->Self {
        let (write,mut input)=mpsc::channel::<String>(4);
        let task=tokio::spawn(async move {
            let bridge=secpar_clipboard::Bridge::start();
            let mut tick=tokio::time::interval(std::time::Duration::from_millis(150));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    text=input.recv()=>match text {Some(text)=>{if store.lock().unwrap().config().allow_clip {bridge.set(text);}},None=>break},
                    _=tick.tick()=>{
                        if !store.lock().unwrap().config().allow_clip || out.is_closed(){break;}
                        match bridge.poll() {
                            Some(secpar_clipboard::Event::Text(data))=>{if !out.send(ServerMsg::Clip{mime:"text/plain".into(),data}){break;}},
                            Some(secpar_clipboard::Event::Unavailable)=>{
                                out.send(ServerMsg::error("clipboard_unavailable","The Linux desktop clipboard is unavailable",Some("clip.sub")));break;
                            },
                            None=>{},
                        }
                    }
                }
            }
        });
        Self{write,task}
    }
    pub fn set(&self,text:String)->bool {secpar_clipboard::valid(&text)&&self.write.try_send(text).is_ok()}
}
impl Drop for Sharing {fn drop(&mut self){self.task.abort();}}
