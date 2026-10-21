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
//! - macOS: inside an `.app` (srv runs nested in it, so the app is the
//!   outermost one); it can only be upgraded in place from a folder the user
//!   can write, not from the mounted `.dmg` or an App-Translocated copy;
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
    /// A macOS app: `app` is the outermost `.app` srv runs inside, and
    /// `blocked` says why it can't be replaced in place (if it can't).
    MacApp { app: PathBuf, blocked: Option<String> },
    LinuxAppImage,
    /// A Linux portable: the release tarball's `AgentMux/` folder, which holds
    /// `agentmux.sh` above `usr/bin/` (or an older folder with the marker).
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
            Self::MacApp { .. } => "macos-app",
            Self::LinuxAppImage => "linux-appimage",
            Self::LinuxPortable { .. } => "linux-portable",
            Self::LinuxPackage { .. } => "linux-package",
            Self::Unknown => "unknown",
        }
    }

    /// Whether AgentMux can download and stage this kind's upgrade itself.
    /// The others still open the release page (or the Store).
    pub fn can_download(&self) -> bool {
        match self {
            Self::MacApp { blocked, .. } => blocked.is_none(),
            _ => matches!(
                self,
                Self::WindowsPortable { .. } | Self::WindowsSetup | Self::LinuxPortable { .. } | Self::LinuxAppImage
            ),
        }
    }

    /// The release file that upgrades this kind to `version` on `arch`
    /// (`std::env::consts::ARCH`), as `.github/workflows/build-*.yml` name them.
    pub fn asset_name(&self, version: &str, arch: &str) -> Option<String> {
        let x64 = arch == "x86_64";
        match self {
            Self::WindowsPortable { .. } if x64 => Some(format!("agentmux-{version}-x64-portable.7z")),
            Self::WindowsSetup if x64 => Some(format!("AgentMux-{version}-x64-setup.exe")),
            Self::MacApp { .. } if arch == "aarch64" => Some(format!("AgentMux_{version}_arm64.dmg")),
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
        "macos" if text.contains(".app/Contents/") => mac_app(exe),
        // The release tarball: `AgentMux/agentmux.sh` above srv's `usr/bin/`.
        "linux" if dir.is_some_and(|d| d.ends_with("usr/bin")) && linux_portable_root(exe).is_some() => {
            InstallKind::LinuxPortable { root: linux_portable_root(exe).unwrap_or_default() }
        }
        "linux" if appimage => InstallKind::LinuxAppImage,
        "linux" => match portable_root {
            Some(root) => InstallKind::LinuxPortable { root },
            None if text.starts_with("/usr/") || text.starts_with("/opt/") => InstallKind::LinuxPackage { debian },
            None => InstallKind::Unknown,
        },
        _ => InstallKind::Unknown,
    }
}

/// The `.app` srv at `exe` belongs to, by the bundle layout: srv runs in a
/// helper app nested in `AgentMux.app/Contents/Helpers/`
/// (`runtime_mode::app_bundle_of`).
pub fn outermost_app(exe: &Path) -> Option<PathBuf> {
    agentmux_common::runtime_mode::app_bundle_of(exe)
}

/// Why `app` can't be replaced in place, if it can't. `read_only` is whether
/// its volume is mounted read-only (a `.dmg` it was run straight from);
/// `parent_writable` whether the user can write the folder that holds it.
fn replace_blocker(app: &Path, read_only: bool, parent_writable: bool) -> Option<String> {
    if read_only {
        return Some(format!("{} is on a read-only volume (run from the .dmg?); drag it to Applications first", app.display()));
    }
    // Gatekeeper's App Translocation runs a quarantined app from a random
    // read-only mount, so what's replaced there would not be the real app.
    if app.to_string_lossy().contains("/AppTranslocation/") {
        return Some(format!("{} is App-Translocated; move it to Applications and open it from there", app.display()));
    }
    if !parent_writable {
        return Some(format!("the folder holding {} isn't writable by this user", app.display()));
    }
    None
}

