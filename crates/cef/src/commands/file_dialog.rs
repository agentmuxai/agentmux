// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one way the host shows a native file dialog (`rfd`): [`show`], with the dialog's typed fields
//! ([`DialogSpec`]) and one of three answers ([`Outcome`]).
//!
//! On macOS and Windows the dialog runs in this process. On Linux it runs in a helper child of the host
//! (`file_dialog_helper`): `rfd` asks the desktop portal, the portal refuses a process that is not dumpable, and
//! the host is not on purpose (`process_hardening`), so from the host itself every dialog ended up as `zenity`,
//! with no title and no start folder, and with no `zenity` as no dialog at all (#196). The helper is a fresh
//! process, dumpable, so the portal accepts it.
//!
//! A caller on CEF's UI thread has no thread to block, so it puts the blocking [`show`] on one with [`spawn`]
//! (the runtime's blocking pool when it is there). A command already runs inside the runtime and uses
//! `tokio::task::spawn_blocking`.

use std::path::PathBuf;

pub(crate) use super::file_dialog_proto::{DialogSpec, Kind};

/// What a dialog came to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    /// The person chose this path.
    Chosen(PathBuf),
    /// The person closed the dialog with nothing chosen.
    Cancelled,
    /// The dialog could not be shown, or its answer could not be used: nobody chose anything. The words say why.
    Failed(String),
}

/// Show the dialog and wait for the person. Blocking, for as long as they take.
pub(crate) fn show(spec: &DialogSpec) -> Outcome {
    #[cfg(target_os = "linux")]
    {
        super::file_dialog_helper::run(spec)
    }
    #[cfg(not(target_os = "linux"))]
    {
        run_here(spec)
    }
}

/// Show a dialog from a command without holding a runtime worker, and without the host's floating panes in the way
/// (`host_dialog_guard`): `Some(path)` when the person chose, `None` when they closed it, and an error in words
/// when it could not open (shown to the person, not taken for a cancel). `what` names the command in that error.
pub(crate) async fn ask(spec: DialogSpec, what: &str) -> Result<Option<PathBuf>, String> {
    let outcome = tokio::task::spawn_blocking(move || {
        let _guard = super::platform::host_dialog_guard();
        show(&spec)
    })
    .await
    .map_err(|e| format!("{what}: {e}"))?;
    match outcome {
        Outcome::Chosen(path) => Ok(Some(path)),
        Outcome::Cancelled => Ok(None),
        Outcome::Failed(why) => Err(format!("{what}: {why}")),
    }
}

/// Show the dialog in this process (what the helper does, and what macOS and Windows do). A panic in the dialog
/// code is a failed dialog, not a dead thread.
pub(crate) fn run_here(spec: &DialogSpec) -> Outcome {
    let shown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut dialog = rfd::FileDialog::new();
        if !spec.title.is_empty() {
            dialog = dialog.set_title(spec.title.as_str());
        }
        if let Some(name) = &spec.file_name {
            dialog = dialog.set_file_name(name.as_str());
        }
        if let Some(dir) = spec.dir.as_ref().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        for f in &spec.filters {
            let extensions: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
            dialog = dialog.add_filter(f.label.as_str(), &extensions);
        }
        match spec.kind {
            Kind::Save => dialog.save_file(),
            Kind::Open => dialog.pick_file(),
        }
    }));
    match shown {
        Ok(Some(path)) => Outcome::Chosen(path),
        Ok(None) => Outcome::Cancelled,
        Err(_) => Outcome::Failed("the file window stopped before it answered".into()),
    }
}

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

/// Whether anything can show a file dialog now. Always so on macOS and Windows. On Linux the helper asks the
/// desktop portal, which needs the person's session bus, and `rfd` runs `zenity` when the portal refuses: with
/// neither a session bus nor `zenity` on the PATH `rfd` answers "nothing chosen" at once, which looks like a cancel.
pub(crate) fn can_ask() -> bool {
    #[cfg(target_os = "linux")]
    {
        backend_available(
            program_on_path("zenity", std::env::var_os("PATH").as_deref()),
            session_bus_present(
                std::env::var_os("DBUS_SESSION_BUS_ADDRESS").as_deref(),
                std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
            ),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

#[cfg(any(target_os = "linux", test))]
fn backend_available(zenity_on_path: bool, session_bus: bool) -> bool {
    zenity_on_path || session_bus
}

/// Whether an executable file `name` is in a folder of `path_var` (a PATH value).
#[cfg(any(target_os = "linux", test))]
fn program_on_path(name: &str, path_var: Option<&std::ffi::OsStr>) -> bool {
    let Some(path_var) = path_var else { return false };
    std::env::split_paths(path_var).any(|dir| dir.join(name).is_file())
}

/// Whether the person's D-Bus session bus can be reached: an address in the environment, or the socket systemd
/// puts in the runtime folder (`$XDG_RUNTIME_DIR/bus`).
#[cfg(any(target_os = "linux", test))]
fn session_bus_present(address: Option<&std::ffi::OsStr>, runtime_dir: Option<&std::ffi::OsStr>) -> bool {
    address.is_some_and(|a| !a.is_empty())
        || runtime_dir.is_some_and(|d| !d.is_empty() && std::path::Path::new(d).join("bus").exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_dialog_needs_the_portal_or_zenity() {
        assert!(backend_available(true, false), "zenity answers if the portal is not there");
        assert!(backend_available(false, true), "the portal is reached over the session bus");
        assert!(!backend_available(false, false), "neither: rfd would answer nothing at once");
    }

    #[test]
    fn the_session_bus_is_an_address_or_the_socket_in_the_runtime_folder() {
        use std::ffi::OsStr;
        let dir = tempfile::tempdir().unwrap();
        assert!(!session_bus_present(None, None));
        assert!(!session_bus_present(Some(OsStr::new("")), Some(dir.path().as_os_str())), "no socket yet");
        assert!(session_bus_present(Some(OsStr::new("unix:path=/run/user/1000/bus")), None));
        std::fs::write(dir.path().join("bus"), b"").unwrap();
        assert!(session_bus_present(None, Some(dir.path().as_os_str())));
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
