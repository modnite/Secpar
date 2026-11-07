//! A stalled renderer must never apply backpressure to network keepalives.
//! Critical state is bounded/coalesced; compressed video resumes at an IDR after
//! any UI drop. Media and telemetry are disposable when the UI cannot consume.
use super::Event;
use std::collections::VecDeque;
use tokio::sync::mpsc::{self,error::TrySendError};
pub(super) struct UiEvents {
    sender:mpsc::Sender<Event>,
    pending:VecDeque<Event>,
    waiting_keyframe:bool,
}
impl UiEvents {
    pub fn new(sender:mpsc::Sender<Event>)->Self {Self{sender,pending:VecDeque::new(),waiting_keyframe:false}}
    pub fn flush(&mut self)->Result<(),String> {
        if self.sender.is_closed(){return Err("Client window closed".into());}
        while let Some(event)=self.pending.pop_front() {
            match self.sender.try_send(event) {
                Ok(())=>{},Err(TrySendError::Full(event))=>{self.pending.push_front(event);break;},
                Err(TrySendError::Closed(_))=>return Err("Client window closed".into()),
            }
        }
        Ok(())
    }
    pub fn needs_keyframe(&self)->bool {self.waiting_keyframe && self.pending.is_empty() && self.sender.capacity()>0}
    pub fn emit(&mut self,event:Event)->Result<(),String> {
        self.flush()?;
        let video=matches!(event,Event::Video(_));
        if let Event::Video(frame)=&event {
            if self.waiting_keyframe && !frame.header.keyframe{return Ok(());}
        }
        let critical=!matches!(event,Event::Video(_)|Event::Audio{..}|Event::Stats{..}|Event::Cursor{..});
        if critical {
            // Only the newest value of each state type matters while covered.
            let kind=std::mem::discriminant(&event);
            if let Some(slot)=self.pending.iter_mut().find(|e|std::mem::discriminant(*e)==kind){*slot=event;return Ok(());}
            if !self.pending.is_empty(){
                if self.pending.len()>=16{return Err("Client state queue is full".into());}
                self.pending.push_back(event);return Ok(());
            }
        } else if !self.pending.is_empty() {
            if video{self.waiting_keyframe=true;}return Ok(());
        }
        match self.sender.try_send(event) {
            Ok(())=>{if video{self.waiting_keyframe=false;}Ok(())},
            Err(TrySendError::Closed(_))=>Err("Client window closed".into()),
            Err(TrySendError::Full(event))=>{
                if critical {
                    if self.pending.len()>=16{return Err("Client state queue is full".into());}
                    self.pending.push_back(event);
                } else if video {self.waiting_keyframe=true;}
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn covered_ui_keeps_latest_clipboard_and_stream_state_without_unbounded_events() {
        let(tx,mut rx)=mpsc::channel(1);let mut events=UiEvents::new(tx);
        events.emit(Event::Status("Direct".into())).unwrap();
        for n in 0..10000 {
            events.emit(Event::Clipboard(format!("Copied text {n}"))).unwrap();
            events.emit(Event::StreamInfo{width:1280,height:720,fps:60}).unwrap();
            events.emit(Event::Stats{rtt_ms:Some(3),mbps:1.,received_fps:60.}).unwrap();
        }
        assert!(matches!(rx.try_recv(),Ok(Event::Status(s))if s=="Direct"));events.flush().unwrap();
        assert!(matches!(rx.try_recv(),Ok(Event::Clipboard(s))if s=="Copied text 9999"));events.flush().unwrap();
        assert!(matches!(rx.try_recv(),Ok(Event::StreamInfo{width:1280,height:720,fps:60})));events.flush().unwrap();
        assert!(rx.try_recv().is_err(),"Covered-window telemetry must not form a backlog");
    }
}