/// Whether the volume holding `path` is mounted read-only.
// `MNT_RDONLY` is BSD's (macOS included), not Linux's; only macOS asks.
#[cfg(target_os = "macos")]
fn on_read_only_volume(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return true };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path and `st` a zeroed statfs the call fills.
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return true;
    }
    (st.f_flags as u64) & (libc::MNT_RDONLY as u64) != 0
}

#[cfg(not(target_os = "macos"))]
fn on_read_only_volume(_: &Path) -> bool {
    false
}

/// Whether this user can create and remove entries in `dir`.
#[cfg(unix)]
fn writable_dir(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(dir.as_os_str().as_bytes())
        // SAFETY: `c` is a NUL-terminated path.
        .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) } == 0)
}

#[cfg(not(unix))]
fn writable_dir(_: &Path) -> bool {
    true
}

/// The macOS app srv at `exe` runs in, and whether it can be replaced in place.
fn mac_app(exe: &Path) -> InstallKind {
    let app = outermost_app(exe).unwrap_or_else(|| exe.to_path_buf());
    let parent = app.parent().unwrap_or(&app);
    // Whoever else can rename entries beside the app could swap a checked
    // bundle during the launcher's swap; the launcher refuses the same.
    let blocked = replace_blocker(&app, on_read_only_volume(&app), writable_dir(parent)).or_else(|| {
        agentmux_common::runtime_mode::folder_others_can_change(parent).map(|who| format!("{who}; drag AgentMux to Applications first"))
    });
    if let Some(reason) = &blocked {
        tracing::info!(app = %app.display(), "update: this AgentMux can't upgrade itself in place: {reason}");
    }
    InstallKind::MacApp { app, blocked }
}

/// The Linux portable folder above srv at `exe` (`AgentMux/usr/bin/srv`):
/// the one holding `agentmux.sh`.
pub fn linux_portable_root(exe: &Path) -> Option<PathBuf> {
    let root = exe.parent()?.parent()?.parent()?;
    root.join("agentmux.sh").is_file().then(|| root.to_path_buf())
}

