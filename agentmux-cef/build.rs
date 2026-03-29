fn main() {
    // Emit the target triple so we can locate sidecar binaries at runtime.
    println!(
        "cargo:rustc-env=AGENTMUX_TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap()
    );

    // Windows: embed application icon
    #[cfg(target_os = "windows")]
    {
        // Only embed icon if the resource file exists
        let icon_path = std::path::Path::new("resources/win/agentmux.ico");
        if icon_path.exists() {
            let _ = winres::WindowsResource::new()
                .set_icon(icon_path.to_str().unwrap())
                .compile();
        }
    }
}
