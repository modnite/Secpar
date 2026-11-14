//! Who may connect to this hosting device right now.
//!
//! The answer comes from the signed access list of the account service and from
//! nowhere else. The last list is kept in a file together with the service's
//! public keys and checked again when it is loaded, so connections on the LAN
//! keep working while the service is unreachable, until the list expires.
//!
//! The file sits in the hosting part's own data folder. Someone who can write
//! there already controls the hosting part, so the saved keys are trusted as far
//! as that folder is.
use crate::account::{verify_access, AccessList, Error, Key, Result, Service};
use secpar_tunnel::{Fingerprint, Identity};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};

/// What the hosting device knows about a device that is allowed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    /// Another device of the same account. Full control.
    Own { name: String },
    /// A friend's device, with what that friend may use.
    Guest { name: String, account: String, handle: String, keyboard: bool, mouse: bool },
}

/// What a refresh from the account service led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refreshed {
    /// A new list is in use.
    Updated,
    /// The service could not be reached or answered badly. The saved list stays in use.
    Kept(Error),
    /// The service no longer knows this device as a host: nobody is allowed in.
    Removed,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    access: String,
    keys: Vec<Key>,
}
#[derive(Default)]
struct State {
    list: Option<AccessList>,
    /// Friends who are asked about each time, accepted until the given time.
    accepted: HashMap<Fingerprint, u64>,
}

pub struct Access {
    origin: String,
    host: Fingerprint,
    file: PathBuf,
    state: Mutex<State>,
}
impl Access {
    /// Loads the saved list for the hosting device `host`, if there is one that
    /// still checks out at `at` (seconds since 1970). Otherwise nobody is allowed
    /// in until `update` or `refresh` succeeds.
    pub fn open(origin: &str, host: Fingerprint, file: PathBuf, at: u64) -> Self {
        let list = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .and_then(|saved| verify_access(&saved.access, &saved.keys, origin, &host, at).ok());
        Self { origin: origin.into(), host, file, state: Mutex::new(State { list, accepted: HashMap::new() }) }
    }

    /// Takes a signed list into use after checking it, and saves it. A list that
    /// does not check out changes nothing.
    pub fn update(&self, access: &str, keys: &[Key], at: u64) -> Result<()> {
        let list = verify_access(access, keys, &self.origin, &self.host, at)?;
        let bytes = serde_json::to_vec(&Saved { access: access.into(), keys: keys.to_vec() }).map_err(|_| Error::Invalid("The access list could not be saved"))?;
        let mut state = self.state.lock().unwrap();
        // Accepted friends stay accepted only while the new list still has them as guests.
        state.accepted.retain(|device, _| list.guests.iter().any(|g| &g.fingerprint == device));
        state.list = Some(list);
        drop(state);
        // In use even if saving fails; the next refresh saves again.
        let part = self.file.with_extension("part");
        if std::fs::write(&part, bytes).and_then(|_| std::fs::rename(&part, &self.file)).is_err() {
            let _ = std::fs::remove_file(&part);
        }
        Ok(())
    }

    /// Forgets the list: nobody is allowed in. For a device removed from the account.
    pub fn clear(&self) {
        *self.state.lock().unwrap() = State::default();
        let _ = std::fs::remove_file(&self.file);
    }

    /// The owner accepted a friend who is asked about each time. Returns false,
    /// and changes nothing, unless the device is a friend's device on the list.
    pub fn accept(&self, device: &Fingerprint, until: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        let listed = state.list.as_ref().is_some_and(|l| l.guests.iter().any(|g| &g.fingerprint == device));
        if listed {
            state.accepted.insert(*device, until);
        }
        listed
    }
    /// Ends an acceptance early, for example when the owner disconnects a guest.
    pub fn dismiss(&self, device: &Fingerprint) {
        self.state.lock().unwrap().accepted.remove(device);
    }

    /// The device, if it may connect at `at`: one of the owner's devices, a
    /// trusted friend's device, or a friend's device accepted for now.
    pub fn peer(&self, device: &Fingerprint, at: u64) -> Option<Peer> {
        let state = self.state.lock().unwrap();
        let list = state.list.as_ref().filter(|l| l.expires_at > at)?;
        if let Some(own) = list.devices.iter().find(|d| &d.fingerprint == device) {
            return Some(Peer::Own { name: own.name.clone() });
        }
        let guest = list.guests.iter().find(|g| &g.fingerprint == device)?;
        let accepted = state.accepted.get(device).is_some_and(|until| *until > at);
        (guest.trusted || accepted).then(|| Peer::Guest {
            name: guest.name.clone(),
            account: guest.account.clone(),
            handle: guest.handle.clone(),
            keyboard: guest.keyboard,
            mouse: guest.mouse,
        })
    }

