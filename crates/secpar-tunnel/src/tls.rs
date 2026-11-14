//! TLS 1.3 for the tunnel. No certificate authority is involved: each side
//! checks the other's certificate against a fingerprint it already trusts.
use crate::{fingerprint, Allowed, Error, Fingerprint, Identity, Result};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    DigitallySignedStruct, DistinguishedName, SignatureScheme,
};
use std::sync::Arc;

const ALPN: &[u8] = b"secpar-tunnel/1";

fn signature12(
    provider: &CryptoProvider,
    m: &[u8],
    c: &CertificateDer<'_>,
    s: &DigitallySignedStruct,
) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
    rustls::crypto::verify_tls12_signature(m, c, s, &provider.signature_verification_algorithms)
}
fn signature13(
    provider: &CryptoProvider,
    m: &[u8],
    c: &CertificateDer<'_>,
    s: &DigitallySignedStruct,
) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
    rustls::crypto::verify_tls13_signature(m, c, s, &provider.signature_verification_algorithms)
}

#[derive(Debug)]
struct PinnedHost {
    pin: Fingerprint,
    provider: Arc<CryptoProvider>,
}
impl ServerCertVerifier for PinnedHost {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if fingerprint(end.as_ref()) != self.pin {
            return Err(rustls::Error::General("Host is not the expected computer".into()));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        signature12(&self.provider, m, c, s)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        signature13(&self.provider, m, c, s)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

#[derive(Debug)]
struct AllowedDevices {
    allowed: Arc<Allowed>,
    provider: Arc<CryptoProvider>,
}
impl ClientCertVerifier for AllowedDevices {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        end: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        if !self.allowed.contains(&fingerprint(end.as_ref())) {
            self.allowed.turned_away();
            return Err(rustls::Error::General("Device is not allowed on this host".into()));
        }
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        signature12(&self.provider, m, c, s)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        signature13(&self.provider, m, c, s)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn chain(identity: &Identity) -> (Vec<CertificateDer<'static>>, rustls::pki_types::PrivateKeyDer<'static>) {
    (
        vec![CertificateDer::from(identity.certificate_der().to_vec())],
        PrivatePkcs8KeyDer::from(identity.key_der().to_vec()).into(),
    )
}

pub fn server(
    identity: &Identity,
    allowed: Arc<Allowed>,
) -> Result<quinn::crypto::rustls::QuicServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let (certificates, key) = chain(identity);
    let mut config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| Error("Tunnel TLS configuration failed"))?
        .with_client_cert_verifier(Arc::new(AllowedDevices { allowed, provider }))
        .with_single_cert(certificates, key)
        .map_err(|_| Error("Device identity is not usable"))?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    quinn::crypto::rustls::QuicServerConfig::try_from(config)
        .map_err(|_| Error("Tunnel TLS configuration failed"))
}

pub fn client(
    identity: &Identity,
    host_pin: Fingerprint,
) -> Result<quinn::crypto::rustls::QuicClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let (certificates, key) = chain(identity);
    let mut config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| Error("Tunnel TLS configuration failed"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedHost {
            pin: host_pin,
            provider,
        }))
        .with_client_auth_cert(certificates, key)
        .map_err(|_| Error("Device identity is not usable"))?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    quinn::crypto::rustls::QuicClientConfig::try_from(config)
        .map_err(|_| Error("Tunnel TLS configuration failed"))
}
