//! Linux side of file transfer. The transfer logic is shared (secpar-host-core `files`); this
//! file adds free space (statvfs), the browse roots, the received-files folder and a notice.
use secpar_host_core::{
    files::{self, Hooks, Incoming},
    proto::FsEntry,
};
use std::{
    collections::HashMap,
    ffi::CString,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

/// Free bytes available to this user on the volume holding `dir`.
pub fn free_space(dir: &Path) -> Option<u64> {
    let path = CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid C string and `stat` a writable statvfs for the call.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

/// Text that is safe in a notification body: no markup, no control characters, bounded.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A desktop notice through `notify-send`, when it is installed. A missing tool is not an error.
fn received(path: &Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=Secpar", "File received"])
        .arg(format!("{}\nSaved to {}", plain(&name), plain(&folder)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut child| std::thread::spawn(move || child.wait()));
}

/// An `Incoming` that checks free disk space and shows a notice.
pub fn incoming(dir: PathBuf) -> Incoming {
    Incoming::with_hooks(dir, Hooks { free_space: Some(free_space), received: Some(received) })
}

/// `XDG_*_DIR="$HOME/..."` lines of `user-dirs.dirs`, as name (`DOWNLOAD`) to folder.
pub fn parse_user_dirs(text: &str, home: &Path) -> HashMap<String, PathBuf> {
    let mut found = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        let Some(name) = key.trim().strip_prefix("XDG_").and_then(|k| k.strip_suffix("_DIR")) else { continue };
        let value = value.trim().trim_matches('"');
        let path = match value.strip_prefix("$HOME") {
            Some("") => home.to_path_buf(),
            Some(rest) if rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
            Some(_) => continue,
            None if value.starts_with('/') => PathBuf::from(value),
            None => continue,
        };
        // XDG points a disabled folder at $HOME itself.
        if path != home {
            found.insert(name.to_string(), path);
        }
    }
    found
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute())
}

fn user_dirs(home: &Path) -> HashMap<String, PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    std::fs::read_to_string(config.join("user-dirs.dirs")).map(|t| parse_user_dirs(&t, home)).unwrap_or_default()
}

/// The folder for received files: the configured one, else `<Downloads>/Secpar`.
pub fn downloads_dir(configured: &str, xdg_download: Option<PathBuf>, home: &Path) -> PathBuf {
    if Path::new(configured).is_absolute() {
        return PathBuf::from(configured);
    }
    xdg_download.filter(|p| p.is_absolute()).unwrap_or_else(|| home.join("Downloads")).join("Secpar")
}

/// Where this user's received files go, read from the environment and `user-dirs.dirs`.
pub fn files_dir(configured: &str) -> PathBuf {
    let Some(home) = home() else { return downloads_dir(configured, None, Path::new("/tmp")) };
    let xdg = std::env::var_os("XDG_DOWNLOAD_DIR").map(PathBuf::from).or_else(|| user_dirs(&home).remove("DOWNLOAD"));
    downloads_dir(configured, xdg, &home)
}

/// The "" listing: home, the user's folders and the file system root.
pub fn roots(home: &Path, dirs: &HashMap<String, PathBuf>) -> Vec<FsEntry> {
    let entry = |name: &str, path: &Path| FsEntry { name: name.into(), path: path.to_string_lossy().into(), dir: true, size: 0, mtime: 0 };
    let mut v = vec![entry("Home", home)];
    for (key, name) in [("DESKTOP", "Desktop"), ("DOCUMENTS", "Documents"), ("DOWNLOAD", "Downloads"), ("PICTURES", "Pictures"), ("VIDEOS", "Videos"), ("MUSIC", "Music")] {
        let path = dirs.get(key).cloned().unwrap_or_else(|| home.join(name));
        if path.is_dir() {
            v.push(entry(name, &path));
        }
    }
    v.push(entry("/", Path::new("/")));
    v
}

