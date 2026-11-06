//! Machine-scoped enrollment for the user's native Linux host.
//! User login sessions remain in the desktop's OS keyring.
use crate::store::{read_private, write_private};
use secpar_host_core::account::{Account, CredentialStorage};
use std::{path::{Path, PathBuf}, sync::Arc};

struct Storage(PathBuf);
impl CredentialStorage for Storage {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        match read_private(&self.0) {
            Ok(bytes) if bytes.len() <= 8192 => Ok(Some(bytes)),
            Ok(_) => Err("Saved host link is invalid".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("Could not read private host account file".into()),
        }
    }
    fn save(&self, bytes: &[u8]) -> Result<(), String> {
        write_private(&self.0, bytes).map_err(|_| "Could not save private host account file".into())
    }
    fn remove(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.0) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not remove saved host account file".into()),
        }
    }
}
pub fn open(directory: &Path, fingerprint: &str) -> Result<Arc<Account>, String> {
    Account::open(Arc::new(Storage(directory.join("host-account.json"))), fingerprint)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn machine_file_is_private_atomic_and_rejects_public_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage(dir.path().join("host-account.json"));
        storage.save(b"disposable-machine-fixture").unwrap();
        assert_eq!(std::fs::metadata(&storage.0).unwrap().permissions().mode() & 0o777,0o600);
        assert_eq!(storage.load().unwrap().unwrap(),b"disposable-machine-fixture");
        storage.save(b"replacement-machine-fixture").unwrap();
        assert_eq!(storage.load().unwrap().unwrap(),b"replacement-machine-fixture");
        std::fs::set_permissions(&storage.0,std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(storage.load().is_err());
        storage.remove().unwrap(); assert!(storage.load().unwrap().is_none());
    }
}
