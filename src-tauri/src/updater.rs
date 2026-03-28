// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App update checker: queries GitHub Releases API on startup and emits
//! an `app-update-status` Tauri event when a newer version is available.

use serde::{Deserialize, Serialize};
use tauri::Emitter;

// ─── Install type detection ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)] // Variants are platform-specific
pub enum InstallType {
    Msix,
    Nsis,
    Portable,
    Dmg,
    AppImage,
    Unknown,
}

/// Detect how the app was installed based on environment and filesystem clues.
pub fn detect_install_type() -> InstallType {
    // 1. MSIX: check for Windows package identity
    #[cfg(target_os = "windows")]
    {
        if is_msix_package() {
            return InstallType::Msix;
        }
    }

    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return InstallType::Unknown,
    };
    let exe_dir = match exe_path.parent() {
        Some(d) => d,
        None => return InstallType::Unknown,
    };

    // 2. Portable: bin/ directory with sidecar next to the exe
    if exe_dir.join("bin").join("agentmuxsrv-rs.x64.exe").exists()
        || exe_dir.join("bin").join("agentmuxsrv-rs").exists()
    {
        return InstallType::Portable;
    }

    // 3. Platform-specific checks
    #[cfg(target_os = "windows")]
    {
        // NSIS: check for uninstall registry key
        if is_nsis_install() {
            return InstallType::Nsis;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let path_str = exe_path.to_string_lossy();
        if path_str.contains("/Applications/") || path_str.contains(".app/") {
            return InstallType::Dmg;
        }
    }

    #[cfg(target_os = "linux")]
    {
        if std::env::var("APPIMAGE").is_ok() {
            return InstallType::AppImage;
        }
        let path_str = exe_path.to_string_lossy();
        if path_str.contains(".AppImage") {
            return InstallType::AppImage;
        }
    }

    InstallType::Unknown
}

/// Check if the process is running inside an MSIX package (Windows only).
#[cfg(target_os = "windows")]
fn is_msix_package() -> bool {
    // GetCurrentPackageFullName returns ERROR_INSUFFICIENT_BUFFER (122) if packaged,
    // or APPMODEL_ERROR_NO_PACKAGE (15700) if not.
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut len: u32 = 0;
    let result = unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) };
    // ERROR_INSUFFICIENT_BUFFER = 122 means we ARE in a package
    result == 122
}

/// Check if AgentMux was installed via NSIS (Windows only).
#[cfg(target_os = "windows")]
fn is_nsis_install() -> bool {
    use windows_sys::Win32::System::Registry::*;
    let subkey = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AgentMux\0"
        .encode_utf16()
        .collect::<Vec<u16>>();
    let mut hkey = std::ptr::null_mut();
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        )
    };
    if result == 0 {
        unsafe { RegCloseKey(hkey) };
        return true;
    }
    // Also check HKCU (NSIS per-user install)
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        )
    };
    if result == 0 {
        unsafe { RegCloseKey(hkey) };
        return true;
    }
    false
}

// ─── GitHub release check ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatusPayload {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_type: Option<InstallType>,
}

/// Compare two semver version strings. Returns true if `remote` > `local`.
fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|s| s.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let r = parse(remote);
    let l = parse(local);
    for i in 0..r.len().max(l.len()) {
        let rv = r.get(i).copied().unwrap_or(0);
        let lv = l.get(i).copied().unwrap_or(0);
        if rv > lv {
            return true;
        }
        if rv < lv {
            return false;
        }
    }
    false
}

/// Check GitHub for the latest release and return update info if newer.
async fn check_github_release(
    current_version: &str,
) -> Result<Option<(String, String)>, String> {
    let client = reqwest::Client::builder()
        .user_agent("AgentMux-Updater")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;

    let resp = client
        .get("https://api.github.com/repos/agentmuxai/agentmux/releases/latest")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }

    let release: GitHubRelease = resp
        .json()
        .await
        .map_err(|e| format!("json parse error: {e}"))?;

    let remote_version = release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name);

    if is_newer(remote_version, current_version) {
        Ok(Some((remote_version.to_string(), release.html_url)))
    } else {
        Ok(None)
    }
}

// ─── Startup check ─────────────────────────────────────────────────────────

/// Spawn a one-shot update check 10 seconds after app start.
pub fn spawn_update_check(app: tauri::AppHandle) {
    let version = app
        .config()
        .version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let install_type = detect_install_type();

    tracing::info!(
        version = %version,
        install_type = ?install_type,
        "update checker initialized"
    );

    tauri::async_runtime::spawn(async move {
        // Don't compete with startup
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        match check_github_release(&version).await {
            Ok(Some((new_version, release_url))) => {
                tracing::info!(
                    current = %version,
                    available = %new_version,
                    "update available"
                );
                let payload = UpdateStatusPayload {
                    status: "available".to_string(),
                    version: Some(new_version),
                    release_url: Some(release_url),
                    install_type: Some(install_type),
                };
                let _ = app.emit("app-update-status", &payload);
            }
            Ok(None) => {
                tracing::info!("app is up-to-date (v{})", version);
            }
            Err(e) => {
                tracing::debug!("update check failed (non-fatal): {}", e);
            }
        }
    });
}

// ─── Install update command ─────────────────────────────────────────────────

/// Handle the "install_update" Tauri command. Opens the appropriate update
/// target based on install type.
#[tauri::command]
pub fn install_update() {
    let install_type = detect_install_type();
    tracing::info!(install_type = ?install_type, "install_update triggered");

    match install_type {
        InstallType::Msix => {
            // Open Microsoft Store page
            // TODO: Replace with actual Store product ID once published
            if let Err(e) = tauri_plugin_opener::open_url(
                "ms-windows-store://pdp/?productid=9P3JFPWWDZRC",
                None::<&str>,
            ) {
                tracing::warn!("failed to open Microsoft Store: {e}");
            }
        }
        _ => {
            // All other types: open GitHub releases page
            if let Err(e) = tauri_plugin_opener::open_url(
                "https://github.com/agentmuxai/agentmux/releases/latest",
                None::<&str>,
            ) {
                tracing::warn!("failed to open GitHub releases page: {e}");
            }
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("0.33.0", "0.32.97"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.32.98", "0.32.97"));
        assert!(!is_newer("0.32.97", "0.32.97"));
        assert!(!is_newer("0.32.96", "0.32.97"));
        assert!(!is_newer("0.31.0", "0.32.97"));
    }

    #[test]
    fn test_is_newer_different_lengths() {
        assert!(is_newer("0.33.0.1", "0.33.0"));
        assert!(!is_newer("0.33", "0.33.0"));
        assert!(is_newer("1.0", "0.99.99"));
    }

    #[test]
    fn test_detect_install_type_returns_something() {
        // Just ensure it doesn't panic
        let _ = detect_install_type();
    }
}
