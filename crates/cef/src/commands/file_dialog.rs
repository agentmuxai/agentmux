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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

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
