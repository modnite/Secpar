//! Authentication contract shared with the rendezvous Worker.
//! Keep host signatures and device tickets in separate HMAC domains.

fn sign(key: &[u8], message: &str) -> [u8; 32] {
    ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key),
        message.as_bytes(),
    )
    .as_ref()
    .try_into()
    .expect("SHA256 length")
}

pub fn host_signature(key: &[u8], pc: &str, timestamp: u64) -> [u8; 32] {
    sign(key, &format!("host:{pc}:{timestamp}"))
}

pub fn device_ticket(key: &[u8], pc: &str, device: &str) -> [u8; 32] {
    sign(key, &format!("ticket:{pc}:{device}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_independent_worker_vectors_and_separates_identities() {
        // Node crypto.createHmac vectors, also used by the Windows host contract.
        let key = [7; 32];
        assert_eq!(
            crate::hex(&host_signature(&key, "pc1", 1_700_000_000)),
            "9e9f4441bcfd13a524fd922878e5e207cc90974f6f56c46410c2b866a1cc63c1"
        );
        assert_eq!(
            crate::hex(&device_ticket(&key, "pc1", "dev1")),
            "029bbe741b2c22d8aaced498d91ee5c01f746208275adb6a1bce457a2e95433b"
        );
        assert_ne!(
            device_ticket(&key, "pc1", "dev1"),
            device_ticket(&key, "pc2", "dev1")
        );
        assert_ne!(
            device_ticket(&key, "pc1", "dev1"),
            device_ticket(&key, "pc1", "dev2")
        );
        assert_ne!(
            host_signature(&key, "pc1", 1),
            device_ticket(&key, "pc1", "1")
        );
    }
}
