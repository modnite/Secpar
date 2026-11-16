//! The client end of the managed engine: the tunnel to the host's engine and the
//! pairing that lets this device in. `docs/gamestream-engine.md` is the contract.
//!
//! `gamestream.info` and `gamestream.pair` travel on the authenticated control
//! connection, so nothing here opens an account, asks for a PIN or trusts a LAN
//! address. The stream component then talks to the tunnel's local end and never
//! sees the host's network.
use crate::control::{GameStreamInfo, GameStreamPairing};
use secpar_tunnel::{Identity, Ports};
use std::{
    net::{IpAddr, SocketAddr, UdpSocket},
    time::Duration,
};

/// Where the stream component finds the host's engine on this computer.
pub const LOCAL_IP: &str = "127.0.0.2";
/// The port the component uses; the tunnel carries the engine's own ports.
pub const ENGINE_PORT: u16 = 46989;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// This device's certificate towards the host's tunnel. It is kept per computer,
/// so the grant the host recorded still matches on the next stream.
pub type TunnelIdentity = Identity;

/// The host's tunnel endpoint, taken from its `gamestream.info` answer.
pub struct Endpoint {
    pub addresses: Vec<SocketAddr>,
    pub pin: [u8; 32],
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if !crate::control::is_fingerprint(text) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}
pub(crate) fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The address, its certificate pin and its port. Addresses are tried in the
/// order the host gave them, LAN first. A host that lists no address is reached
/// through the address the control connection already used.
pub fn endpoint(info: &GameStreamInfo, control_host: Option<&str>) -> Result<Endpoint, String> {
    let port = info.tunnel_port.ok_or("This computer does not offer a stream tunnel")?;
    let pin = info
        .tunnel_certificate_sha256
        .as_deref()
        .and_then(unhex)
        .ok_or("This computer's stream tunnel certificate is invalid")?;
    let mut addresses: Vec<SocketAddr> = info
        .tunnel_addresses
        .iter()
        .filter_map(|address| address.parse::<IpAddr>().ok())
        .map(|address| SocketAddr::new(address, port))
        .take(8)
        .collect();
    if addresses.is_empty() {
        let host = control_host.and_then(|host| host.parse::<IpAddr>().ok());
        let host = host.ok_or("This computer did not give an address for its stream tunnel")?;
        addresses.push(SocketAddr::new(host, port));
    }
    Ok(Endpoint { addresses, pin })
}

/// The tunnel socket's address on the route to the host, as the host needs it to
/// punch towards this device.
fn route_candidate(socket: &UdpSocket, toward: Option<IpAddr>) -> Option<String> {
    let remote = toward.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, 1)));
    let route = UdpSocket::bind("0.0.0.0:0").ok()?;
    route.connect(SocketAddr::new(remote, 53)).ok()?;
    let (local, mine) = (route.local_addr().ok()?, socket.local_addr().ok()?);
    Some(SocketAddr::new(local.ip(), mine.port()).to_string())
}

/// Where the host can reach this device's tunnel socket: the address on its route
/// and, when STUN answers, the address the internet sees. Both are sent in
/// `gamestream.pair` so the host can open NATs towards either.
pub fn candidates(socket: &UdpSocket, toward: Option<IpAddr>) -> Vec<String> {
    let mut candidates: Vec<String> = route_candidate(socket, toward).into_iter().collect();
    if let Some(public) = secpar_network::stun::public_addr(socket) {
        let public = public.to_string();
        if !candidates.contains(&public) {
            candidates.push(public);
        }
    }
    candidates.truncate(8);
    candidates
}

/// The `gamestream.pair` body for this component and this device's tunnel.
pub fn pairing(identity: &Identity, component: (&str, &str), pin: &str, candidates: &[String]) -> GameStreamPairing {
    GameStreamPairing {
        client_id: component.0.to_string(),
        certificate_sha256: component.1.to_string(),
        pin: pin.to_string(),
        tunnel_certificate_sha256: hex(&identity.fingerprint()),
        tunnel_candidates: candidates.to_vec(),
    }
}

/// A tunnel socket that is open on the network and is waiting for the host to let
/// this device in. Holding it keeps the same UDP port for the punch and the
/// connection.
pub struct Prepared {
    socket: UdpSocket,
    identity: Identity,
    endpoint: Endpoint,
    /// This device's UDP endpoints, for `gamestream.pair`.
    pub candidates: Vec<String>,
}

/// Bind the socket that will carry the tunnel and find the addresses the host can
/// punch towards. Blocking work stays off the async runtime.
pub async fn prepare(info: &GameStreamInfo, control_host: Option<&str>, identity: Identity) -> Result<Prepared, String> {
    let endpoint = endpoint(info, control_host)?;
    let toward = endpoint.addresses.first().map(SocketAddr::ip);
    let (socket, candidates) = tokio::task::spawn_blocking(move || {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|_| "Could not open a tunnel socket".to_string())?;
        let candidates = candidates(&socket, toward);
        Ok::<_, String>((socket, candidates))
    })
    .await
    .map_err(|_| "Could not open a tunnel socket")??;
    Ok(Prepared { socket, identity, endpoint, candidates })
}
impl Prepared {
    /// What to send in `gamestream.pair` for this socket and component.
    pub fn pairing(&self, component: (&str, &str), pin: &str) -> GameStreamPairing {
        pairing(&self.identity, component, pin, &self.candidates)
    }
}

