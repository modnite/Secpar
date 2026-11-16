//! The requests Moonlight's own apps make before the protocol core takes over:
//! the client certificate, PIN pairing, server info, the app list and launch.
//! The engine is reached through Secpar's tunnel, which already pins the
//! computer, so the engine's own self-signed certificate is not checked again.
use aes::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use rsa::{pkcs1v15::SigningKey, pkcs8::{DecodePrivateKey, EncodePrivateKey}, signature::{RandomizedSigner, SignatureEncoding}, RsaPrivateKey};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};

const UNIQUE_ID: &str = "0123456789ABCDEF";

/// This device's certificate towards one computer's engine, kept beside its profile.
#[derive(Clone)]
pub struct Identity {
    pub certificate_pem: String,
    pub key_der: Vec<u8>,
}

impl Identity {
    /// Loads the identity from `folder`, or makes and saves a new one.
    pub fn load_or_create(folder: &Path) -> Result<Self, String> {
        let (cert, key) = (folder.join("client.pem"), folder.join("client.key"));
        if let (Ok(certificate_pem), Ok(key_der)) = (std::fs::read_to_string(&cert), std::fs::read(&key)) {
            return Ok(Self { certificate_pem, key_der });
        }
        let mut rng = rand_core();
        let private = RsaPrivateKey::new(&mut rng, 2048).map_err(|_| "key")?;
        let key_der = private.to_pkcs8_der().map_err(|_| "key")?.as_bytes().to_vec();
        let pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&key_der.as_slice().into(), &rcgen::PKCS_RSA_SHA256).map_err(|_| "key")?;
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|_| "certificate")?;
        params.distinguished_name.push(rcgen::DnType::CommonName, "NVIDIA GameStream Client");
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = rcgen::date_time_ymd(2044, 1, 1);
        let certificate_pem = params.self_signed(&pair).map_err(|_| "certificate")?.pem();
        std::fs::create_dir_all(folder).map_err(|_| "folder")?;
        std::fs::write(&cert, &certificate_pem).map_err(|_| "folder")?;
        std::fs::write(&key, &key_der).map_err(|_| "folder")?;
        Ok(Self { certificate_pem, key_der })
    }

    fn certificate_der(&self) -> Vec<u8> {
        let body: String = self.certificate_pem.lines().filter(|l| !l.starts_with("-----")).collect();
        base64_decode(&body)
    }
}

fn rand_core() -> rand_core::OsRng {
    rand_core::OsRng
}

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    let _ = getrandom::getrandom(&mut out);
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (text.len() % 2 == 0).then(|| (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()).flatten()
}

fn base64_decode(text: &str) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes() {
        let Some(v) = T.iter().position(|&t| t == c) else { continue };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// The signature bytes of a DER certificate: the last element of its outer sequence.
fn signature_of(der: &[u8]) -> Option<Vec<u8>> {
    fn header(d: &[u8]) -> Option<(usize, usize)> {
        let first = *d.get(1)? as usize;
        if first < 0x80 {
            return Some((2, first));
        }
        let n = first & 0x7f;
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *d.get(2 + i)? as usize;
        }
        Some((2 + n, len))
    }
    let (start, len) = header(der)?;
    let mut at = start;
    let end = start + len;
    let mut last = None;
    while at < end {
        let (h, l) = header(&der[at..])?;
        last = Some((at + h, l));
        at += h + l;
    }
    let (from, length) = last?;
    // A BIT STRING starts with the count of unused bits.
    der.get(from + 1..from + length).map(<[u8]>::to_vec)
}

fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(xml[start..end].trim().to_string())
}

struct AcceptAny;
impl std::fmt::Debug for AcceptAny {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel-pinned")
    }
}
impl rustls::client::danger::ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self, _: &rustls::pki_types::CertificateDer<'_>, _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>, _: &[u8], _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self, _: &[u8], _: &rustls::pki_types::CertificateDer<'_>, _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self, _: &[u8], _: &rustls::pki_types::CertificateDer<'_>, _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

/// The engine of one computer, reached at `address` (the tunnel's local end).
pub struct Engine {
    address: String,
    http_port: u16,
    https_port: u16,
    identity: Identity,
    plain: reqwest::Client,
    secure: reqwest::Client,
}

/// What the engine says about itself, as the protocol core needs it.
pub struct ServerInfo {
    pub app_version: String,
    pub gfe_version: String,
    pub codec_support: i32,
    pub paired: bool,
    pub current_game: u32,
}

