//! Carries the streaming engines' TCP and UDP traffic between two Secpar devices
//! over one QUIC connection. Both ends prove a device identity: the connecting
//! side pins the host's certificate, the host accepts only certificates on its
//! allow-list. The engines stay on loopback and are never open on the network.
//!
//! Host side: `serve` accepts connections and forwards to the engine's ports.
//! Connecting side: `connect` listens on a loopback address with the same port
//! numbers, so the stream component talks to it as if it were the host.
//!
//! Wire format: a TCP connection is one QUIC stream that starts with the port as
//! two bytes; a UDP packet is one QUIC datagram that starts with the port as two
//! bytes. Stream port 0 is a readiness check that the host answers with one byte.
mod tls;

use bytes::{BufMut, Bytes, BytesMut};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream, UdpSocket},
};

pub type Fingerprint = [u8; 32];
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}

pub fn fingerprint(certificate_der: &[u8]) -> Fingerprint {
    ring::digest::digest(&ring::digest::SHA256, certificate_der)
        .as_ref()
        .try_into()
        .expect("SHA-256 is 32 bytes")
}

/// A device's certificate and private key, both DER.
#[derive(Clone)]
pub struct Identity {
    certificate: Vec<u8>,
    key: Vec<u8>,
}
impl Identity {
    pub fn generate() -> Result<Self> {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["secpar".into()])
                .map_err(|_| Error("Could not create a device identity"))?;
        Ok(Self {
            certificate: cert.der().to_vec(),
            key: signing_key.serialize_der(),
        })
    }
    pub fn from_der(certificate: Vec<u8>, key: Vec<u8>) -> Self {
        Self { certificate, key }
    }
    pub fn certificate_der(&self) -> &[u8] {
        &self.certificate
    }
    pub fn key_der(&self) -> &[u8] {
        &self.key
    }
    pub fn fingerprint(&self) -> Fingerprint {
        fingerprint(&self.certificate)
    }
}

/// The certificates a host accepts. Shared, so access can change while it runs.
#[derive(Debug, Default)]
pub struct Allowed {
    devices: RwLock<HashSet<Fingerprint>>,
    refused: tokio::sync::Notify,
}
impl Allowed {
    pub fn new(devices: impl IntoIterator<Item = Fingerprint>) -> Arc<Self> {
        Arc::new(Self { devices: RwLock::new(devices.into_iter().collect()), refused: tokio::sync::Notify::new() })
    }
    pub fn set(&self, devices: impl IntoIterator<Item = Fingerprint>) {
        *self.devices.write().unwrap() = devices.into_iter().collect();
    }
    pub fn contains(&self, device: &Fingerprint) -> bool {
        self.devices.read().unwrap().contains(device)
    }
    /// Called when a device that is not allowed tried to connect.
    pub(crate) fn turned_away(&self) {
        self.refused.notify_one();
    }
}

/// The engine ports carried by the tunnel.
#[derive(Clone, Debug)]
pub struct Ports {
    pub tcp: Vec<u16>,
    pub udp: Vec<u16>,
}
impl Ports {
    /// The ports Sunshine derives from its base port: HTTPS, HTTP and RTSP over
    /// TCP; video, control and audio over UDP.
    pub fn engine(base: u16) -> Self {
        Self {
            tcp: vec![base - 5, base, base + 21],
            udp: vec![base + 9, base + 10, base + 11],
        }
    }
}

fn transport() -> Arc<quinn::TransportConfig> {
    let mut t = quinn::TransportConfig::default();
    t.max_idle_timeout(Some(Duration::from_secs(20).try_into().expect("valid timeout")));
    t.keep_alive_interval(Some(Duration::from_secs(4)));
    t.datagram_receive_buffer_size(Some(8 << 20));
    t.datagram_send_buffer_size(8 << 20);
    // Video packets are close to a full network packet. Start near the usual
    // LAN size; QUIC falls back by itself on paths that cannot carry it.
    t.initial_mtu(1400);
    Arc::new(t)
}

fn datagram(port: u16, payload: &[u8]) -> Bytes {
    let mut b = BytesMut::with_capacity(payload.len() + 2);
    b.put_u16(port);
    b.put_slice(payload);
    b.freeze()
}

async fn splice(mut send: quinn::SendStream, mut recv: quinn::RecvStream, tcp: TcpStream) {
    let _ = tcp.set_nodelay(true);
    let (mut from_tcp, mut to_tcp) = tcp.into_split();
    let up = async {
        let _ = tokio::io::copy(&mut from_tcp, &mut send).await;
        let _ = send.finish();
    };
    let down = async {
        let _ = tokio::io::copy(&mut recv, &mut to_tcp).await;
        let _ = to_tcp.shutdown().await;
    };
    tokio::join!(up, down);
}

