use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::rand::{SecureRandom, SystemRandom};
use secpar_host_core::PairingStore;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

#[cfg(test)]
use std::time::Duration;

pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0; N];
    SystemRandom::new().fill(&mut b).expect("OS random source");
    b
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn hash(token: &str) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, token.as_bytes()).as_ref())
}

/// A new PC id, in the form the Windows host uses. Existing stores keep the id they have.
fn uuid4() -> String {
    let mut b = random::<16>();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

pub fn private_directory(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(io::Error::other("Unsafe Secpar data directory"));
        }
        #[cfg(target_os = "linux")]
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::other("Secpar directory belongs to another user"));
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    Ok(())
}
pub(crate) fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
        return Err(io::Error::other("Invalid Secpar state file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::other(
                "Secpar state file permissions must be 0600",
            ));
        }
    }
    std::fs::read(path)
}
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("No parent directory"))?;
    private_directory(directory)?;
    let temporary = directory.join(format!(".secpar-{}.tmp", hex(&random::<12>())));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(windows)]
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(&temporary, path)?;
        #[cfg(unix)]
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub autostart: bool,
    pub display: String,
    pub max_quality: String,
    pub allow_files: bool,
    pub allow_clip: bool,
    /// Friends may connect (they still need approval unless set to connect at any time).
    pub friends: bool,
    pub anywhere: bool,
    pub files_dir: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            autostart: false,
            display: String::new(),
            max_quality: "auto".into(),
            allow_files: false,
            allow_clip: true,
            friends: true,
            anywhere: false,
            files_dir: String::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    token_sha256: String,
    #[serde(default)]
    revoked: bool,
    #[serde(default)]
    account_owner: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Data {
    id: String,
    devices: Vec<Device>,
    config: Config,
    master: String,
    #[serde(default)]
    clipboard_ready: bool,
    #[serde(default)]
    portal_restore: Option<String>,
    /// The rendezvous Worker accepted `master` as this PC's relay key (trust on first use);
    /// later connections leave out `register`.
    #[serde(default)]
    relay_registered: bool,
}
pub struct Store {
    path: PathBuf,
    data: Data,
    codes: secpar_host_core::pairing_codes::PairingCodes,
    account_owner: Option<String>,
}
impl Store {
    pub fn open(dir: &Path) -> io::Result<Self> {
        private_directory(dir)?;
        let path = dir.join("owner-devices.json");
        let mut data: Data = match read_private(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Data {
                id: uuid4(),
                devices: vec![],
                config: Config::default(),
                master: URL_SAFE_NO_PAD.encode(random::<32>()),
                clipboard_ready: true,
                portal_restore: None,
                relay_registered: false,
            },
            Err(e) => return Err(e),
        };
        // Older Linux builds rejected clipboard sharing altogether. Enable the
        // newly supported feature once; subsequent explicit choices persist.
        if !data.clipboard_ready { data.config.allow_clip=true;data.clipboard_ready=true; }
        let s = Self {
            path,
            data,
            codes: Default::default(), account_owner: None,
        };
        s.save()?;
        Ok(s)
    }
    fn save(&self) -> io::Result<()> {
        write_private(&self.path, &serde_json::to_vec(&self.data)?)
    }
    pub fn id(&self) -> &str {
        &self.data.id
    }
    pub fn devices(&self) -> Vec<Device> {
        self.data
            .devices
            .iter()
            .filter(|d| !d.revoked)
            .cloned()
            .collect()
    }
    pub fn config(&self) -> Config {
        self.data.config.clone()
    }
    pub fn set_config(&mut self, config: Config) -> io::Result<()> {
        if config.anywhere {
            return Err(io::Error::other(
                "This Linux host does not open router ports yet",
            ));
        }
        let old = self.data.config.clone();
        self.data.config = config;
        if let Err(e) = self.save() {
            self.data.config = old;
            return Err(e);
        }
        Ok(())
    }
    pub fn portal_token(&self) -> Option<String> {
        self.data.portal_restore.clone()
    }
    pub fn set_portal_token(&mut self, token: Option<String>) -> io::Result<()> {
        self.data.portal_restore = token;
        self.save()
    }
    pub fn new_otp(&mut self) -> String {
        self.new_otp_at(Instant::now())
    }
    fn new_otp_at(&mut self, now: Instant) -> String { self.codes.manual(now) }
    fn take_at(&mut self, otp: &str, now: Instant) -> bool { self.codes.take(otp,now) }
    pub fn owner_otp(&mut self, request: &str, owner: &str) -> Result<String, String> {
        if self.account_owner.as_deref() != Some(owner) { return Err("Host account changed".into()); }
        self.codes.owner(request, owner, Instant::now()).map_err(str::to_owned)
    }
    pub fn set_account_owner(&mut self, owner: Option<String>) -> Vec<String> {
        if self.account_owner != owner { self.codes.clear_owner(); }
        self.account_owner = owner;
        self.data.devices.iter().filter(|d| !d.revoked && d.account_owner.is_some() && d.account_owner != self.account_owner).map(|d| d.id.clone()).collect()
    }
    fn add_device_owned(&mut self, name: &str, owner: Option<String>) -> io::Result<(String,String)> {
        if self.data.devices.len() >= 128 { return Err(io::Error::other("Device limit reached")); }
        let id = hex(&random::<8>());
        let token = URL_SAFE_NO_PAD.encode(random::<32>());
        self.data.devices.push(Device { id:id.clone(), name:name.chars().take(40).collect(), token_sha256:hash(&token), revoked:false, account_owner:owner });
        if let Err(e) = self.save() { self.data.devices.pop(); return Err(e); }
        Ok((id,token))
    }

}
impl PairingStore for Store {
    fn take_otp(&mut self, otp: &str) -> bool {
        self.take_at(otp, Instant::now())
    }
    fn add_device(&mut self, name: &str) -> io::Result<(String,String)> { self.add_device_owned(name,None) }
    fn pair_device(&mut self, otp: &str, name: &str) -> Result<(String,String), &'static str> {
        let origin = self.codes.consume(otp, Instant::now()).ok_or("bad_otp")?;
        let owner = match origin {
            secpar_host_core::pairing_codes::Origin::Manual => None,
            secpar_host_core::pairing_codes::Origin::Account(owner) if self.account_owner.as_deref() == Some(&owner) => Some(owner),
            _ => return Err("bad_otp"),
        };
        self.add_device_owned(name,owner).map_err(|_| "internal")
    }
    fn auth(&self, token: &str) -> Result<String, &'static str> {
        let h = hash(token);
        match self.data.devices.iter().find(|d| d.token_sha256 == h) {
            Some(d) if d.revoked || (d.account_owner.is_some() && d.account_owner != self.account_owner) => Err("revoked"),
            Some(d) => Ok(d.id.clone()),
            None => Err("unauthorized"),
        }
    }
    fn revoke(&mut self, id: &str) -> io::Result<()> {
        let old = self.data.clone();
        self.data
            .devices
            .iter_mut()
            .filter(|d| d.id == id)
            .for_each(|d| d.revoked = true);
        if let Err(e) = self.save() {
            self.data = old;
            return Err(e);
        }
        Ok(())
    }
    fn ticket(&self, id: &str) -> [u8; 32] {
        let key = URL_SAFE_NO_PAD
            .decode(&self.data.master)
            .expect("valid master key");
        // `master` is the docs/remote-protocol.md relayKey.
        secpar_network::relay_auth::device_ticket(&key, self.id(), id)
    }
}
impl secpar_host_core::rendezvous::RelaySource for Store {
    fn pc_id(&self) -> String {
        self.data.id.clone()
    }
    fn relay_key(&self) -> Vec<u8> {
        URL_SAFE_NO_PAD.decode(&self.data.master).expect("valid master key")
    }
    fn relay_registered(&self) -> bool {
        self.data.relay_registered
    }
    fn set_relay_registered(&mut self) -> io::Result<()> {
        if self.data.relay_registered {
            return Ok(());
        }
        self.data.relay_registered = true;
        if let Err(e) = self.save() {
            self.data.relay_registered = false;
            return Err(e);
        }
        Ok(())
    }
    fn revoked(&self) -> Vec<String> {
        self.data.devices.iter().filter(|d| d.revoked).map(|d| d.id.clone()).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use secpar_host_core::rendezvous::RelaySource;
    #[test]
    fn relay_key_is_the_private_master_key_and_registration_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        let (pc, key) = (RelaySource::pc_id(&s), s.relay_key());
        assert_eq!((pc.len(), pc.matches('-').count(), key.len()), (36, 4, 32));
        assert!(!RelaySource::relay_registered(&s));
        let (id, _) = s.add_device("Phone").unwrap();
        assert!(RelaySource::revoked(&s).is_empty());
        s.revoke(&id).unwrap();
        s.set_relay_registered().unwrap();
        drop(s);
        let s = Store::open(dir.path()).unwrap();
        assert_eq!((RelaySource::pc_id(&s), s.relay_key()), (pc, key), "created once");
        assert!(RelaySource::relay_registered(&s));
        assert_eq!(RelaySource::revoked(&s), vec![id]);
    }
    #[test]
    fn clipboard_support_migration_runs_once_and_preserves_later_opt_out() {
        let directory=tempfile::tempdir().unwrap();
        let mut store=Store::open(directory.path()).unwrap();
        let (id,token)=store.add_device("Existing device").unwrap();
        store.data.clipboard_ready=false;store.data.config.allow_clip=false;store.save().unwrap();drop(store);
        let mut store=Store::open(directory.path()).unwrap();
        assert!(store.config().allow_clip);assert_eq!(store.auth(&token),Ok(id.clone()));
        let mut config=store.config();config.allow_clip=false;store.set_config(config).unwrap();drop(store);
        let store=Store::open(directory.path()).unwrap();assert!(!store.config().allow_clip);assert_eq!(store.auth(&token),Ok(id));
    }
    #[test]
    fn worker_ticket_upgrade_preserves_existing_device_authentication() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let (id, token) = store.add_device("Existing device").unwrap();
        // Existing schema has no migration/version field. Reopen its unchanged
        // private state using a deterministic key/PC so the Worker vector applies.
        store.data.id = "pc1".into();
        store.data.master = URL_SAFE_NO_PAD.encode([7; 32]);
        store.save().unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.auth(&token), Ok(id.clone()));
        assert_eq!(URL_SAFE_NO_PAD.encode(store.ticket("dev1")),
            "Apu-dBssItiqztSY2R7lwB90YggnWttqG85Fei6VQzs");
        assert_eq!(store.ticket(&id), secpar_network::relay_auth::device_ticket(&[7; 32], "pc1", &id));
    }
    #[test]
    fn account_pairing_is_private_per_device_and_revoked_when_unlinked() {
        let dir=tempfile::tempdir().unwrap(); let mut store=Store::open(dir.path()).unwrap();
        store.set_account_owner(Some("alice".into()));
        let manual=store.new_otp();
        let code=store.owner_otp("00000000-0000-4000-8000-000000000001","alice").unwrap();
        assert!(store.owner_otp("00000000-0000-4000-8000-000000000002","bob").is_err());
        let (id,token)=store.pair_device(&code,"Laptop").unwrap();
        assert!(store.pair_device(&code,"Replay").is_err());
        let (_,manual_token)=store.pair_device(&manual,"Manual").unwrap();
        assert!(store.auth(&token).is_ok());
        assert_eq!(store.set_account_owner(None),vec![id.clone()]);
        assert_eq!(store.auth(&token),Err("revoked")); assert!(store.auth(&manual_token).is_ok());
        store.revoke(&id).unwrap(); drop(store);
        let mut restored=Store::open(dir.path()).unwrap(); restored.set_account_owner(Some("alice".into()));
        assert_eq!(restored.auth(&token),Err("revoked")); assert!(restored.auth(&manual_token).is_ok());
    }
    #[test]
    fn owner_credentials_survive_restart_without_plaintext_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        let (id, token) = s.add_device("Test").unwrap();
        let pc = s.id().to_string();
        let ticket = s.ticket(&id);
        drop(s);
        let mut s = Store::open(dir.path()).unwrap();
        assert_eq!(s.id(), pc);
        assert_eq!(s.auth(&token), Ok(id.clone()));
        assert_eq!(s.ticket(&id), ticket);
        assert!(
            !String::from_utf8(std::fs::read(dir.path().join("owner-devices.json")).unwrap())
                .unwrap()
                .contains(&token)
        );
        s.revoke(&id).unwrap();
        assert_eq!(s.auth(&token), Err("revoked"));
    }
    #[test]
    fn otp_is_single_use_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        let now = Instant::now();
        let otp = s.new_otp_at(now);
        assert!(!s.take_at("wrong", now));
        assert!(s.take_at(&otp, now));
        assert!(!s.take_at(&otp, now));
        let otp = s.new_otp_at(now);
        assert!(!s.take_at(&otp, now + Duration::from_secs(121)));
    }
    #[cfg(unix)]
    #[test]
    fn secrets_are_private_and_symlinks_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        s.add_device("Test").unwrap();
        assert_eq!(
            std::fs::metadata(dir.path().join("owner-devices.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let target = dir.path().join("other");
        symlink(dir.path().join("owner-devices.json"), &target).unwrap();
        assert!(read_private(&target).is_err());
    }
}
