//! TURN over TLS, for networks that block UDP (RFC 8656 section 3.1). STUN messages and
//! ChannelData are framed by their own lengths, ChannelData padded to four bytes. The relay
//! still reaches the computer over UDP; only this side's path to the relay is TCP.

use secpar_network::turn::Link;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_rustls::rustls::{pki_types::ServerName, ClientConfig, RootCertStore};

/// Sends framed messages to the relay through the TLS connection's writer task.
pub struct TlsLink(mpsc::UnboundedSender<Vec<u8>>);

impl Link for TlsLink {
    fn send(&self, b: &[u8]) {
        let _ = self.0.send(b.to_vec());
    }
}

/// The whole frame's length from its first four bytes.
pub fn frame_len(head: [u8; 4]) -> usize {
    let len = u16::from_be_bytes([head[2], head[3]]) as usize;
    if head[0] & 0xC0 == 0x40 { 4 + len.div_ceil(4) * 4 } else { 20 + len }
}

/// Connects to `host:port` with the system's usual certificate checks. Every frame the relay
/// sends goes to `deliver`; the connection closes when the returned link is dropped.
pub async fn connect(host: &str, port: u16, deliver: impl Fn(&[u8]) + Send + 'static) -> Result<(Arc<TlsLink>, SocketAddr), String> {
    let addr = tokio::net::lookup_host((host, port)).await.map_err(|e| e.to_string())?.find(SocketAddr::is_ipv4).ok_or("no address")?;
    let tcp = tokio::time::timeout(std::time::Duration::from_secs(5), tokio::net::TcpStream::connect(addr)).await.map_err(|_| "timed out")?.map_err(|e| e.to_string())?;
    let _ = tcp.set_nodelay(true);
    let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    let name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp).await.map_err(|e| e.to_string())?;
    let (mut read, mut write) = tokio::io::split(tls);
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(b) = rx.recv().await {
            if write.write_all(&b).await.is_err() {
                break;
            }
        }
        let _ = write.shutdown().await;
    });
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4 + 0xFFFF + 20];
        loop {
            if read.read_exact(&mut buf[..4]).await.is_err() {
                break;
            }
            let n = frame_len([buf[0], buf[1], buf[2], buf[3]]);
            if read.read_exact(&mut buf[4..n]).await.is_err() {
                break;
            }
            deliver(&buf[..n]);
        }
    });
    Ok((Arc::new(TlsLink(tx)), addr))
}

#[cfg(test)]
mod tests {
    /// Network: an unsigned Allocate over TLS on 443 comes back as a framed 401 from the relay.
    #[tokio::test]
    #[ignore]
    async fn cloudflare_relay_answers_over_tls() {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let (link, _) = super::connect("turn.cloudflare.com", 443, move |b| { let _ = tx.send(b.to_vec()); }).await.unwrap();
        let m = secpar_network::turn::Msg::new(0x0003).add(0x0019, [17u8, 0, 0, 0]).encode(None);
        secpar_network::turn::Link::send(&*link, &m);
        let answer = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        let reply = secpar_network::turn::Msg::decode(&answer).unwrap();
        let code = reply.get(0x0009).map(|e| e[2] as u16 * 100 + e[3] as u16);
        assert_eq!(code, Some(401));
    }

    #[test]
    fn frames_are_measured_from_their_header() {
        assert_eq!(super::frame_len([0x01, 0x01, 0, 8]), 28);
        assert_eq!(super::frame_len([0x40, 0x00, 0, 5]), 12);
        assert_eq!(super::frame_len([0x40, 0x01, 0x04, 0xB0]), 4 + 1200);
    }
}
