//! A purpose-bound proof of this host's existing TLS key. No account password or
//! user bearer token is passed to the elevated service to create this proof.
use crate::tls::Identity;
pub use secpar_host_core::account_link::{Challenge, Proof, ORIGIN};
pub fn prove(identity: &Identity, challenge: Challenge, name: &str) -> Result<Proof, String> {
    secpar_host_core::account_link::prove(identity, challenge, name)
}
#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}, Engine};
    use ring::{rand::SystemRandom, signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING}};
    use ring::signature::{self, KeyPair};
    fn identity() -> Identity {
        fn der(pem: &str) -> Vec<u8> {
            STANDARD
                .decode(
                    pem.lines()
                        .filter(|line| !line.starts_with("-----"))
                        .collect::<String>(),
                )
                .unwrap()
        }
        let cert_der = der(include_str!(
            "../../../cloud/accounts/test/fixtures/host-cert.pem"
        ));
        let key_der = der(include_str!(
            "../../../cloud/accounts/test/fixtures/host-key.pem"
        ));
        let fingerprint_hex = crate::tls::hex(&crate::tls::sha256(&cert_der));
        Identity {
            cert_der,
            key_der,
            fingerprint_hex,
        }
    }
    fn challenge() -> Challenge {
        Challenge {
            challenge: "1".repeat(64),
            owner: "interop-fixture".into(),
            origin: ORIGIN.into(),
            issued_at: 1_800_000_000,
            expires_at: 1_800_000_300,
        }
    }
    #[test]
    fn proof_uses_the_existing_identity_and_fixed_width_jws_signature() {
        let identity = identity();
        let proof = prove(&identity, challenge(), "Fixture Windows").unwrap();
        let parts: Vec<_> = proof.proof.split('.').collect();
        assert_eq!(parts.len(), 3);
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        assert_eq!(signature.len(), 64);
        let key = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &identity.key_der,
            &SystemRandom::new(),
        )
        .unwrap();
        signature::UnparsedPublicKey::new(
            &signature::ECDSA_P256_SHA256_FIXED,
            key.public_key().as_ref(),
        )
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["tls_fingerprint"], identity.fingerprint_hex);
        assert_eq!(
            claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
            90
        );
        assert_eq!(claims["purpose"], "secpar-host-enrollment");
    }
    #[test]
    fn proof_rejects_other_services_and_unbounded_challenges() {
        let identity = identity();
        let mut bad = challenge();
        bad.origin = "https://elsewhere.invalid".into();
        assert!(prove(&identity, bad, "PC").is_err());
        let mut bad = challenge();
        bad.expires_at = bad.issued_at + 301;
        assert!(prove(&identity, bad, "PC").is_err());
        let mut bad = challenge();
        bad.challenge = "not-a-nonce".into();
        assert!(prove(&identity, bad, "PC").is_err());
    }
    #[test]
    #[ignore = "Regenerates the public cross-language fixture"]
    fn write_public_interop_fixture() {
        let proof = prove(&identity(), challenge(), "Fixture Windows").unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../cloud/accounts/test/fixtures/rust-host-proof.json");
        std::fs::write(path, serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
    }
}
