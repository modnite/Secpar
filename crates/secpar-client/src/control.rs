use crate::pairing::Pairing;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::net::TcpStream;
use tokio_rustls::rustls::{
    self,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use tokio_tungstenite::{
    tungstenite::{protocol::WebSocketConfig, Message},
    Connector, MaybeTlsStream, WebSocketStream,
};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

// These contain credentials. Callers must use an OS credential store, not ordinary UI persistence.
#[derive(Clone, Serialize, Deserialize)]
pub struct Relay {
    pub url: String,
    pub dev: String,
    pub ticket: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Computer {
    pub id: String,
    pub name: String,
    pub hosts: Vec<String>,
    pub port: u16,
    pub fingerprint: [u8; 32],
    pub token: String,
    pub relay: Option<Relay>,
}
pub struct Connected {
    pub socket: Socket,
    pub computer: Computer,
    pub direct_host: Option<String>,
}

impl Relay {
    pub fn signed_url(&self, pc: &str, now: u64, path: &str) -> Result<String, String> {
        if !matches!(path, "phone" | "turn") {
            return Err("Unsupported relay operation".into());
        }
        let mut url = url::Url::parse(&self.url).map_err(|_| "Invalid relay address")?;
        if url.scheme() != "wss"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Invalid relay address".into());
        }
        let key = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&self.ticket)
            .map_err(|_| "Invalid relay ticket")?;
        if key.len() != 32 {
            return Err("Invalid relay ticket".into());
        }
        let tag = ring::hmac::sign(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key),
            format!("phone:{pc}:{}:{now}", self.dev).as_bytes(),
        );
        let sig: String = tag.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        url.set_path(&format!("/v1/{path}"));
        url.set_query(None);
        url.set_fragment(None);
        url.query_pairs_mut()
            .append_pair("pc", pc)
            .append_pair("dev", &self.dev)
            .append_pair("ts", &now.to_string())
            .append_pair("sig", &sig);
        Ok(url.to_string())
    }
}

#[derive(Debug)]
struct Pinned {
    fingerprint: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
}
impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let hash = ring::digest::digest(&ring::digest::SHA256, end.as_ref());
        if hash.as_ref() != self.fingerprint {
            return Err(rustls::Error::General(
                "Host certificate does not match the pairing code".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            m,
            c,
            s,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            m,
            c,
            s,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

async fn open(url: &str, pin: Option<[u8; 32]>) -> Result<Socket, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let connector = pin.map(|fingerprint| {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Pinned {
                fingerprint,
                provider,
            }))
            .with_no_client_auth();
        Connector::Rustls(Arc::new(cfg))
    });
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    tokio::time::timeout(
        Duration::from_secs(6),
        tokio_tungstenite::connect_async_tls_with_config(url, Some(config), true, connector),
    )
    .await
    .map_err(|_| "Connection timed out")?
    .map(|(s, _)| s)
    .map_err(|_| "Could not establish a secure connection".into())
}
pub async fn send(socket: &mut Socket, value: Value) -> Result<(), String> {
    socket
        .send(Message::text(value.to_string()))
        .await
        .map_err(|_| "Connection closed".into())
}
pub async fn next(socket: &mut Socket) -> Result<Value, String> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(t))) => {
                return serde_json::from_str(&t).map_err(|_| "Host sent an invalid response".into())
            }
            Some(Ok(Message::Ping(p))) => socket
                .send(Message::Pong(p))
                .await
                .map_err(|_| "Connection closed")?,
            Some(Ok(Message::Pong(_))) | Some(Ok(Message::Binary(_))) => {}
            _ => return Err("Connection closed".into()),
        }
    }
}
fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn remote(v: &Value) -> Option<Relay> {
    if v["t"] != "remote" {
        return None;
    }
    let r = Relay {
        url: v["url"].as_str()?.into(),
        dev: v["dev"].as_str()?.into(),
        ticket: v["ticket"].as_str()?.into(),
    };
    r.signed_url("validation", 0, "phone").ok()?;
    Some(r)
}

