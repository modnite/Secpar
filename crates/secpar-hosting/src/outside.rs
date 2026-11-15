//! Being reachable from outside the home network.
//!
//! Two small steps, both optional: ask a public server which address this
//! network has on the internet, and ask the router to pass the tunnel's port
//! through to this computer. If the router does not do that by itself, the same
//! port can be forwarded by hand. Nothing here opens the engine: what comes in is
//! only the tunnel, which checks every device against the account's list.
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

/// Answers "which address did this packet come from" and nothing else.
pub const STUN: &str = "stun.cloudflare.com:3478";

/// The address this network has on the internet, as a public server sees it.
pub async fn public_address() -> Option<IpAddr> {
    let server = tokio::net::lookup_host(STUN).await.ok()?.find(SocketAddr::is_ipv4)?;
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.ok()?;
    // A binding request: type 1, no attributes, the fixed cookie, a transaction id.
    let mut request = [0u8; 20];
    request[1] = 1;
    request[4..8].copy_from_slice(&0x2112_A442u32.to_be_bytes());
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos().to_be_bytes();
    request[8..20].copy_from_slice(&id[4..16]);
    socket.send_to(&request, server).await.ok()?;
    let mut answer = [0u8; 512];
    let (length, from) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut answer)).await.ok()?.ok()?;
    if from != server || length < 20 || answer[8..20] != request[8..20] {
        return None;
    }
    // Walk the attributes for XOR-MAPPED-ADDRESS (0x0020), IPv4.
    let mut at = 20;
    while at + 4 <= length {
        let kind = u16::from_be_bytes([answer[at], answer[at + 1]]);
        let size = u16::from_be_bytes([answer[at + 2], answer[at + 3]]) as usize;
        let value = at + 4;
        if kind == 0x0020 && size >= 8 && value + 8 <= length && answer[value + 1] == 1 {
            let mut ip = [0u8; 4];
            for (i, byte) in ip.iter_mut().enumerate() {
                *byte = answer[value + 4 + i] ^ request[4 + i];
            }
            let ip = Ipv4Addr::from(ip);
            // An address that only exists inside a network is no use from outside.
            return (!ip.is_private() && !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()).then_some(IpAddr::V4(ip));
        }
        at = value + size.div_ceil(4) * 4;
    }
    None
}

/// Asks the router to pass UDP `port` through to this computer for an hour.
/// Returns whether it agreed. Call again before the hour is over.
pub async fn open_on_router(local: Ipv4Addr, port: u16) -> bool {
    use igd_next::{aio::tokio::search_gateway, PortMappingProtocol, SearchOptions};
    let mut options = SearchOptions::default();
    options.timeout = Some(Duration::from_secs(3));
    let Ok(gateway) = search_gateway(options).await else { return false };
    let to = SocketAddr::new(IpAddr::V4(local), port);
    gateway.add_port(PortMappingProtocol::UDP, port, to, 3600, "Secpar").await.is_ok()
}
