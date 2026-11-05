//! Secrets live in the OS credential store, never the UI settings or logs.
const SERVICE: &str = "com.secpar.desktop";

pub fn load(name: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(SERVICE, name).map_err(|_| unavailable())?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}
pub fn save(name: &str, value: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, name)
        .and_then(|entry| entry.set_password(value))
        .map_err(|_| unavailable())
}
pub fn forget(name: &str) -> Result<(), String> {
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
