//! Test support (the `testing` feature): a STUN + TURN server on loopback, so a host's transport
//! can be tested without a network.

use crate::turn::{read_xor_addr, xor_addr, Msg};
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn sa(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

/// Test support: a fake STUN + TURN server on loopback: Binding, Allocate (with a 401 round), Refresh,
/// CreatePermission, ChannelBind, Send indications and ChannelData to a "peer" socket.
pub struct FakeServer {
    pub addr: SocketAddr,
    log: Arc<Mutex<Vec<String>>>,
    data: Arc<Mutex<Vec<Vec<u8>>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl FakeServer {
    pub const RELAYED: &'static str = "198.51.100.77:50001";
    pub fn start() -> FakeServer {
        use std::sync::atomic::Ordering;
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let addr = sock.local_addr().unwrap();
        let (log, stop) = (Arc::new(Mutex::new(vec![])), Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let data = Arc::new(Mutex::new(vec![]));
        let (l, s, d) = (log.clone(), stop.clone(), data.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !s.load(Ordering::Relaxed) {
                let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
                let pkt = &buf[..n];
                if pkt[0] >= 0x40 {
                    l.lock().unwrap().push(format!("channeldata {:04x}", u16::from_be_bytes([pkt[0], pkt[1]])));
                    let len = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
                    d.lock().unwrap().push(pkt[4..4 + len].to_vec());
                    continue;
                }
                let Some(m) = Msg::decode(pkt) else { continue };
                let reply = |typ: u16, extra: Vec<(u16, Vec<u8>)>| {
                    let mut r = Msg { typ, txid: m.txid, attrs: extra };
                    if typ == 0x0101 {
                        r.attrs.push((0x0020, xor_addr(match from {
                            SocketAddr::V4(v) => v,
                            _ => unreachable!(),
                        })));
                    }
                    r.encode(None)
                };
                let authed = m.get(0x0006).is_some();
                let out = match m.typ {
                    0x0001 => {
                        l.lock().unwrap().push("binding".into());
                        reply(0x0101, vec![])
                    }
                    0x0016 => {
                        l.lock().unwrap().push("send".into());
                        d.lock().unwrap().push(m.get(0x0013).unwrap_or_default().to_vec());
                        continue;
                    }
                    0x0003 | 0x0004 | 0x0008 | 0x0009 if !authed => {
                        reply(m.typ | 0x0110, vec![(0x0009, vec![0, 0, 4, 1]), (0x0014, b"fake".to_vec()), (0x0015, b"n0nce".to_vec())])
                    }
                    0x0003 => {
                        l.lock().unwrap().push("allocate".into());
                        reply(0x0103, vec![(0x0016, xor_addr(match sa(Self::RELAYED) { SocketAddr::V4(v) => v, _ => unreachable!() }))])
                    }
                    0x0004 => {
                        let life = m.get(0x000D).map(|v| u32::from_be_bytes(v.try_into().unwrap())).unwrap_or(1);
                        l.lock().unwrap().push(format!("refresh {life}"));
                        reply(0x0104, vec![])
                    }
                    0x0008 => {
                        let p = m.get(0x0012).and_then(read_xor_addr).unwrap();
                        l.lock().unwrap().push(format!("permit {p}"));
                        reply(0x0108, vec![])
                    }
                    0x0009 => {
                        let p = m.get(0x0012).and_then(read_xor_addr).unwrap();
                        l.lock().unwrap().push(format!("bind {p}"));
                        reply(0x0109, vec![])
                    }
                    _ => continue,
                };
                let _ = sock.send_to(&out, from);
            }
        });
        FakeServer { addr, log, data, stop }
    }
    /// The payloads of everything sent through the relay (ChannelData and Send indications).
    pub fn relayed(&self) -> Vec<Vec<u8>> {
        self.data.lock().unwrap().clone()
    }
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}
impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

