//! Access from anywhere (docs/remote-protocol.md): the rendezvous Worker connection and STUN.

pub mod rendezvous;
pub mod stun;
pub mod turn;

/// The Worker's base URL; `PCR_RENDEZVOUS_URL` at build time overrides it (empty turns it off).
pub fn url() -> Option<&'static str> {
    secpar_host_core::rendezvous::url()
}

/// `sig = HMAC-SHA256(relayKey, "host:" + pc + ":" + ts)`, hex.
pub fn host_sig(relay_key: &[u8], pc: &str, ts: u64) -> String {
    secpar_host_core::rendezvous::host_sig(relay_key, pc, ts)
}

/// Fetches this host's TURN credentials from the Worker (`/v1/turn?role=host`) and returns the
/// first UDP TURN server. Blocking; call off the async runtime's worker threads.
pub fn turn_server(store: &std::sync::Mutex<crate::store::Store>) -> Option<(std::net::SocketAddr, String, String)> {
    let base = url()?;
    let (key, pc) = {
        let s = store.lock().unwrap();
        (s.relay_key(), s.pc_id().to_string())
    };
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let url = secpar_host_core::relay::turn_url(base, &pc, ts, &host_sig(&key, &pc, ts));
    match secpar_host_core::relay::fetch_turn_blocking(&url) {
        Ok(s) => Some((s.addr, s.user, s.pass)),
        Err(e) => {
            crate::log::line(&format!("turn: {e}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_sig_is_hmac_hex() {
        // Expected values from Node's crypto.createHmac, independent of ring.
        let key = [7u8; 32];
        let sig = host_sig(&key, "pc1", 1_700_000_000);
        assert_eq!(sig.len(), 64);
        assert_eq!(sig, crate::tls::hex(&crate::store::hmac(&key, "host:pc1:1700000000")));
        assert_eq!(sig, "9e9f4441bcfd13a524fd922878e5e207cc90974f6f56c46410c2b866a1cc63c1");
        // ticket, as the phone and Worker derive it (Node crypto)
        assert_eq!(crate::store::b64url(&crate::store::hmac(&key, "ticket:pc1:dev1")), "Apu-dBssItiqztSY2R7lwB90YggnWttqG85Fei6VQzs");
    }
}
