fn main() {
    // Emit the full target triple so sidecar.rs can locate the bundled binary by name.
    // e.g. "x86_64-pc-windows-msvc", "aarch64-apple-darwin", "x86_64-unknown-linux-gnu"
    println!(
        "cargo:rustc-env=AGENTMUX_TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap_or_default()
    );
    tauri_build::build()
}
