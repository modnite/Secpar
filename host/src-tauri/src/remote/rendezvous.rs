//! The host's one WebSocket to the rendezvous Worker. The client, framing and reconnect logic
//! are shared with every desktop host (`secpar_host_core::rendezvous`); this adapts the Windows
//! store to it.

use crate::agent::AgentHandle;
use crate::store::Store;
use std::sync::{Arc, Mutex};

pub use secpar_host_core::rendezvous::{bin_env, close_env, connect, revoke_env, text_env, Ws};

impl secpar_host_core::rendezvous::RelaySource for Store {
    fn pc_id(&self) -> String {
        Store::pc_id(self).to_string()
    }
    fn relay_key(&self) -> Vec<u8> {
        Store::relay_key(self)
    }
    fn relay_registered(&self) -> bool {
        Store::relay_registered(self)
    }
    fn set_relay_registered(&mut self) -> std::io::Result<()> {
        Store::set_relay_registered(self)
    }
    fn revoked(&self) -> Vec<String> {
        Store::revoked(self)
    }
}

/// `<base>/v1/host?pc=..&ts=..&sig=..[&register=..]`
pub fn host_url(base: &str, store: &Store, ts: u64, register: bool) -> String {
    secpar_host_core::rendezvous::host_url(base, store.pc_id(), &store.relay_key(), ts, register)
}

/// Keeps the Worker connection up for the life of the process.
pub async fn run(agent: AgentHandle, store: Arc<Mutex<Store>>, base: String) {
    secpar_host_core::rendezvous::run(agent, store, base, crate::log::line).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::b64url;

    #[test]
    fn host_url_signs_and_registers() {
        let dir = std::env::temp_dir().join(format!("pcr-rdv-url-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir).unwrap();
        let (pc, key) = (s.pc_id().to_string(), s.relay_key());
        let u = host_url("wss://w.example", &s, 1_700_000_000, false);
        assert_eq!(u, format!("wss://w.example/v1/host?pc={pc}&ts=1700000000&sig={}", super::super::host_sig(&key, &pc, 1_700_000_000)));
        assert!(host_url("wss://w.example", &s, 1, true).ends_with(&format!("&register={}", b64url(&key))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