type Connections = Arc<Mutex<Vec<(Fingerprint, quinn::Connection)>>>;

fn peer_fingerprint(connection: &quinn::Connection) -> Option<Fingerprint> {
    let certificates = connection
        .peer_identity()?
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .ok()?;
    Some(fingerprint(certificates.first()?.as_ref()))
}

/// Answers a request a connected device sends to the host itself, not to the
/// engine. It is given the device's certificate fingerprint, which the tunnel
/// has already checked, and the request; what it returns is sent back.
pub type Handler = Arc<
    dyn Fn(Fingerprint, Vec<u8>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<u8>> + Send>>
        + Send
        + Sync,
>;
type SharedHandler = Arc<std::sync::RwLock<Option<Handler>>>;
/// The largest request or answer on the host's own channel.
pub const MAX_REQUEST: usize = 8192;

/// Stream "port" for requests to the host itself. Port 0 is the readiness check;
/// neither is ever an engine port.
const REQUEST: u16 = 1;

/// The host end. Dropping it stops accepting and closes every connection.
pub struct Host {
    endpoint: quinn::Endpoint,
    puncher: std::net::UdpSocket,
    allowed: Arc<Allowed>,
    connections: Connections,
    handler: SharedHandler,
}
impl Host {
    /// Sets who answers requests to the host itself. Without one they get no answer.
    pub fn on_request(&self, handler: Handler) {
        *self.handler.write().unwrap() = Some(handler);
    }
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.endpoint
            .local_addr()
            .map_err(|_| Error("Tunnel has no local address"))
    }
    /// Send a small packet to each address from the tunnel's own port, so a
    /// router in front of this host lets that device's packets in.
    pub fn punch(&self, candidates: &[SocketAddr]) {
        for candidate in candidates {
            let _ = self.puncher.send_to(&[0], candidate);
        }
    }
    /// Replace the allow-list and close the tunnel of every device no longer on it.
    pub fn set_allowed(&self, devices: impl IntoIterator<Item = Fingerprint>) {
        self.allowed.set(devices);
        self.connections.lock().unwrap().retain(|(device, connection)| {
            let keep = self.allowed.contains(device);
            if !keep {
                connection.close(1u32.into(), b"access removed");
            }
            keep
        });
    }
    /// Resolves the next time a device that is not allowed tries to connect. The
    /// host can take that as a hint that its list may be out of date.
    pub async fn turned_away(&self) {
        self.allowed.refused.notified().await;
    }
    /// Devices with an open tunnel.
    pub fn connected(&self) -> Vec<Fingerprint> {
        self.connections.lock().unwrap().iter().map(|(device, _)| *device).collect()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.endpoint.close(0u32.into(), b"");
    }
}

/// Accept allowed devices on `bind` and forward their traffic to `engine`.
pub async fn serve(
    bind: SocketAddr,
    identity: &Identity,
    allowed: Arc<Allowed>,
    engine: IpAddr,
    ports: Ports,
) -> Result<Host> {
    let socket =
        std::net::UdpSocket::bind(bind).map_err(|_| Error("Could not open the tunnel port"))?;
    serve_on(socket, identity, allowed, engine, ports)
}

/// Like `serve`, on a socket the caller already owns, for example one whose
/// public address was discovered for a direct connection. Call inside the runtime.
pub fn serve_on(
    socket: std::net::UdpSocket,
    identity: &Identity,
    allowed: Arc<Allowed>,
    engine: IpAddr,
    ports: Ports,
) -> Result<Host> {
    let mut config =
        quinn::ServerConfig::with_crypto(Arc::new(tls::server(identity, allowed.clone())?));
    config.transport_config(transport());
    let opened = |_| Error("Could not open the tunnel port");
    socket.set_nonblocking(true).map_err(opened)?;
    let puncher = socket.try_clone().map_err(opened)?;
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(config),
        socket,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(opened)?;
    let connections = Connections::default();
    let handler = SharedHandler::default();
    let (accepting, open, handling) = (endpoint.clone(), connections.clone(), handler.clone());
    tokio::spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            let (ports, open, handling) = (ports.clone(), open.clone(), handling.clone());
            tokio::spawn(async move {
                let Ok(connection) = incoming.await else { return };
                let Some(device) = peer_fingerprint(&connection) else { return };
                let id = connection.stable_id();
                open.lock().unwrap().push((device, connection.clone()));
                host_connection(connection, device, handling, engine, ports).await;
                open.lock().unwrap().retain(|(_, c)| c.stable_id() != id);
            });
        }
    });
    Ok(Host {
        endpoint,
        puncher,
        allowed,
        connections,
        handler,
    })
}

