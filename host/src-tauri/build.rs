fn main() {
    // Refresh the PE icon resources whenever the application icon changes.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    tauri_build::build()
}
