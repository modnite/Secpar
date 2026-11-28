//! NAT-PMP (RFC 6886) port mapping, for routers without UPnP or with it switched off. A mapping
//! lets the router forward any packet for the stream port to us, so a peer's first punch gets in
//! even before ours has gone out; plain simultaneous punching fails through some home routers.
//! Blocking and std only; the router is the default gateway, asked on UDP 5351.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

const PORT: u16 = 5351;
/// Retries at 0, 250 and 750 ms: a router that speaks NAT-PMP answers in a few ms.
const TRIES: [u64; 3] = [0, 250, 750];
const GIVE_UP: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Udp = 1,
    Tcp = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Router {
    pub gateway: Ipv4Addr,
    /// 5351; tests point it at a fake router.
    port: u16,
}

impl Router {
    /// The default gateway, if it answers NAT-PMP (an external address request).
    pub fn discover() -> Option<Router> {
        let r = Router { gateway: default_gateway()?, port: PORT };
        r.external_ip().map(|_| r)
    }

    /// The router's public address.
    pub fn external_ip(&self) -> Option<Ipv4Addr> {
        let reply = self.ask(&[0, 0], 128, 12)?;
        Some(Ipv4Addr::new(reply[8], reply[9], reply[10], reply[11]))
    }

    /// Forwards `external` on the router to `internal` on this PC for `lifetime` seconds (0 removes
    /// the mapping). Returns the external port the router gave, which may differ.
    pub fn map(&self, proto: Proto, internal: u16, external: u16, lifetime: u32) -> Result<u16, String> {
        let mut req = vec![0, proto as u8, 0, 0];
        req.extend_from_slice(&internal.to_be_bytes());
        req.extend_from_slice(&external.to_be_bytes());
        req.extend_from_slice(&lifetime.to_be_bytes());
        let reply = self.ask(&req, 128 + proto as u8, 16).ok_or("no NAT-PMP answer")?;
        Ok(u16::from_be_bytes([reply[10], reply[11]]))
    }

    /// This PC's address on the router's network.
    pub fn local_ip(&self) -> Option<Ipv4Addr> {
        let s = UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect(SocketAddrV4::new(self.gateway, self.port)).ok()?;
        match s.local_addr().ok()?.ip() {
            std::net::IpAddr::V4(ip) => Some(ip),
            _ => None,
        }
    }

    /// Sends `req` and waits for a successful answer with opcode `op` of at least `len` bytes.
    fn ask(&self, req: &[u8], op: u8, len: usize) -> Option<Vec<u8>> {
        let s = UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect(SocketAddrV4::new(self.gateway, self.port)).ok()?;
        let t0 = Instant::now();
        let mut buf = [0u8; 64];
        for (i, at) in TRIES.iter().enumerate() {
            let next = TRIES.get(i + 1).map_or(GIVE_UP, |n| Duration::from_millis(*n));
            if let Some(w) = (t0 + Duration::from_millis(*at)).checked_duration_since(Instant::now()) {
                std::thread::sleep(w);
            }
            let _ = s.send(req);
            while let Some(left) = (t0 + next).checked_duration_since(Instant::now()) {
                s.set_read_timeout(Some(left.max(Duration::from_millis(1)))).ok()?;
                let Ok(n) = s.recv(&mut buf) else { break };
                if let Some(reply) = parse(&buf[..n], op, len) {
                    return Some(reply.to_vec());
                }
            }
        }
        None
    }
}

/// An answer: version 0, the expected opcode, result 0 (success), long enough.
fn parse(b: &[u8], op: u8, len: usize) -> Option<&[u8]> {
    (b.len() >= len && b[0] == 0 && b[1] == op && b[2] == 0 && b[3] == 0).then_some(b)
}

#[cfg(windows)]
fn default_gateway() -> Option<Ipv4Addr> {
    // MIB_IPFORWARDROW: fourteen DWORDs; the next hop is the fourth.
    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetBestRoute(dest: u32, source: u32, row: *mut [u32; 14]) -> u32;
    }
    let mut row = [0u32; 14];
    // SAFETY: `row` is the size of MIB_IPFORWARDROW; 8.8.8.8 picks the internet route.
    let dest = u32::from_ne_bytes([8, 8, 8, 8]);
    (unsafe { GetBestRoute(dest, 0, &mut row) } == 0).then(|| Ipv4Addr::from(row[3].to_ne_bytes())).filter(|g| g.is_private())
}

#[cfg(target_os = "linux")]
fn default_gateway() -> Option<Ipv4Addr> {
    gateway_from_route_table(&std::fs::read_to_string("/proc/net/route").ok()?)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn default_gateway() -> Option<Ipv4Addr> {
    None
}

/// The default route's gateway in `/proc/net/route` (hex, little-endian).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn gateway_from_route_table(table: &str) -> Option<Ipv4Addr> {
    table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 3 || f[1] != "00000000" {
            return None;
        }
        let g = u32::from_str_radix(f[2], 16).ok()?;
        Some(Ipv4Addr::from(g.to_le_bytes())).filter(|g| !g.is_unspecified())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_default_route() {
        let t = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\n\
                 wlan0\t0064A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\n\
                 wlan0\t00000000\t0164A8C0\t0003\t0\t0\t600\t00000000\n";
        assert_eq!(gateway_from_route_table(t), Some(Ipv4Addr::new(192, 168, 100, 1)));
        assert_eq!(gateway_from_route_table("Iface\n"), None);
    }

    /// Network: this PC's router, read-only (asks for the public address, maps nothing).
    #[test]
    #[ignore]
    fn this_router() {
        eprintln!("gateway {:?}, NAT-PMP {:?}", default_gateway(), Router::discover().and_then(|r| r.external_ip()).map(|_| "answers"));
    }

    /// A fake router on loopback: the mapping and external address requests round-trip.
    #[test]
    fn maps_through_a_fake_router() {
        let fake = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = fake.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut b = [0u8; 64];
            for _ in 0..2 {
                let (n, from) = fake.recv_from(&mut b).unwrap();
                let reply = match b[1] {
                    0 => vec![0, 128, 0, 0, 0, 0, 0, 1, 203, 0, 113, 7],
                    op => {
                        let mut r = vec![0, 128 + op, 0, 0, 0, 0, 0, 1];
                        r.extend_from_slice(&b[4..6]);
                        r.extend_from_slice(&40000u16.to_be_bytes());
                        r.extend_from_slice(&b[8..12]);
                        r
                    }
                };
                assert!(n >= 2);
                fake.send_to(&reply, from).unwrap();
            }
        });
        let r = Router { gateway: Ipv4Addr::LOCALHOST, port };
        assert_eq!(r.external_ip(), Some(Ipv4Addr::new(203, 0, 113, 7)));
        assert_eq!(r.map(Proto::Udp, 48998, 48998, 7200), Ok(40000));
    }
}
