//! Files between two computers. One side writes what the other sends into
//! `Downloads/Secpar`, under a temporary name until the last piece has arrived,
//! so a file that is still coming is never there under its own name. A name that
//! is already taken gets " (2)" and so on. The sending side reads the file in
//! pieces and reports how far it is.
use secpar_hosting::{file_name_ok, send_file_chunk, Files, Progress, FILE_CHUNK, MAX_FILE};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

/// Files this computer takes from other computers.
pub struct Downloads {
    /// The folder they are written into, made when the first one arrives.
    root: PathBuf,
    /// Files that are still arriving, by the device sending them and the name it
    /// gave, so two devices may send files of one name at the same time.
    arriving: Mutex<HashMap<(String, String), Arriving>>,
    /// The last file that arrived, for the window to show.
    last: Mutex<Option<Received>>,
}
struct Arriving {
    temp: PathBuf,
    /// The name the file will have here, decided when its first piece arrives.
    name: String,
    total: u64,
    written: u64,
    file: File,
}
/// A file that has just arrived.
#[derive(Clone, Serialize)]
pub struct Received {
    /// The name it was saved under here, which may have " (2)" added.
    pub name: String,
    /// The folder it went into.
    pub folder: String,
}

impl Downloads {
    /// Files go into this person's downloads folder, in a `Secpar` folder inside it.
    pub fn open() -> Arc<Self> {
        Arc::new(Self::at(downloads_folder().unwrap_or_else(|| PathBuf::from(".")).join("Secpar")))
    }
    /// Files go into `root`, which is made when the first one arrives.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), arriving: Default::default(), last: Default::default() }
    }
    /// The last file that arrived since this part started, if any.
    pub fn last(&self) -> Option<Received> {
        self.last.lock().ok()?.clone()
    }
    pub fn folder(&self) -> String {
        self.root.display().to_string()
    }

    /// The name this computer saves `wanted` under: the same name when it is
    /// free, otherwise " (2)", " (3)" and so on. Names still arriving count as
    /// taken, so two files of one name never write into the same place.
    fn free_name(&self, wanted: &str, arriving: &HashMap<(String, String), Arriving>) -> String {
        let taken = |name: &str| self.root.join(name).exists() || arriving.values().any(|a| a.name == name);
        if !taken(wanted) {
            return wanted.to_string();
        }
        let path = Path::new(wanted);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(wanted);
        let extension = path.extension().and_then(|e| e.to_str());
        for number in 2..1000 {
            let name = match extension {
                Some(extension) => format!("{stem} ({number}).{extension}"),
                None => format!("{stem} ({number})"),
            };
            if !taken(&name) {
                return name;
            }
        }
        wanted.to_string()
    }
}

impl Files for Downloads {
    fn receive(&self, from: &str, name: &str, total: u64, offset: u64, data: &[u8]) -> Option<Progress> {
        let key = (from.to_string(), name.to_string());
        let mut arriving = self.arriving.lock().ok()?;
        // The first piece, or a file of the same name started again from its
        // beginning: whatever was there for that name is thrown away first.
        if offset == 0 && arriving.get(&key).is_none_or(|a| a.written > 0) {
            if let Some(old) = arriving.remove(&key) {
                let _ = std::fs::remove_file(old.temp);
            }
            std::fs::create_dir_all(&self.root).ok()?;
            let final_name = self.free_name(name, &arriving);
            let temp = self.root.join(format!(".{final_name}.part"));
            let _ = std::fs::remove_file(&temp);
            let file = File::create(&temp).ok()?;
            arriving.insert(key.clone(), Arriving { temp, name: final_name, total, written: 0, file });
        }
        let written = {
            let arriving = arriving.get_mut(&key)?;
            // Pieces arrive in order and never past the size that was announced.
            if arriving.written != offset || arriving.total != total {
                return None;
            }
            arriving.file.write_all(data).ok()?;
            arriving.written += data.len() as u64;
            arriving.written
        };
        if written < total {
            if let Some(arriving) = arriving.get(&key) {
                // Written out from here on, so a crash loses nothing already taken.
                let _ = arriving.file.sync_data();
            }
            return Some(Progress { received: written, saved: None });
        }
        // The last piece: the file is closed, then given its own name. Only a
        // finished file ever has the name the other computer gave.
        let Arriving { temp, name: final_name, file, .. } = arriving.remove(&key)?;
        drop(file);
        let mut saved = final_name;
        if std::fs::rename(&temp, self.root.join(&saved)).is_err() {
            // Something took the name while the file was coming.
            saved = self.free_name(name, &arriving);
            if std::fs::rename(&temp, self.root.join(&saved)).is_err() {
                let _ = std::fs::remove_file(&temp);
                return None;
            }
        }
        drop(arriving);
        if let Ok(mut last) = self.last.lock() {
            *last = Some(Received { name: saved.clone(), folder: self.folder() });
        }
        Some(Progress { received: written, saved: Some(saved) })
    }
}

