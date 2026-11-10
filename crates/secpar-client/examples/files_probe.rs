//! Runs the desktop's file transfer code against a real host: pairs with a one-time link, browses,
//! uploads three files (about 250 KB, 800 KB and 10 MB), downloads them back with `fs.get` and
//! compares SHA-256 hashes. A development check, not part of the product.
//!
//!   files_probe <file holding the pairing link> <work folder>
//!
//! The link is read from a file so it never shows up in a process list or a log.
use ring::digest::{digest, SHA256};
use secpar_client::control::Connected;
use secpar_client::files::{run, Notice, Phase, Request, Transfer};
use secpar_client::pairing::Pairing;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

fn sha(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    digest(&SHA256, &bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Deterministic noise, so a failure can be reproduced.
fn make(path: &Path, size: usize, seed: u64) {
    let mut x = seed | 1;
    let data: Vec<u8> = (0..size)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    std::fs::write(path, data).unwrap();
}

async fn wait_for(notices: &mut mpsc::UnboundedReceiver<Notice>, seen: &mut HashMap<String, Transfer>, what: &str, mut done: impl FnMut(&HashMap<String, Transfer>) -> bool) {
    let end = Instant::now() + Duration::from_secs(180);
    while !done(seen) {
        let left = end.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, notices.recv()).await {
            Ok(Some(Notice::Transfer(t))) => {
                if t.phase == Phase::Failed {
                    println!("  FAILED {}: {}", t.name, t.error);
                }
                seen.insert(t.id.clone(), t);
            }
            Ok(Some(Notice::Closed(why))) => panic!("connection closed while waiting for {what}: {why}"),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (link_file, work) = (PathBuf::from(&args[1]), PathBuf::from(&args[2]));
    let link = std::fs::read_to_string(link_file).unwrap();
    let pairing = Pairing::parse(link.trim()).expect("pairing link");
    let conn = Connected::pair(pairing, "files-probe").await.expect("pairing");
    println!("paired with {}", conn.computer.name);
    let (src, dl) = (work.join("src"), work.join("dl"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dl).unwrap();
    let files = [("probe-250k.bin", 250 * 1024, 11u64), ("probe-800k.bin", 800 * 1000, 22), ("probe-10m.bin", 10 * 1024 * 1024, 33)];
    for (name, size, seed) in files {
        make(&src.join(name), size, seed);
    }
    let (req_tx, req_rx) = mpsc::unbounded_channel();
    let (note_tx, mut notices) = mpsc::unbounded_channel();
    let worker = tokio::spawn(run(conn, dl.clone(), req_rx, note_tx, Arc::new(|| {})));

    // Browsing
    loop {
        match notices.recv().await.expect("notice") {
            Notice::Ready { browse, upload } => {
                println!("ready: browse={browse} upload={upload}");
                break;
            }
            other => println!("unexpected before ready: {other:?}"),
        }
    }
    req_tx.send(Request::List(String::new())).unwrap();
    loop {
        match notices.recv().await.expect("notice") {
            Notice::Listed { path, entries } => {
                println!("roots listed ({} entries) for path {:?}: {}", entries.len(), path, entries.iter().map(|e| e.name.clone()).take(6).collect::<Vec<_>>().join(", "));
                break;
            }
            Notice::Problem(p) => panic!("list refused: {p}"),
            _ => {}
        }
    }

    // Upload, host to PC: file.done carries the path the host stored it at.
    let mut seen = HashMap::new();
    let started = Instant::now();
    for (name, _, _) in files {
        req_tx.send(Request::Send(src.join(name))).unwrap();
    }
    wait_for(&mut notices, &mut seen, "uploads", |s| s.values().filter(|t| t.upload && t.finished()).count() == files.len()).await;
    println!("uploads took {:.1}s", started.elapsed().as_secs_f32());
    let mut ok = true;
    let mut host_paths = HashMap::new();
    for t in seen.values().filter(|t| t.upload) {
        println!("  up   {:<16} {:>9} bytes  {:?}  host path {}", t.name, t.size, t.phase, t.path);
        ok &= t.phase == Phase::Done;
        host_paths.insert(t.name.clone(), t.path.clone());
    }

    // Download, using the host's own listing to find each file again.
    let started = Instant::now();
    for (name, _, _) in files {
        let path = host_paths.get(name).expect("upload path").clone();
        req_tx.send(Request::Get(path)).unwrap();
    }
    wait_for(&mut notices, &mut seen, "downloads", |s| s.values().filter(|t| !t.upload && t.finished()).count() == files.len()).await;
    println!("downloads took {:.1}s", started.elapsed().as_secs_f32());
    for (name, size, _) in files {
        // The host may have renamed an upload (probe-250k (1).bin), so match on the size and base name.
        let got = seen.values().find(|t| !t.upload && t.size == size as u64 && t.name.starts_with(name.trim_end_matches(".bin")));
        match got {
            Some(t) if t.phase == Phase::Done => {
                let (a, b) = (sha(&src.join(name)), sha(Path::new(&t.path)));
                println!("  down {:<16} {:>9} bytes  sha256 {}  {}", name, size, &b[..16], if a == b { "MATCH" } else { "MISMATCH" });
                ok &= a == b;
            }
            _ => {
                println!("  down {name}: missing or failed");
                ok = false;
            }
        }
    }
    req_tx.send(Request::Stop).unwrap();
    let _ = worker.await;
    println!("RESULT {}", if ok { "PASS" } else { "FAIL" });
    std::process::exit(if ok { 0 } else { 1 });
}