/// A tunnel that is open and pinned to the host's engine.
pub struct Tunnel {
    client: secpar_tunnel::Client,
}
impl Tunnel {
    /// What the stream component connects to on this computer.
    pub fn local_address(&self) -> String {
        format!("{LOCAL_IP}:{ENGINE_PORT}")
    }
    /// Whether the tunnel is gone, without waiting.
    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }
    /// Resolves when the tunnel is gone.
    pub async fn closed(&self) {
        self.client.closed().await
    }
}

/// Open the tunnel on a prepared socket. The host must already have recorded this
/// device's certificate, which is what `gamestream.pair` does.
pub async fn connect(prepared: Prepared) -> Result<Tunnel, String> {
    let Prepared { socket, identity, endpoint, .. } = prepared;
    let local: IpAddr = LOCAL_IP.parse().map_err(|_| "Invalid local address")?;
    let connect = secpar_tunnel::connect_on(
        socket,
        &endpoint.addresses,
        &identity,
        endpoint.pin,
        local,
        Ports::engine(ENGINE_PORT),
    );
    let client = tokio::time::timeout(CONNECT_TIMEOUT, connect)
        .await
        .map_err(|_| "The stream tunnel timed out".to_string())?
        .map_err(|error| error.to_string())?;
    Ok(Tunnel { client })
}

/// The whole sequence for a host that already has a pending window of its own:
/// prepare the socket, ask the host to let this device in, then open the tunnel.

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> GameStreamInfo {
        GameStreamInfo {
            http_port: 46989,
            https_port: 46984,
            certificate_sha256: "a".repeat(64),
            tunnel_port: Some(51234),
            tunnel_certificate_sha256: Some("b".repeat(64)),
            tunnel_addresses: vec!["192.168.1.20".into(), "203.0.113.7".into()],
        }
    }

    #[test]
    fn the_tunnel_endpoint_uses_every_address_with_one_port() {
        let endpoint = endpoint(&info(), None).unwrap();
        assert_eq!(endpoint.pin, [0xbb; 32]);
        assert_eq!(
            endpoint.addresses,
            ["192.168.1.20:51234".parse::<SocketAddr>().unwrap(), "203.0.113.7:51234".parse().unwrap()]
        );
    }

    #[test]
    fn a_host_without_a_tunnel_or_addresses_is_refused_plainly() {
        let mut missing_tunnel = info();
        missing_tunnel.tunnel_port = None;
        assert_eq!(endpoint(&missing_tunnel, None).err().unwrap(), "This computer does not offer a stream tunnel");
        let mut bad_pin = info();
        bad_pin.tunnel_certificate_sha256 = Some("ABC".into());
        assert_eq!(endpoint(&bad_pin, None).err().unwrap(), "This computer's stream tunnel certificate is invalid");
        let mut no_addresses = info();
        no_addresses.tunnel_addresses.clear();
        assert_eq!(
            endpoint(&no_addresses, None).err().unwrap(),
            "This computer did not give an address for its stream tunnel"
        );
        // The address the control connection used is the fallback.
        assert_eq!(endpoint(&no_addresses, Some("192.168.0.126")).unwrap().addresses, ["192.168.0.126:51234".parse::<SocketAddr>().unwrap()]);
        assert!(endpoint(&no_addresses, Some("not an address")).is_err());
    }

    #[test]
    fn the_route_candidate_is_this_computers_address_on_the_hosts_network() {
        let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        let candidate = route_candidate(&socket, Some("127.0.0.1".parse().unwrap())).unwrap();
        assert_eq!(candidate, format!("127.0.0.1:{port}"));
        assert_eq!(candidate.parse::<SocketAddr>().unwrap().port(), port, "the same socket the tunnel uses");
    }

    #[test]
    fn fingerprints_round_trip_as_lowercase_hex() {
        assert_eq!(unhex(&hex(&[0xab; 32])), Some([0xab; 32]));
        assert_eq!(unhex(&"A".repeat(64)), None);
        assert_eq!(unhex(""), None);
    }

    #[test]
    fn the_pairing_carries_the_component_this_device_and_its_endpoints() {
        let identity = Identity::generate().unwrap();
        let body = pairing(&identity, ("ABCDEF", &"c".repeat(64)), "4321", &["192.168.1.30:54321".into()]);
        assert_eq!(body.client_id, "ABCDEF");
        assert_eq!(body.certificate_sha256, "c".repeat(64));
        assert_eq!(body.pin, "4321");
        assert_eq!(body.tunnel_certificate_sha256, hex(&identity.fingerprint()));
        assert_eq!(body.tunnel_candidates, ["192.168.1.30:54321"]);
    }

    #[tokio::test]
    async fn preparing_without_a_tunnel_never_opens_a_socket() {
        let mut info = info();
        info.tunnel_port = None;
        let error = prepare(&info, None, Identity::generate().unwrap()).await.err().unwrap();
        assert_eq!(error, "This computer does not offer a stream tunnel");
    }
}
