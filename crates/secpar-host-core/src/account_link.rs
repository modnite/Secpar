//! A purpose-bound proof of this host's existing TLS key. No account password or
//! user bearer token is passed to the elevated service to create this proof.
use crate::IdentitySource;
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

pub fn prove(identity: &dyn IdentitySource, challenge: Challenge, name: &str) -> Result<Proof, String> {
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
        "tls_fingerprint":ring::digest::digest(&ring::digest::SHA256, identity.certificate()).as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>(),"name":name.trim(),
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
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, identity.private_key(), &rng)
        .map_err(|_| "Host identity cannot sign an account-link proof")?;
    let signature = key
        .sign(&rng, input.as_bytes())
        .map_err(|_| "Could not sign account-link proof")?;
    let encoded = STANDARD.encode(identity.certificate());
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

