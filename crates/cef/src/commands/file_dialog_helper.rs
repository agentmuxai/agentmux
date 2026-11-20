// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux file dialogs, run in a child of the host (#196). The contract is `file_dialog_proto`; why it exists is
//! there too: the host is not dumpable on purpose, so the desktop portal refuses it, and a dialog from a
//! hardened process ended up as `zenity` with no title and no start folder.
//!
//! Host side: [`run`] starts the host's own executable with `--internal-file-dialog` and the dialog's typed
//! fields, with an empty environment but a short list of what a dialog needs to find the desktop, waits for it
//! (killing that child by its own handle if the person leaves it open past [`HELPER_TIMEOUT`]) and reads the
//! one line it prints. Child side: [`run_and_exit`], called first thing in `run()`: it shows the dialog on its
//! own Tokio runtime (the portal's D-Bus library needs one) and prints the chosen path. It loads no CEF, state,
//! settings or Vault and is never marked non-dumpable, so the portal accepts it.

// The helper is used on Linux only; the module builds everywhere so its decisions are tested everywhere.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::file_dialog::{run_here, Outcome};
use super::file_dialog_proto::{parse_answer, DialogSpec, EXIT_CANCELLED, EXIT_FAILED, MAX_ANSWER_BYTES};

/// How long a dialog may stay open before the host stops waiting for it and ends its helper. A person who walked
/// away from a dialog is a cancel, not a failure.
const HELPER_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How often the host looks at whether the helper has ended.
const POLL: Duration = Duration::from_millis(100);

/// The environment a dialog needs to find the desktop session and its portal, and to start at all, and nothing
/// else: no secret, no token, no agent variable reaches the helper. `LD_LIBRARY_PATH` is the host's own: the
/// packaged `agentmux-cef` finds `libcef.so` only through it (no runpath), so without it the helper dies in the
/// dynamic loader (exit 127) before it shows anything (charlie, 2026-11-20). `LD_PRELOAD` is never passed on.
const ENV_KEPT: &[&str] = &[
    "LD_LIBRARY_PATH",
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LANGUAGE",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XDG_SESSION_ID",
    "XDG_CURRENT_DESKTOP",
    "XDG_DATA_DIRS",
    "XDG_DATA_HOME",
    "XDG_CONFIG_DIRS",
    "XDG_CONFIG_HOME",
    "DESKTOP_SESSION",
    "DBUS_SESSION_BUS_ADDRESS",
    "GDK_BACKEND",
];

/// Whether the helper keeps the variable `name` of the host's environment: the list above and the locale settings.
fn keeps_env(name: &str) -> bool {
    ENV_KEPT.contains(&name) || name.starts_with("LC_")
}

/// Show the dialog in a helper process and wait for its answer. Blocking, for as long as the person takes.
pub(crate) fn run(spec: &DialogSpec) -> Outcome {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return Outcome::Failed(format!("the app's own program could not be found: {e}")),
    };
    let mut cmd = Command::new(exe);
    cmd.args(spec.to_args()).env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    for (name, value) in std::env::vars_os() {
        if name.to_str().is_some_and(keeps_env) {
            cmd.env(name, value);
        }
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Outcome::Failed(format!("the file window's helper could not start: {e}")),
    };
    // The answer is a line of at most MAX_ANSWER_BYTES; one byte more shows it was longer.
    let reader = child.stdout.take().map(|out| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.take(MAX_ANSWER_BYTES as u64 + 1).read_to_end(&mut buf);
            buf
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= HELPER_TIMEOUT => {
                // Ended by its own handle, never by name.
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(target: "file-dialog", "a file dialog was left open too long: its helper was ended");
                return Outcome::Cancelled;
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(e) => return Outcome::Failed(format!("the file window's helper could not be waited for: {e}")),
        }
    };
    let out = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    decide(status.code(), &out)
}

