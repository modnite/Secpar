//! Verification only: a valid signature is NOT permission to open a session.
//! Admission also needs an active machine-scoped online lease, local consent,
//! an unlocked desktop, and permission checks on both control and PRS input.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

#[derive(Deserialize)]
pub struct PublicKey {
    kid: String,
    kty: String,
    crv: String,
    x: String,
    y: String,
    alg: Option<String>,
    #[serde(rename = "use")]
    usage: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    typ: String,
    kid: String,
}
#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    jti: String,
    iat: u64,
    exp: u64,
    permissions: Vec<String>,
}
pub struct VerifiedGrant {
    request: String,
    guest: String,
    permissions: BTreeSet<String>,
    token_expires: u64,
}
impl VerifiedGrant {
    pub fn request(&self) -> &str {
        &self.request
    }
    pub fn guest(&self) -> &str {
        &self.guest
    }
    pub fn permissions(&self) -> &BTreeSet<String> {
        &self.permissions
    }
    pub fn token_expires(&self) -> u64 {
        self.token_expires
    }
}

fn decode(value: &str) -> Result<Vec<u8>, &'static str> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "Invalid guest grant")?;
    if URL_SAFE_NO_PAD.encode(&bytes) != value {
        return Err("Invalid guest grant");
    }
    Ok(bytes)
}
fn id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
/// `keys` must come from Secpar's fixed HTTPS JWKS endpoint, never from the token.
/// `computer` is the locally installed machine registration, not client input.
pub fn verify(
    token: &str,
    computer: &str,
    keys: &[PublicKey],
    now: u64,
) -> Result<VerifiedGrant, &'static str> {
    let invalid = "Invalid guest grant";
    if token.len() > 4096 || !id(computer) || keys.is_empty() || keys.len() > 5 {
        return Err(invalid);
    }
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 || parts[0].len() > 512 || parts[1].len() > 2048 {
        return Err(invalid);
    }
    let header: Header = serde_json::from_slice(&decode(parts[0])?).map_err(|_| invalid)?;
    if header.alg != "ES256"
        || header.typ != "JWT"
        || header.kid.is_empty()
        || header.kid.len() > 128
    {
        return Err(invalid);
    }
    let matching: Vec<_> = keys.iter().filter(|k| k.kid == header.kid).collect();
    if matching.len() != 1 {
        return Err(invalid);
    }
    let key = matching[0];
    if key.kty != "EC"
        || key.crv != "P-256"
        || key.alg.as_deref().is_some_and(|a| a != "ES256")
        || key.usage.as_deref().is_some_and(|u| u != "sig")
    {
        return Err(invalid);
    }
    let (x, y, signature) = (decode(&key.x)?, decode(&key.y)?, decode(parts[2])?);
    if x.len() != 32 || y.len() != 32 || signature.len() != 64 {
        return Err(invalid);
    }
    let mut public = Vec::with_capacity(65);
    public.push(4);
    public.extend(x);
    public.extend(y);
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public)
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .map_err(|_| invalid)?;
    let claims: Claims = serde_json::from_slice(&decode(parts[1])?).map_err(|_| invalid)?;
    if claims.iss != crate::account_link::ORIGIN
        || claims.aud != computer
        || !id(&claims.sub)
        || !id(&claims.jti)
        || claims.iat > now.saturating_add(5)
        || claims.exp <= now
        || claims.exp <= claims.iat
        || claims.exp - claims.iat > 60
        || claims.permissions.is_empty()
        || claims.permissions.len() > 7
    {
        return Err(invalid);
    }
    let allowed = [
        "view",
        "audio",
        "gamepad",
        "keyboard",
        "mouse",
        "clipboard",
        "files",
    ];
    let mut permissions = BTreeSet::new();
    for permission in claims.permissions {
        if !allowed.contains(&permission.as_str()) || !permissions.insert(permission) {
            return Err(invalid);
        }
    }
    Ok(VerifiedGrant {
        request: claims.jti,
        guest: claims.sub,
        permissions,
        token_expires: claims.exp,
    })
}

