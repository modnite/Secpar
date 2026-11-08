//! Platform-neutral control/pairing engine shared by every Secpar desktop host.
pub mod agent;
pub mod account;
pub mod account_link;
pub mod audio;
pub mod files;
pub mod guest_policy;
pub mod limits;
pub mod media;
pub mod pad;
pub mod pairing_codes;
pub mod proto;
pub mod relay;
pub mod rendezvous;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub primary: bool,
    #[serde(skip)]
    pub rect: [i32; 4],
}

/// The platform owns persistence/encryption of its host TLS identity.
pub trait IdentitySource: Send + Sync {
    fn certificate(&self) -> &[u8];
    fn private_key(&self) -> &[u8];
}

/// Owner-device credentials only. Guest admission never calls add_device.
pub trait PairingStore: Send {
    fn take_otp(&mut self, otp: &str) -> bool;
    fn add_device(&mut self, name: &str) -> std::io::Result<(String, String)>;
    /// Authorization and device creation run under one store lock, so concurrent
    /// pairings cannot exchange the account context of their one-use codes.
    fn pair_device(&mut self, otp: &str, name: &str) -> Result<(String, String), &'static str> {
        if !self.take_otp(otp) { return Err("bad_otp"); }
        self.add_device(name).map_err(|_| "internal")
    }
    fn auth(&self, token: &str) -> Result<String, &'static str>;
    fn revoke(&mut self, device: &str) -> std::io::Result<()>;
    fn ticket(&self, device: &str) -> [u8; 32];
}
