fn main() {
    println!("cargo:rerun-if-env-changed=SECPAR_RELEASE_TAG");
    println!(
        "cargo:rustc-env=SECPAR_RELEASE_TAG={}",
        std::env::var("SECPAR_RELEASE_TAG")
            .unwrap_or_else(|_| "v0.1.0-secpar-desktop-dev.8".into())
    );
    println!("cargo:rerun-if-changed=../host/src-tauri/icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        tauri_winres::WindowsResource::new()
            .set_icon("../host/src-tauri/icons/icon.ico")
            .set("ProductName", "Secpar")
            .set("FileDescription", "Secpar Desktop development client")
            .compile()
            .expect("Embed the Secpar triangle icon");
    }
}
