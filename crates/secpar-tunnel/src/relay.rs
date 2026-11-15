//! Sending the tunnel through a relay when two devices cannot reach each other.
//!
//! Each side asks the relay (a TURN server) for an address of its own and tells
//! the other side that address. Packets to a device known by its relay address
//! go through this side's relay; packets arriving through it are handed on as
//! if they came from that address. The relay sees only the tunnel's encrypted
//! packets. The TURN client is the one Secpar 3 uses.
use quinn::{
    udp::{RecvMeta, Transmit},
    AsyncUdpSocket, UdpPoller,
};
use secpar_network::turn::Turn;
use std::{
    collections::{HashSet, VecDeque},
    fmt,
    io::{self, IoSliceMut},
    net::{SocketAddr, SocketAddrV4, UdpSocket},
    pin::Pin,
    sync::{mpsc, Arc, Mutex},
    task::{ready, Context, Poll},
    time::Duration,
};

/// A question to a public server, "which address do you see me at", in flight.
type Asked = (SocketAddr, [u8; 12], mpsc::Sender<SocketAddr>);

/// Reads the address out of a STUN answer to the request with this id.
fn seen_address(answer: &[u8], id: &[u8; 12]) -> Option<SocketAddr> {
    if answer.len() < 20 || answer[8..20] != id[..] {
        return None;
    }
    let mut at = 20;
    while at + 4 <= answer.len() {
        let kind = u16::from_be_bytes([answer[at], answer[at + 1]]);
        let size = u16::from_be_bytes([answer[at + 2], answer[at + 3]]) as usize;
        let value = at + 4;
        // XOR-MAPPED-ADDRESS, IPv4.
        if kind == 0x0020 && size >= 8 && value + 8 <= answer.len() && answer[value + 1] == 1 {
            let cookie = 0x2112_A442u32.to_be_bytes();
            let port = u16::from_be_bytes([answer[value + 2] ^ cookie[0], answer[value + 3] ^ cookie[1]]);
            let mut ip = [0u8; 4];
            for (i, byte) in ip.iter_mut().enumerate() {
                *byte = answer[value + 4 + i] ^ cookie[i];
            }
            return Some(SocketAddr::from((ip, port)));
        }
        at = value + size.div_ceil(4) * 4;
    }
    None
}

/// One tunnel socket's use of the relay. The calls that talk to the relay block
/// for up to a few seconds: run them off the async threads.
pub struct Relay {
    control: UdpSocket,
    turn: Mutex<Option<Arc<Turn>>>,
    peers: Mutex<HashSet<SocketAddrV4>>,
    asked: Mutex<Option<Asked>>,
}
impl Relay {
    pub(crate) fn new(control: UdpSocket) -> Arc<Self> {
        Arc::new(Self { control, turn: Mutex::default(), peers: Mutex::default(), asked: Mutex::default() })
    }
    /// The address and port this socket has on the internet, as the public
    /// server `stun` sees it. Another device can send there once this side has
    /// sent something to that device first.
    pub fn seen_from_outside(&self, stun: SocketAddr) -> Option<SocketAddr> {
        let mut request = [0u8; 20];
        request[1] = 1;
        request[4..8].copy_from_slice(&0x2112_A442u32.to_be_bytes());
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos().to_be_bytes();
        request[8..20].copy_from_slice(&id[4..16]);
        let (found, finding) = mpsc::channel();
        *self.asked.lock().unwrap() = Some((stun, request[8..20].try_into().ok()?, found));
        let mut seen = None;
        for wait in [300, 600, 1200] {
            let _ = self.control.send_to(&request, stun);
            if let Ok(address) = finding.recv_timeout(Duration::from_millis(wait)) {
                seen = Some(address);
                break;
            }
        }
        *self.asked.lock().unwrap() = None;
        seen.filter(|a| matches!(a, SocketAddr::V4(v4) if secpar_network::turn::is_public(v4)))
    }
    fn turn(&self) -> Option<Arc<Turn>> {
        self.turn.lock().unwrap().clone()
    }
    /// This side's address at the relay, once it has one.
    pub fn address(&self) -> Option<SocketAddr> {
        self.turn().and_then(|t| *t.relayed.lock().unwrap()).map(SocketAddr::V4)
    }
    /// Asks the relay for an address, replacing an earlier one.
    pub fn start(&self, server: SocketAddr, user: String, pass: String) -> Option<SocketAddr> {
        self.stop();
        let turn = Arc::new(Turn::new(server, user, pass));
        // In place before asking, so the answer is recognised when it arrives.
        *self.turn.lock().unwrap() = Some(turn.clone());
        match turn.allocate(&self.control) {
            Some(address) => Some(SocketAddr::V4(address)),
            None => {
                *self.turn.lock().unwrap() = None;
                None
            }
        }
    }
    /// From now on, reach `peer` (the other side's relay address) through the relay.
    pub fn add(&self, peer: SocketAddr) {
        let (Some(turn), SocketAddr::V4(peer)) = (self.turn(), peer) else { return };
        self.peers.lock().unwrap().insert(peer);
        turn.permit(&self.control, &[peer]);
    }
    /// Keeps the relay address and its peers for another ten minutes. Call every
    /// four minutes while it is needed.
    pub fn keep(&self) {
        let Some(turn) = self.turn() else { return };
        turn.refresh(&self.control, 600);
        let peers: Vec<_> = self.peers.lock().unwrap().iter().copied().collect();
        turn.permit(&self.control, &peers);
    }
    /// Gives the relay address back.
    pub fn stop(&self) {
        self.peers.lock().unwrap().clear();
        if let Some(turn) = self.turn.lock().unwrap().take() {
            // Sent without waiting for the answer.
            let control = self.control.try_clone();
            std::thread::spawn(move || control.map(|control| turn.refresh(&control, 0)));
        }
    }
    fn through(&self, to: SocketAddr) -> Option<(Arc<Turn>, SocketAddrV4)> {
        let SocketAddr::V4(to) = to else { return None };
        if !self.peers.lock().unwrap().contains(&to) {
            return None;
        }
        Some((self.turn()?, to))
    }
}

