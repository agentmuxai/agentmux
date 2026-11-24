// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which websocket RPC commands a connection may call without being the AgentMux window.
//!
//! Holding the instance key opens a connection; it does not make the connection the window. A command is
//! the window's alone unless it is listed here: a new command is refused to everything but the window until
//! someone decides it is safe to open. `server::dispatch_guard` applies these lists at dispatch, and
//! `scripts/gen-operator-commands.mjs` writes the frontend's list of commands that wait for the window's
//! proof from them and from [`super::action_commands`].

/// Calls any connection may make: they read what the window itself shows to anyone who can open it, they
/// only subscribe this connection, or the handler decides per caller (`setconfig` per key, `notify.ack` by the
/// launcher's proof). The window makes these before its own proof lands.
pub const OPEN: &[&str] = &[
    "app.update-check", "app.update-status", "eventreadhistory", "eventsub", "eventunsub", "eventunsuball",
    "fileindexlines", "fileprobe", "filesearchbytes", "filesearchtext", "fs.list", "fs.places", "getfullconfig",
    "getwaveairatelimit", "jobs.list", "kanban.list", "listeditordir", "notify.ack", "presence.status",
    "readeditorfile", "routeannounce", "routeunannounce", "setconfig", "waveinfo", "widgets.catalog", "widgets.list",
    "widgets.readfile", "operator.challenge",
];

/// Calls a connection registered as an agent may make although they are the window's: the handler lets the
/// agent act on its own pane only (`server::pane_caller`) and refuses the rest.
pub const OWN_PANE: &[&str] = &["agentinput", "agentstop", "controllerresync", "createsubblock", "deletesubblock", "setmeta"];

/// Whether any connection may call `command`.
pub fn is_open(command: &str) -> bool {
    OPEN.contains(&command)
}

/// Whether a connection registered as an agent may reach `command`'s handler, which checks the pane.
pub fn agent_reaches_handler(command: &str) -> bool {
    is_open(command) || OWN_PANE.contains(&command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc::action_commands::{ALWAYS, LIVE_INPUT, READ_ONLY, UNLESS_OPERATOR};

    #[test]
    fn the_lists_name_commands_that_are_decided_and_do_not_overlap() {
        let decided: std::collections::HashSet<&str> =
            [ALWAYS, UNLESS_OPERATOR, LIVE_INPUT, READ_ONLY].iter().flat_map(|l| l.iter().copied()).collect();
        for name in OPEN.iter().chain(OWN_PANE) {
            // `operator.challenge` is in UNLESS_OPERATOR; every listed name must be a command that exists.
            assert!(decided.contains(name), "{name} is in no list of action_commands.rs");
        }
        for name in OWN_PANE {
            assert!(!OPEN.contains(name), "{name} is in OPEN and OWN_PANE");
        }
    }

    #[test]
    fn only_the_decided_reads_are_open() {
        // What an unproven connection must not read or change: another agent's transcript, memory and
        // history, the person's typed history, instructions, exports, and any object's meta.
        for name in [
            "agent.output", "agent:session:read", "agent:memory:read_file", "getmemory", "listrecentsessions", "session:export",
            "agent.input-history", "getagentcontent", "bundle.export", "exportagents", "getmeta", "searchagenthistory",
        ] {
            assert!(!is_open(name), "{name} must be the window's alone");
        }
    }
}
