//! File transfer: phone → PC into the received-files folder (resumable), and PC → phone offers.
//! The portable logic lives in secpar-host-core and is shared with Linux; this file adds the
//! Windows free-space check, the toast and the drive listing.

pub use secpar_host_core::files::*;
use crate::proto::FsEntry;
use std::path::{Path, PathBuf};

fn free_space(dir: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    // SAFETY: valid path string and out-pointer for the duration of the call.
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(dir.as_os_str()), Some(&mut free), None, None).ok()? };
    Some(free)
}

/// An `Incoming` that checks free disk space and shows the "File received" toast.
pub fn incoming(dir: PathBuf) -> Incoming {
    Incoming::with_hooks(dir, Hooks { free_space: Some(free_space), received: Some(crate::notify::file_received) })
}

/// `fs.list`: a folder's entries (folders first), or for "" the drives and the user's folders.
pub fn list(path: &str, home: &Path) -> std::io::Result<Vec<FsEntry>> {
    if path.is_empty() {
        let mut v: Vec<FsEntry> = ["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"]
            .iter()
            .map(|n| home.join(n))
            .filter(|p| p.is_dir())
            .map(|p| FsEntry { name: p.file_name().unwrap().to_string_lossy().into(), path: p.to_string_lossy().into(), dir: true, size: 0, mtime: 0 })
            .collect();
        // SAFETY: no arguments.
        let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        for i in 0..26 {
            if mask & (1 << i) != 0 {
                let d = format!("{}:\\", (b'A' + i as u8) as char);
                v.push(FsEntry { name: d.clone(), path: d, dir: true, size: 0, mtime: 0 });
            }
        }
        return Ok(v);
    }
    list_dir(path, |_, m| {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x6 != 0 // hidden or system
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{chunk_frame, ServerMsg};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pcr-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    const ID: &str = "0123abcd-0000-4000-8000-00000000beef";

    fn sha(b: &[u8]) -> String {
        crate::tls::hex(&crate::tls::sha256(b))
    }

    #[test]
    fn sanitize_rejects_traversal_and_reserved() {
        assert_eq!(sanitize(r"..\..\Windows\evil.dll"), "evil.dll");
        assert_eq!(sanitize("../../etc/passwd"), "passwd");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("con.txt"), "_con.txt");
        assert_eq!(sanitize("NUL.txt"), "_NUL.txt");
        assert_eq!(sanitize("com1.tar.gz"), "_com1.tar.gz");
        assert_eq!(sanitize("lpt9"), "_lpt9");
        assert_eq!(sanitize("console.txt"), "console.txt");
        assert_eq!(sanitize("a<b>c:d\"e|f?g*h"), "a_b_c_d_e_f_g_h");
        assert_eq!(sanitize("report.pdf. . "), "report.pdf");
        assert_eq!(sanitize("tab\there\u{0}"), "tab_here_");
        assert_eq!(sanitize(".."), "file");
        assert_eq!(sanitize(""), "file");
        assert_eq!(sanitize("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize("héllo wörld.txt"), "héllo wörld.txt");
        assert!(sanitize(&"x".repeat(500)).chars().count() <= 200);
    }

    #[test]
    fn clash_adds_number() {
        let d = tmp("clash");
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a.txt"));
        std::fs::write(d.join("a.txt"), "1").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a (1).txt"));
        std::fs::write(d.join("a (1).txt"), "1").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a (2).txt"));
        std::fs::write(d.join("noext"), "1").unwrap();
        assert_eq!(unique_path(&d, "noext"), d.join("noext (1)"));
    }

    #[test]
    fn uuid_parses() {
        let id = parse_uuid(ID).unwrap();
        assert_eq!(id[0], 0x01);
        assert_eq!(id[15], 0xef);
        assert_eq!(parse_uuid("nope"), None);
        assert_eq!(format_uuid(&id), ID);
    }

    #[test]
    fn receives_whole_file() {
        let d = tmp("whole");
        let data = b"hello world, this is a file".to_vec();
        let mut rx = Incoming::new(d.clone());
        assert_eq!(rx.begin(ID, "../note.txt", data.len() as u64), ServerMsg::FileReady { id: ID.into(), offset: 0 });
        let id = parse_uuid(ID).unwrap();
        assert_eq!(rx.chunk(&chunk_frame(&id, 0, &data[..10])), None);
        assert_eq!(rx.chunk(&chunk_frame(&id, 10, &data[10..])), None);
        let done = rx.end(ID, &sha(&data));
        let path = d.join("note.txt");
        assert_eq!(done, ServerMsg::FileDone { id: ID.into(), path: path.to_string_lossy().into() });
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no .part left");
    }

    #[test]
    fn resume_appends_to_part() {
        let d = tmp("resume");
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let id = parse_uuid(ID).unwrap();
        {
            let mut rx = Incoming::new(d.clone());
            rx.begin(ID, "big.bin", data.len() as u64);
            rx.chunk(&chunk_frame(&id, 0, &data[..40_000]));
        } // connection dropped
        let mut rx = Incoming::new(d.clone());
        assert_eq!(rx.begin(ID, "big.bin", data.len() as u64), ServerMsg::FileReady { id: ID.into(), offset: 40_000 });
        rx.chunk(&chunk_frame(&id, 40_000, &data[40_000..]));
        assert!(matches!(rx.end(ID, &sha(&data)), ServerMsg::FileDone { .. }));
        assert_eq!(std::fs::read(d.join("big.bin")).unwrap(), data);
    }

    #[test]
    fn bad_hash_deletes_part() {
        let d = tmp("badhash");
        let id = parse_uuid(ID).unwrap();
        let mut rx = Incoming::new(d.clone());
        rx.begin(ID, "x.bin", 3);
        rx.chunk(&chunk_frame(&id, 0, b"abc"));
        assert_eq!(rx.end(ID, &sha(b"abd")), ServerMsg::FileError { id: ID.into(), code: "bad_hash".into() });
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
    }

    #[test]
    fn more_data_than_announced_is_too_large() {
        let d = tmp("toolarge");
        let id = parse_uuid(ID).unwrap();
        let mut rx = Incoming::new(d.clone());
        rx.begin(ID, "x.bin", 2);
        assert_eq!(rx.chunk(&chunk_frame(&id, 0, b"abc")), Some(ServerMsg::FileError { id: ID.into(), code: "too_large".into() }));
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
    }

    #[test]
    fn unknown_or_stale_chunks_are_ignored() {
        let d = tmp("stale");
        let id = parse_uuid(ID).unwrap();
        let mut rx = Incoming::new(d.clone());
        assert_eq!(rx.chunk(&chunk_frame(&id, 0, b"abc")), None, "no begin yet");
        rx.begin(ID, "x.bin", 6);
        rx.chunk(&chunk_frame(&id, 0, b"abc"));
        rx.chunk(&chunk_frame(&id, 0, b"abc")); // duplicate
        rx.chunk(&chunk_frame(&id, 3, b"def"));
        assert!(matches!(rx.end(ID, &sha(b"abcdef")), ServerMsg::FileDone { .. }));
    }

    #[test]
    fn outgoing_chunks_from_offset_hash_whole_file() {
        let d = tmp("out");
        std::fs::create_dir_all(&d).unwrap();
        let data: Vec<u8> = (0..(CHUNK * 2 + 100)).map(|i| (i % 7) as u8).collect();
        let p = d.join("s.bin");
        std::fs::write(&p, &data).unwrap();
        let id = parse_uuid(ID).unwrap();
        let mut frames = vec![];
        let hash = stream(&p, &id, 1000, &mut |f| {
            frames.push(f);
            true
        })
        .unwrap();
        assert_eq!(hash, sha(&data));
        let mut got = vec![];
        let mut next = 1000u64;
        for f in &frames {
            let Some(crate::proto::Binary::FileChunk { id: fid, offset, data }) = crate::proto::parse_binary(f) else { panic!() };
            assert_eq!((fid, offset), (id, next));
            assert!(data.len() <= CHUNK);
            next += data.len() as u64;
            got.extend(data);
        }
        assert_eq!(got, &data[1000..]);
    }

    #[test]
    fn relative_paths_cannot_escape() {
        assert_eq!(safe_rel("Photos/2024/a.jpg"), Path::new("Photos").join("2024").join("a.jpg"));
        assert_eq!(safe_rel("../../x.txt"), PathBuf::from("x.txt"));
        assert_eq!(safe_rel("a/../../b/x"), Path::new("a").join("b").join("x"));
        assert_eq!(safe_rel(r"C:\Windows\x.dll"), Path::new("C_").join("Windows").join("x.dll"));
        assert_eq!(safe_rel(r"\host\share\x"), Path::new("host").join("share").join("x"));
        assert_eq!(safe_rel("CON/aux.txt"), Path::new("_CON").join("_aux.txt"));
        assert_eq!(safe_rel(""), PathBuf::from("file"));
        assert_eq!(safe_rel("/../"), PathBuf::from("file"));
        for p in ["../x", r"..\..\x", "a/../../../x", r"C:\x", r"\h\s\x"] {
            assert!(safe_rel(p).components().all(|c| matches!(c, std::path::Component::Normal(_))), "{p}");
        }
    }

    #[test]
    fn upload_into_subfolder_and_cancel() {
        let d = tmp("sub");
        let mut rx = Incoming::new(d.clone());
        assert!(matches!(rx.begin(ID, "Trip/day 1/a.txt", 3), ServerMsg::FileReady { offset: 0, .. }));
        let id = parse_uuid(ID).unwrap();
        rx.chunk(&chunk_frame(&id, 0, b"abc"));
        let ServerMsg::FileDone { path, .. } = rx.end(ID, &sha(b"abc")) else { panic!() };
        assert_eq!(PathBuf::from(path), d.join("Trip").join("day 1").join("a.txt"));
        rx.begin(ID, "../b.txt", 3);
        rx.chunk(&chunk_frame(&id, 0, b"a"));
        rx.cancel(ID);
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
        assert!(matches!(rx.end(ID, &sha(b"abc")), ServerMsg::FileError { .. }));
    }

    #[test]
    fn folder_offers_keep_relative_names() {
        let d = tmp("tree");
        std::fs::create_dir_all(d.join("Album").join("raw")).unwrap();
        std::fs::write(d.join("Album").join("a.jpg"), b"1").unwrap();
        std::fs::write(d.join("Album").join("raw").join("b.dng"), b"22").unwrap();
        let out = Outgoing::default();
        let mut names: Vec<(String, u64)> = offer_tree(&out, &d.join("Album"))
            .unwrap()
            .into_iter()
            .map(|m| match m { ServerMsg::FileOffer { name, size, .. } => (name, size), _ => panic!() })
            .collect();
        names.sort();
        assert_eq!(names, vec![("Album/a.jpg".into(), 1), ("Album/raw/b.dng".into(), 2)]);
        assert_eq!(out.lock().unwrap().len(), 2);
        let listed = list(d.join("Album").to_str().unwrap(), &d).unwrap();
        assert_eq!(listed.iter().map(|e| (e.name.as_str(), e.dir)).collect::<Vec<_>>(), vec![("raw", true), ("a.jpg", false)]);
    }
}