impl Engine {
    pub fn new(address: &str, http_port: u16, https_port: u16, identity: Identity) -> Result<Self, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let certificate = rustls::pki_types::CertificateDer::from(identity.certificate_der());
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(identity.key_der.clone().into());
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|_| "tls")?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny))
            .with_client_auth_cert(vec![certificate], key)
            .map_err(|_| "tls")?;
        let build = |b: reqwest::ClientBuilder| b.timeout(Duration::from_secs(70)).connect_timeout(Duration::from_secs(8)).build().map_err(|_| "http".to_string());
        Ok(Self {
            address: address.into(),
            http_port,
            https_port,
            identity,
            plain: build(reqwest::Client::builder())?,
            secure: build(reqwest::Client::builder().tls_backend_preconfigured(tls))?,
        })
    }

    async fn get(&self, secure: bool, path: &str, query: &str) -> Result<String, String> {
        let url = if secure {
            format!("https://{}:{}/{path}?uniqueid={UNIQUE_ID}&uuid={}&{query}", self.address, self.https_port, hex(&random::<16>()))
        } else {
            format!("http://{}:{}/{path}?uniqueid={UNIQUE_ID}&uuid={}&{query}", self.address, self.http_port, hex(&random::<16>()))
        };
        let client = if secure { &self.secure } else { &self.plain };
        let answer = client.get(url).send().await.map_err(|_| format!("{path}_unreachable"))?;
        answer.text().await.map_err(|_| format!("{path}_unreadable"))
    }

    pub async fn server_info(&self) -> Result<ServerInfo, String> {
        let xml = self.get(true, "serverinfo", "").await?;
        Ok(ServerInfo {
            app_version: tag(&xml, "appversion").unwrap_or_default(),
            gfe_version: tag(&xml, "GfeVersion").unwrap_or_default(),
            codec_support: tag(&xml, "ServerCodecModeSupport").and_then(|v| v.parse().ok()).unwrap_or(1),
            paired: tag(&xml, "PairStatus").as_deref() == Some("1"),
            current_game: tag(&xml, "currentgame").and_then(|v| v.parse().ok()).unwrap_or(0),
        })
    }

    /// Moonlight's PIN pairing. The computer must be told the same PIN through
    /// Secpar's own connection while this runs; the first request waits for it.
    pub async fn pair(&self, pin: &str) -> Result<(), String> {
        let ok = |xml: &str| tag(xml, "paired").as_deref() == Some("1");
        let salt = random::<16>();
        let key: [u8; 16] = Sha256::digest([&salt[..], pin.as_bytes()].concat())[..16].try_into().unwrap();
        let aes = Aes128::new(GenericArray::from_slice(&key));
        let ecb = |encrypt: bool, data: &[u8]| -> Vec<u8> {
            let mut out = data.to_vec();
            for block in out.chunks_mut(16) {
                let g = GenericArray::from_mut_slice(block);
                if encrypt { aes.encrypt_block(g) } else { aes.decrypt_block(g) }
            }
            out
        };
        let base = "devicename=roth&updateState=1";
        let xml = self
            .get(false, "pair", &format!("{base}&phrase=getservercert&salt={}&clientcert={}", hex(&salt), hex(self.identity.certificate_pem.as_bytes())))
            .await?;
        if !ok(&xml) || tag(&xml, "plaincert").unwrap_or_default().is_empty() {
            return Err("pairing_refused".into());
        }
        let challenge = random::<16>();
        let xml = self.get(false, "pair", &format!("{base}&clientchallenge={}", hex(&ecb(true, &challenge)))).await?;
        let response = tag(&xml, "challengeresponse").and_then(|h| unhex(&h)).filter(|r| ok(&xml) && r.len() >= 48).ok_or("pairing_failed")?;
        let response = ecb(false, &response);
        let server_challenge = &response[32..48];
        let secret = random::<16>();
        let signature = signature_of(&self.identity.certificate_der()).ok_or("pairing_failed")?;
        let hash = Sha256::digest([server_challenge, &signature, &secret].concat());
        let xml = self.get(false, "pair", &format!("{base}&serverchallengeresp={}", hex(&ecb(true, &hash)))).await?;
        if !ok(&xml) {
            return Err("pairing_failed".into());
        }
        let private = RsaPrivateKey::from_pkcs8_der(&self.identity.key_der).map_err(|_| "pairing_failed")?;
        let signed = SigningKey::<Sha256>::new(private).sign_with_rng(&mut rand_core(), &secret).to_vec();
        let xml = self.get(false, "pair", &format!("{base}&clientpairingsecret={}", hex(&[&secret[..], &signed].concat()))).await?;
        if !ok(&xml) {
            return Err("pairing_failed".into());
        }
        let xml = self.get(true, "pair", &format!("{base}&phrase=pairchallenge")).await?;
        if !ok(&xml) {
            return Err("pairing_failed".into());
        }
        Ok(())
    }

    /// Starts the desktop session, or resumes the one already running, and
    /// returns the RTSP address the protocol core opens.
    pub async fn launch(&self, width: u32, height: u32, fps: u32, input_key: &[u8; 16], input_key_id: i32, running: bool) -> Result<String, String> {
        let common = format!(
            "rikey={}&rikeyid={input_key_id}&surroundAudioInfo=196610&localAudioPlayMode=0&remoteControllersBitmap=0&gcmap=0&gcpersist=0",
            hex(input_key)
        );
        let xml = if running {
            self.get(true, "resume", &common).await?
        } else {
            let list = self.get(true, "applist", "").await?;
            let id = list
                .split("<App>")
                .skip(1)
                .find(|app| tag(app, "AppTitle").as_deref() == Some("Desktop"))
                .or_else(|| list.split("<App>").nth(1))
                .and_then(|app| tag(app, "ID"))
                .ok_or("no_desktop_app")?;
            self.get(true, "launch", &format!("appid={id}&mode={width}x{height}x{fps}&additionalStates=1&sops=0&{common}")).await?
        };
        tag(&xml, "sessionUrl0").filter(|url| !url.is_empty()).ok_or_else(|| "launch_refused".to_string())
    }
}
