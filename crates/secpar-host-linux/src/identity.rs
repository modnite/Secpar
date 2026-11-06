use crate::store::{hex, private_directory, read_private, write_private};
use secpar_host_core::IdentitySource;
use serde::{Deserialize, Serialize};
use std::{io, path::Path};
#[derive(Serialize, Deserialize)]
pub struct Identity {
    certificate: Vec<u8>,
    private_key: Vec<u8>,
}
impl IdentitySource for Identity {
    fn certificate(&self) -> &[u8] {
        &self.certificate
    }
    fn private_key(&self) -> &[u8] {
        &self.private_key
    }
}
impl Identity {
    pub fn open(dir: &Path) -> io::Result<Self> {
        private_directory(dir)?;
        let path = dir.join("host-identity.json");
        match read_private(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
                    .map_err(io::Error::other)?;
                let mut p = rcgen::CertificateParams::new(vec!["secpar.local".into()])
                    .map_err(io::Error::other)?;
                p.distinguished_name
                    .push(rcgen::DnType::CommonName, "Secpar");
                let cert = p.self_signed(&key).map_err(io::Error::other)?;
                let identity = Self {
                    certificate: cert.der().to_vec(),
                    private_key: key.serialize_der(),
                };
                write_private(&path, &serde_json::to_vec(&identity)?)?;
                Ok(identity)
            }
            Err(e) => Err(e),
        }
    }
    pub fn fingerprint(&self) -> String {
        hex(ring::digest::digest(&ring::digest::SHA256, &self.certificate).as_ref())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_is_stable_and_corrupt_state_is_not_silently_replaced() {
        let d = tempfile::tempdir().unwrap();
        let a = Identity::open(d.path()).unwrap();
        assert_eq!(
            a.fingerprint(),
            Identity::open(d.path()).unwrap().fingerprint()
        );
        write_private(&d.path().join("host-identity.json"), b"broken").unwrap();
        assert!(Identity::open(d.path()).is_err());
    }
}
