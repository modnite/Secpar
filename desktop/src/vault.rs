//! Secrets live in the OS credential store, never the UI settings or logs.
const SERVICE: &str = "com.secpar.desktop";

/// What this run saved or read. When the credential store stops answering, or refuses a save, the
/// app keeps using what it already holds instead of treating the secret as gone.
fn session() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, String>> {
    static HELD: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();
    HELD.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}
pub fn load(name: &str) -> Result<Option<String>, String> {
    match read(name) {
        Ok(Some(value)) => { session().insert(name.into(), value.clone()); Ok(Some(value)) }
        Ok(None) => Ok(session().get(name).cloned()),
        Err(note) => session().get(name).cloned().map(|value| Ok(Some(value))).unwrap_or(Err(note)),
    }
}
fn read(name: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(SERVICE, name).map_err(|_| unavailable())?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}
pub fn save(name: &str, value: &str) -> Result<(), String> {
    session().insert(name.into(), value.into());
    keyring::Entry::new(SERVICE, name)
        .and_then(|entry| entry.set_password(value))
        .map_err(|_| unavailable())
}
pub fn forget(name: &str) -> Result<(), String> {
    session().remove(name);
    match keyring::Entry::new(SERVICE, name).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(unavailable()),
    }
}
fn unavailable() -> String {
    "Secure storage is unavailable. Unlock your system keychain to remember this device.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
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
