// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Linux side of "Upgrade & restart"
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4.2, phase
//! 3e), for `upgrade_restart`: checking what srv names to start (a portable's
//! `AgentMux/agentmux.sh`, or an AppImage), asking the user with `zenity` or
//! `kdialog`, and what the new version must not inherit.

use std::path::{Path, PathBuf};

/// What this version's own start set for its own files, which the new
/// version must not inherit: `agentmux.sh` and AppRun point `LD_LIBRARY_PATH`
/// at this version's `usr/bin` (the new launcher would load this `libcef.so`),
/// and AppRun's `APPIMAGE`… would make it think it is this AppImage. Both set
/// their own again when the new version starts.
#[cfg_attr(windows, allow(dead_code))]
pub const NOT_INHERITED: &[&str] = &["LD_LIBRARY_PATH", "APPIMAGE", "APPDIR", "OWD", "ARGV0"];

/// Set on the new version: AppImageLauncher, when installed, otherwise stops a
/// freshly downloaded AppImage at its "integrate?" window until someone answers.
#[cfg_attr(windows, allow(dead_code))]
pub const APPIMAGELAUNCHER_OFF: (&str, &str) = ("APPIMAGELAUNCHER_DISABLE", "1");

/// The Linux portable folder (`AgentMux/`, holding `agentmux.sh`) the launcher
/// in `exe_dir` (its `usr/bin/`) belongs to.
pub fn portable_root(exe_dir: &Path) -> Option<PathBuf> {
    if !exe_dir.ends_with("usr/bin") {
        return None;
    }
    let root = exe_dir.parent()?.parent()?;
    root.join("agentmux.sh").is_file().then(|| root.to_path_buf())
}