impl Connected {
    pub async fn pair(p: Pairing, device: &str) -> Result<Self, String> {
        let mut routes: Vec<(String, Option<String>)> =
            p.relay_url.iter().cloned().map(|r| (r, None)).collect();
        routes.extend(
            p.direct_urls()
                .into_iter()
                .zip(p.hosts.iter().cloned().map(Some)),
        );
        let mut last = "No connection address".to_string();
        for (url, direct_host) in routes {
            let mut ws = match open(&url, direct_host.as_ref().map(|_| p.fingerprint)).await {
                Ok(s) => s,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            send(
                &mut ws,
                json!({"t":"pair","otp":p.otp,"device":device,"pin":""}),
            )
            .await?;
            let v = tokio::time::timeout(Duration::from_secs(8), next(&mut ws))
                .await
                .map_err(|_| "Host did not answer pairing")??;
            if v["t"] != "paired" {
                return Err(
                    "Pairing was refused. Generate a fresh pairing link on the host.".into(),
                );
            }
            let id = v["pc"]["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("Host did not identify itself")?
                .to_string();
            if p.pc_id.as_ref().is_some_and(|expected| *expected != id) {
                let _ = ws.close(None).await;
                return Err("Host identity does not match the pairing code".into());
            }
            let token = v["token"]
                .as_str()
                .ok_or("Host did not return a device credential")?
                .to_string();
            if base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&token)
                .map(|v| v.len())
                .unwrap_or(0)
                != 32
            {
                return Err("Host returned an invalid device credential".into());
            }
            let r = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let m = next(&mut ws).await?;
                    if let Some(r) = remote(&m) {
                        return Ok::<_, String>(r);
                    }
                }
            })
            .await
            .ok()
            .and_then(Result::ok);
            if direct_host.is_none() && r.is_none() {
                return Err(
                    "Host paired but did not provide remote reconnection credentials".into(),
                );
            }
            let name = v["pc"]["name"].as_str().unwrap_or(&p.name).to_string();
            let computer = Computer {
                id,
                name,
                hosts: p.hosts,
                port: p.port,
                fingerprint: p.fingerprint,
                token,
                relay: r,
            };
            return Ok(Self {
                socket: ws,
                computer,
                direct_host,
            });
        }
        Err(last)
    }
    pub async fn reconnect(computer: Computer, device: &str) -> Result<Self, String> {
        let p = Pairing {
            name: computer.name.clone(),
            hosts: computer.hosts.clone(),
            port: computer.port,
            fingerprint: computer.fingerprint,
            otp: String::new(),
            pc_id: Some(computer.id.clone()),
            relay_url: None,
        };
        let mut routes: Vec<_> = p
            .direct_urls()
            .into_iter()
            .zip(computer.hosts.iter().cloned().map(Some))
            .collect();
        if let Some(r) = &computer.relay {
            routes.push((r.signed_url(&computer.id, now_s(), "phone")?, None));
        }
        let mut last = "Computer is offline".into();
        for (url, direct_host) in routes {
            let mut ws = match open(&url, direct_host.as_ref().map(|_| computer.fingerprint)).await
            {
                Ok(s) => s,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            send(
                &mut ws,
                json!({"t":"hello","v":1,"token":computer.token,"device":device}),
            )
            .await?;
            let v = tokio::time::timeout(Duration::from_secs(8), next(&mut ws))
                .await
                .map_err(|_| "Host did not answer")??;
            if v["t"] != "welcome" {
                return Err("Access was refused or revoked. Pair this computer again.".into());
            }
            if v["pc"]["id"].as_str() != Some(&computer.id) {
                return Err("Host identity changed".into());
            }
            return Ok(Self {
                socket: ws,
                computer,
                direct_host,
            });
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_rustls::{rustls::pki_types::PrivatePkcs8KeyDer, TlsAcceptor};
    #[tokio::test]
    async fn pinned_tls_pairs_reconnects_and_rejects_other_certificate() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin: [u8; 32] = ring::digest::digest(&ring::digest::SHA256, cert.der())
            .as_ref()
            .try_into()
            .unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
            )
            .unwrap();
        let accept = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7; 32]);
        let server_token = token.clone();
        let server = tokio::spawn(async move {
            for mode in ["pair", "hello", "bad-certificate"] {
                let (s, _) = listener.accept().await.unwrap();
                let tls = accept.accept(s).await;
                if mode == "bad-certificate" {
                    assert!(tls.is_err());
                    continue;
                }
                let mut ws = tokio_tungstenite::accept_async(tls.unwrap()).await.unwrap();
                let m = ws.next().await.unwrap().unwrap().into_text().unwrap();
                let m: Value = serde_json::from_str(&m).unwrap();
                assert_eq!(m["t"], mode);
                if mode == "hello" {
                    assert_eq!(m["token"], server_token);
                }
                ws.send(Message::text(json!({"t":if mode=="pair"{"paired"}else{"welcome"},"pc":{"id":"pc1","name":"Test PC"},"token":server_token}).to_string())).await.unwrap();
                if mode == "pair" {
                    ws.send(Message::text(json!({"t":"remote","url":"wss://relay.example","dev":"desktop1","ticket":server_token}).to_string())).await.unwrap();
                }
                while let Some(Ok(m)) = ws.next().await {
                    if m.is_close() {
                        break;
                    }
                }
            }
        });
        let p = Pairing {
            name: "Test".into(),
            hosts: vec!["127.0.0.1".into()],
            port: addr.port(),
            fingerprint: pin,
            otp: "once".into(),
            relay_url: None,
            pc_id: Some("pc1".into()),
        };
        let mut c = Connected::pair(p, "Desktop").await.unwrap();
        assert!(c.computer.relay.is_some());
        assert_eq!(c.computer.token, token);
        let computer = c.computer.clone();
        c.socket.close(None).await.unwrap();
        drop(c);
        let mut c = Connected::reconnect(computer, "Desktop").await.unwrap();
        c.socket.close(None).await.unwrap();
        drop(c);
        assert!(open(&format!("wss://{addr}/v1"), Some([0; 32]))
            .await
            .is_err());
        server.await.unwrap();
    }
}

#[cfg(test)]
#[path = "integration.rs"]
mod integration;
