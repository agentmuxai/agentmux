// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Upgrade & restart" (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md`
//! §4.1, phase 3b): restart a Windows portable into the newer release srv
//! downloaded, verified and unpacked next to it (`backend/update_download.rs`).
//!
//! srv asks with an `AGENTMUXSRV-UPGRADE` line on its stderr
//! (`agentmux_common::srv_stderr`); the reader in `srv_spawner` hands it here.
//! The supervisor then:
//! 1. checks the folder again ([`staged_launcher`]): an AgentMux portable of
//!    exactly that version, newer than this one, and this install is a
//!    portable too;
//! 2. asks the user with a native dialog ([`confirm`]), which no page or agent
//!    tool in AgentMux can click: a restart stops every agent, so it takes the
//!    user's own answer here, whoever asked srv;
//! 3. quits the host the way the tray's Quit does, which closes the windows and
//!    saves the session;
//! 4. after srv has stopped (so the new version copies complete data,
//!    `srv/src/bootstrap/carry_forward.rs`), starts the new launcher
//!    ([`relaunch`]) and exits.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::srv_stderr::UpgradeRequest;
use tokio::sync::mpsc;

/// A confirmed restart that hasn't happened within this long (the host
/// never quit) no longer restarts into the new version on a later quit.
pub const CONFIRMED_FOR: Duration = Duration::from_secs(5 * 60);

static REQUESTS: Mutex<Option<mpsc::UnboundedSender<UpgradeRequest>>> = Mutex::new(None);
/// A Yes counts only this long after its dialog opened (the dialog itself
/// can't time out). Shorter than srv's wait (`RESTART_GRACE`, 3 minutes in
/// `srv/src/backend/update_restart.rs`), so by the time the panel says the
/// restart didn't happen, no open dialog can still start one.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(150);

/// The latest request asked about: only its dialog's Yes counts.
static LATEST_ASK: AtomicU64 = AtomicU64::new(0);

/// The supervisor's end of srv's upgrade requests. One per process: it lasts
/// across srv respawns, since the reader of each new srv sends here too.
pub fn requests() -> mpsc::UnboundedReceiver<UpgradeRequest> {
    let (tx, rx) = mpsc::unbounded_channel();
    *REQUESTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    rx
}

/// From the srv stderr reader.
pub fn forward(req: UpgradeRequest) {
    if let Some(tx) = REQUESTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(req);
    }
}

/// `major.minor.patch`, or `None`.
fn version_core(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['+', '-']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// A portable folder. `setup.exe` installs the same `runtime/`, marker and
/// all, so a folder with Inno Setup's uninstaller (`unins000.exe`) isn't one.
fn is_portable(folder: &Path) -> bool {
    let uninstaller = std::fs::read_dir(folder).is_ok_and(|entries| {
        entries.filter_map(|e| e.ok()).any(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("unins") && name.ends_with(".exe")
        })
    });
    folder.join("runtime").join("agentmux-portable.marker").is_file() && !uninstaller
}

/// The launcher to restart into, if `req` names an AgentMux portable of a
/// newer version than `own_version` and this launcher (in `own_dir`) is a
/// portable too.
pub fn staged_launcher(req: &UpgradeRequest, own_dir: &Path, own_version: &str) -> Result<PathBuf, String> {
    let launcher = &req.launcher;
    let named_right = launcher.file_name().is_some_and(|n| n.eq_ignore_ascii_case("agentmux.exe"));
    if !launcher.is_absolute() || !named_right || !launcher.is_file() {
        return Err(format!("{} isn't an AgentMux launcher", launcher.display()));
    }
    let folder = launcher.parent().ok_or("the launcher has no folder")?;
    if !is_portable(own_dir) {
        return Err("this AgentMux isn't a portable install".into());
    }
    if !is_portable(folder) || !folder.join("runtime").join(format!("agentmux-{}.exe", req.version)).is_file() {
        return Err(format!("{} isn't an AgentMux {} portable", folder.display(), req.version));
    }
    if folder == own_dir {
        return Err("that is the running AgentMux".into());
    }
    match (version_core(&req.version), version_core(own_version)) {
        (Some(new), Some(own)) if new > own => Ok(launcher.clone()),
        _ => Err(format!("{} isn't newer than {own_version}", req.version)),
    }
}

/// Ask the user on a thread of its own, and call `on_yes` only for an
/// explicit Yes that still counts ([`answer_counts`]): one given within
/// [`ANSWER_WITHIN`], to the latest request. Any other Yes is told it expired.
pub fn ask_then(version: String, launcher: PathBuf, on_yes: impl FnOnce(PathBuf) + Send + 'static) {
    let ask = LATEST_ASK.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        let opened = Instant::now();
        if !confirm(&version, &launcher) {
            return;
        }
        if answer_counts(ask, LATEST_ASK.load(Ordering::SeqCst), opened.elapsed()) {
            on_yes(launcher);
        } else {
            crate::log(&format!("upgrade: a Yes {}s after asking, or to an older request, was ignored", opened.elapsed().as_secs()));
            crate::show_fatal_dialog(
                "AgentMux upgrade",
                "This upgrade request expired, so AgentMux didn't restart.\n\nStart it again from the version panel.",
            );
        }
    });
}

