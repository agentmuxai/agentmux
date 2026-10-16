// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How this AgentMux was installed, and which release file upgrades it
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4.2).
//!
//! Read from where the running srv binary is, so nothing has to be passed
//! down from the launcher:
//! - Windows portable: srv is in `<folder>/runtime/`, beside
//!   `agentmux-portable.marker`, and `<folder>/agentmux.exe` is the launcher.
//!   `setup.exe` installs the same `runtime/` (marker included), so a folder
//!   with Inno Setup's uninstaller (`unins000.exe`) is a `setup.exe` install;
//! - Microsoft Store: under `WindowsApps`; the Store updates it;
//! - Windows `setup.exe`: any other Windows install;
//! - macOS: inside an `.app`;
//! - Linux: an AppImage (`$APPIMAGE`), a portable folder (the marker), or a
//!   `.deb`/`.rpm` (under `/usr` or `/opt`).

use std::path::{Path, PathBuf};

const PORTABLE_MARKER: &str = "agentmux-portable.marker";

/// How this AgentMux was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// A Windows portable folder; `root` holds `agentmux.exe`.
    WindowsPortable { root: PathBuf },
    WindowsSetup,
    WindowsStore,
    MacApp,
    LinuxAppImage,
    LinuxPortable { root: PathBuf },
    /// A `.deb` (`debian`) or `.rpm` install.
    LinuxPackage { debian: bool },
    Unknown,
}

impl InstallKind {
    /// The name the UI sees.
    pub fn label(&self) -> &'static str {
        match self {
            Self::WindowsPortable { .. } => "windows-portable",
            Self::WindowsSetup => "windows-setup",
            Self::WindowsStore => "windows-store",
            Self::MacApp => "macos-app",
            Self::LinuxAppImage => "linux-appimage",
            Self::LinuxPortable { .. } => "linux-portable",
            Self::LinuxPackage { .. } => "linux-package",
            Self::Unknown => "unknown",
        }
    }

    /// Whether AgentMux can download and stage this kind's upgrade itself.
    /// The others still open the release page (or the Store).
    pub fn can_download(&self) -> bool {
        matches!(self, Self::WindowsPortable { .. } | Self::WindowsSetup)
    }

    /// The release file that upgrades this kind to `version` on `arch`
    /// (`std::env::consts::ARCH`), as `.github/workflows/build-*.yml` name them.
    pub fn asset_name(&self, version: &str, arch: &str) -> Option<String> {
        let x64 = arch == "x86_64";
        match self {
            Self::WindowsPortable { .. } if x64 => Some(format!("agentmux-{version}-x64-portable.7z")),
            Self::WindowsSetup if x64 => Some(format!("AgentMux-{version}-x64-setup.exe")),
            Self::MacApp if arch == "aarch64" => Some(format!("AgentMux_{version}_arm64.dmg")),
            Self::LinuxAppImage if x64 => Some(format!("AgentMux_{version}_amd64.AppImage")),
            Self::LinuxPortable { .. } if x64 => Some(format!("AgentMux_{version}_amd64-portable.tar.xz")),
            Self::LinuxPackage { debian: true } if x64 => Some(format!("AgentMux_{version}_amd64.deb")),
            Self::LinuxPackage { debian: false } if x64 => Some(format!("AgentMux-{version}-1.x86_64.rpm")),
            _ => None,
        }
    }
}

/// Whether `dir` holds Inno Setup's uninstaller (`unins000.exe`, or
/// `unins001.exe`… when one was already there): a `setup.exe` install.
pub fn has_uninstaller(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(|e| e.ok()).any(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("unins") && name.ends_with(".exe")
        })
    })
}

/// The kind for an srv binary at `exe` on `os` (`std::env::consts::OS`).
fn detect_at(exe: &Path, os: &str, appimage: bool, debian: bool) -> InstallKind {
    let dir = exe.parent();
    let portable_root = dir
        .filter(|d| d.join(PORTABLE_MARKER).is_file())
        .and_then(|d| d.parent())
        .map(Path::to_path_buf);
    let text = exe.to_string_lossy().replace('\\', "/");
    match os {
        "windows" => {
            if text.to_ascii_lowercase().contains("/windowsapps/") {
                InstallKind::WindowsStore
            } else if let Some(root) =
                portable_root.filter(|r| r.join("agentmux.exe").is_file() && !has_uninstaller(r))
            {
                InstallKind::WindowsPortable { root }
            } else if dir.and_then(Path::parent).is_some_and(|app| has_uninstaller(app) && app.join("agentmux.exe").is_file()) {
                // srv is in `{app}\runtime\`; the launcher checks the same uninstaller.
                InstallKind::WindowsSetup
            } else {
                InstallKind::Unknown
            }
        }
        "macos" if text.contains(".app/Contents/") => InstallKind::MacApp,
        "linux" if appimage => InstallKind::LinuxAppImage,
        "linux" => match portable_root {
            Some(root) => InstallKind::LinuxPortable { root },
            None if text.starts_with("/usr/") || text.starts_with("/opt/") => InstallKind::LinuxPackage { debian },
            None => InstallKind::Unknown,
        },
        _ => InstallKind::Unknown,
    }
}

