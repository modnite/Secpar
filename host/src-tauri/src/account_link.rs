//! A purpose-bound proof of this host's existing TLS key. No account password or
//! user bearer token is passed to the elevated service to create this proof.
use crate::tls::Identity;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use ring::{
    rand::SystemRandom,
    signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
};
use serde::{Deserialize, Serialize};

pub const ORIGIN: &str = "https://accounts.secpar.win";

#[derive(Deserialize)]
pub struct Challenge {
    pub challenge: String,
    pub owner: String,
    pub origin: String,
    pub issued_at: u64,
    pub expires_at: u64,
}
#[derive(Serialize)]
pub struct Proof {
    pub certificate: String,
    pub proof: String,
}

pub fn prove(identity: &Identity, challenge: Challenge, name: &str) -> Result<Proof, String> {
    if challenge.origin != ORIGIN
        || challenge.challenge.len() != 64
        || !challenge
            .challenge
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || challenge.owner.is_empty()
        || challenge.owner.len() > 80
        || !challenge
            .owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || challenge.issued_at < 1_600_000_000
        || challenge.issued_at > 9_007_199_254_740_000
        || challenge.expires_at <= challenge.issued_at
        || challenge.expires_at - challenge.issued_at > 300
        || name.trim().is_empty()
        || name.encode_utf16().count() > 80
    {
        return Err("Invalid Secpar account-link challenge".into());
    }
    let header = serde_json::json!({"alg":"ES256","typ":"secpar-host-enrollment+jwt"});
    let claims = serde_json::json!({
        "iss":"secpar-host","aud":ORIGIN,"sub":challenge.owner,
        "purpose":"secpar-host-enrollment","challenge":challenge.challenge,
        "tls_fingerprint":identity.fingerprint_hex,"name":name.trim(),
        "platform":std::env::consts::OS,
        // Server-supplied time avoids a wrong local clock extending a challenge.
        // The account service independently verifies both age and nonce expiry.
        "iat":challenge.issued_at,"exp":challenge.issued_at.saturating_add(90).min(challenge.expires_at)
    });
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let rng = SystemRandom::new();
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &identity.key_der, &rng)
        .map_err(|_| "Host identity cannot sign an account-link proof")?;
    let signature = key
        .sign(&rng, input.as_bytes())
        .map_err(|_| "Could not sign account-link proof")?;
    let encoded = STANDARD.encode(&identity.cert_der);
    let lines = encoded
        .as_bytes()
        .chunks(64)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Proof {
        certificate: format!("-----BEGIN CERTIFICATE-----\n{lines}\n-----END CERTIFICATE-----\n"),
        proof: format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
