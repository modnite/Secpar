use super::*;
use crate::session::{self, Command, Event, Options};
use secpar_stream::{
    frame::{self, Codec, FrameHeader},
    Crypt, Dir, Header, Type,
};
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::{mpsc, oneshot},
};
use tokio_rustls::{rustls::pki_types::PrivatePkcs8KeyDer, TlsAcceptor};

/// Exercises the real TLS/WebSocket/UDP path without capturing or controlling this desktop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streams_with_packet_loss_and_sends_authenticated_input() {
    tokio::time::timeout(Duration::from_secs(12),async{
        let _=rustls::crypto::ring::default_provider().install_default();
        let rcgen::CertifiedKey{cert,signing_key}=rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin:[u8;32]=ring::digest::digest(&ring::digest::SHA256,cert.der()).as_ref().try_into().unwrap();
        let cfg=rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![cert.der().clone()],PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into()).unwrap();
        let accept=TlsAcceptor::from(Arc::new(cfg));let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
        let udp=UdpSocket::bind("127.0.0.1:0").await.unwrap();let stream_addr=udp.local_addr().unwrap();
        let(tx_ack,rx_ack)=oneshot::channel();
        let server=tokio::spawn(async move{
            let(tcp,_)=listener.accept().await.unwrap();let mut ws=tokio_tungstenite::accept_async(accept.accept(tcp).await.unwrap()).await.unwrap();
            let pair:Value=serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();assert_eq!(pair["t"],"pair");
            let token=base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([17;32]);
            for v in [json!({"t":"paired","pc":{"id":"pc1","name":"Fixture"},"token":token}),json!({"t":"remote","url":"wss://relay.example","dev":"dev1","ticket":token})]{ws.send(Message::text(v.to_string())).await.unwrap();}
            let start:Value=serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();assert_eq!(start["t"],"stream.start");assert_eq!(start["codecs"],json!(["h264"]));
            let crypt=Crypt::new(&[251;32],42);
            ws.send(Message::text(json!({"t":"stream.ready","codec":"h264","w":320,"h":240,"fps":60,"port":stream_addr.port(),"session":42,"key":base64::engine::general_purpose::STANDARD.encode([251;32]),"cands":[stream_addr.to_string()]}).to_string())).await.unwrap();
            ws.send(Message::text(json!({"t":"error","code":"audio_unavailable","of":"stream.start","msg":"Synthetic audio source unavailable"}).to_string())).await.unwrap();
            ws.send(Message::text(json!({"t":"clip","mime":"text/plain","data":"Host clipboard fixture"}).to_string())).await.unwrap();
            let mut b=[0;2048];
            let(n,peer)=udp.recv_from(&mut b).await.unwrap();let(h,probe)=crypt.open(Dir::PhoneToHost,&b[..n]).unwrap();assert_eq!(h.typ,Type::Probe);
            let mut echo=100u64.to_be_bytes().to_vec();echo.extend_from_slice(&probe[..8]);
            let header=Header{typ:Type::Probe,flags:0,k:0,seq:1,frame_id:0,frag:0,count:0};
            udp.send_to(&crypt.seal(Dir::HostToPhone,&header,&echo),peer).await.unwrap();
            let payload=vec![93;7000];let fh=FrameHeader{len:7000,capture_us:100,keyframe:true,codec:Codec::H264};
            let audio_header=Header{typ:Type::Audio,seq:2,frame_id:1,..header};
            udp.send_to(&crypt.seal(Dir::HostToPhone,&audio_header,&[0,0,0,0,0,0,0,100,1,2,3]),peer).await.unwrap();
            let mut packets=frame::packetize(7,&fh,&payload,30);packets.remove(0);packets.reverse();
            for(i,(mut h,data))in packets.into_iter().enumerate(){h.seq=i as u32+3;let sealed=crypt.seal(Dir::HostToPhone,&h,&data);udp.send_to(&sealed,peer).await.unwrap();udp.send_to(&sealed,peer).await.unwrap();}
            loop{let(n,_)=udp.recv_from(&mut b).await.unwrap();let Some((h,p))=crypt.open(Dir::PhoneToHost,&b[..n])else{continue;};if h.typ==Type::Input{let input:Value=serde_json::from_slice(&p).unwrap();assert_eq!(input[0]["t"],"key");assert_eq!(input[0]["vk"],65);assert_eq!(input[0]["i"],1);assert_eq!(input[0]["down"],true);tx_ack.send(()).unwrap();break;}}
            let mut copied=false;
            loop{let m=ws.next().await.unwrap().unwrap();if let Message::Text(t)=m{let v:Value=serde_json::from_str(&t).unwrap();
                if v["t"]=="clip" {assert_eq!(v["data"],"Client clipboard fixture");copied=true;}
                if v["t"]=="stream.stop"{assert!(copied,"Client clipboard must use reliable control path");break;}}}
            let _=ws.close(None).await;
        });
        let p=Pairing{name:"Fixture".into(),hosts:vec!["127.0.0.1".into()],port,fingerprint:pin,otp:"once".into(),relay_url:None,pc_id:Some("pc1".into())};
        let c=Connected::pair(p,"Secpar test").await.unwrap();let(tx,rx)=mpsc::channel(8);let(ev_tx,mut ev_rx)=mpsc::channel(8);
        let client=tokio::spawn(session::run(c,Options{discover_public:false,..Default::default()},rx,ev_tx));
        let (mut heard_audio,mut saw_video)=(false,false);
        let (mut audio_notice,mut clipboard,mut negotiated)=(false,false,false);
        loop {
            let event=ev_rx.recv().await.unwrap();
            if let Event::Audio{ref packet,..}=event {assert_eq!(packet,&vec![1,2,3]);heard_audio=true;}
            if let Event::AudioUnavailable(ref msg)=event {assert_eq!(msg,"Synthetic audio source unavailable");audio_notice=true;}
            if let Event::Clipboard(ref text)=event {assert_eq!(text,"Host clipboard fixture");clipboard=true;}
            if let Event::StreamInfo{width,height,fps}=event {assert_eq!((width,height,fps),(320,240,60));negotiated=true;}
            if let Event::Video(frame)=event {assert_eq!(frame.id,7);assert_eq!(frame.data,vec![93;7000]);saw_video=true;}
            // Control uses TLS/WebSocket; media uses UDP. Neither transport
            // orders delivery against the other, even on the loopback device.
            if saw_video && heard_audio && audio_notice && clipboard && negotiated {break;}
        }
        assert!(heard_audio);
        assert!(audio_notice && clipboard && negotiated,"Optional host audio failure must not end video or clipboard delivery");
        tx.send(Command::Clipboard("Client clipboard fixture".into())).await.unwrap();
        tx.send(Command::Input(json!({"t":"key","vk":65,"down":true}))).await.unwrap();rx_ack.await.unwrap();
        tx.send(Command::Stop).await.unwrap();client.await.unwrap().unwrap();server.await.unwrap();
    }).await.expect("Loopback stream must finish promptly");
}

