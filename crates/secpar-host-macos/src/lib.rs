//! Native macOS hosting engine for the one Secpar desktop app.
//! The control, admission and session logic is platform neutral and tested anywhere;
//! only `native` talks to macOS and it is compiled on macOS alone.
pub mod annexb;
pub mod backend;
pub mod convert;
pub mod keymap;
pub mod media;
pub mod platform;
pub mod session;

#[cfg(target_os = "macos")]
pub mod native;
#[cfg(target_os = "macos")]
mod native_input;
#[cfg(target_os = "macos")]
mod native_media;

/// Per-user private data directory (`~/Library/Application Support/secpar`).
pub fn config_directory(home: Option<std::ffi::OsString>) -> Result<std::path::PathBuf, String> {
    let home = std::path::PathBuf::from(home.ok_or("No user home directory is available")?);
    if !home.is_absolute() {
        return Err("No user home directory is available".into());
    }
    Ok(home.join("Library/Application Support/secpar"))
}
#[cfg(test)]
mod tests {
    #[test]
    fn data_directory_requires_an_absolute_home() {
        assert!(super::config_directory(None).is_err());
        assert!(super::config_directory(Some("relative".into())).is_err());
        let d = super::config_directory(Some(std::env::temp_dir().into_os_string())).unwrap();
        assert!(d.ends_with("Library/Application Support/secpar"));
    }
}
