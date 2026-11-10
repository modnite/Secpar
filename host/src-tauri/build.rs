fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=native/nvenc/bridge.cpp");
        println!("cargo:rerun-if-changed=native/nvenc/nvEncodeAPI.h");
        cc::Build::new().cpp(true).std("c++17")
            .flag_if_supported("/EHsc").flag_if_supported("/O2")
            .file("native/nvenc/bridge.cpp").compile("secpar_nvenc");
    }
    // Refresh the PE icon resources whenever the application icon changes.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    #[cfg(feature = "webui")]
    tauri_build::build();
}