/// What the helper's exit code and output come to. Pure, so it is tested without a dialog.
fn decide(code: Option<i32>, out: &[u8]) -> Outcome {
    match code {
        Some(0) => match parse_answer(out) {
            Ok(path) => Outcome::Chosen(path),
            Err(why) => Outcome::Failed(format!("the file window's helper gave an answer that could not be used: {why}")),
        },
        Some(EXIT_CANCELLED) => Outcome::Cancelled,
        Some(EXIT_FAILED) => Outcome::Failed("the file window could not open".into()),
        Some(other) => Outcome::Failed(format!("the file window's helper ended with code {other}")),
        None => Outcome::Failed("the file window's helper was ended by a signal".into()),
    }
}

/// `run()` calls this before anything else (before the host hardens itself, starts CEF or reads any state). On Linux,
/// when the host was started as the helper (the flag is its first argument), it shows the dialog and exits; in any
/// other case, and on other systems, it returns at once.
pub(crate) fn run_and_exit_if_asked() {
    #[cfg(target_os = "linux")]
    if std::env::args().nth(1).as_deref() == Some(super::file_dialog_proto::HELPER_FLAG) {
        run_and_exit();
    }
}

/// The helper's side. Shows the dialog and exits: 0 with the chosen path on stdout, [`EXIT_CANCELLED`] when nothing
/// was chosen, [`EXIT_FAILED`] when the fields could not be read or the dialog could not open.
fn run_and_exit() -> ! {
    use std::io::Write;
    let spec = match DialogSpec::from_args(std::env::args().skip(1)) {
        Ok(spec) => spec,
        Err(_) => std::process::exit(EXIT_FAILED),
    };
    // The portal's D-Bus library runs in Tokio mode, so the dialog's thread must be inside a runtime.
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build() else {
        std::process::exit(EXIT_FAILED);
    };
    let outcome = rt.block_on(async move { tokio::task::spawn_blocking(move || run_here(&spec)).await });
    match outcome {
        Ok(Outcome::Chosen(path)) => {
            let mut out = std::io::stdout().lock();
            let ok = writeln!(out, "{}", path.to_string_lossy()).and_then(|_| out.flush()).is_ok();
            std::process::exit(if ok { 0 } else { EXIT_FAILED });
        }
        Ok(Outcome::Cancelled) => std::process::exit(EXIT_CANCELLED),
        Ok(Outcome::Failed(_)) | Err(_) => std::process::exit(EXIT_FAILED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_gets_what_a_dialog_needs_and_no_secret() {
        for name in ["DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "HOME", "LC_ALL", "LC_TIME", "LD_LIBRARY_PATH"] {
            assert!(keeps_env(name), "{name} is kept");
        }
        for name in ["AGENTMUX_AUTH_KEY", "AGENTMUX_IPC_TOKEN", "AWS_SECRET_ACCESS_KEY", "GITHUB_TOKEN", "AGENTMUX_AGENT_ID", "RUST_LOG", "LD_PRELOAD"] {
            assert!(!keeps_env(name), "{name} is not kept");
        }
    }

    #[test]
    fn the_exit_code_and_output_decide_the_answer() {
        let path = std::env::temp_dir().join("chosen.txt");
        let line = format!("{}\n", path.display());
        assert!(matches!(decide(Some(0), line.as_bytes()), Outcome::Chosen(p) if p == path));
        assert!(matches!(decide(Some(EXIT_CANCELLED), b""), Outcome::Cancelled));
        assert!(matches!(decide(Some(EXIT_FAILED), b""), Outcome::Failed(_)));
        assert!(matches!(decide(Some(139), b""), Outcome::Failed(_)), "a crash is a failure");
        assert!(matches!(decide(None, b""), Outcome::Failed(_)), "ended by a signal");
        // A helper that says 0 but prints junk did not choose anything.
        assert!(matches!(decide(Some(0), b"relative.txt\n"), Outcome::Failed(_)));
        assert!(matches!(decide(Some(0), b""), Outcome::Failed(_)));
        let two = format!("{}\n{}\n", path.display(), path.display());
        assert!(matches!(decide(Some(0), two.as_bytes()), Outcome::Failed(_)));
    }
}