/// Whether `name` is the release's AppImage for `version`
/// (`AgentMux_<v>_amd64.AppImage`), or the copy srv makes beside one recorded
/// for another channel (`AgentMux_<v>_amd64-2.AppImage`).
fn is_appimage_name(name: &str, version: &str) -> bool {
    name.strip_prefix(&format!("AgentMux_{version}_amd64"))
        .and_then(|rest| rest.strip_suffix(".AppImage"))
        .is_some_and(|n| n.is_empty() || n.strip_prefix('-').is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Check the new version srv names at `target` against this install. This
/// install is a portable in `own_root`, or the AppImage `own_appimage`; the
/// new version, of `version`, is where srv puts it: beside this one, or in
/// this version's `updates/<v>/` in `data_dir`.
/// - A portable: `agentmux.sh` of a folder whose `usr/share/agentmux/VERSION`
///   says `version` and which holds that version's srv.
/// - An AppImage: that version's release file, executable.
pub fn check_target(
    target: &Path,
    version: &str,
    own_root: Option<&Path>,
    own_appimage: Option<&Path>,
    data_dir: &Path,
) -> Result<(), String> {
    if !target.is_absolute() || !target.is_file() {
        return Err(format!("{} isn't an AgentMux {version}", target.display()));
    }
    let dir = target.parent().ok_or("the new version has no folder")?;
    let updates = data_dir.join("updates").join(version);
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name == "agentmux.sh" {
        let own = own_root.ok_or("this AgentMux isn't a Linux portable")?;
        let says = std::fs::read_to_string(dir.join("usr/share/agentmux/VERSION")).unwrap_or_default();
        if says.trim() != version || !dir.join("usr/bin").join(format!("agentmux-srv-{version}-linux.x64")).is_file() {
            return Err(format!("{} isn't an AgentMux {version} portable", dir.display()));
        }
        if dir == own {
            return Err("that is the running AgentMux".into());
        }
        if dir.parent() != own.parent() && dir.parent() != Some(updates.as_path()) {
            return Err(format!("{} isn't where AgentMux put the new version", dir.display()));
        }
        return Ok(());
    }
    let own = own_appimage.ok_or("this AgentMux isn't an AppImage")?;
    if !is_appimage_name(&name, version) || !is_executable(target) {
        return Err(format!("{} isn't the AgentMux {version} AppImage", target.display()));
    }
    if target == own {
        return Err("that is the running AgentMux".into());
    }
    if Some(dir) != own.parent() && dir != updates {
        return Err(format!("{} isn't where AgentMux put the new version", target.display()));
    }
    Ok(())
}

/// `text` for zenity's `--text`, which reads Pango markup: a path's `&`, `<`
/// or `>` shown as they are.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn pango_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The dialog's process while it is open (0 when none).
#[cfg(target_os = "linux")]
static DIALOG_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Ask the user `body` in a native dialog, `zenity` (GNOME and most desktops)
/// or else `kdialog` (KDE): a separate program no page or agent in AgentMux
/// can click. `true` only for an explicit "Upgrade & restart". With neither
/// installed, nothing restarts: the new version is staged, and the user can
/// start it.
#[cfg(target_os = "linux")]
pub fn confirm(title: &str, body: &str) -> bool {
    use std::sync::atomic::Ordering;
    let markup = pango_escape(body);
    let zenity_title = format!("--title={title}");
    let zenity_text = format!("--text={markup}");
    let dialogs: [(&str, Vec<&str>); 2] = [
        (
            "zenity",
            vec!["--question", &zenity_title, &zenity_text, "--ok-label=Upgrade & restart", "--cancel-label=Not now", "--width=460"],
        ),
        ("kdialog", vec!["--title", title, "--yes-label", "Upgrade & restart", "--no-label", "Not now", "--yesno", body]),
    ];
    for (program, args) in dialogs {
        let mut cmd = std::process::Command::new(program);
        cmd.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        // Not this version's libraries: they could break the dialog's own.
        for key in NOT_INHERITED {
            cmd.env_remove(key);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                let pid = child.id();
                DIALOG_PID.store(pid, Ordering::SeqCst);
                let yes = child.wait().is_ok_and(|s| s.success());
                // Only if it's still ours: a newer dialog may have opened
                // after this one was closed.
                let _ = DIALOG_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
                return yes;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                crate::log(&format!("upgrade: couldn't open {program} to ask: {e}"));
                return false;
            }
        }
    }
    crate::log("upgrade: neither zenity nor kdialog is installed to ask with; the new version is staged, start it yourself");
    false
}

/// Close the open dialog (as "Not now").
#[cfg(target_os = "linux")]
pub fn dismiss() {
    let pid = DIALOG_PID.swap(0, std::sync::atomic::Ordering::SeqCst);
    if pid != 0 {
        // SAFETY: signals the dialog this launcher started and still waits on.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appimage_names_are_the_release_s_or_srv_s_copy() {
        assert!(is_appimage_name("AgentMux_0.59.30_amd64.AppImage", "0.59.30"));
        assert!(is_appimage_name("AgentMux_0.59.30_amd64-2.AppImage", "0.59.30"));
        assert!(!is_appimage_name("AgentMux_0.59.31_amd64.AppImage", "0.59.30"));
        assert!(!is_appimage_name("AgentMux_0.59.30_amd64-x.AppImage", "0.59.30"));
        assert!(!is_appimage_name("AgentMux_0.59.30_amd64.AppImage.part", "0.59.30"));
    }

    #[test]
    fn paths_show_as_they_are_in_zenity() {
        assert_eq!(pango_escape("/home/a&b/<x>"), "/home/a&amp;b/&lt;x&gt;");
    }

    fn portable(at: &Path, version: &str) -> PathBuf {
        std::fs::create_dir_all(at.join("usr/bin")).unwrap();
        std::fs::create_dir_all(at.join("usr/share/agentmux")).unwrap();
        std::fs::write(at.join("agentmux.sh"), b"").unwrap();
        std::fs::write(at.join("usr/share/agentmux/VERSION"), format!("{version}\n")).unwrap();
        std::fs::write(at.join("usr/bin").join(format!("agentmux-srv-{version}-linux.x64")), b"").unwrap();
        at.join("agentmux.sh")
    }

    #[test]
    fn only_that_versions_portable_beside_this_one_or_in_updates_starts() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let own = root.path().join("AgentMux");
        portable(&own, "0.59.29");
        assert_eq!(portable_root(&own.join("usr/bin")).as_deref(), Some(own.as_path()));
        assert_eq!(portable_root(&own.join("usr/lib")), None);
        let new = portable(&root.path().join("AgentMux-0.59.30"), "0.59.30");
        assert!(check_target(&new, "0.59.30", Some(&own), None, &data).is_ok());
        let staged = portable(&data.join("updates/0.59.30/AgentMux-0.59.30"), "0.59.30");
        assert!(check_target(&staged, "0.59.30", Some(&own), None, &data).is_ok());
        // Not that version, not this install's kind, the running one, or elsewhere.
        assert!(check_target(&new, "0.59.31", Some(&own), None, &data).is_err());
        assert!(check_target(&new, "0.59.30", None, Some(Path::new("/x.AppImage")), &data).is_err());
        assert!(check_target(&own.join("agentmux.sh"), "0.59.29", Some(&own), None, &data).is_err());
        let far = portable(&root.path().join("elsewhere/AgentMux-0.59.30"), "0.59.30");
        assert!(check_target(&far, "0.59.30", Some(&own), None, &data).is_err());
        assert!(check_target(Path::new("agentmux.sh"), "0.59.30", Some(&own), None, &data).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn only_that_versions_executable_appimage_beside_this_one_starts() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let own = root.path().join("AgentMux_0.59.29_amd64.AppImage");
        std::fs::write(&own, b"").unwrap();
        let new = root.path().join("AgentMux_0.59.30_amd64.AppImage");
        std::fs::write(&new, b"").unwrap();
        assert!(check_target(&new, "0.59.30", None, Some(&own), &data).is_err(), "not executable yet");
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check_target(&new, "0.59.30", None, Some(&own), &data).is_ok());
        assert!(check_target(&new, "0.59.30", Some(root.path()), None, &data).is_err(), "not an AppImage install");
        assert!(check_target(&new, "0.59.31", None, Some(&own), &data).is_err());
    }
}