/// How the running AgentMux was installed.
pub fn detect() -> InstallKind {
    let Ok(exe) = std::env::current_exe() else { return InstallKind::Unknown };
    detect_at(
        &exe,
        std::env::consts::OS,
        std::env::var_os("APPIMAGE").is_some(),
        Path::new("/etc/debian_version").exists(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portable(root: &Path) -> PathBuf {
        let runtime = root.join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(runtime.join(PORTABLE_MARKER), b"").unwrap();
        std::fs::write(root.join("agentmux.exe"), b"").unwrap();
        runtime.join("agentmux-srv-0.59.26-windows.x64.exe")
    }

    #[test]
    fn windows_installs_are_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("agentmux-0.59.26-x64-portable");
        let exe = portable(&root);
        assert_eq!(detect_at(&exe, "windows", false, false), InstallKind::WindowsPortable { root });
        let store = Path::new(r"C:\Program Files\WindowsApps\AgentMux.AgentMux_0.59.26.0_x64__vqr1k32tkfk4y\runtime\srv.exe");
        assert_eq!(detect_at(store, "windows", false, false), InstallKind::WindowsStore);
        // Neither a portable nor an install with the uninstaller: not one
        // AgentMux knows how to upgrade (the launcher would refuse it).
        let other = Path::new(r"C:\Users\u\AppData\Local\Programs\AgentMux\runtime\srv.exe");
        assert_eq!(detect_at(other, "windows", false, false), InstallKind::Unknown);
        // setup.exe ships the portable's runtime/, marker and all: its
        // uninstaller is what tells the two apart.
        let installed = dir.path().join("Programs/AgentMux");
        let exe = portable(&installed);
        std::fs::write(installed.join("unins000.exe"), b"").unwrap();
        assert_eq!(detect_at(&exe, "windows", false, false), InstallKind::WindowsSetup);
    }

    #[test]
    fn mac_and_linux_installs_are_told_apart() {
        let app = Path::new("/Applications/AgentMux.app/Contents/Helpers/AgentMux Server.app/Contents/MacOS/srv");
        assert_eq!(detect_at(app, "macos", false, false), InstallKind::MacApp);
        let bin = Path::new("/tmp/.mount_AgentM/usr/bin/agentmux-srv");
        assert_eq!(detect_at(bin, "linux", true, true), InstallKind::LinuxAppImage);
        let deb = Path::new("/opt/AgentMux/agentmux-srv");
        assert_eq!(detect_at(deb, "linux", false, true), InstallKind::LinuxPackage { debian: true });
        let dir = tempfile::tempdir().unwrap();
        let exe = portable(dir.path());
        assert_eq!(detect_at(&exe, "linux", false, false), InstallKind::LinuxPortable { root: dir.path().to_path_buf() });
    }

    #[test]
    fn each_kind_names_the_release_file_ci_publishes() {
        let p = InstallKind::WindowsPortable { root: PathBuf::new() };
        assert_eq!(p.asset_name("0.59.27", "x86_64").as_deref(), Some("agentmux-0.59.27-x64-portable.7z"));
        assert_eq!(p.asset_name("0.59.27", "aarch64"), None);
        assert_eq!(InstallKind::MacApp.asset_name("0.59.27", "aarch64").as_deref(), Some("AgentMux_0.59.27_arm64.dmg"));
        assert_eq!(InstallKind::WindowsStore.asset_name("0.59.27", "x86_64"), None);
        assert_eq!(
            InstallKind::LinuxPackage { debian: false }.asset_name("0.59.27", "x86_64").as_deref(),
            Some("AgentMux-0.59.27-1.x86_64.rpm")
        );
        assert!(p.can_download());
        assert!(InstallKind::WindowsSetup.can_download());
        assert!(!InstallKind::WindowsStore.can_download());
    }
}
