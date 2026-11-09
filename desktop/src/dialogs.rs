//! The system file and folder pickers, with no extra crates. Windows uses the common dialogs
//! through `windows-sys`. Linux asks KDE's `kdialog` or GTK's `zenity`, whichever is installed.
//! Both calls block until the dialog closes, so callers run them on their own thread.
use std::path::{Path, PathBuf};

/// Why no dialog appeared. Shown as is.
#[allow(dead_code)]
pub const UNAVAILABLE: &str = "No file picker is available here. Drop files on the window instead.";

/// Files to upload. Empty when the dialog was cancelled.
pub fn pick_files() -> Result<Vec<PathBuf>, String> {
    imp::pick_files()
}
/// A folder to save into. `None` when the dialog was cancelled.
pub fn pick_folder(start: &Path) -> Result<Option<PathBuf>, String> {
    imp::pick_folder(start)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
    use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    use windows_sys::Win32::UI::Shell::{SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub fn pick_files() -> Result<Vec<PathBuf>, String> {
        // 64K characters hold well over a thousand selected names.
        let mut buffer = vec![0u16; 65_536];
        let title = wide("Upload files");
        // SAFETY: the struct is zeroed and then filled with pointers that outlive the call.
        let ok = unsafe {
            let mut dialog: OPENFILENAMEW = std::mem::zeroed();
            dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
            dialog.hwndOwner = GetForegroundWindow();
            dialog.lpstrFile = buffer.as_mut_ptr();
            dialog.nMaxFile = buffer.len() as u32;
            dialog.lpstrTitle = title.as_ptr();
            dialog.Flags = OFN_ALLOWMULTISELECT | OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
            GetOpenFileNameW(&mut dialog) != 0
        };
        if !ok {
            return Ok(vec![]);
        }
        Ok(split_selection(&buffer))
    }

    /// One selected file is a full path. Several are the folder, then each name, all separated
    /// by NUL and ended by two NULs.
    pub fn split_selection(buffer: &[u16]) -> Vec<PathBuf> {
        let parts: Vec<std::ffi::OsString> = buffer.split(|c| *c == 0).take_while(|p| !p.is_empty()).map(std::ffi::OsString::from_wide).collect();
        match parts.len() {
            0 => vec![],
            1 => vec![PathBuf::from(&parts[0])],
            _ => parts[1..].iter().map(|name| Path::new(&parts[0]).join(name)).collect(),
        }
    }

    pub fn pick_folder(_start: &Path) -> Result<Option<PathBuf>, String> {
        let title = wide("Save downloads in");
        let mut name = vec![0u16; 260];
        // SAFETY: COM is started on this dialog thread, the pointers outlive the calls and the
        // returned item list is freed once.
        unsafe {
            CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
            let mut info: BROWSEINFOW = std::mem::zeroed();
            info.hwndOwner = GetForegroundWindow();
            info.pszDisplayName = name.as_mut_ptr();
            info.lpszTitle = title.as_ptr();
            info.ulFlags = BIF_NEWDIALOGSTYLE | BIF_RETURNONLYFSDIRS;
            let item = SHBrowseForFolderW(&info);
            if item.is_null() {
                return Ok(None);
            }
            let mut path = vec![0u16; 1024];
            let found = SHGetPathFromIDListW(item, path.as_mut_ptr()) != 0;
            CoTaskMemFree(item as *const _);
            if !found {
                return Ok(None);
            }
            let len = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(&path[..len]))))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn buf(parts: &[&str]) -> Vec<u16> {
            let mut v: Vec<u16> = parts.iter().flat_map(|p| wide(p)).collect();
            v.push(0);
            v
        }
        #[test]
        fn one_file_is_a_full_path() {
            assert_eq!(split_selection(&buf(&["C:\\a\\b.txt"])), vec![PathBuf::from("C:\\a\\b.txt")]);
        }
        #[test]
        fn several_files_share_the_folder() {
            assert_eq!(split_selection(&buf(&["C:\\a", "b.txt", "c d.txt"])), vec![PathBuf::from("C:\\a\\b.txt"), PathBuf::from("C:\\a\\c d.txt")]);
        }
        #[test]
        fn an_empty_buffer_selects_nothing() {
            assert!(split_selection(&[0, 0]).is_empty());
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::process::Command;

    /// Runs the first picker that exists. `Ok(None)` means it ran and was cancelled.
    fn run(candidates: &[(&str, Vec<String>)]) -> Result<Option<String>, String> {
        for (program, args) in candidates {
            match Command::new(program).args(args).output() {
                Ok(out) if out.status.success() => return Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned())),
                Ok(out) if out.status.code() == Some(1) => return Ok(None),
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => continue,
            }
        }
        Err(UNAVAILABLE.into())
    }

    pub fn lines(text: &str) -> Vec<PathBuf> {
        text.lines().map(str::trim_end).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
    }

    pub fn pick_files() -> Result<Vec<PathBuf>, String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let text = run(&[
            ("kdialog", vec!["--title".into(), "Upload files".into(), "--getopenfilename".into(), home.clone(), "--multiple".into(), "--separate-output".into()]),
            ("zenity", vec!["--file-selection".into(), "--multiple".into(), "--separator=\n".into(), "--title=Upload files".into()]),
        ])?;
        Ok(text.map(|t| lines(&t)).unwrap_or_default())
    }

    pub fn pick_folder(start: &Path) -> Result<Option<PathBuf>, String> {
        let start = start.to_string_lossy().into_owned();
        let text = run(&[
            ("kdialog", vec!["--title".into(), "Save downloads in".into(), "--getexistingdirectory".into(), start.clone()]),
            ("zenity", vec!["--file-selection".into(), "--directory".into(), "--title=Save downloads in".into(), format!("--filename={start}/")]),
        ])?;
        Ok(text.and_then(|t| lines(&t).into_iter().next()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn picker_output_is_one_path_per_line() {
            assert_eq!(lines("/a/b.txt\n/c d/e.txt\n\n"), vec![PathBuf::from("/a/b.txt"), PathBuf::from("/c d/e.txt")]);
            assert!(lines("").is_empty());
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use super::*;
    pub fn pick_files() -> Result<Vec<PathBuf>, String> {
        Err(UNAVAILABLE.into())
    }
    pub fn pick_folder(_start: &Path) -> Result<Option<PathBuf>, String> {
        Err(UNAVAILABLE.into())
    }
}