async fn host_connection(connection: quinn::Connection, device: Fingerprint, handler: SharedHandler, engine: IpAddr, ports: Ports) {
    let streams = {
        let connection = connection.clone();
        let tcp = ports.tcp.clone();
        async move {
            while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                let (tcp, handler) = (tcp.clone(), handler.clone());
                tokio::spawn(async move {
                    let mut port = [0u8; 2];
                    if recv.read_exact(&mut port).await.is_err() {
                        return;
                    }
                    let port = u16::from_be_bytes(port);
                    if port == 0 {
                        let _ = send.write_all(&[1]).await;
                        let _ = send.finish();
                        return;
                    }
                    if port == REQUEST {
                        let Ok(request) = recv.read_to_end(MAX_REQUEST).await else { return };
                        let Some(handler) = handler.read().unwrap().clone() else { return };
                        let answer = handler(device, request).await;
                        if answer.len() <= MAX_REQUEST {
                            let _ = send.write_all(&answer).await;
                            let _ = send.finish();
                        }
                        return;
                    }
                    if !tcp.contains(&port) {
                        return;
                    }
                    if let Ok(stream) = TcpStream::connect((engine, port)).await {
                        splice(send, recv, stream).await;
                    }
                });
            }
        }
    };
    let datagrams = {
        let connection = connection.clone();
        async move {
            let mut sockets: HashMap<u16, Arc<UdpSocket>> = HashMap::new();
            while let Ok(packet) = connection.read_datagram().await {
                if packet.len() < 2 {
                    continue;
                }
                let port = u16::from_be_bytes([packet[0], packet[1]]);
                if !ports.udp.contains(&port) {
                    continue;
                }
                let socket = match sockets.get(&port) {
                    Some(socket) => socket.clone(),
                    None => {
                        let local: SocketAddr = if engine.is_ipv4() {
                            "0.0.0.0:0".parse().unwrap()
                        } else {
                            "[::]:0".parse().unwrap()
                        };
                        let Ok(socket) = UdpSocket::bind(local).await else {
                            continue;
                        };
                        if socket.connect((engine, port)).await.is_err() {
                            continue;
                        }
                        let socket = Arc::new(socket);
                        sockets.insert(port, socket.clone());
                        // Replies from the engine go back as datagrams.
                        let (reply, back) = (socket.clone(), connection.clone());
                        tokio::spawn(async move {
                            let mut buffer = vec![0u8; 65536];
                            loop {
                                tokio::select! {
                                    _ = back.closed() => break,
                                    got = reply.recv(&mut buffer) => match got {
                                        Ok(n) => {
                                            if let Err(quinn::SendDatagramError::ConnectionLost(_)) =
                                                back.send_datagram(datagram(port, &buffer[..n]))
                                            {
                                                break;
                                            }
                                        }
                                        // A refused send shows up here once; keep going.
                                        Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
                                    },
                                }
                            }
                        });
                        socket
                    }
                };
                let _ = socket.send(&packet[2..]).await;
            }
        }
    };
    tokio::join!(streams, datagrams);
}

/// The connecting end. Dropping it closes the tunnel and its local ports.
pub struct Client {
    connection: quinn::Connection,
    endpoint: quinn::Endpoint,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Client {
    /// Asks the host itself something, outside the engine's traffic, and waits
    /// for its answer. The host knows which device is asking from the tunnel.
    pub async fn request(&self, request: &[u8]) -> Result<Vec<u8>> {
        const FAILED: Error = Error("The host did not answer");
        if request.len() > MAX_REQUEST {
            return Err(Error("The request is too large"));
        }
        let exchange = async {
            let (mut send, mut recv) = self.connection.open_bi().await.ok()?;
            send.write_all(&REQUEST.to_be_bytes()).await.ok()?;
            send.write_all(request).await.ok()?;
            send.finish().ok()?;
            recv.read_to_end(MAX_REQUEST).await.ok()
        };
        match tokio::time::timeout(Duration::from_secs(30), exchange).await {
            Ok(Some(answer)) if !answer.is_empty() => Ok(answer),
            _ => Err(FAILED),
        }
    }
    /// Whether the tunnel is gone, without waiting.
    pub fn is_closed(&self) -> bool {
        self.connection.close_reason().is_some()
    }
    /// Resolves when the tunnel is gone.
    pub async fn closed(&self) {
        self.connection.closed().await;
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        self.connection.close(0u32.into(), b"");
        self.endpoint.close(0u32.into(), b"");
    }
}

/// Connect to a host's tunnel at `host`, which must present the certificate
/// `host_pin`, and offer its engine ports on the loopback address `listen`.
pub async fn connect(
    host: SocketAddr,
    identity: &Identity,
    host_pin: Fingerprint,
    listen: IpAddr,
    ports: Ports,
) -> Result<Client> {
    let local: SocketAddr = if host.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };
    let socket =
        std::net::UdpSocket::bind(local).map_err(|_| Error("Could not open a tunnel socket"))?;
    connect_on(socket, &[host], identity, host_pin, listen, ports).await
}

