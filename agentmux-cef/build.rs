fn main() {
    // Emitting any rerun-if-changed directive disables Cargo's default
    // "rerun build.rs if any package file changed", so every input is listed
    // explicitly. The git-ref entries make the embedded commit hash / build
    // timestamp refresh whenever HEAD moves, even if no package file changed.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    println!("cargo:rerun-if-env-changed=TARGET");
    for git_path in ["../.git/HEAD", "../.git/refs", "../.git/packed-refs"] {
        if std::path::Path::new(git_path).exists() {
            println!("cargo:rerun-if-changed={git_path}");
        }
    }
    #[cfg(target_os = "windows")]
    println!("cargo:rerun-if-changed=resources/win/agentmux.ico");

    // Emit the target triple so we can locate sidecar binaries at runtime.
    println!(
        "cargo:rustc-env=AGENTMUX_TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap()
    );

    // Embed the git short hash + build timestamp for the Instance panel's
    // Build / Time rows. Falls back to "unknown" / 0 when git is unavailable
    // (e.g. building from a source tarball) — never fails the build.
    let git_hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=AGENTMUX_GIT_HASH={git_hash}");

    let build_time_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    println!("cargo:rustc-env=AGENTMUX_BUILD_TIME={build_time_ms}");

    // Windows: embed application icon + version info into PE VERSIONINFO resource.
    // FileDescription controls the "Name" column in Task Manager's Processes tab.
    // The actual exe filename controls WER crash dump names and Event Viewer entries.
    #[cfg(target_os = "windows")]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", &format!("AgentMux v{}", version));
        res.set("ProductName", "AgentMux");
        res.set("CompanyName", "AgentMux");
        res.set("InternalName", "agentmux");
        let icon_path = std::path::Path::new("resources/win/agentmux.ico");
        if icon_path.exists() {
            res.set_icon(icon_path.to_str().unwrap());
        }
        res.compile().expect("winres compile failed");
    }
}
