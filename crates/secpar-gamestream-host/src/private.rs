//! Backend state is readable by its owner, administrators and SYSTEM only.
use std::{fs, io, path::Path};

pub fn directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(io::Error::other(
            "Backend state must be an ordinary directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    windows_directory(path)?;
    Ok(())
}

pub fn create(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

pub fn ordinary_or_missing(path: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(io::Error::other("Backend state contains a redirected file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(io::Error::other("Backend state contains a linked file"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("Backend state contains a reparse point"));
        }
    }
    Ok(())
}

pub fn read(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    read_inner(path, limit, true)
}
pub fn certificate(path: &Path) -> io::Result<Vec<u8>> {
    read_inner(path, 16384, false)
}
fn read_inner(path: &Path, limit: usize, secret: bool) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit as u64 {
        return Err(io::Error::other("Invalid private backend file"));
    }
    #[cfg(not(unix))]
    let _ = secret;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if secret && meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::other("Backend file permissions are not private"));
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("Backend file exceeds its limit"));
    }
    Ok(bytes)
}

#[cfg(windows)]
fn windows_directory(path: &Path) -> io::Result<()> {
    use std::{
        os::windows::{ffi::OsStrExt, fs::MetadataExt},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, SetFileSecurityW, TokenUser, DACL_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    if fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
        return Err(io::Error::other("Backend state cannot be a reparse point"));
    }
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let mut needed = 0;
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed);
            if needed == 0 || needed > 65536 {
                return Err(io::Error::other("Could not read backend owner"));
            }
            let mut buffer = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
            if GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut sid = ptr::null_mut();
            if ConvertSidToStringSidW((*(buffer.as_ptr() as *const TOKEN_USER)).User.Sid, &mut sid)
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut length = 0;
            while *sid.add(length) != 0 {
                length += 1;
            }
            let owner = String::from_utf16_lossy(std::slice::from_raw_parts(sid, length));
            LocalFree(sid.cast());
            let sddl: Vec<u16> =
                format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{owner})")
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
            let mut descriptor = ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let ok = SetFileSecurityW(
                wide.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            );
            let error = io::Error::last_os_error();
            LocalFree(descriptor);
            if ok == 0 {
                Err(error)
            } else {
                Ok(())
            }
        })();
        CloseHandle(token);
        result
    }
}
