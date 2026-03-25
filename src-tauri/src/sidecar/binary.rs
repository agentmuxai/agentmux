use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// The live receiver and child handle returned after a successful process spawn.
pub struct SpawnedSidecar {
    pub rx: tokio::sync::mpsc::Receiver<CommandEvent>,
    pub child: CommandChild,
}

/// Resolve the binary path, deploy the bundled wsh, set environment variables,
/// and spawn the agentmuxsrv-rs process.
///
/// Called by both `spawn_backend` (initial launch) and `restart_backend`
/// (user-initiated restart). Does **not** modify `AppState` — the caller is
/// responsible for storing the child handle, PID, and start time.
pub fn build_sidecar_command(
    app: &tauri::AppHandle,
    auth_key: &str,
    version_data_home: &std::path::Path,
    version_config_home: &std::path::Path,
    config_dir: &std::path::Path,
    version_instance_id: &str,
) -> Result<SpawnedSidecar, String> {
    let backend_name = "agentmuxsrv-rs";
    let shell = app.shell();

    // Resolve the binary: portable → dev → versioned installer copy.
    let portable_path = std::env::current_exe().ok().and_then(|exe_path| {
        let exe_dir = exe_path.parent()?;

        // Portable release: bin/{name}.x64.exe next to the app exe.
        let portable_binary = exe_dir.join("bin").join(format!("{}.x64.exe", backend_name));
        if portable_binary.exists() {
            tracing::info!("Using portable {} at: {:?}", backend_name, portable_binary);
            return Some(portable_binary);
        }

        // Dev mode (tauri dev / cargo run): sync:dev:binaries copies the binary as
        // {name}[.exe] (no triple suffix) into target/debug/ alongside the host binary.
        let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
        let dev_binary = exe_dir.join(format!("{}{}", backend_name, exe_suffix));
        if dev_binary.exists() {
            tracing::info!("Using dev-mode {} at: {:?}", backend_name, dev_binary);
            return Some(dev_binary);
        }

        None
    });

    let sidecar_cmd = if let Some(portable_exe) = portable_path {
        // Portable: path is already version-isolated by the portable ZIP directory.
        shell.command(portable_exe)
    } else {
        // Installer / dev fallback: copy to version-isolated app-local data dir so
        // tauri-build can hash the source binary for the next version without hitting
        // a Windows read-lock from the currently-running process.
        let versioned_path = super::ensure_versioned_sidecar(app, backend_name)?;
        shell.command(versioned_path)
    };

    // Resolve AGENTMUX_APP_PATH (directory containing the host exe).
    let app_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();

    // Deploy the bundled wsh binary so the backend can find it.
    deploy_wsh(&app_path);

    let app_path_str = app_path.to_string_lossy().to_string();
    tracing::info!("Setting AGENTMUX_APP_PATH to: {}", app_path_str);

    let (rx, child) = sidecar_cmd
        .args([
            "--wavedata",
            &version_data_home.to_string_lossy(),
            "--instance",
            version_instance_id,
        ])
        .env("AGENTMUX_AUTH_KEY", auth_key)
        .env("AGENTMUX_CONFIG_HOME", version_config_home.to_string_lossy().to_string())
        .env("AGENTMUX_DATA_HOME", version_data_home.to_string_lossy().to_string())
        .env("AGENTMUX_SETTINGS_DIR", config_dir.to_string_lossy().to_string())
        .env("AGENTMUX_APP_PATH", &app_path_str)
        .env("AGENTMUX_DEV", if cfg!(debug_assertions) { "1" } else { "" })
        .env("WCLOUD_ENDPOINT", "https://api.agentmux.ai/central")
        .env("WCLOUD_WS_ENDPOINT", "wss://wsapi.agentmux.ai/")
        .spawn()
        .map_err(|e| format!("Failed to spawn agentmuxsrv-rs: {}", e))?;

    Ok(SpawnedSidecar { rx, child })
}

/// Copy the bundled `wsh` binary to `bin/wsh-{version}-{os}-{arch}` next to the
/// app exe.  Best-effort — logs a warning on failure but never aborts the spawn.
fn deploy_wsh(app_path: &std::path::Path) {
    let bin_dir = app_path.join("bin");
    if let Err(e) = std::fs::create_dir_all(&bin_dir) {
        tracing::warn!("Failed to create bin dir for wsh: {}", e);
        return;
    }

    let bundled_wsh = app_path.join("wsh");
    if !bundled_wsh.exists() {
        tracing::warn!("Bundled wsh not found at: {}", bundled_wsh.display());
        return;
    }

    let version = env!("CARGO_PKG_VERSION");
    let (goos, goarch) = if cfg!(target_os = "macos") {
        ("darwin", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    } else if cfg!(target_os = "linux") {
        ("linux", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    } else {
        ("windows", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    };

    let wsh_name = format!("wsh-{}-{}.{}", version, goos, goarch);
    let dest = bin_dir.join(&wsh_name);

    if dest.exists() {
        return; // already deployed
    }

    if let Err(e) = std::fs::copy(&bundled_wsh, &dest) {
        tracing::warn!("Failed to copy wsh to {}: {}", dest.display(), e);
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }

    tracing::info!("Deployed wsh to: {}", dest.display());
}