/// Whether a Yes to request `ask`, given `waited` after its dialog opened,
/// still counts when `latest` is the newest request.
fn answer_counts(ask: u64, latest: u64, waited: Duration) -> bool {
    ask == latest && waited < ANSWER_WITHIN
}

#[cfg(target_os = "windows")]
fn confirm(version: &str, launcher: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    };
    let folder = launcher.parent().unwrap_or(launcher).display().to_string();
    let body = format!(
        "Restart AgentMux to upgrade to {version}?\n\n\
         Every window closes and every agent stops, then AgentMux {version} opens \
         from:\n{folder}\n\nYour tabs come back and agents resume their conversations."
    );
    let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (title, body) = (wide("Upgrade AgentMux"), wide(&body));
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
    answer == IDYES
}

#[cfg(not(target_os = "windows"))]
fn confirm(_version: &str, _launcher: &Path) -> bool {
    // Only the Windows portable restarts itself so far (phase 3b).
    false
}

/// Whether a restart confirmed at `at` still stands.
pub fn still_confirmed(at: Instant) -> bool {
    at.elapsed() < CONFIRMED_FOR
}

/// Start the new launcher, detached, without this AgentMux's own environment:
/// `AGENTMUX` would make it think it was launched from a pane, and the
/// `AGENTMUX_*` this launcher set on itself (background service, tray) would
/// override the new one's settings.
pub fn relaunch(launcher: &Path) -> std::io::Result<()> {
    use agentmux_common::win32::NoWindow;
    let mut cmd = std::process::Command::new(launcher);
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if k == "AGENTMUX" || k.starts_with("AGENTMUX_") {
            cmd.env_remove(&key);
        }
    }
    if let Some(folder) = launcher.parent() {
        cmd.current_dir(folder);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .no_window()
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portable(dir: &Path, version: &str) -> PathBuf {
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        std::fs::write(dir.join("runtime/agentmux-portable.marker"), b"").unwrap();
        std::fs::write(dir.join(format!("runtime/agentmux-{version}.exe")), b"").unwrap();
        std::fs::write(dir.join("agentmux.exe"), b"").unwrap();
        dir.join("agentmux.exe")
    }

    fn req(version: &str, launcher: &Path) -> UpgradeRequest {
        UpgradeRequest { version: version.into(), launcher: launcher.to_path_buf() }
    }

    #[test]
    fn only_a_newer_portable_beside_a_portable_is_restarted_into() {
        let root = tempfile::tempdir().unwrap();
        let own = root.path().join("agentmux-0.59.26-x64-portable");
        portable(&own, "0.59.26");
        let new = portable(&root.path().join("agentmux-0.59.27-x64-portable"), "0.59.27");
        assert_eq!(staged_launcher(&req("0.59.27", &new), &own, "0.59.26").unwrap(), new);

        // Not newer, or not the version it claims.
        assert!(staged_launcher(&req("0.59.27", &new), &own, "0.59.27").is_err());
        assert!(staged_launcher(&req("0.59.28", &new), &own, "0.59.26").is_err());
        // Not a launcher, or not absolute.
        let other = root.path().join("agentmux-0.59.27-x64-portable/runtime/agentmux-0.59.27.exe");
        assert!(staged_launcher(&req("0.59.27", &other), &own, "0.59.26").is_err());
        assert!(staged_launcher(&req("0.59.27", Path::new("agentmux.exe")), &own, "0.59.26").is_err());
        // This AgentMux isn't a portable, including a setup.exe install,
        // which has the portable's runtime/ and marker too.
        let installed = root.path().join("Programs/AgentMux");
        portable(&installed, "0.59.26");
        std::fs::write(installed.join("unins000.exe"), b"").unwrap();
        assert!(staged_launcher(&req("0.59.27", &new), &installed, "0.59.26").is_err());
    }

    #[test]
    fn a_late_or_superseded_yes_does_not_count() {
        assert!(answer_counts(3, 3, Duration::from_secs(10)));
        assert!(!answer_counts(3, 3, ANSWER_WITHIN));
        assert!(!answer_counts(2, 3, Duration::from_secs(10)));
        // The dialog must stop counting before srv gives up (3 minutes).
        assert!(ANSWER_WITHIN < Duration::from_secs(3 * 60));
    }

    #[test]
    fn a_confirmation_lapses() {
        assert!(still_confirmed(Instant::now()));
        if let Some(long_ago) = Instant::now().checked_sub(CONFIRMED_FOR + Duration::from_secs(1)) {
            assert!(!still_confirmed(long_ago));
        }
    }
}
