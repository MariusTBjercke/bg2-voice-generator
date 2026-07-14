fn main() {
    // tauri-build only emits `cargo:rerun-if-changed` for tauri.conf.json +
    // capabilities, NOT for the icon files, so watch the icons dir explicitly.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