/// A minimized/covered UI must not stop reports/probes or strand compressed video.
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn paused_ui_keeps_network_alive_and_recovers_video_at_a_keyframe() {paused_ui(false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn paused_ui_delivers_video_and_audio_without_waiting_for_a_repaint() {paused_ui(true).await;}
async fn paused_ui(direct:bool) {
    use std::sync::atomic::{AtomicUsize,Ordering};
    tokio::time::timeout(Duration::from_secs(12),async {
        let _=rustls::crypto::ring::default_provider().install_default();
        let rcgen::CertifiedKey{cert,signing_key}=rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin:[u8;32]=ring::digest::digest(&ring::digest::SHA256,cert.der()).as_ref().try_into().unwrap();
        let cfg=rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![cert.der().clone()],PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into()).unwrap();
        let accept=TlsAcceptor::from(Arc::new(cfg));
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
        let mut udp=UdpSocket::bind("127.0.0.1:0").await.unwrap();let address=udp.local_addr().unwrap();
        let(rebind_ack,rebound)=oneshot::channel();
        let probes=Arc::new(AtomicUsize::new(0));let observed=probes.clone();
        let audio_packets=Arc::new(AtomicUsize::new(0));let heard=audio_packets.clone();
        let server=tokio::spawn(async move {
            let(tcp,_)=listener.accept().await.unwrap();
            let mut ws=tokio_tungstenite::accept_async(accept.accept(tcp).await.unwrap()).await.unwrap();
            let _=ws.next().await.unwrap().unwrap();
            let token=base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([17;32]);
            for value in [json!({"t":"paired","pc":{"id":"pc1","name":"Fixture"},"token":token}),json!({"t":"remote","url":"wss://relay.example","dev":"dev1","ticket":token})] {
                ws.send(Message::text(value.to_string())).await.unwrap();
            }
            let _=ws.next().await.unwrap().unwrap();
            ws.send(Message::text(json!({"t":"stream.ready","codec":"h264","w":320,"h":240,"fps":60,"port":address.port(),"session":42,"key":base64::engine::general_purpose::STANDARD.encode([251;32]),"cands":[address.to_string()]}).to_string())).await.unwrap();
            let crypt=Crypt::new(&[251;32],42);let mut peer=None;let mut seq=1;let mut id=1;let mut key=true;
            let mut route_ack=Some(rebind_ack);let mut migrating=false;let mut last_auth=std::time::Instant::now();
            let mut bytes=[0u8;2048];let mut tick=tokio::time::interval(Duration::from_millis(20));
            loop {tokio::select! {
                packet=udp.recv_from(&mut bytes)=>{
                    let(n,from)=packet.unwrap();let Some((header,payload))=crypt.open(Dir::PhoneToHost,&bytes[..n])else{continue};peer=Some(from);last_auth=std::time::Instant::now();
                    if header.typ==Type::Probe && payload.len()>=8 {
                        observed.fetch_add(1,Ordering::Relaxed);
                        if migrating {if let Some(ack)=route_ack.take(){let _=ack.send(());}}
                        let mut echo=100u64.to_be_bytes().to_vec();echo.extend_from_slice(&payload[..8]);
                        let h=Header{typ:Type::Probe,flags:0,k:0,seq,frame_id:0,frag:0,count:0};seq+=1;
                        udp.send_to(&crypt.seal(Dir::HostToPhone,&h,&echo),from).await.unwrap();
                    }
                    if header.typ==Type::Idr {key=true;}
                },
                _=tick.tick()=>if let Some(to)=peer {
                    assert!(last_auth.elapsed()<Duration::from_secs(5),"Fixture host timed out waiting for client keepalives");
                    let data=vec![93;128];let fh=FrameHeader{len:128,capture_us:id as u64*20000,keyframe:key,codec:Codec::H264};key=false;
                    let audio=Header{typ:Type::Audio,flags:0,k:0,seq,frame_id:id,frag:0,count:0};seq+=1;
                    udp.send_to(&crypt.seal(Dir::HostToPhone,&audio,&[0,0,0,0,0,0,0,100,1,2,3]),to).await.unwrap();
                    for(mut header,data)in frame::packetize(id,&fh,&data,0){header.seq=seq;seq+=1;udp.send_to(&crypt.seal(Dir::HostToPhone,&header,&data),to).await.unwrap();}id+=1;
                },
                message=ws.next()=>{
                    let Some(Ok(Message::Text(text)))=message else{break};
                    let value:Value=serde_json::from_str(&text).unwrap();
                    if value["t"]=="stream.stop" {break;}
                    if value["t"]=="stream.display" && value["id"]=="rebind-fixture" {
                        udp=UdpSocket::bind("127.0.0.1:0").await.unwrap();peer=None;key=true;migrating=true;
                        ws.send(Message::text(json!({"t":"stream.cand","c":udp.local_addr().unwrap().to_string()}).to_string())).await.unwrap();
                    }
                }
            }}
        });
        let pairing=Pairing{name:"Fixture".into(),hosts:vec!["127.0.0.1".into()],port,fingerprint:pin,otp:"once".into(),relay_url:None,pc_id:Some("pc1".into())};
        let connection=Connected::pair(pairing,"Secpar test").await.unwrap();
        let(tx,rx)=mpsc::channel(8);let(events,mut ui)=mpsc::channel(8);
        let video_packets=Arc::new(AtomicUsize::new(0));let decoded=video_packets.clone();
        let video_sink:Option<Arc<dyn Fn(crate::receiver::VideoFrame)+Send+Sync>>=direct.then(||Arc::new(move|frame:crate::receiver::VideoFrame| {
            assert_eq!(frame.data,vec![93;128]);decoded.fetch_add(1,Ordering::Relaxed);
        }) as Arc<dyn Fn(crate::receiver::VideoFrame)+Send+Sync>);
        let client=tokio::spawn(session::run(connection,Options{discover_public:false,clipboard:false,audio:true,audio_sink:Some(Arc::new(move|packet|{
            assert_eq!(packet,vec![1,2,3]);heard.fetch_add(1,Ordering::Relaxed);
        })),video_sink,..Default::default()},rx,events));
        if direct {while video_packets.load(Ordering::Relaxed)==0 {tokio::task::yield_now().await;}}
        else {loop {if matches!(ui.recv().await,Some(Event::Video(_))){break;}}}
        let video_before=video_packets.load(Ordering::Relaxed);
        let before=probes.load(Ordering::Relaxed);let audio_before=audio_packets.load(Ordering::Relaxed);
        // Leave a full UI queue alone for longer than the Windows host silence timeout.
        tokio::time::sleep(Duration::from_millis(6300)).await;
        let kept_alive=probes.load(Ordering::Relaxed)>=before+2;
        if !kept_alive {
            drop(ui);let _=client.await;let _=server.await;
            panic!("A paused UI blocked authenticated network keepalives");
        }
        assert!(audio_packets.load(Ordering::Relaxed)>audio_before+100,"Audio delivery must continue while UI is paused");
        if direct {
            assert!(video_packets.load(Ordering::Relaxed)>video_before+100,"Video must reach the decoder while UI is paused");
            while let Ok(event)=ui.try_recv(){assert!(!matches!(event,Event::Video(_)),"A direct frame must not also queue on the UI");}
        } else {loop {if let Some(Event::Video(frame))=ui.recv().await {
            if frame.header.keyframe && frame.id>100 {break;}
        }}}
        // A newly advertised path cannot send until the client punches it.
        // Keep the UI paused again while route recovery probes candidates.
        tx.send(Command::Display("rebind-fixture".into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(4),rebound).await.expect("Client stayed pinned to a dead confirmed route").unwrap();
        tx.send(Command::Stop).await.unwrap();drop(ui);
        let _=client.await.unwrap();server.await.unwrap();
    }).await.expect("Paused-UI stream must recover promptly");
}