/// A file this computer is sending, as the window shows it.
#[derive(Clone, Serialize)]
pub struct Transfer {
    pub id: u64,
    pub name: String,
    /// The bytes the other computer has taken so far.
    pub sent: u64,
    pub total: u64,
    /// `sending`, `sent` or `failed`.
    pub state: String,
    /// The name the other computer saved it under, once it has.
    pub saved: Option<String>,
    /// Why it failed, in the words the hosting side uses.
    pub error: Option<String>,
}

/// The files this computer has sent or is sending, newest last. The window reads
/// it to show how far a file is; the last few are enough for that.
#[derive(Default)]
pub struct Transfers {
    next: AtomicU64,
    entries: Mutex<Vec<Transfer>>,
}
impl Transfers {
    fn start(&self, name: &str, total: u64) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(Transfer { id, name: name.into(), sent: 0, total, state: "sending".into(), saved: None, error: None });
            // Only the last few are shown, so the list cannot grow without end.
            while entries.len() > 8 {
                entries.remove(0);
            }
        }
        id
    }
    fn change(&self, id: u64, change: impl FnOnce(&mut Transfer)) {
        if let Ok(mut entries) = self.entries.lock() {
            if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
                change(entry);
            }
        }
    }
    fn progress(&self, id: u64, sent: u64) {
        self.change(id, |entry| entry.sent = sent);
    }
    fn finish(&self, id: u64, saved: Option<String>) {
        self.change(id, |entry| {
            entry.state = "sent".into();
            entry.sent = entry.total;
            entry.saved = saved;
        });
    }
    fn fail(&self, id: u64, error: &str) {
        self.change(id, |entry| {
            entry.state = "failed".into();
            entry.error = Some(error.into());
        });
    }
    pub fn list(&self) -> Vec<Transfer> {
        self.entries.lock().map(|entries| entries.clone()).unwrap_or_default()
    }
    /// The entry that is still being sent, if one is.
    pub fn sending(&self) -> Option<Transfer> {
        self.entries.lock().ok()?.iter().find(|e| e.state == "sending").cloned()
    }
}

/// Starts sending the file at `path` to the computer at the other end of
/// `tunnel`, in pieces, and returns the entry the window follows. The sending
/// itself runs in the background.
pub fn send(transfers: Arc<Transfers>, tunnel: Arc<secpar_tunnel::Client>, path: PathBuf) -> Result<u64, &'static str> {
    let metadata = std::fs::metadata(&path).map_err(|_| "not_a_file")?;
    if !metadata.is_file() {
        return Err("not_a_file");
    }
    let total = metadata.len();
    if total == 0 {
        return Err("empty_file");
    }
    if total > MAX_FILE {
        return Err("too_large");
    }
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("bad_name")?;
    if !file_name_ok(name) {
        return Err("bad_name");
    }
    let name = name.to_string();
    let id = transfers.start(&name, total);
    tokio::spawn(async move {
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(_) => return transfers.fail(id, "not_a_file"),
        };
        let mut buffer = vec![0u8; FILE_CHUNK];
        let mut offset = 0u64;
        loop {
            let read = match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => return transfers.fail(id, "not_a_file"),
            };
            let answer = send_file_chunk(&tunnel, &name, total, offset, &buffer[..read]).await;
            match answer.received {
                Some(received) if received == offset + read as u64 => {
                    offset = received;
                    transfers.progress(id, offset);
                    if offset >= total {
                        return transfers.finish(id, answer.saved);
                    }
                }
                // The other computer holds a different amount than this piece
                // should have brought it to, so the file cannot be finished.
                Some(_) => return transfers.fail(id, "not_saved"),
                None => return transfers.fail(id, answer.error.as_deref().unwrap_or("unknown")),
            }
        }
        // The file ended earlier than its size said it would.
        transfers.fail(id, "not_a_file");
    });
    Ok(id)
}

