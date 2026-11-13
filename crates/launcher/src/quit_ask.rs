// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The tray's Quit asks first when it would stop agents that are working.
//!
//! Quitting stops every agent. A window's own close warns before it stops agents in the middle of
//! a turn; the tray's Quit went straight to the host. It is also the Quit used with no window
//! open, when the agents running are the ones nobody is looking at. So the launcher asks srv who
//! is working (`GET /agentmux/quit-question`) and, when srv says to ask, puts the question in a
//! native dialog of its own: one that needs no window and that no page or agent can answer.
//!
//! Quitting must always work. No answer from srv in time (srv is gone, hung or
//! from a version without the question), or no way to show a dialog, quits as before. Only an
//! explicit "keep running" stops the quit.
//!
//! The restart into a new version is not asked here: it has its own question
//! (`upgrade_restart`), and goes to `host_quit::quit_host` directly.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use agentmux_common::quit_question::{ask_srv, goes_ahead};
#[cfg(any(target_os = "windows", target_os = "linux"))]
use agentmux_common::quit_question::DIALOG_TITLE;
#[cfg(target_os = "linux")]
use agentmux_common::quit_question::{KEEP_RUNNING, QUIT};

/// srv's web endpoint (`host:port`) and its key, from the last srv this launcher started.
static SRV: Mutex<Option<(String, String)>> = Mutex::new(None);

/// srv is up at `web_endpoint` (`srv_spawner::spawn_srv`, on every start of it).
pub fn srv_is_at(web_endpoint: &str, auth_key: &str) {
    *SRV.lock().unwrap_or_else(|e| e.into_inner()) = Some((web_endpoint.to_string(), auth_key.to_string()));
}

/// The tray's "Quit AgentMux". Asks on a thread of its own, so the tray's other items keep working
/// while the dialog is open; a second Quit while one is being asked about is dropped.
pub fn quit_from_tray(data_dir: &Path, dir_hash: &str) {
    static ASKING: AtomicBool = AtomicBool::new(false);
    if ASKING.swap(true, Ordering::SeqCst) {
        crate::log("quit: already asking about a quit");
        return;
    }
    let (dir, hash) = (data_dir.to_path_buf(), dir_hash.to_string());
    let spawned = std::thread::Builder::new().name("agentmux-quit-ask".into()).spawn(move || {
        let srv = SRV.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let question = srv.and_then(|(endpoint, key)| ask_srv(&endpoint, &key));
        if let Some(q) = &question {
            crate::log(&format!("quit: {} of {} agents working; asking", q.working.len(), q.agents));
        }
        let go = goes_ahead(question.as_ref(), confirm);
        ASKING.store(false, Ordering::SeqCst);
        if go {
            crate::host_quit::quit_host(&dir, &hash);
        } else {
            crate::log("quit: the person chose to keep running");
        }
    });
    if spawned.is_err() {
        ASKING.store(false, Ordering::SeqCst);
        crate::host_quit::quit_host(data_dir, dir_hash);
    }
}

/// Ask in a native dialog whose default is to keep running. `Some(true)`: quit. `Some(false)`:
/// keep running. `None`: no dialog could be shown.
#[cfg(target_os = "windows")]
fn confirm(body: &str) -> Option<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    };
    let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (title, body) = (wide(DIALOG_TITLE), wide(body));
    // A window of this process, answered by the person: nothing a page or an agent can reach.
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
    (answer != 0).then_some(answer == IDYES)
}

/// What `zenity` or `kdialog` said, from its exit code and what it wrote to stderr. Both exit 0
/// for the first button (Quit) and 1 for the other one or a closed dialog (keep running). Anything
/// else is a dialog that was never answered: `None`. So is an exit of 1 from a dialog that could
/// not reach the display, which it says on stderr.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_answer(exit_code: Option<i32>, stderr: &str) -> Option<bool> {
    // How GTK and Qt say they have no display to draw on.
    const NO_DISPLAY: [&str; 5] = ["open display", "connect to display", "x display", "wl_display", "init server"];
    let said = stderr.to_ascii_lowercase();
    match exit_code {
        Some(0) => Some(true),
        Some(1) if !NO_DISPLAY.iter().any(|phrase| said.contains(phrase)) => Some(false),
        _ => None,
    }
}

/// Linux: `zenity`, or else `kdialog`, as the upgrade question uses. With neither installed there
/// is no dialog.
#[cfg(target_os = "linux")]
fn confirm(body: &str) -> Option<bool> {
    let zenity_title = format!("--title={DIALOG_TITLE}");
    let zenity_text = format!("--text={}", crate::upgrade_linux::pango_escape(body));
    let zenity_ok = format!("--ok-label={QUIT}");
    let zenity_cancel = format!("--cancel-label={KEEP_RUNNING}");
    let dialogs: [(&str, Vec<&str>); 2] = [
        ("zenity", vec!["--question", &zenity_title, &zenity_text, &zenity_ok, &zenity_cancel, "--default-cancel", "--width=460"]),
        ("kdialog", vec!["--title", DIALOG_TITLE, "--yes-label", QUIT, "--no-label", KEEP_RUNNING, "--warningyesno", body]),
    ];
    for (program, args) in dialogs {
        let mut cmd = std::process::Command::new(program);
        cmd.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        // Not this version's libraries: they could break the dialog's own.
        for key in crate::upgrade_linux::NOT_INHERITED {
            cmd.env_remove(key);
        }
        cmd.stderr(std::process::Stdio::piped());
        match cmd.output() {
            Ok(out) => {
                let answer = linux_answer(out.status.code(), &String::from_utf8_lossy(&out.stderr));
                if answer.is_none() {
                    crate::log(&format!("quit: {program} did not show the question ({}); quitting", out.status));
                }
                return answer;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                crate::log(&format!("quit: couldn't open {program} to ask: {e}"));
                return None;
            }
        }
    }
    crate::log("quit: neither zenity nor kdialog is installed to ask with");
    None
}

/// macOS: the dialog the host's own Quit uses too (`agentmux_common::quit_question`).
#[cfg(target_os = "macos")]
fn confirm(body: &str) -> Option<bool> {
    match agentmux_common::quit_question::confirm_macos(body) {
        Ok(answer) => Some(answer),
        Err(why) => {
            crate::log(&format!("quit: {why}; quitting"));
            None
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn confirm(_body: &str) -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_that_failed_is_not_an_answer_to_keep_running() {
        // Linux: 0 is Quit, 1 is the other button or a closed dialog.
        assert_eq!(linux_answer(Some(0), ""), Some(true));
        assert_eq!(linux_answer(Some(1), ""), Some(false));
        assert_eq!(linux_answer(Some(1), "Gtk-Message: GtkDialog mapped without a transient parent"), Some(false));
        // It never reached the screen: no answer, so the quit goes ahead.
        assert_eq!(linux_answer(Some(1), "(zenity:4242): Gtk-WARNING **: cannot open display: "), None);
        assert_eq!(linux_answer(Some(1), "qt.qpa.xcb: could not connect to display"), None);
        assert_eq!(linux_answer(Some(5), ""), None, "zenity's timeout");
        assert_eq!(linux_answer(Some(255), ""), None);
        assert_eq!(linux_answer(None, ""), None, "ended by a signal");
    }
}