/// One token may admit one session. Call only after online approval and the busy
/// check, immediately before registering that session. Never use as offline auth.
#[derive(Default)]
pub struct ReplayGuard(Mutex<HashMap<String, u64>>);
impl ReplayGuard {
    pub fn claim(&self, grant: &VerifiedGrant, now: u64) -> Result<(), &'static str> {
        if grant.token_expires <= now {
            return Err("Guest grant expired");
        }
        let mut used = self.0.lock().map_err(|_| "Guest admission unavailable")?;
        used.retain(|_, expires| *expires > now);
        if used.len() >= 256 || used.contains_key(&grant.request) {
            return Err("Guest grant unavailable");
        }
        used.insert(grant.request.clone(), grant.token_expires);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const COMPUTER: &str = "00000000-0000-4000-8000-000000000001";
    #[derive(Deserialize)]
    struct Vector {
        keys: Vec<PublicKey>,
        token: String,
    }
    fn fixture() -> Vector {
        serde_json::from_str(include_str!(
            "../../../cloud/accounts/test/fixtures/guest-grant.json"
        ))
        .unwrap()
    }
    fn signed(
        change: impl FnOnce(&mut serde_json::Value, &mut serde_json::Value),
    ) -> (String, Vec<PublicKey>) {
        use base64::engine::general_purpose::STANDARD;
        use ring::{
            rand::SystemRandom,
            signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
        };
        let vector = fixture();
        let parts: Vec<_> = vector.token.split('.').collect();
        let mut header = serde_json::from_slice(&decode(parts[0]).unwrap()).unwrap();
        let mut claims = serde_json::from_slice(&decode(parts[1]).unwrap()).unwrap();
        change(&mut header, &mut claims);
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap()),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        );
        let key = STANDARD
            .decode(
                include_str!("../../../cloud/accounts/test/fixtures/host-key.pem")
                    .lines()
                    .filter(|l| !l.starts_with("-----"))
                    .collect::<String>(),
            )
            .unwrap();
        let rng = SystemRandom::new();
        let signer =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &key, &rng).unwrap();
        (
            format!(
                "{input}.{}",
                URL_SAFE_NO_PAD.encode(signer.sign(&rng, input.as_bytes()).unwrap())
            ),
            vector.keys,
        )
    }
    #[test]
    fn verifies_the_jose_grant_and_rejects_other_hosts_or_expired_tokens() {
        let vector = fixture();
        let grant = verify(&vector.token, COMPUTER, &vector.keys, 1_800_000_010).unwrap();
        assert!(grant.permissions.contains("keyboard"));
        assert!(verify(
            &vector.token,
            "00000000-0000-4000-8000-000000000009",
            &vector.keys,
            1_800_000_010
        )
        .is_err());
        assert!(verify(&vector.token, COMPUTER, &vector.keys, 1_800_000_060).is_err());
        assert!(verify(&vector.token, COMPUTER, &vector.keys, 1_799_999_994).is_err());
    }
    #[test]
    fn refuses_algorithm_confusion_token_keys_and_privileged_claims_even_when_signed() {
        for change in [
            ("alg", serde_json::json!("HS256")),
            ("jku", serde_json::json!("https://elsewhere.invalid/keys")),
            ("crit", serde_json::json!(["extension"])),
            ("kid", serde_json::json!("unknown")),
        ] {
            let (token, keys) = signed(|header, _| header[change.0] = change.1);
            assert!(verify(&token, COMPUTER, &keys, 1_800_000_010).is_err());
        }
        for permissions in [
            serde_json::json!(["unlock"]),
            serde_json::json!(["power"]),
            serde_json::json!(["view", "view"]),
            serde_json::json!([]),
        ] {
            let (token, keys) = signed(|_, claims| claims["permissions"] = permissions);
            assert!(verify(&token, COMPUTER, &keys, 1_800_000_010).is_err());
        }
        let (token, keys) = signed(|_, claims| claims["exp"] = serde_json::json!(1_800_000_061u64));
        assert!(verify(&token, COMPUTER, &keys, 1_800_000_010).is_err());
    }
    #[test]
    fn rejects_tampering_and_duplicate_signing_key_ids() {
        let mut vector = fixture();
        let parts: Vec<_> = vector.token.split('.').collect();
        let mut signature = decode(parts[2]).unwrap();
        signature[0] ^= 1;
        let changed = format!(
            "{}.{}.{}",
            parts[0],
            parts[1],
            URL_SAFE_NO_PAD.encode(signature)
        );
        assert!(verify(&changed, COMPUTER, &vector.keys, 1_800_000_010).is_err());
        vector.keys.push(fixture().keys.remove(0));
        assert!(verify(&vector.token, COMPUTER, &vector.keys, 1_800_000_010).is_err());
    }
    #[test]
    fn admits_only_once_under_concurrent_replay() {
        let guard = std::sync::Arc::new(ReplayGuard::default());
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let guard = guard.clone();
                std::thread::spawn(move || {
                    let vector = fixture();
                    let grant =
                        verify(&vector.token, COMPUTER, &vector.keys, 1_800_000_010).unwrap();
                    guard.claim(&grant, 1_800_000_010).is_ok()
                })
            })
            .collect();
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|admitted| *admitted)
                .count(),
            1
        );
    }
}
