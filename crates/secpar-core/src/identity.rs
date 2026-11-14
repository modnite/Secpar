//! The device's permanent identity: one certificate and key, created on first run
//! and kept in the credential store. The account service knows a device by the
//! certificate's fingerprint, so a second identity would be a second device.
//!
//! Rules, from the Secpar 4 plan:
//! - A store that cannot be read is an error to show the user. It never leads to
//!   a new identity.
//! - A new identity is only used once it has been saved.
//! - The fingerprint is also written to a small plain file. If that file exists
//!   but the store holds no identity, the key is really gone: the caller must ask
//!   the user before `reset` makes this installation a new device.
use crate::store::{SecretStore, Unavailable};
use secpar_tunnel::{Fingerprint, Identity};
use std::path::Path;

const NAME: &str = "device-identity";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The credential store could not be read or written. Try again once it is unlocked.
    StoreUnavailable,
    /// This installation had the identity with this fingerprint, and the store no longer holds it.
    Missing { previous: String },
    /// The stored identity, or the fingerprint file, cannot be read as one.
    Damaged,
    /// The fingerprint file could not be written.
    MarkerNotWritten,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::StoreUnavailable => "Secure storage is unavailable. Unlock your system keychain and try again.",
            Error::Missing { .. } => "This device's key is no longer in secure storage.",
            Error::Damaged => "This device's saved key is damaged.",
            Error::MarkerNotWritten => "Secpar could not write to its settings folder.",
        })
    }
}
impl std::error::Error for Error {}
impl From<Unavailable> for Error {
    fn from(_: Unavailable) -> Self {
        Error::StoreUnavailable
    }
}

pub fn hex(fingerprint: &Fingerprint) -> String {
    fingerprint.iter().map(|b| format!("{b:02x}")).collect()
}

