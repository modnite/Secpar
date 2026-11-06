//! Windows adapter for the shared desktop hosting control engine.
#[cfg(test)]
mod tests;
pub use secpar_host_core::agent::{
    AgentHandle, BoxFut, Event, Handlers, Outbox, Session, MAX_FRAME,
};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

impl secpar_host_core::IdentitySource for crate::tls::Identity {
    fn certificate(&self) -> &[u8] {
        &self.cert_der
    }
    fn private_key(&self) -> &[u8] {
        &self.key_der
    }
}
impl secpar_host_core::PairingStore for crate::store::Store {
    fn take_otp(&mut self, otp: &str) -> bool {
        self.take_otp(otp)
    }
    fn add_device(&mut self, name: &str) -> std::io::Result<(String, String)> {
        self.add_device(name)
    }
    fn pair_device(&mut self, otp: &str, name: &str) -> Result<(String, String), &'static str> {
        self.pair_device(otp, name)
    }
    fn auth(&self, token: &str) -> Result<String, &'static str> {
        self.auth(token)
    }
    fn revoke(&mut self, device: &str) -> std::io::Result<()> {
        self.revoke(device)
    }
    fn ticket(&self, device: &str) -> [u8; 32] {
        self.ticket(device)
    }
}
pub struct Agent;
impl Agent {
    pub async fn start(
        identity: &crate::tls::Identity,
        store: Arc<Mutex<crate::store::Store>>,
        handlers: Arc<dyn Handlers>,
        bind: SocketAddr,
        idle: Duration,
    ) -> std::io::Result<AgentHandle> {
        secpar_host_core::agent::Agent::start(
            identity,
            store,
            handlers,
            bind,
            idle,
            crate::remote::url().map(str::to_owned),
        )
        .await
    }
}
