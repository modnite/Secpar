// The release version comes from version.txt in the repository root, so the
// window, the updater, the installer and the AppImage all name one version.
fn main() {
    println!("cargo:rerun-if-changed=../../version.txt");
    let version = std::fs::read_to_string("../../version.txt")
        .expect("version.txt in the repository root")
        .trim()
        .to_string();
    let parts: Vec<&str> = version.split('.').collect();
    assert!(parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok()), "version.txt must be X.Y.Z, found {version}");
    println!("cargo:rustc-env=SECPAR_VERSION={version}");
}
