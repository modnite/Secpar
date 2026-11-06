//! Windows machine credential storage; networking/proofs are shared with Linux.
use crate::win::dpapi;
use std::{path::{Path, PathBuf}, sync::Arc};
pub use secpar_host_core::account::{Account, Registration, State};
use secpar_host_core::account::CredentialStorage;
pub fn open(dir: &Path, fingerprint: &str) -> Result<Arc<Account>, String> {
    Account::open(Arc::new(Storage(dir.join("host-account.bin"))), fingerprint)
}
struct Storage(PathBuf);
impl CredentialStorage for Storage {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(&self.0) {
            Ok(bytes) if bytes.len() <= 16384 => dpapi::unprotect(&bytes).map(Some).map_err(|_| "Could not unlock saved host link".into()),
            Ok(_) => Err("Saved host link is invalid".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("Could not read saved host link".into()),
        }
    }
    fn save(&self, plain: &[u8]) -> Result<(), String> {
        let encrypted = dpapi::protect(plain).map_err(|_| "Could not protect host link")?;
        let temp = self.0.with_extension("tmp");
        std::fs::write(&temp, encrypted).and_then(|_| std::fs::rename(&temp, &self.0)).map_err(|_| "Could not save host link".into())
    }
    fn remove(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.0) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not remove saved host credential".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn machine_storage_preserves_dpapi_format_and_replaces_then_removes() {
        let dir = std::env::temp_dir().join(format!("secpar-machine-storage-{}", crate::tls::hex(&crate::store::random::<8>())));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = Storage(dir.join("host-account.bin"));
        let plain = br#"{"credential":"h1_disposable-machine-fixture"}"#;
        storage.save(plain).unwrap();
        let bytes = std::fs::read(&storage.0).unwrap();
        assert!(!bytes.windows(plain.len()).any(|window| window == plain));
        assert_eq!(storage.load().unwrap().unwrap(), plain);
        storage.save(plain).unwrap();
        storage.remove().unwrap(); assert!(storage.load().unwrap().is_none());
        std::fs::remove_dir(dir).unwrap();
    }
}
