//! Minimal blocking UPnP IGD client (SSDP discovery, device description, SOAP) for mapping the
//! stream's UDP port on the router. Std only, so every desktop host can use it without an HTTP
//! stack. It speaks plain HTTP to the router on the local network only.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

const SSDP: &str = "239.255.255.250:1900";
const SEARCH_TARGET: &str = "urn:schemas-upnp-org:device:InternetGatewayDevice:1";
const IO_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    fn name(self) -> &'static str {
        match self {
            Proto::Tcp => "TCP",
            Proto::Udp => "UDP",
        }
    }
}

/// The router: where to send SOAP and which WAN service to ask.
#[derive(Debug, Clone, PartialEq)]
pub struct Gateway {
    pub addr: SocketAddr,
    path: String,
    service: String,
}

impl Gateway {
    /// Finds the router within `wait` (multicast, so only on a network that allows it).
    pub fn discover(wait: Duration) -> Result<Gateway, String> {
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        let req = format!("M-SEARCH * HTTP/1.1\r\nHOST: {SSDP}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {SEARCH_TARGET}\r\n\r\n");
        let to = SSDP.parse::<SocketAddr>().map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        let mut buf = [0u8; 2048];
        let mut sent = 0;
        while t0.elapsed() < wait {
            if sent < 2 && t0.elapsed() >= Duration::from_millis(700 * sent) {
                let _ = sock.send_to(req.as_bytes(), to);
                sent += 1;
            }
            let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
            let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
            let Some(loc) = header(&String::from_utf8_lossy(&buf[..n]), "location") else { continue };
            if let Ok(g) = Gateway::from_location(&loc) {
                return Ok(g);
            }
        }
        Err("no UPnP router answered".into())
    }

    /// Reads the device description at `location` (`http://host:port/path`).
    pub fn from_location(location: &str) -> Result<Gateway, String> {
        let (addr, path) = split_url(location)?;
        let (status, body) = http(addr, "GET", &path, &[], "")?;
        if status != 200 {
            return Err(format!("device description: HTTP {status}"));
        }
        let (service, control) = pick_service(&body).ok_or("the router has no WAN connection service")?;
        let (addr, path) = match control.strip_prefix("http://") {
            Some(_) => split_url(&control)?,
            None => (addr, if control.starts_with('/') { control } else { format!("/{control}") }),
        };
        Ok(Gateway { addr, path, service })
    }

    /// This PC's address on the router's network.
    pub fn local_ip(&self) -> Option<Ipv4Addr> {
        let s = UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect(self.addr).ok()?;
        match s.local_addr().ok()?.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        }
    }

    fn soap(&self, action: &str, args: &str) -> Result<String, String> {
        let body = format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{args}</u:{action}></s:Body></s:Envelope>",
            self.service
        );
        let action_header = format!("\"{}#{action}\"", self.service);
        let (status, reply) = http(self.addr, "POST", &self.path, &[("SOAPAction", &action_header), ("Content-Type", "text/xml; charset=\"utf-8\"")], &body)?;
        if status == 200 {
            Ok(reply)
        } else {
            Err(format!("{action}: HTTP {status} {}", tag(&reply, "errorDescription").unwrap_or_default()))
        }
    }

    /// Maps `port` on the router to `local`. Lease 0 = permanent; some routers only accept a finite one.
    pub fn map(&self, proto: Proto, port: u16, local: SocketAddrV4) -> Result<(), String> {
        let add = |lease: u32| {
            self.soap(
                "AddPortMapping",
                &format!(
                    "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>{}</NewProtocol><NewInternalPort>{}</NewInternalPort><NewInternalClient>{}</NewInternalClient><NewEnabled>1</NewEnabled><NewPortMappingDescription>Secpar</NewPortMappingDescription><NewLeaseDuration>{lease}</NewLeaseDuration>",
                    proto.name(),
                    local.port(),
                    local.ip()
                ),
            )
            .map(|_| ())
        };
        add(0).or_else(|_| add(7 * 24 * 3600))
    }

    pub fn unmap(&self, proto: Proto, port: u16) -> Result<(), String> {
        self.soap("DeletePortMapping", &format!("<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>{}</NewProtocol>", proto.name())).map(|_| ())
    }

    /// The router's public address, when it has one (not behind another NAT).
    pub fn external_ip(&self) -> Option<IpAddr> {
        let r = self.soap("GetExternalIPAddress", "").ok()?;
        let ip: IpAddr = tag(&r, "NewExternalIPAddress")?.trim().parse().ok()?;
        match ip {
            IpAddr::V4(v) if v.is_private() || v.is_loopback() || v.is_unspecified() || v.is_link_local() || (v.octets()[0] == 100 && v.octets()[1] & 0xC0 == 64) => None,
            ip => Some(ip),
        }
    }
}

