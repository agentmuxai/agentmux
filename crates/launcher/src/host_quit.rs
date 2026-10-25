// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Quitting the host: the tray's "Quit AgentMux" and the restart into a new
//! version the user agreed to.
//!
//! These used to send `quit_app` to the host's HTTP port with the token from
//! the port file, which made that token able to quit the app. They now go over
//! the launcher's own pipe to the host (`Command::QuitApp`), so the token only
//! opens and raises windows.
//!
//! A host from another version may not know `QuitApp` (it would drop it
//! without a word), so for one the launcher falls back to the HTTP command
//! that host still takes. A host that has not registered yet is waited for, as
//! the tray always did.

use std::sync::{Arc, OnceLock};

use agentmux_common::ipc::{ClientKind, Command};
use tokio::sync::Mutex;

use crate::host_pipe::HostPipe;
use crate::state::State;

struct Installed {
    pipe: Arc<HostPipe>,
    state: Arc<Mutex<State>>,
    runtime: tokio::runtime::Handle,
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();

/// Called by the supervisor once it has the host pipe and the launcher state,
/// from inside its runtime.
pub fn install(pipe: Arc<HostPipe>, state: Arc<Mutex<State>>) {
    let _ = INSTALLED.set(Installed { pipe, state, runtime: tokio::runtime::Handle::current() });
}

/// What to try next, given what the launcher knows of the host.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// The host has not registered: try again shortly.
    Wait,
    /// The host is this version: send `QuitApp` down the pipe.
    Pipe,
    /// The host is another version: use the HTTP command it knows.
    Http,
}

fn step(host_version: Option<&str>, own_version: &str) -> Step {
    match host_version {
        None => Step::Wait,
        Some(v) if v == own_version => Step::Pipe,
        Some(_) => Step::Http,
    }
}

/// Quits the host. Blocking: runs on the tray's thread or a blocking task,
/// never on the async runtime.
pub fn quit_host(data_dir: &std::path::Path, dir_hash: &str) {
    const MAX_ATTEMPTS: u32 = 30;
    const DELAY: std::time::Duration = std::time::Duration::from_secs(1);
    let Some(ctx) = INSTALLED.get() else {
        crate::tray::forward_tray_cmd(data_dir, dir_hash, "quit_app");
        return;
    };
    for attempt in 1..=MAX_ATTEMPTS {
        let host_version = ctx.runtime.block_on(async {
            let st = ctx.state.lock().await;
            st.processes.values().find(|p| p.kind == ClientKind::Host).map(|p| p.version.clone())
        });
        match step(host_version.as_deref(), env!("CARGO_PKG_VERSION")) {
            Step::Http => return crate::tray::forward_tray_cmd(data_dir, dir_hash, "quit_app"),
            Step::Pipe => {
                match ctx.runtime.block_on(ctx.pipe.try_send_command_no_buffer(&Command::QuitApp)) {
                    Ok(()) => {
                        crate::log(&format!("quit: sent to the host over its pipe (attempt {attempt})"));
                        return;
                    }
                    Err(e) => crate::log(&format!("quit: the host pipe refused it (attempt {attempt}): {e:?}")),
                }
            }
            Step::Wait => {}
        }
        std::thread::sleep(DELAY);
    }
    crate::log("quit: gave up; the host never took the request");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_that_has_not_registered_is_waited_for() {
        assert_eq!(step(None, "1.2.3"), Step::Wait);
    }

    #[test]
    fn a_host_of_this_version_gets_the_pipe_command() {
        assert_eq!(step(Some("1.2.3"), "1.2.3"), Step::Pipe);
    }

    #[test]
    fn a_host_of_another_version_gets_the_http_command_it_knows() {
        assert_eq!(step(Some("1.2.2"), "1.2.3"), Step::Http);
        assert_eq!(step(Some("1.2.4"), "1.2.3"), Step::Http);
        assert_eq!(step(Some(""), "1.2.3"), Step::Http);
    }
}
