fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=native/nvenc/bridge.cpp");
        println!("cargo:rerun-if-changed=native/nvenc/nvEncodeAPI.h");
        cc::Build::new().cpp(true).std("c++17")
            .flag_if_supported("/EHsc").flag_if_supported("/O2")
            .file("native/nvenc/bridge.cpp").compile("secpar_nvenc");
        if std::env::var_os("CARGO_FEATURE_FFMPEG").is_some() {
            println!("cargo:rerun-if-env-changed=SECPAR_FFMPEG_DIR");
            println!("cargo:rerun-if-changed=native/ffmpeg/bridge.cpp");
            let root=std::path::PathBuf::from(std::env::var_os("SECPAR_FFMPEG_DIR")
                .expect("Build the pinned encoder dependencies and set SECPAR_FFMPEG_DIR"));
            cc::Build::new().cpp(true).std("c++17").flag_if_supported("/EHsc")
                .flag_if_supported("/O2").flag_if_supported("/Gy").flag_if_supported("/Gw")
                .include(root.join("include")).file("native/ffmpeg/bridge.cpp").compile("secpar_avcodec");
            println!("cargo:rustc-link-search=native={}",root.join("lib").display());
            for library in ["avcodec","avutil","vpl"] {println!("cargo:rustc-link-lib=static={library}");}
            for library in ["d3d11","dxgi","dxva2","dxguid","ole32","advapi32","user32","bcrypt","cfgmgr32"] {
                println!("cargo:rustc-link-lib={library}");
            }
        }
    }
    // Refresh the PE icon resources whenever the application icon changes.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    #[cfg(feature = "webui")]
    tauri_build::build();
}
