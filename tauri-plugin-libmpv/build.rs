fn main() {
    tauri_plugin::Builder::new(&[
        "init",
        "destroy",
        "command",
        "set_property",
        "get_property",
        "set_video_margin_ratio",
    ])
    .android_path("android")
    .ios_path("ios")
    .build();

    // Linux: link the LGPL libmpv.so directly for src/mpv_sys.rs. Previously the libmpv2-sys
    // crate's build script did this via pkg-config; now that we use our own ISC bindings we
    // emit the link flags ourselves. The release CI installs the bundled LGPL libmpv to
    // /usr/local/lib with a `libmpv.so` symlink (so `-lmpv` resolves at link time), and
    // linuxdeploy bundles the resulting libmpv.so.2 (SONAME) into the AppImage at package time.
    // Uses CARGO_CFG_TARGET_OS (the build TARGET), not cfg!, so it's correct under cross-compiles.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=dylib=mpv");
        println!("cargo:rustc-link-search=native=/usr/local/lib");
    }
}