/// What a successful stream-port mapping gives the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapped {
    pub gateway: Gateway,
    pub port: u16,
    pub public: Option<IpAddr>,
}

/// Discovers the router and maps UDP `port` (external = internal). Blocking, up to `wait` + a few seconds.
pub fn map_udp(port: u16, wait: Duration) -> Result<Mapped, String> {
    let gateway = Gateway::discover(wait)?;
    map_udp_on(gateway, port)
}

pub fn map_udp_on(gateway: Gateway, port: u16) -> Result<Mapped, String> {
    let ip = gateway.local_ip().ok_or("no address on the router's network")?;
    gateway.map(Proto::Udp, port, SocketAddrV4::new(ip, port))?;
    let public = gateway.external_ip();
    Ok(Mapped { gateway, port, public })
}

fn header(msg: &str, name: &str) -> Option<String> {
    msg.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

/// `http://host[:port]/path` to (address, path). Hosts must be IP literals or resolve locally.
fn split_url(url: &str) -> Result<(SocketAddr, String), String> {
    let rest = url.strip_prefix("http://").ok_or("not an http URL")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let host = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    let addr = host.to_socket_addrs().map_err(|e| e.to_string())?.find(SocketAddr::is_ipv4).ok_or("no IPv4 address")?;
    Ok((addr, path))
}

/// One HTTP/1.1 request with `Connection: close`; returns (status, body).
fn http(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Result<(u16, String), String> {
    let mut s = TcpStream::connect_timeout(&addr, IO_TIMEOUT).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(IO_TIMEOUT)).ok();
    s.set_write_timeout(Some(IO_TIMEOUT)).ok();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req += &format!("{k}: {v}\r\n");
    }
    req += "\r\n";
    req += body;
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    let _ = s.take(1 << 20).read_to_end(&mut raw); // a timeout after the reply is fine
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("short HTTP reply")?;
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("bad HTTP status")?;
    let body = if header(head, "transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) { dechunk(body) } else { body.to_string() };
    Ok((status, body))
}

fn dechunk(mut b: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = b.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16) else { break };
        if n == 0 || rest.len() < n {
            break;
        }
        out += &rest[..n];
        b = rest[n..].trim_start_matches("\r\n");
    }
    out
}

/// The text of the first `<name>...</name>`.
fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let i = xml.find(&open)? + open.len();
    let j = xml[i..].find(&format!("</{name}>"))? + i;
    Some(xml[i..j].trim().to_string())
}