/// Like `connect`, on a socket the caller already owns, trying several addresses
/// for the same host. Addresses are tried in order a quarter of a second apart
/// (put the LAN address first) and the first one that answers as the expected
/// host and accepts this device is used.
pub async fn connect_on(
    socket: std::net::UdpSocket,
    candidates: &[SocketAddr],
    identity: &Identity,
    host_pin: Fingerprint,
    listen: IpAddr,
    ports: Ports,
) -> Result<Client> {
    let opened = |_| Error("Could not open a tunnel socket");
    socket.set_nonblocking(true).map_err(opened)?;
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        None,
        socket,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(opened)?;
    let mut config = quinn::ClientConfig::new(Arc::new(tls::client(identity, host_pin)?));
    config.transport_config(transport());

    let mut attempts = tokio::task::JoinSet::new();
    for (order, &candidate) in candidates.iter().enumerate() {
        let (endpoint, config) = (endpoint.clone(), config.clone());
        attempts.spawn(async move {
            tokio::time::sleep(Duration::from_millis(250 * order as u64)).await;
            let attempt = async {
                let connection = endpoint.connect_with(config, candidate, "secpar").ok()?.await.ok()?;
                // The host only learns whether it accepts this device after the
                // handshake, so ask before calling the tunnel usable.
                let (mut send, mut recv) = connection.open_bi().await.ok()?;
                send.write_all(&[0, 0]).await.ok()?;
                send.finish().ok()?;
                let mut answer = [0u8; 1];
                recv.read_exact(&mut answer).await.ok()?;
                Some(connection)
            };
            tokio::time::timeout(Duration::from_secs(8), attempt).await.ok().flatten()
        });
    }
    let mut connection = None;
    while let Some(finished) = attempts.join_next().await {
        if let Ok(Some(won)) = finished {
            connection = Some(won);
            break;
        }
    }
    drop(attempts);
    let connection = connection
        .ok_or(Error("Could not reach the host, or it did not accept this device"))?;

    let mut tasks = Vec::new();
    for port in ports.tcp {
        let listener = TcpListener::bind((listen, port))
            .await
            .map_err(|_| Error("A local tunnel port is already in use"))?;
        let connection = connection.clone();
        tasks.push(tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let connection = connection.clone();
                tokio::spawn(async move {
                    if let Ok((mut send, recv)) = connection.open_bi().await {
                        if send.write_all(&port.to_be_bytes()).await.is_ok() {
                            splice(send, recv, stream).await;
                        }
                    }
                });
            }
        }));
    }
    // UDP: remember where the local program sends from, so replies return there.
    let mut sockets: HashMap<u16, (Arc<UdpSocket>, Arc<Mutex<Option<SocketAddr>>>)> = HashMap::new();
    for port in ports.udp {
        let socket = Arc::new(
            UdpSocket::bind((listen, port))
                .await
                .map_err(|_| Error("A local tunnel port is already in use"))?,
        );
        let peer = Arc::new(Mutex::new(None));
        sockets.insert(port, (socket.clone(), peer.clone()));
        let connection = connection.clone();
        tasks.push(tokio::spawn(async move {
            let mut buffer = vec![0u8; 65536];
            loop {
                match socket.recv_from(&mut buffer).await {
                    Ok((n, from)) => {
                        *peer.lock().unwrap() = Some(from);
                        if let Err(quinn::SendDatagramError::ConnectionLost(_)) =
                            connection.send_datagram(datagram(port, &buffer[..n]))
                        {
                            break;
                        }
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
                }
            }
        }));
    }
    {
        let connection = connection.clone();
        tasks.push(tokio::spawn(async move {
            while let Ok(packet) = connection.read_datagram().await {
                if packet.len() < 2 {
                    continue;
                }
                let port = u16::from_be_bytes([packet[0], packet[1]]);
                if let Some((socket, peer)) = sockets.get(&port) {
                    let to = *peer.lock().unwrap();
                    if let Some(to) = to {
                        let _ = socket.send_to(&packet[2..], to).await;
                    }
                }
            }
        }));
    }
    Ok(Client {
        connection,
        endpoint,
        tasks,
    })
}

#[cfg(test)]
mod tests;