/// This person's downloads folder, as the system reports it. A folder that was
/// moved elsewhere is found; when it cannot be asked, the usual place is used.
#[cfg(windows)]
fn downloads_folder() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED},
        UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath},
    };
    // SAFETY: the shell writes a terminated path into `path`, which is read only
    // while it is valid and then freed with CoTaskMemFree. Starting COM twice is
    // harmless here; its answer only matters for the call below.
    let known = unsafe {
        let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let mut path: *mut u16 = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Downloads, 0, std::ptr::null_mut(), &mut path) != 0 || path.is_null() {
            None
        } else {
            let length = (0..).take_while(|&i| *path.add(i) != 0).count();
            let folder = OsString::from_wide(std::slice::from_raw_parts(path, length));
            CoTaskMemFree(path as *const _);
            Some(PathBuf::from(folder))
        }
    };
    known.or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("Downloads")))
}

/// Reads `XDG_DOWNLOAD_DIR` from the user directories file, as the desktop sets
/// it, and falls back to `$HOME/Downloads`.
#[cfg(unix)]
fn downloads_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let configured = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).ok().and_then(|text| {
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=")?.trim().trim_matches('"');
            Some(PathBuf::from(value.replace("$HOME", &home.display().to_string())))
        })
    });
    Some(configured.unwrap_or_else(|| home.join("Downloads")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder that is not this person's downloads folder.
    fn folder() -> (tempfile::TempDir, Arc<Downloads>) {
        let folder = tempfile::tempdir().unwrap();
        let downloads = Arc::new(Downloads::at(folder.path().join("Secpar")));
        (folder, downloads)
    }

    #[test]
    fn a_file_is_only_given_its_name_once_it_is_complete() {
        let (_folder, downloads) = folder();
        let data = b"hello there";
        let first = downloads.receive("laptop", "note.txt", data.len() as u64, 0, &data[..5]).unwrap();
        assert_eq!((first.received, first.saved), (5, None));
        let root = downloads.folder();
        assert!(!Path::new(&root).join("note.txt").exists(), "a partial file never has its own name");
        let second = downloads.receive("laptop", "note.txt", data.len() as u64, 5, &data[5..]).unwrap();
        assert_eq!(second.saved.as_deref(), Some("note.txt"));
        assert_eq!(std::fs::read(Path::new(&root).join("note.txt")).unwrap(), data);
        assert_eq!(downloads.last().unwrap().name, "note.txt");
        // The temporary file is gone with the name it was given.
        assert!(!Path::new(&root).join(".note.txt.part").exists());
    }

    #[test]
    fn a_name_that_is_taken_gets_a_number_and_pieces_must_come_in_order() {
        let (_folder, downloads) = folder();
        std::fs::create_dir_all(downloads.folder()).unwrap();
        std::fs::write(Path::new(&downloads.folder()).join("note.txt"), b"old").unwrap();
        let data = b"new text";
        downloads.receive("laptop", "note.txt", data.len() as u64, 0, data).unwrap();
        assert_eq!(downloads.last().unwrap().name, "note (2).txt");
        assert_eq!(std::fs::read(Path::new(&downloads.folder()).join("note (2).txt")).unwrap(), data);
        assert_eq!(std::fs::read(Path::new(&downloads.folder()).join("note.txt")).unwrap(), b"old");
        // A piece that does not follow on from what is held is refused.
        assert!(downloads.receive("laptop", "other.bin", 10, 4, b"xxxx").is_none());
        assert!(downloads.receive("laptop", "other.bin", 10, 5, b"xxxx").is_none());
        // Two devices sending one name at the same time are kept apart.
        let piece = |from: &str, offset: u64, data: &[u8]| downloads.receive(from, "from-two.bin", 8, offset, data);
        assert_eq!(piece("laptop", 0, b"aaaa").unwrap().saved, None);
        assert_eq!(piece("phone", 0, b"bbbb").unwrap().saved, None);
        assert_eq!(piece("laptop", 4, b"aaaa").unwrap().saved.as_deref(), Some("from-two.bin"));
        assert_eq!(piece("phone", 4, b"bbbb").unwrap().saved.as_deref(), Some("from-two (2).bin"));
        assert_eq!(std::fs::read(Path::new(&downloads.folder()).join("from-two.bin")).unwrap(), b"aaaaaaaa");
        assert_eq!(std::fs::read(Path::new(&downloads.folder()).join("from-two (2).bin")).unwrap(), b"bbbbbbbb");
    }
}
