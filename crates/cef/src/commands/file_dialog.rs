// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a blocking native file dialog (`rfd`) runs.
//!
//! On Linux `rfd` asks the desktop portal over D-Bus, and its D-Bus library runs in Tokio mode here (the tray
//! enables it), so the dialog must be opened from a thread that is inside the app's Tokio runtime. A plain
//! `std::thread` has no reactor: the dialog panicked and no Save As window ever appeared (#196, Linux D0).
//!
//! A command already runs inside the runtime, so its dialog uses `tokio::task::spawn_blocking` (the Vault
//! export and import, the layout dialogs and the file pickers do). Code that starts from CEF's UI thread has no
//! runtime of its own: it uses [`spawn`], the one way to put a dialog on the runtime's blocking pool from there.

/// Run `job`, which may block on a dialog for as long as the person takes, on the app's Tokio runtime so the
/// dialog has a reactor. Without the runtime (a unit test, or a start-up that has not installed it yet) it runs on
/// a plain thread: the dialog may then fail, which its caller must treat as an answer rather than wait for.
pub(crate) fn spawn(job: impl FnOnce() + Send + 'static) {
    spawn_on(crate::credential_broker::runtime_handle(), job);
}

fn spawn_on(runtime: Option<tokio::runtime::Handle>, job: impl FnOnce() + Send + 'static) {
    match runtime {
        Some(runtime) => drop(runtime.spawn_blocking(job)),
        None => drop(std::thread::spawn(job)),
    }
}

/// A "cancel" that comes back sooner than a person could have looked at a window and closed it is no answer: the
/// dialog never opened (the portal refused and nothing else could ask). Only a last resort: [`can_ask`] is the
/// check made before asking, but a backend that is there and still fails shows only as this.
pub(crate) const QUICK_CANCEL: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether anything can show a file dialog now. Always so on macOS and Windows. On Linux `rfd` asks the desktop
/// portal first and runs `zenity` when the portal refuses, and the portal refuses a process that is not
/// dumpable (it cannot open `/proc/<pid>/root`), which the host is on purpose (`process_hardening`; a dev build
/// and `AGENTMUX_ALLOW_PTRACE=1` are exempt). So today on Linux every dialog is `zenity`'s; with neither a
/// dumpable host nor `zenity` on the PATH `rfd` answers "nothing chosen" at once, which looks like a cancel.
pub(crate) fn can_ask() -> bool {
    #[cfg(target_os = "linux")]
    {
        backend_available(program_on_path("zenity", std::env::var_os("PATH").as_deref()), dumpable())
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

#[cfg(any(target_os = "linux", test))]
fn backend_available(zenity_on_path: bool, dumpable: bool) -> bool {
    zenity_on_path || dumpable
}

/// Whether an executable file `name` is in a folder of `path_var` (a PATH value).
#[cfg(any(target_os = "linux", test))]
fn program_on_path(name: &str, path_var: Option<&std::ffi::OsStr>) -> bool {
    let Some(path_var) = path_var else { return false };
    std::env::split_paths(path_var).any(|dir| dir.join(name).is_file())
}

/// Whether this process is dumpable, the state the portal needs of a process that asks it for a dialog.
#[cfg(target_os = "linux")]
fn dumpable() -> bool {
    // SAFETY: PR_GET_DUMPABLE takes no pointer and only reads the process's flag.
    unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) == 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_dialog_needs_the_portal_or_zenity() {
        assert!(backend_available(true, false), "zenity answers for the portal's refusal");
        assert!(backend_available(false, true), "a dumpable host is accepted by the portal");
        assert!(!backend_available(false, false), "neither: rfd would answer nothing at once");
    }

    #[test]
    fn a_program_is_found_only_on_the_path_given() {
        let with = tempfile::tempdir().unwrap();
        let without = tempfile::tempdir().unwrap();
        std::fs::write(with.path().join("zenity"), b"").unwrap();
        let path = std::env::join_paths([without.path(), with.path()]).unwrap();
        assert!(program_on_path("zenity", Some(&path)));
        assert!(!program_on_path("zenity", Some(without.path().as_os_str())));
        assert!(!program_on_path("zenity", None));
    }

    #[test]
    fn a_job_runs_inside_the_runtime_when_there_is_one() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let (tx, rx) = mpsc::channel();
        spawn_on(Some(rt.handle().clone()), move || {
            // What the portal's D-Bus library needs: a runtime to spawn its tasks on.
            tx.send(tokio::runtime::Handle::try_current().is_ok()).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "the dialog's thread is inside the runtime");
    }

    #[test]
    fn a_job_still_runs_with_no_runtime() {
        let (tx, rx) = mpsc::channel();
        spawn_on(None, move || tx.send(tokio::runtime::Handle::try_current().is_ok()).unwrap());
        assert!(!rx.recv_timeout(Duration::from_secs(5)).unwrap(), "a plain thread, which its caller must expect to fail");
    }
}
