//! Secrets live in the Windows Credential Manager, or on Linux in a file only the user can read;
//! never in the UI settings or logs. Linux doesn't use KWallet or GNOME Keyring: a locked wallet
//! prompts, and a waiting prompt holds up every other program that uses it, browsers included.
#[cfg(not(target_os = "linux"))]
const SERVICE: &str = "com.secpar.desktop";

/// What this run saved or read: None for an item the store doesn't have. Each item is read from the
/// store once per run. Background loops look items up every second, and on Linux every read opens
/// a new secret service session and asks KWallet or GNOME Keyring to unlock a locked item, which can
/// leave a prompt waiting that holds up every other program using the store (Chromium's cookies).
/// When the store refuses a save, the app keeps using what it already holds.
#[derive(Default)]
struct Held {
    items: std::collections::HashMap<String, Option<String>>,
    /// The last read that failed: the store isn't asked again for a while.
    failed: Option<std::time::Instant>,
}
const RETRY: std::time::Duration = std::time::Duration::from_secs(60);

fn session() -> std::sync::MutexGuard<'static, Held> {
    static HELD: std::sync::OnceLock<std::sync::Mutex<Held>> = std::sync::OnceLock::new();
    HELD.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}
pub fn load(name: &str) -> Result<Option<String>, String> {
    {
        let held = session();
        if let Some(item) = held.items.get(name) {
            return Ok(item.clone());
        }
        if held.failed.is_some_and(|at| at.elapsed() < RETRY) {
            return Err(unavailable());
        }
    }
    // Read without holding the lock: an unlock prompt can take as long as the user does.
    let result = read(name);
    let mut held = session();
    match result {
        Ok(item) => {
            held.failed = None;
            Ok(held.items.entry(name.into()).or_insert(item).clone())
        }
        Err(note) => {
            held.failed = Some(std::time::Instant::now());
            Err(note)
        }
    }
}
#[cfg(target_os = "linux")]
use file::{delete, read, write};

#[cfg(not(target_os = "linux"))]
fn read(name: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(SERVICE, name).map_err(|_| unavailable())?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}
#[cfg(not(target_os = "linux"))]
fn write(name: &str, value: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, name)
        .and_then(|entry| entry.set_password(value))
        .map_err(|_| unavailable())
}
#[cfg(not(target_os = "linux"))]
fn delete(name: &str) -> Result<(), String> {
    match keyring::Entry::new(SERVICE, name).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(unavailable()),
    }
}
pub fn save(name: &str, value: &str) -> Result<(), String> {
    session().items.insert(name.into(), Some(value.into()));
    write(name, value)
}
pub fn forget(name: &str) -> Result<(), String> {
    session().items.insert(name.into(), None);
    delete(name)
}
#[cfg(not(target_os = "linux"))]
fn unavailable() -> String {
    "Secure storage is unavailable. Unlock your system keychain to remember this device.".into()
}
#[cfg(target_os = "linux")]
fn unavailable() -> String {
    "Secpar could not read or save its key file (~/.local/share/secpar/secrets.json).".into()
}

/// `$XDG_DATA_HOME/secpar/secrets.json` (normally `~/.local/share`): one JSON object of names and
/// values, the folder 0700 and the file 0600. Saves go to a new file that replaces the old one.
#[cfg(target_os = "linux")]
mod file {
    use super::unavailable;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    /// One change at a time from this process.
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn path() -> Result<PathBuf, String> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .ok_or_else(unavailable)?;
        Ok(base.join("secpar/secrets.json"))
    }

    pub(super) fn all(path: &Path) -> Result<BTreeMap<String, String>, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| unavailable()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(_) => Err(unavailable()),
        }
    }

    pub(super) fn store(path: &Path, items: &BTreeMap<String, String>) -> Result<(), String> {
        let dir = path.parent().ok_or_else(unavailable)?;
        std::fs::create_dir_all(dir).map_err(|_| unavailable())?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        let fresh = path.with_extension("json.new");
        let bytes = serde_json::to_vec(items).map_err(|_| unavailable())?;
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&fresh).map_err(|_| unavailable())?;
        // The mode only applies to a new file; a leftover from an interrupted save is fixed here.
        file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(|_| unavailable())?;
        file.write_all(&bytes).and_then(|()| file.sync_all()).map_err(|_| unavailable())?;
        std::fs::rename(&fresh, path).map_err(|_| unavailable())
    }

    pub(super) fn change(path: &Path, name: &str, value: Option<&str>) -> Result<(), String> {
        let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        let mut items = all(path)?;
        let changed = match value {
            Some(v) => items.insert(name.into(), v.into()).as_deref() != Some(v),
            None => items.remove(name).is_some(),
        };
        if changed { store(path, &items) } else { Ok(()) }
    }

    pub fn read(name: &str) -> Result<Option<String>, String> {
        Ok(all(&path()?)?.remove(name))
    }
    pub fn write(name: &str, value: &str) -> Result<(), String> {
        change(&path()?, name, Some(value))
    }
    pub fn delete(name: &str) -> Result<(), String> {
        change(&path()?, name, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn the_key_file_keeps_values_and_only_the_user_reads_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("secpar-vault-{}", std::process::id()));
        let path = dir.join("secpar/secrets.json");
        file::change(&path, "a", Some("one")).unwrap();
        file::change(&path, "b", Some("two")).unwrap();
        file::change(&path, "a", None).unwrap();
        let items = file::all(&path).unwrap();
        assert_eq!((items.get("a"), items.get("b").map(String::as_str)), (None, Some("two")));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "Requires an unlocked OS credential store"]
    fn real_system_credential_roundtrip() {
        let name = format!(
            "validation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        save(&name, "Secpar disposable storage check").unwrap();
        let loaded = load(&name);
        let removed = forget(&name);
        assert_eq!(
            loaded.unwrap().as_deref(),
            Some("Secpar disposable storage check")
        );
        removed.unwrap();
        assert!(load(&name).unwrap().is_none());
    }
}
