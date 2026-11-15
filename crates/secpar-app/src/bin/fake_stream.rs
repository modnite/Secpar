//! A test-only stand-in for the Moonlight stream component.
//!
//! It lets the window's whole Connect path run without a hosting engine or media
//! hardware: `pair` writes the settings identity the real component would write
//! and exits, and `stream` stays alive until the window kills it. Built only with
//! the `fake-stream` feature; never part of a release package.
use std::{env, fs, path::Path, thread, time::Duration};

const BODY: &str = "MIIBvTCCASagAwIBAgIBADANBgkqhkiG9w0BAQsFADAjMSEwHwYDVQQDDBhOVklESUEgR2FtZVN0cmVhbSBDbGllbnQwHhcNMjYxMTE0MjExMDUyWhcNNDYxMTA5MjExMDUyWjAjMSEwHwYDVQQDDBhOVklESUEgR2FtZVN0cmVhbSBDbGllbnQ=";

fn write_identity(folder: &Path) {
    let pem = format!("-----BEGIN CERTIFICATE-----\n{BODY}\n-----END CERTIFICATE-----\n");
    let text = format!(
        "[General]\ncertificate=\"@ByteArray({})\"\nuniqueid=0123456789ABCDEF\n",
        pem.replace('\n', "\\n")
    );
    let settings = folder.join("Secpar");
    fs::create_dir_all(&settings).unwrap();
    // Write in one step so the relative-information reader never sees half a file.
    let temporary = settings.join("SecparStream.ini.tmp");
    fs::write(&temporary, text).unwrap();
    fs::rename(&temporary, settings.join("SecparStream.ini")).unwrap();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let current = env::current_dir().unwrap();
    match args.get(1).map(String::as_str) {
        Some("pair") => {
            write_identity(&current);
            // Give the window time to read the identity and ask the host to pair.
            thread::sleep(Duration::from_millis(1200));
        }
        Some("stream") => {
            fs::write(current.join("stream-running"), b"").unwrap();
            // A real component blocks in its window; the window kills this one.
            thread::sleep(Duration::from_secs(120));
        }
        _ => std::process::exit(2),
    }
}