fn encode(identity: &Identity) -> Vec<u8> {
    let certificate = identity.certificate_der();
    let mut out = (certificate.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(certificate);
    out.extend_from_slice(identity.key_der());
    out
}
fn decode(secret: &[u8]) -> Option<Identity> {
    let length = u32::from_be_bytes(secret.get(..4)?.try_into().ok()?) as usize;
    let certificate = secret.get(4..4usize.checked_add(length)?)?;
    let key = secret.get(4 + length..)?;
    if certificate.is_empty() || key.is_empty() {
        return None;
    }
    Some(Identity::from_der(certificate.to_vec(), key.to_vec()))
}

fn read_marker(marker: &Path) -> Result<Option<String>, Error> {
    match std::fs::read_to_string(marker) {
        Ok(text) => {
            let text = text.trim();
            if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
                Ok(Some(text.to_ascii_lowercase()))
            } else {
                Err(Error::Damaged)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::Damaged),
    }
}

fn create(store: &dyn SecretStore, marker: &Path) -> Result<Identity, Error> {
    let identity = Identity::generate().map_err(|_| Error::Damaged)?;
    store.save(NAME, &encode(&identity))?;
    // Read it back: an identity the store did not keep must not be used.
    if store.load(NAME)?.as_deref() != Some(encode(&identity).as_slice()) {
        return Err(Error::StoreUnavailable);
    }
    if let Some(folder) = marker.parent() {
        std::fs::create_dir_all(folder).map_err(|_| Error::MarkerNotWritten)?;
    }
    std::fs::write(marker, hex(&identity.fingerprint())).map_err(|_| Error::MarkerNotWritten)?;
    Ok(identity)
}

/// This device's identity, created on the first run. `marker` is the plain file
/// that records the fingerprint, inside Secpar's settings folder.
pub fn device(store: &dyn SecretStore, marker: &Path) -> Result<Identity, Error> {
    let previous = read_marker(marker)?;
    match store.load(NAME)? {
        Some(secret) => {
            let identity = decode(&secret).ok_or(Error::Damaged)?;
            let current = hex(&identity.fingerprint());
            if previous.as_deref() != Some(current.as_str()) {
                // The store is the truth; repair the note beside it.
                std::fs::write(marker, &current).map_err(|_| Error::MarkerNotWritten)?;
            }
            Ok(identity)
        }
        None => match previous {
            Some(previous) => Err(Error::Missing { previous }),
            None => create(store, marker),
        },
    }
}

/// Make this installation a new device. Only after the user agreed: the old
/// device record stays on the account until they remove it there.
pub fn reset(store: &dyn SecretStore, marker: &Path) -> Result<Identity, Error> {
    store.forget(NAME)?;
    match std::fs::remove_file(marker) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::MarkerNotWritten),
    }
    create(store, marker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use std::sync::atomic::Ordering::Relaxed;

    fn setup() -> (MemoryStore, tempfile::TempDir, std::path::PathBuf) {
        let folder = tempfile::tempdir().unwrap();
        let marker = folder.path().join("settings").join("device");
        (MemoryStore::default(), folder, marker)
    }

    #[test]
    fn the_first_run_creates_one_identity_and_later_runs_return_it() {
        let (store, _folder, marker) = setup();
        let first = device(&store, &marker).unwrap();
        let second = device(&store, &marker).unwrap();
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), hex(&first.fingerprint()));
    }

    #[test]
    fn an_unreadable_store_is_an_error_and_creates_nothing() {
        let (store, _folder, marker) = setup();
        let first = device(&store, &marker).unwrap();
        store.fail_reads.store(true, Relaxed);
        assert_eq!(device(&store, &marker).err(), Some(Error::StoreUnavailable));
        store.fail_reads.store(false, Relaxed);
        assert_eq!(device(&store, &marker).unwrap().fingerprint(), first.fingerprint());
    }

    #[test]
    fn a_first_run_that_cannot_save_gives_no_identity() {
        let (store, _folder, marker) = setup();
        store.fail_writes.store(true, Relaxed);
        assert_eq!(device(&store, &marker).err(), Some(Error::StoreUnavailable));
        assert!(!marker.exists());
        store.fail_writes.store(false, Relaxed);
        assert!(device(&store, &marker).is_ok());
    }

    #[test]
    fn a_lost_key_is_reported_and_only_reset_makes_a_new_device() {
        let (store, _folder, marker) = setup();
        let first = device(&store, &marker).unwrap();
        store.forget(NAME).unwrap();
        assert_eq!(
            device(&store, &marker).err(),
            Some(Error::Missing { previous: hex(&first.fingerprint()) })
        );
        let replaced = reset(&store, &marker).unwrap();
        assert_ne!(replaced.fingerprint(), first.fingerprint());
        assert_eq!(device(&store, &marker).unwrap().fingerprint(), replaced.fingerprint());
    }

    #[test]
    fn a_damaged_secret_is_not_replaced_silently() {
        let (store, _folder, marker) = setup();
        device(&store, &marker).unwrap();
        store.save(NAME, b"not an identity").unwrap();
        assert_eq!(device(&store, &marker).err(), Some(Error::Damaged));
    }

    #[test]
    fn the_identity_works_as_a_tunnel_identity() {
        let (store, _folder, marker) = setup();
        let identity = device(&store, &marker).unwrap();
        // The same bytes come back, so the certificate and key still belong together.
        let again = decode(&encode(&identity)).unwrap();
        assert_eq!(again.certificate_der(), identity.certificate_der());
        assert_eq!(again.key_der(), identity.key_der());
    }

    #[test]
    #[ignore = "Uses the real system credential store"]
    fn real_system_store_round_trip() {
        use crate::store::SystemStore;
        let name = format!("validation-{}", std::process::id());
        SystemStore.save(&name, b"disposable check").unwrap();
        let loaded = SystemStore.load(&name);
        SystemStore.forget(&name).unwrap();
        assert_eq!(loaded.unwrap().as_deref(), Some(&b"disposable check"[..]));
        assert_eq!(SystemStore.load(&name).unwrap(), None);
    }
}