/// The WAN IP/PPP connection service in a device description: (serviceType, controlURL).
fn pick_service(xml: &str) -> Option<(String, String)> {
    let mut best: Option<(u8, String, String)> = None;
    let mut rest = xml;
    while let Some(i) = rest.find("<service>") {
        let end = rest[i..].find("</service>").map_or(rest.len(), |e| i + e);
        let block = &rest[i..end];
        if let (Some(st), Some(cu)) = (tag(block, "serviceType"), tag(block, "controlURL")) {
            let rank = if st.contains("WANIPConnection:2") {
                3
            } else if st.contains("WANIPConnection") {
                2
            } else if st.contains("WANPPPConnection") {
                1
            } else {
                0
            };
            if rank > best.as_ref().map_or(0, |b| b.0) {
                best = Some((rank, st, cu));
            }
        }
        rest = &rest[end..];
    }
    best.map(|(_, s, c)| (s, c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const DESC: &str = "<root><device><deviceList><device><deviceList><device><serviceList>\
        <service><serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType><controlURL>/common</controlURL></service>\
        <service><serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType><controlURL>/ppp</controlURL></service>\
        <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/ip</controlURL></service>\
        </serviceList></device></deviceList></device></deviceList></device></root>";

    /// A fake router on loopback: serves the description, answers SOAP, records the requests.
    fn fake_router(refuse_permanent: bool, external: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/desc.xml", l.local_addr().unwrap());
        let log = Arc::new(Mutex::new(vec![]));
        let lg = log.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { break };
                let mut buf = vec![0u8; 8192];
                let mut n = 0;
                loop {
                    let k = s.read(&mut buf[n..]).unwrap_or(0);
                    n += k;
                    let t = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let done = t.split_once("\r\n\r\n").is_some_and(|(h, b)| b.len() >= header(h, "content-length").and_then(|v| v.parse().ok()).unwrap_or(0));
                    if k == 0 || done {
                        break;
                    }
                }
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let first = req.lines().next().unwrap_or("").to_string();
                let reply = |status: &str, body: &str| format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let out = if first.starts_with("GET /desc.xml") {
                    reply("200 OK", DESC)
                } else if let Some(action) = header(&req, "soapaction") {
                    lg.lock().unwrap().push(format!("{first} {action} {}", req.split_once("\r\n\r\n").map_or("", |x| x.1)));
                    if first != "POST /ctl/ip HTTP/1.1" {
                        reply("404 Not Found", "")
                    } else if action.contains("AddPortMapping") && refuse_permanent && req.contains("<NewLeaseDuration>0<") {
                        reply("500 Internal Server Error", "<errorDescription>OnlyPermanentLeasesSupported</errorDescription>")
                    } else if action.contains("GetExternalIPAddress") {
                        reply("200 OK", &format!("<NewExternalIPAddress>{external}</NewExternalIPAddress>"))
                    } else {
                        reply("200 OK", "<ok/>")
                    }
                } else {
                    reply("404 Not Found", "")
                };
                let _ = s.write_all(out.as_bytes());
            }
        });
        (url, log)
    }

    #[test]
    fn picks_the_wan_ip_service_and_parses_replies() {
        assert_eq!(pick_service(DESC), Some(("urn:schemas-upnp-org:service:WANIPConnection:1".into(), "/ctl/ip".into())));
        assert_eq!(pick_service("<root></root>"), None);
        assert_eq!(header("HTTP/1.1 200 OK\r\nLocation: http://192.168.0.1:5000/a.xml\r\n", "location").as_deref(), Some("http://192.168.0.1:5000/a.xml"));
        assert_eq!(split_url("http://192.168.0.1:5000/a.xml").unwrap(), ("192.168.0.1:5000".parse().unwrap(), "/a.xml".to_string()));
        assert_eq!(split_url("http://192.168.0.1").unwrap().0.port(), 80);
        assert!(split_url("https://192.168.0.1/").is_err());
        assert_eq!(dechunk("5\r\nhello\r\n4\r\n you\r\n0\r\n\r\n"), "hello you");
    }

    #[test]
    fn maps_a_udp_port_and_reports_the_public_address() {
        let (url, log) = fake_router(false, "203.0.113.7");
        let g = Gateway::from_location(&url).unwrap();
        let m = map_udp_on(g.clone(), 48998).unwrap();
        assert_eq!((m.port, m.public), (48998, Some("203.0.113.7".parse().unwrap())));
        let first = log.lock().unwrap()[0].clone();
        assert!(first.contains("AddPortMapping") && first.contains("<NewExternalPort>48998<") && first.contains("<NewProtocol>UDP<") && first.contains("<NewInternalClient>127.0.0.1<"), "{first}");
        g.unmap(Proto::Udp, 48998).unwrap();
        assert!(log.lock().unwrap().iter().any(|l| l.contains("DeletePortMapping") && l.contains("<NewExternalPort>48998<")));
    }

    #[test]
    fn falls_back_to_a_finite_lease_and_ignores_private_external_addresses() {
        let (url, log) = fake_router(true, "192.168.1.2");
        let g = Gateway::from_location(&url).unwrap();
        let m = map_udp_on(g, 50001).unwrap();
        assert_eq!(m.public, None, "double NAT: the router's address is not public");
        let log = log.lock().unwrap().clone();
        assert!(log[0].contains("<NewLeaseDuration>0<") && log[1].contains("<NewLeaseDuration>604800<"), "{log:?}");
    }

    #[test]
    fn a_router_that_refuses_reports_the_reason() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let g = Gateway { addr: l.local_addr().unwrap(), path: "/ctl".into(), service: "urn:x:service:WANIPConnection:1".into() };
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                let body = "<errorDescription>ConflictInMappingEntry</errorDescription>";
                let _ = s.write_all(format!("HTTP/1.1 500 Error\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
            }
        });
        let e = g.map(Proto::Udp, 1, "127.0.0.1:1".parse().unwrap()).unwrap_err();
        assert!(e.contains("500") && e.contains("ConflictInMappingEntry"), "{e}");
    }

    #[test]
    fn discovery_gives_up_without_a_router() {
        // No answer on a quiet network (or no multicast at all): an error, never a hang.
        let t0 = Instant::now();
        let r = Gateway::discover(Duration::from_millis(300));
        if let Ok(g) = r {
            eprintln!("router on this network: {}", g.addr);
        }
        assert!(t0.elapsed() < Duration::from_secs(5));
    }
}
