//! Compiles the Moonlight protocol core from the pinned source, without its
//! OpenSSL crypto file: `src/crypto.rs` provides those few functions instead.
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../upstream/src/sunshine/third-party/moonlight-common-c");
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let mut build = cc::Build::new();
    build
        .include(root.join("src"))
        .include(root.join("enet/include"))
        .include(root.join("nanors"))
        .include(root.join("nanors/deps"))
        .include(root.join("nanors/deps/obl"))
        .define("NDEBUG", None)
        .warnings(false);
    if target == "windows" {
        build.define("HAS_QOS_FLOWID", None).define("HAS_PQOS_FLOWID", None);
    } else {
        for flag in [
            "HAS_SOCKLEN_T", "HAS_FCNTL", "HAS_IOCTL", "HAS_POLL", "HAS_GETADDRINFO", "HAS_GETNAMEINFO",
            "HAS_INET_PTON", "HAS_INET_NTOP", "HAS_MSGHDR_FLAGS", "HAVE_CLOCK_GETTIME", "HAVE_CLOCK_MONOTONIC",
        ] {
            build.define(flag, "1");
        }
    }
    for entry in std::fs::read_dir(root.join("src")).expect("moonlight-common-c source") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".c") && name != "PlatformCrypto.c" {
            build.file(&path);
        }
    }
    let enet = if target == "windows" { "win32.c" } else { "unix.c" };
    for name in ["callbacks.c", "compress.c", "host.c", "list.c", "packet.c", "peer.c", "protocol.c", enet] {
        build.file(root.join("enet").join(name));
    }
    for name in ["nanors/rs.c", "nanors/deps/obl/oblas_common.c", "nanors/deps/obl/oblas_lite.c"] {
        build.file(root.join(name));
    }
    build.file("src/shim.c");
    build.compile("moonlight");
    if target == "windows" {
        for lib in ["ws2_32", "winmm", "qwave"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
    println!("cargo:rerun-if-changed=src/shim.c");
}