/// The tunnel's socket, with the relay in front of it for the peers that need it.
pub(crate) struct Socket {
    pub inner: Arc<dyn AsyncUdpSocket>,
    pub relay: Arc<Relay>,
    /// Packets taken out of one relay delivery, waiting to be handed on.
    pub waiting: Mutex<VecDeque<(SocketAddr, Vec<u8>)>>,
}
impl fmt::Debug for Socket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("tunnel socket")
    }
}
impl AsyncUdpSocket for Socket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        self.inner.clone().create_io_poller()
    }
    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        let Some((turn, to)) = self.relay.through(transmit.destination) else { return self.inner.try_send(transmit) };
        let size = transmit.segment_size.unwrap_or(transmit.contents.len()).max(1);
        for part in transmit.contents.chunks(size) {
            let wrapped = turn.wrap(to, part);
            self.inner.try_send(&Transmit { destination: turn.server, ecn: None, contents: &wrapped, segment_size: None, src_ip: None })?;
        }
        Ok(())
    }
    fn poll_recv(&self, cx: &mut Context, bufs: &mut [IoSliceMut<'_>], meta: &mut [RecvMeta]) -> Poll<io::Result<usize>> {
        loop {
            if let Some((from, data)) = self.waiting.lock().unwrap().pop_front() {
                let length = data.len().min(bufs[0].len());
                bufs[0][..length].copy_from_slice(&data[..length]);
                let mut one = RecvMeta::default();
                one.addr = from;
                one.len = length;
                one.stride = length;
                meta[0] = one;
                return Poll::Ready(Ok(1));
            }
            let turn = self.relay.turn();
            let asked = self.relay.asked.lock().unwrap().as_ref().map(|a| a.0);
            if turn.is_none() && asked.is_none() {
                return self.inner.poll_recv(cx, bufs, meta);
            }
            // One delivery at a time, so one from the relay can be taken apart.
            let count = ready!(self.inner.poll_recv(cx, &mut bufs[..1], &mut meta[..1]))?;
            if count != 0 && asked == Some(meta[0].addr) {
                if let Some((_, id, found)) = self.relay.asked.lock().unwrap().as_ref() {
                    if let Some(address) = seen_address(&bufs[0][..meta[0].len], id) {
                        let _ = found.send(address);
                    }
                }
                continue;
            }
            let Some(turn) = turn.filter(|turn| count != 0 && meta[0].addr == turn.server) else { return Poll::Ready(Ok(count)) };
            let mut waiting = self.waiting.lock().unwrap();
            for part in bufs[0][..meta[0].len].chunks(meta[0].stride.max(1)) {
                if let Some((peer, data)) = turn.on_packet(part) {
                    waiting.push_back((SocketAddr::V4(peer), data));
                }
            }
        }
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
    fn max_transmit_segments(&self) -> usize {
        self.inner.max_transmit_segments()
    }
    fn max_receive_segments(&self) -> usize {
        self.inner.max_receive_segments()
    }
    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }
}
