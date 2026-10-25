// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The forwarding token: what the `ipc-port-<hash>` file carries.
//!
//! The host's IPC token (`AppState::ipc_token`) proves a request comes from the
//! operator's window, so it opens every command, including the ones that are
//! the operator's alone. The launcher only needs the host to open a window,
//! raise one or quit, for a second launch, a notification click and the tray,
//! and it reads the port file to do it. That file now carries a second token
//! that opens exactly those commands, so reading the file (or the process
//! that reads it) no longer yields the operator's proof.
//!
//! An older launcher sends whatever the file holds as its bearer token, and an
//! older host's file holds the full token, so both pairings keep working.

mod owner_only_file;

use std::sync::OnceLock;

use agentmux_common::secret_eq::secret_eq;

/// The commands the forwarding token opens. Nothing else, ever: adding one
/// here gives it to every process that can read the port file.
pub(crate) const FORWARD_COMMANDS: &[&str] = &["open_new_window", "focus_window"];

/// Set by a launcher that quits the app over its own pipe (`Command::QuitApp`),
/// in the environment it starts the host with.
pub(crate) const QUIT_OVER_PIPE_ENV: &str = "AGENTMUX_LAUNCHER_QUIT_PIPE";

/// Whether the forwarding token may also quit the app. Only for a host an
/// older launcher started (its tray and restart quit through the port file) or
/// one with no launcher at all; a launcher that quits over the pipe says so
/// in the host's environment, which no other process can change afterwards.
fn forward_may_quit() -> bool {
    static MAY: OnceLock<bool> = OnceLock::new();
    *MAY.get_or_init(|| std::env::var_os(QUIT_OVER_PIPE_ENV).is_none_or(|v| v.is_empty()))
}

/// This run's forwarding token, made on first use.
pub(crate) fn forward_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// Publishes `<port>:<forwarding token>` for the launcher, owner-only. The
/// operator's token is never written to disk.
pub(crate) fn publish_port_file(path: &std::path::Path, port: u16) {
    owner_only_file::publish(path, format!("{port}:{}", forward_token()).as_bytes());
}

/// What a bearer token opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IpcAccess {
    /// The operator's token: every command.
    Full,
    /// The forwarding token: [`FORWARD_COMMANDS`].
    Forward,
    None,
}

/// The access `presented` gives against this run's tokens.
pub(crate) fn access_for(presented: Option<&str>, ipc_token: &str, forward: &str) -> IpcAccess {
    let Some(presented) = presented else { return IpcAccess::None };
    if secret_eq(presented.as_bytes(), ipc_token.as_bytes()) {
        IpcAccess::Full
    } else if secret_eq(presented.as_bytes(), forward.as_bytes()) {
        IpcAccess::Forward
    } else {
        IpcAccess::None
    }
}

/// Whether `access` may run `cmd`.
pub(crate) fn allows(access: IpcAccess, cmd: &str) -> bool {
    match access {
        IpcAccess::Full => true,
        IpcAccess::Forward => FORWARD_COMMANDS.contains(&cmd) || (cmd == "quit_app" && forward_may_quit()),
        IpcAccess::None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "operator-token";
    const F: &str = "forward-token";

    #[test]
    fn each_token_is_told_apart_and_anything_else_gets_nothing() {
        assert_eq!(access_for(Some(H), H, F), IpcAccess::Full);
        assert_eq!(access_for(Some(F), H, F), IpcAccess::Forward);
        assert_eq!(access_for(Some("guess"), H, F), IpcAccess::None);
        assert_eq!(access_for(Some(""), H, F), IpcAccess::None);
        assert_eq!(access_for(None, H, F), IpcAccess::None);
    }

    #[test]
    fn the_operators_token_runs_every_command() {
        for cmd in ["open_new_window", "vault_op", "desktop_request", "upgrade_request", "ssh_approval_decide"] {
            assert!(allows(IpcAccess::Full, cmd), "{cmd}");
        }
    }

    #[test]
    fn the_forwarding_token_opens_and_raises_windows_and_nothing_more() {
        for cmd in ["open_new_window", "focus_window"] {
            assert!(allows(IpcAccess::Forward, cmd), "{cmd}");
        }
        for cmd in [
            "vault_op",
            "vault",
            "desktop_request",
            "desktop_stop",
            "upgrade_request",
            "app_update",
            "ssh_approval_decide",
            "widget_approval",
            "browser_bypass",
            "get_ipc_token",
            "",
            "OPEN_NEW_WINDOW",
            "open_new_window ",
        ] {
            assert!(!allows(IpcAccess::Forward, cmd), "{cmd:?}");
        }
        assert!(!allows(IpcAccess::None, "open_new_window"));
    }

    // The test process has no launcher, so it is the "older launcher or none" case.
    #[test]
    fn the_forwarding_token_may_quit_only_for_a_host_no_pipe_launcher_started() {
        assert!(!FORWARD_COMMANDS.contains(&"quit_app"));
        assert_eq!(allows(IpcAccess::Forward, "quit_app"), std::env::var_os(QUIT_OVER_PIPE_ENV).is_none());
    }

    #[test]
    fn the_forwarding_token_is_not_the_operators_and_stays_the_same_for_the_run() {
        assert_eq!(forward_token(), forward_token());
        assert!(!forward_token().is_empty());
    }
}