/// How the running AgentMux was installed.
pub fn detect() -> InstallKind {
    let Ok(exe) = std::env::current_exe() else { return InstallKind::Unknown };
    detect_at(
        &exe,
        std::env::consts::OS,
        agentmux_common::runtime_mode::running_appimage(&exe).is_some(),
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
        assert!(matches!(detect_at(app, "macos", false, false), InstallKind::MacApp { app, .. } if app == Path::new("/Applications/AgentMux.app")));
        let bin = Path::new("/tmp/.mount_AgentM/usr/bin/agentmux-srv");
        assert_eq!(detect_at(bin, "linux", true, true), InstallKind::LinuxAppImage);
        let deb = Path::new("/opt/AgentMux/agentmux-srv");
        assert_eq!(detect_at(deb, "linux", false, true), InstallKind::LinuxPackage { debian: true });
        let dir = tempfile::tempdir().unwrap();
        let exe = portable(dir.path());
        assert_eq!(detect_at(&exe, "linux", false, false), InstallKind::LinuxPortable { root: dir.path().to_path_buf() });
        // The release tarball: `AgentMux/agentmux.sh` above `usr/bin/`, no
        // marker. It wins over an `APPIMAGE` a pane passed down.
        let tarball = tempfile::tempdir().unwrap();
        let root = tarball.path().join("AgentMux");
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::write(root.join("agentmux.sh"), b"").unwrap();
        let srv = root.join("usr/bin/agentmux-srv-0.59.30-linux.x64");
        for appimage in [false, true] {
            assert_eq!(detect_at(&srv, "linux", appimage, true), InstallKind::LinuxPortable { root: root.clone() });
        }
        assert!(InstallKind::LinuxPortable { root }.can_download() && InstallKind::LinuxAppImage.can_download());
    }

    #[test]
    fn each_kind_names_the_release_file_ci_publishes() {
        let p = InstallKind::WindowsPortable { root: PathBuf::new() };
        assert_eq!(p.asset_name("0.59.27", "x86_64").as_deref(), Some("agentmux-0.59.27-x64-portable.7z"));
        assert_eq!(p.asset_name("0.59.27", "aarch64"), None);
        assert_eq!(InstallKind::MacApp { app: PathBuf::new(), blocked: None }.asset_name("0.59.27", "aarch64").as_deref(), Some("AgentMux_0.59.27_arm64.dmg"));
        assert_eq!(InstallKind::WindowsStore.asset_name("0.59.27", "x86_64"), None);
        assert_eq!(
            InstallKind::LinuxPackage { debian: false }.asset_name("0.59.27", "x86_64").as_deref(),
            Some("AgentMux-0.59.27-1.x86_64.rpm")
        );
        assert!(p.can_download());
        assert!(InstallKind::WindowsSetup.can_download());
        assert!(!InstallKind::WindowsStore.can_download());
    }

    const NESTED: &str = "AgentMux.app/Contents/Helpers/AgentMux Server.app/Contents/MacOS/srv";

    #[test]
    fn the_outermost_app_is_the_one_replaced() {
        let exe = Path::new("/Applications").join(NESTED);
        assert_eq!(outermost_app(&exe), Some(PathBuf::from("/Applications/AgentMux.app")));
        // A volume or folder name with spaces, or `.app` inside a name, is no matter.
        let dmg = Path::new("/Volumes/AgentMux 3").join(NESTED);
        assert_eq!(outermost_app(&dmg), Some(PathBuf::from("/Volumes/AgentMux 3/AgentMux.app")));
        let nested_in_app = Path::new("/Users/u/Foo.app/Resources").join(NESTED);
        // A folder around the install that only ends in .app is never replaced.
        assert_eq!(outermost_app(&nested_in_app), Some(PathBuf::from("/Users/u/Foo.app/Resources/AgentMux.app")));
        assert_eq!(outermost_app(Path::new("/usr/local/bin/srv")), None);
        // `.app` only counts at the end of a name.
        assert_eq!(outermost_app(Path::new("/x/my.application/srv")), None);
    }

    #[test]
    fn an_app_is_replaced_in_place_only_where_that_can_work() {
        let app = Path::new("/Applications/AgentMux.app");
        assert_eq!(replace_blocker(app, false, true), None);
        // Run straight from the mounted .dmg.
        let dmg = replace_blocker(Path::new("/Volumes/AgentMux 3/AgentMux.app"), true, true).unwrap();
        assert!(dmg.contains("read-only volume"), "{dmg}");
        let translocated = Path::new("/private/var/folders/ab/xyz/T/AppTranslocation/1234-5678/d/AgentMux.app");
        assert!(replace_blocker(translocated, false, true).unwrap().contains("App-Translocated"));
        assert!(replace_blocker(app, false, false).unwrap().contains("isn't writable"));
        // Each reason alone is enough, and the read-only one is reported first.
        assert!(replace_blocker(translocated, true, false).unwrap().contains("read-only"));
    }

    #[test]
    fn a_mac_app_downloads_only_when_it_can_be_replaced() {
        let kind = |blocked: Option<&str>| InstallKind::MacApp { app: PathBuf::from("/Applications/AgentMux.app"), blocked: blocked.map(String::from) };
        assert!(kind(None).can_download());
        assert!(!kind(Some("read-only")).can_download());
        assert_eq!(kind(None).label(), "macos-app");
    }

    #[cfg(unix)]
    #[test]
    fn the_filesystem_checks_follow_the_real_folder() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        assert!(writable_dir(dir.path()));
        assert!(!on_read_only_volume(dir.path()));
        // A bundle laid out like the release's, in a folder the user can write.
        let exe = dir.path().join(NESTED);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let kind = detect_at(&exe, "macos", false, false);
        assert_eq!(kind, InstallKind::MacApp { app: dir.path().join("AgentMux.app"), blocked: None });
        assert!(kind.can_download());
        // The same bundle in a folder the user can't write (root ignores modes).
        let locked = dir.path().join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let in_locked = locked.join(NESTED);
        let blocked = detect_at(&in_locked, "macos", false, false);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            assert!(!blocked.can_download(), "{blocked:?}");
        }
        assert!(!writable_dir(&dir.path().join("missing")));
    }
}