    /// Every device that may connect at `at`. This is what the tunnel is given.
    pub fn allowed(&self, at: u64) -> Vec<Fingerprint> {
        let known: Vec<Fingerprint> = {
            let state = self.state.lock().unwrap();
            let Some(list) = state.list.as_ref() else { return Vec::new() };
            list.devices.iter().map(|d| d.fingerprint).chain(list.guests.iter().map(|g| g.fingerprint)).collect()
        };
        known.into_iter().filter(|device| self.peer(device, at).is_some()).collect()
    }

    /// Every device the list names, whether or not it may connect right now, and
    /// the account that owns this hosting device. `None` when there is no list.
    pub fn listed(&self) -> Option<(String, Vec<Fingerprint>)> {
        let state = self.state.lock().unwrap();
        let list = state.list.as_ref()?;
        Some((list.owner.clone(), list.devices.iter().map(|d| d.fingerprint).chain(list.guests.iter().map(|g| g.fingerprint)).collect()))
    }

    /// When the list in use stops being valid, if there is one.
    pub fn expires_at(&self) -> Option<u64> {
        self.state.lock().unwrap().list.as_ref().map(|l| l.expires_at)
    }

    /// Asks the account service for a new list with the device's own key.
    pub async fn refresh(&self, service: &Service, identity: &Identity) -> Refreshed {
        let fetched = async {
            let keys = service.keys().await?;
            let (access, list) = service.device_access(identity, &keys).await?;
            Ok::<_, Error>((access, keys, list))
        };
        match fetched.await {
            // Checked a moment ago against the service's own time; use a time inside the list's life here too.
            Ok((access, keys, list)) => match self.update(&access, &keys, list.expires_at - 1) {
                Ok(()) => Refreshed::Updated,
                Err(error) => Refreshed::Kept(error),
            },
            Err(Error::Refused { status: 404, .. }) => {
                self.clear();
                Refreshed::Removed
            }
            Err(error) => Refreshed::Kept(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{account::ORIGIN, identity::hex};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ring::{
        rand::SystemRandom,
        signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
    };

    const HOST: Fingerprint = [7; 32];
    const LAPTOP: Fingerprint = [0x11; 32];
    const TRUSTED: Fingerprint = [0x22; 32];
    const ASKED: Fingerprint = [0x33; 32];

    /// A stand-in for the account service's signing key.
    fn signer() -> (EcdsaKeyPair, Key) {
        let rng = SystemRandom::new();
        let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref(), &rng).unwrap();
        let point = pair.public_key().as_ref().to_vec();
        let key = Key { kid: "k1".into(), kty: "EC".into(), crv: "P-256".into(), x: URL_SAFE_NO_PAD.encode(&point[1..33]), y: URL_SAFE_NO_PAD.encode(&point[33..]) };
        (pair, key)
    }
    fn guest(device: &Fingerprint, handle: &str, trusted: bool) -> serde_json::Value {
        serde_json::json!({"fingerprint": hex(device), "name": "Phone", "account": format!("id-{handle}"), "handle": handle, "trusted": trusted, "keyboard": true, "mouse": false})
    }
    fn list(pair: &EcdsaKeyPair, devices: &[Fingerprint], guests: Vec<serde_json::Value>) -> String {
        let header = serde_json::json!({"alg": "ES256", "typ": "secpar-access+jwt", "kid": "k1"});
        let claims = serde_json::json!({
            "iss": ORIGIN, "aud": "secpar-host", "sub": hex(&HOST), "iat": 1000, "exp": 2000, "owner": "id-1",
            "devices": devices.iter().map(|d| serde_json::json!({"fingerprint": hex(d), "name": "Laptop"})).collect::<Vec<_>>(),
            "guests": guests,
        });
        let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
        let signature = pair.sign(&SystemRandom::new(), input.as_bytes()).unwrap();
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
    }
    fn full(pair: &EcdsaKeyPair) -> String {
        list(pair, &[LAPTOP], vec![guest(&TRUSTED, "bob", true), guest(&ASKED, "carol", false)])
    }

    #[test]
    fn nobody_is_allowed_without_a_list() {
        let folder = tempfile::tempdir().unwrap();
        let access = Access::open(ORIGIN, HOST, folder.path().join("access.json"), 1500);
        assert!(access.allowed(1500).is_empty());
        assert_eq!(access.peer(&LAPTOP, 1500), None);
        assert!(!access.accept(&ASKED, 1900));
    }

    #[test]
    fn own_devices_and_trusted_friends_are_allowed_and_others_only_once_accepted() {
        let folder = tempfile::tempdir().unwrap();
        let (pair, key) = signer();
        let access = Access::open(ORIGIN, HOST, folder.path().join("access.json"), 1500);
        access.update(&full(&pair), &[key], 1500).unwrap();
        assert_eq!(access.allowed(1500), vec![LAPTOP, TRUSTED]);
        assert_eq!(access.peer(&LAPTOP, 1500), Some(Peer::Own { name: "Laptop".into() }));
        assert!(matches!(access.peer(&TRUSTED, 1500), Some(Peer::Guest { keyboard: true, mouse: false, .. })));
        assert_eq!(access.peer(&ASKED, 1500), None);
        // Accepted for a while, then asked about again.
        assert!(access.accept(&ASKED, 1600));
        assert_eq!(access.allowed(1500), vec![LAPTOP, TRUSTED, ASKED]);
        assert_eq!(access.peer(&ASKED, 1600), None);
        assert!(access.accept(&ASKED, 1900));
        access.dismiss(&ASKED);
        assert_eq!(access.peer(&ASKED, 1500), None);
        // A device that is not on the list cannot be accepted at all.
        assert!(!access.accept(&[0x44; 32], 1900));
        assert_eq!(access.peer(&[0x44; 32], 1500), None);
    }

    #[test]
    fn the_saved_list_works_again_after_a_restart_until_it_expires() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("access.json");
        let (pair, key) = signer();
        Access::open(ORIGIN, HOST, file.clone(), 1500).update(&full(&pair), &[key], 1500).unwrap();
        let later = Access::open(ORIGIN, HOST, file.clone(), 1900);
        assert_eq!(later.allowed(1900), vec![LAPTOP, TRUSTED]);
        assert_eq!(later.expires_at(), Some(2000));
        // In use at start, then time passes its expiry.
        assert!(later.allowed(2000).is_empty());
        // Expired before the start: not loaded at all.
        assert!(Access::open(ORIGIN, HOST, file.clone(), 2000).allowed(1500).is_empty());
        // Another hosting device cannot use this file.
        assert!(Access::open(ORIGIN, [8; 32], file, 1500).allowed(1500).is_empty());
    }

    #[test]
    fn a_changed_file_or_a_bad_list_lets_nobody_in() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("access.json");
        let (pair, key) = signer();
        let access = Access::open(ORIGIN, HOST, file.clone(), 1500);
        // Signed by somebody else: refused, nothing saved.
        assert!(access.update(&full(&signer().0), &[key.clone()], 1500).is_err());
        assert!(access.allowed(1500).is_empty());
        assert!(!file.exists());
        access.update(&full(&pair), &[key.clone()], 1500).unwrap();
        // A bad list later leaves the good one in use.
        assert!(access.update("not a list", &[key], 1500).is_err());
        assert_eq!(access.allowed(1500), vec![LAPTOP, TRUSTED]);
        // The saved text is edited to add a device.
        let text = std::fs::read_to_string(&file).unwrap();
        let mut saved: serde_json::Value = serde_json::from_str(&text).unwrap();
        let wider = list(&signer().0, &[LAPTOP, [0x55; 32]], vec![]);
        let parts: Vec<&str> = saved["access"].as_str().unwrap().split('.').collect();
        saved["access"] = format!("{}.{}.{}", parts[0], wider.split('.').nth(1).unwrap(), parts[2]).into();
        std::fs::write(&file, saved.to_string()).unwrap();
        assert!(Access::open(ORIGIN, HOST, file.clone(), 1500).allowed(1500).is_empty());
        std::fs::write(&file, "{").unwrap();
        assert!(Access::open(ORIGIN, HOST, file, 1500).allowed(1500).is_empty());
    }

    #[test]
    fn a_new_list_removes_devices_and_ends_acceptances_it_no_longer_covers() {
        let folder = tempfile::tempdir().unwrap();
        let (pair, key) = signer();
        let access = Access::open(ORIGIN, HOST, folder.path().join("access.json"), 1500);
        access.update(&full(&pair), &[key.clone()], 1500).unwrap();
        assert!(access.accept(&ASKED, 1900));
        // The laptop was removed, bob is no longer trusted, carol is no longer a friend.
        access.update(&list(&pair, &[], vec![guest(&TRUSTED, "bob", false)]), &[key.clone()], 1500).unwrap();
        assert!(access.allowed(1500).is_empty());
        // Carol comes back as a friend: the old acceptance does not.
        access.update(&full(&pair), &[key], 1500).unwrap();
        assert_eq!(access.allowed(1500), vec![LAPTOP, TRUSTED]);
        access.clear();
        assert!(access.allowed(1500).is_empty());
        assert_eq!(access.expires_at(), None);
    }
}
