//! Where secrets are kept. The app uses the system credential store; tests and
//! other platforms supply their own.
use std::{collections::HashMap, sync::Mutex};

/// The store could not be read or written. This is never the same as "nothing
/// is stored": callers must not create new secrets because of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable;

pub trait SecretStore: Send + Sync {
    /// `Ok(None)` only when the store answered and holds nothing under `name`.
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, Unavailable>;
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), Unavailable>;
    fn forget(&self, name: &str) -> Result<(), Unavailable>;
}

/// The operating system's credential store: Credential Manager on Windows, the
/// Secret Service (KWallet, GNOME Keyring) on Linux. Separate from Secpar 3's entries.
#[cfg(not(target_os = "android"))]
pub struct SystemStore;
#[cfg(not(target_os = "android"))]
const SERVICE: &str = "win.secpar.app";

#[cfg(not(target_os = "android"))]
impl SecretStore for SystemStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, Unavailable> {
        let entry = keyring::Entry::new(SERVICE, name).map_err(|_| Unavailable)?;
        match entry.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(Unavailable),
        }
    }
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), Unavailable> {
        keyring::Entry::new(SERVICE, name)
            .and_then(|entry| entry.set_secret(secret))
            .map_err(|_| Unavailable)
    }
    fn forget(&self, name: &str) -> Result<(), Unavailable> {
        match keyring::Entry::new(SERVICE, name).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(Unavailable),
        }
    }
}

/// Keeps each secret as a file in one folder. For Android, where the folder is
/// the app's private storage that no other app can read.
pub struct FileStore(pub std::path::PathBuf);
impl FileStore {
    fn path(&self, name: &str) -> Result<std::path::PathBuf, Unavailable> {
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(Unavailable);
        }
        Ok(self.0.join(format!("{name}.secret")))
    }
}
impl SecretStore for FileStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, Unavailable> {
        match std::fs::read(self.path(name)?) {
            Ok(secret) => Ok(Some(secret)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(Unavailable),
        }
    }
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), Unavailable> {
        let path = self.path(name)?;
        let part = path.with_extension("part");
        std::fs::create_dir_all(&self.0).and_then(|_| std::fs::write(&part, secret)).and_then(|_| std::fs::rename(&part, &path)).map_err(|_| Unavailable)
    }
    fn forget(&self, name: &str) -> Result<(), Unavailable> {
        match std::fs::remove_file(self.path(name)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(Unavailable),
        }
    }
}

/// Keeps secrets in memory. For tests; `fail_reads` and `fail_writes` imitate a
/// locked or broken credential store.
#[derive(Default)]
pub struct MemoryStore {
    secrets: Mutex<HashMap<String, Vec<u8>>>,
    pub fail_reads: std::sync::atomic::AtomicBool,
    pub fail_writes: std::sync::atomic::AtomicBool,
}
impl SecretStore for MemoryStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, Unavailable> {
        if self.fail_reads.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Unavailable);
        }
        Ok(self.secrets.lock().unwrap().get(name).cloned())
    }
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), Unavailable> {
        if self.fail_writes.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Unavailable);
        }
        self.secrets.lock().unwrap().insert(name.into(), secret.to_vec());
        Ok(())
    }
    fn forget(&self, name: &str) -> Result<(), Unavailable> {
        if self.fail_writes.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Unavailable);
        }
        self.secrets.lock().unwrap().remove(name);
        Ok(())
    }
}
