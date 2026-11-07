//! Validate before forwarding to the platform's authenticated engine session.
use super::Session;
use crate::proto::{ClientMsg,ServerMsg};

pub(super) fn dispatch(message:ClientMsg,session:&mut Option<Box<dyn Session>>)->Option<ServerMsg> {
    let (id,valid)=match &message {
        ClientMsg::GameStreamInfo{id}=>(id,true),
        ClientMsg::GameStreamPair{id,client_id,certificate_sha256,pin}=>(id,
            !client_id.is_empty() && client_id.len()<=128 && !client_id.chars().any(char::is_control) &&
            certificate_sha256.len()==64 && certificate_sha256.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) &&
            pin.len()==4 && pin.bytes().all(|b|b.is_ascii_digit())),
        _=>unreachable!("Only engine messages are routed here"),
    };
    let valid_id=!id.is_empty() && id.len()<=64 && id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'));
    if !valid || !valid_id {
        return Some(ServerMsg::GameStreamError{id:if valid_id{id.clone()}else{String::new()},code:"bad_request".into(),msg:"Engine request is invalid".into()});
    }
    match session {
        Some(session)=>session.on_msg(message),
        None=>Some(ServerMsg::GameStreamError{id:id.clone(),code:"unavailable".into(),msg:"This host has not enabled the upstream engine".into()}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent::{AgentHandle,BoxFut,Ctx,Handlers,Outbox},PairingStore,proto::PcInfo};
    use std::{sync::{Arc,Mutex,atomic::{AtomicU64,AtomicUsize,Ordering}},time::Duration};
    use tokio::sync::{mpsc,broadcast};
    use tokio_tungstenite::tungstenite::Message;
    struct Store(bool);
    impl PairingStore for Store {
        fn take_otp(&mut self,_:&str)->bool{false}
        fn add_device(&mut self,_:&str)->std::io::Result<(String,String)>{unreachable!()}
        fn auth(&self,token:&str)->Result<String,&'static str>{match token{"one" if !self.0=>Ok("owner-one".into()),"two"=>Ok("owner-two".into()),_=>Err("unauthorized")}}
        fn revoke(&mut self,id:&str)->std::io::Result<()>{if id=="owner-one"{self.0=true;}Ok(())}
        fn ticket(&self,_:&str)->[u8;32]{[0;32]}
    }
    #[derive(Default)]
    struct State {created:Mutex<Vec<String>>,requests:Mutex<Vec<(String,&'static str)>>,dropped:AtomicUsize}
    struct Platform {state:Arc<State>,enabled:bool}
    struct Legacy;
    impl Session for Legacy {fn on_msg(&mut self,m:ClientMsg)->Option<ServerMsg>{assert!(!m.t().starts_with("gamestream."));None}}
    impl Handlers for Platform {
        fn pc_info(&self)->BoxFut<PcInfo>{Box::pin(async{PcInfo{id:"fixture".into(),name:"Fixture".into(),user:String::new(),macs:vec![],os:"test".into()}})}
        fn caps(&self)->Vec<String>{vec![]}
        fn session(&self,_:&str,_:Outbox)->Box<dyn Session>{Box::new(Legacy)}
        fn gamestream_session(&self,device:&str,_:Outbox)->Option<Box<dyn Session>>{
            if !self.enabled{return None;}
            self.state.created.lock().unwrap().push(device.into());
            Some(Box::new(Engine{device:device.into(),state:self.state.clone()}))
        }
    }
    struct Engine {device:String,state:Arc<State>}
    impl Session for Engine {fn on_msg(&mut self,m:ClientMsg)->Option<ServerMsg>{
        self.state.requests.lock().unwrap().push((self.device.clone(),m.t()));
        match m {ClientMsg::GameStreamInfo{id}=>Some(ServerMsg::GameStreamInfo{id,http_port:48989,https_port:48984,certificate_sha256:"a".repeat(64)}),
            ClientMsg::GameStreamPair{id,..}=>Some(ServerMsg::GameStreamPaired{id}),_=>unreachable!()}
    }}
    impl Drop for Engine {fn drop(&mut self){self.state.dropped.fetch_add(1,Ordering::SeqCst);}}
    fn agent(enabled:bool)->(AgentHandle,Arc<State>){
        let state=Arc::new(State::default());
        (AgentHandle{addr:"127.0.0.1:0".parse().unwrap(),ctx:Arc::new(Ctx{store:Arc::new(Mutex::new(Store(false))),
            relay_url:None,handlers:Arc::new(Platform{state:state.clone(),enabled}),idle:Duration::from_secs(60),
            sessions:Mutex::default(),events:broadcast::channel(32).0,next_id:AtomicU64::new(0),relay:Mutex::default()})},state)
    }
    struct Connection {input:mpsc::Sender<Message>,output:mpsc::Receiver<Message>,task:tokio::task::JoinHandle<()>}
    impl Drop for Connection {fn drop(&mut self){self.task.abort();}}
    fn connection(agent:&AgentHandle)->Connection{
        let (input,mut rx)=mpsc::channel(16);let (tx,output)=mpsc::channel(16);
        let incoming=futures_util::stream::poll_fn(move|cx|rx.poll_recv(cx));
        let writer=Box::pin(futures_util::sink::unfold(tx,|tx,message|async move{tx.send(message).await.map(|_|tx)}));
        let agent=agent.clone();let task=tokio::spawn(async move{agent.serve(incoming,writer).await});
        Connection{input,output,task}
    }
    async fn send(connection:&Connection,value:serde_json::Value){connection.input.send(Message::text(value.to_string())).await.unwrap();}
    async fn next(connection:&mut Connection)->serde_json::Value{
        let message=tokio::time::timeout(Duration::from_secs(2),connection.output.recv()).await.unwrap().unwrap();
        serde_json::from_str(message.to_text().unwrap()).unwrap()
    }
    async fn hello(connection:&Connection,token:&str){send(connection,serde_json::json!({"t":"hello","v":1,"token":token,"device":"spoofed-label"})).await;}

    #[tokio::test]
    async fn pairing_messages_before_auth_or_with_wrong_token_never_create_an_engine_session(){
        let (agent,state)=agent(true);
        let mut first=connection(&agent);
        send(&first,serde_json::json!({"t":"gamestream.pair","id":"request","client_id":"fake","certificate_sha256":"a".repeat(64),"pin":"1234"})).await;
        assert_eq!(next(&mut first).await["code"],"unauthorized");
        let mut second=connection(&agent);hello(&second,"wrong").await;
        assert_eq!(next(&mut second).await["code"],"unauthorized");
        assert!(state.created.lock().unwrap().is_empty());
        assert!(state.requests.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn requests_use_authenticated_device_and_invalid_grants_never_reach_backend(){
        let (agent,state)=agent(true);let mut connection=connection(&agent);hello(&connection,"one").await;
        assert_eq!(next(&mut connection).await["t"],"welcome");
        send(&connection,serde_json::json!({"t":"gamestream.pair","id":"request","client_id":"fixture","certificate_sha256":"a".repeat(64),"pin":"12"})).await;
        assert_eq!(next(&mut connection).await["code"],"bad_request");
        assert!(state.requests.lock().unwrap().is_empty());
        send(&connection,serde_json::json!({"t":"gamestream.info","id":"info-1"})).await;
        assert_eq!(next(&mut connection).await["id"],"info-1");
        assert_eq!(*state.created.lock().unwrap(),vec!["owner-one"]);
        assert_eq!(*state.requests.lock().unwrap(),vec![("owner-one".into(),"gamestream.info")]);
    }
    #[tokio::test]
    async fn revocation_drops_only_its_engine_session_and_other_device_stays_active(){
        let (agent,state)=agent(true);let mut one=connection(&agent);let mut two=connection(&agent);
        hello(&one,"one").await;hello(&two,"two").await;next(&mut one).await;next(&mut two).await;
        agent.revoke("owner-one");assert_eq!(next(&mut one).await["code"],"revoked");
        tokio::time::timeout(Duration::from_secs(2),async{while state.dropped.load(Ordering::SeqCst)!=1{tokio::task::yield_now().await;}}).await.unwrap();
        send(&two,serde_json::json!({"t":"gamestream.info","id":"survives"})).await;
        assert_eq!(next(&mut two).await["id"],"survives");
        assert_eq!(*state.requests.lock().unwrap(),vec![("owner-two".into(),"gamestream.info")]);
    }
    #[tokio::test]
    async fn unconfigured_host_returns_correlated_unavailable_without_legacy_fallthrough(){
        let (agent,state)=agent(false);let mut connection=connection(&agent);hello(&connection,"one").await;next(&mut connection).await;
        send(&connection,serde_json::json!({"t":"gamestream.info","id":"disabled"})).await;
        let reply=next(&mut connection).await;assert_eq!(reply["id"],"disabled");assert_eq!(reply["code"],"unavailable");
        assert!(state.requests.lock().unwrap().is_empty());
    }
}