/// `fs.list`: the roots for "", else a folder with dot-files left out.
pub fn list(path: &str) -> std::io::Result<Vec<FsEntry>> {
    if path.is_empty() {
        let home = home().ok_or_else(|| std::io::Error::other("No home folder"))?;
        let dirs = user_dirs(&home);
        return Ok(roots(&home, &dirs));
    }
    files::list_dir(path, |entry, _| entry.file_name().as_encoded_bytes().first() == Some(&b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("secpar-linux-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn user_dirs_file_is_parsed() {
        let home = Path::new("/home/ada");
        let text = "# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Herunterladen\"\nXDG_MUSIC_DIR=\"/mnt/music\"\nXDG_DESKTOP_DIR=\"$HOME/\"\nXDG_VIDEOS_DIR=\"$HOME\"\nXDG_PICTURES_DIR=\"relative\"\nXDG_TEMPLATES_DIR=\"$HOMEX/t\"\n";
        let dirs = parse_user_dirs(text, home);
        assert_eq!(dirs["DOWNLOAD"], PathBuf::from("/home/ada/Herunterladen"));
        assert_eq!(dirs["MUSIC"], PathBuf::from("/mnt/music"));
        assert!(!dirs.contains_key("DESKTOP"), "$HOME/ is home itself, which means disabled");
        assert!(!dirs.contains_key("VIDEOS"));
        assert!(!dirs.contains_key("PICTURES"));
        assert!(!dirs.contains_key("TEMPLATES"));
    }

    #[test]
    fn received_files_folder_order() {
        let home = Path::new("/home/ada");
        assert_eq!(downloads_dir("", None, home), PathBuf::from("/home/ada/Downloads/Secpar"));
        assert_eq!(downloads_dir("", Some("/data/dl".into()), home), PathBuf::from("/data/dl/Secpar"));
        assert_eq!(downloads_dir("", Some("dl".into()), home), PathBuf::from("/home/ada/Downloads/Secpar"), "a relative XDG path is ignored");
        assert_eq!(downloads_dir("/srv/inbox", Some("/data/dl".into()), home), PathBuf::from("/srv/inbox"));
        assert_eq!(downloads_dir("inbox", None, home), PathBuf::from("/home/ada/Downloads/Secpar"), "a relative setting is ignored");
    }

    #[test]
    fn roots_list_home_known_folders_and_slash() {
        let home = tmp("roots");
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        std::fs::create_dir_all(home.join("Dateien")).unwrap();
        let mut dirs = HashMap::new();
        dirs.insert("DOWNLOAD".to_string(), home.join("Dateien"));
        let names: Vec<(String, String)> = roots(&home, &dirs).into_iter().map(|e| (e.name, e.path)).collect();
        let path = |p: PathBuf| p.to_string_lossy().into_owned();
        assert_eq!(
            names,
            vec![
                ("Home".to_string(), path(home.clone())),
                ("Documents".to_string(), path(home.join("Documents"))),
                ("Downloads".to_string(), path(home.join("Dateien"))),
                ("/".to_string(), "/".to_string()),
            ],
            "missing folders are left out and the user-dirs mapping wins"
        );
    }

    #[test]
    fn listing_hides_dot_files_and_puts_folders_first() {
        let d = tmp("list");
        std::fs::create_dir_all(d.join("b-dir")).unwrap();
        std::fs::create_dir_all(d.join(".hidden-dir")).unwrap();
        std::fs::write(d.join("a.txt"), b"abc").unwrap();
        std::fs::write(d.join(".secret"), b"x").unwrap();
        let v = list(d.to_str().unwrap()).unwrap();
        assert_eq!(v.iter().map(|e| (e.name.as_str(), e.dir, e.size)).collect::<Vec<_>>(), vec![("b-dir", true, 0), ("a.txt", false, 3)]);
        assert!(list(d.join("missing").to_str().unwrap()).is_err());
    }

    #[test]
    fn free_space_of_a_real_folder_is_known() {
        let d = tmp("space");
        assert!(free_space(&d).is_some_and(|n| n > 0));
        assert_eq!(free_space(&d.join("missing")), None);
    }

    #[test]
    fn incoming_uses_the_real_disk_check_and_receives() {
        let d = tmp("incoming");
        let mut rx = incoming(d.clone());
        let id = "0123abcd-0000-4000-8000-00000000beef";
        assert_eq!(
            rx.begin(id, "x/y.txt", u64::MAX / 2),
            secpar_host_core::proto::ServerMsg::FileError { id: id.into(), code: "disk_full".into() },
            "a file larger than the disk is refused up front"
        );
        assert!(matches!(rx.begin(id, "a.txt", 0), secpar_host_core::proto::ServerMsg::FileReady { offset: 0, .. }));
    }

    #[test]
    fn notice_text_has_no_markup() {
        assert_eq!(plain("<b>x</b> & \n y"), "&lt;b&gt;x&lt;/b&gt; &amp;  y");
        assert_eq!(plain(&"é".repeat(500)).chars().count(), 200);
    }
}
