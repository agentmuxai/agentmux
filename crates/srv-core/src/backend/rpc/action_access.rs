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
    "getfullconfig", "getwaveairatelimit", "jobs.list", "kanban.list", "notify.ack", "presence.status",
    "routeannounce", "routeunannounce", "setconfig", "waveinfo", "widgets.catalog", "widgets.list", "operator.challenge",
];

/// The commands that carry an event name, which only the events in [`OPEN_EVENTS`] may be named in by a connection
/// that is not the window: every event is delivered to whoever subscribed to it, and most carry another's words
/// (a pane's output, a person's typing, an agent's turns).
pub const EVENT_COMMANDS: &[&str] = &["eventsub", "eventreadhistory"];

/// The events any connection may subscribe to or read the history of: the launcher's notification events.
pub const OPEN_EVENTS: &[&str] = &["notification", "notification:retract", "notification:state"];

/// Calls a connection registered as an agent may make although they are the window's: the handler lets the
/// agent act on its own pane only (`server::pane_caller`) and refuses the rest.
pub const OWN_PANE: &[&str] = &["agentinput", "agentstop", "controllerresync", "createsubblock", "deletesubblock", "setmeta"];

/// Whether any connection may call `command` with this request: it is open, and when it names an event (`eventsub`,
/// `eventreadhistory`) the event is one of [`OPEN_EVENTS`].
pub fn is_open_for(command: &str, data: &serde_json::Value) -> bool {
    if !OPEN.contains(&command) {
        return false;
    }
    !EVENT_COMMANDS.contains(&command) || data.get("event").and_then(serde_json::Value::as_str).is_some_and(|e| OPEN_EVENTS.contains(&e))
}

/// Whether a connection registered as an agent reaches `command`'s handler although the command is not open: the
/// handler checks the pane.
pub fn is_own_pane(command: &str) -> bool {
    OWN_PANE.contains(&command)
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
        for name in EVENT_COMMANDS {
            assert!(OPEN.contains(name), "{name} names an event and is open only for OPEN_EVENTS");
        }
    }

    #[test]
    fn an_event_is_open_only_when_it_carries_nobody_elses_words() {
        use serde_json::json;
        for event in OPEN_EVENTS {
            assert!(is_open_for("eventsub", &json!({"event": event, "allscopes": true})), "{event}");
            assert!(is_open_for("eventreadhistory", &json!({"event": event, "scope": "", "maxitems": 5})), "{event}");
        }
        // Another pane's output, the person's typing, an agent's turns and the message flow.
        for event in ["blockfile", "userinput", "agentturn", "message-flow", "controllerstatus", "a-new-event", ""] {
            assert!(!is_open_for("eventsub", &json!({"event": event, "allscopes": true})), "{event}");
            assert!(!is_open_for("eventreadhistory", &json!({"event": event, "scope": ""})), "{event}");
        }
        assert!(!is_open_for("eventsub", &json!({})) && !is_open_for("eventsub", &serde_json::Value::Null));
        assert!(is_open_for("eventunsub", &json!("blockfile")), "taking a subscription away reveals nothing");
        assert!(!is_open_for("agent.kill-tree", &json!({})));
    }

    #[test]
    fn only_the_decided_reads_are_open() {
        // What an unproven connection must not read or change: another agent's transcript, memory and
        // history, the person's typed history, instructions, exports, and any object's meta.
        for name in [
            "agent.output", "agent:session:read", "agent:memory:read_file", "getmemory", "listrecentsessions", "session:export",
            "agent.input-history", "getagentcontent", "bundle.export", "exportagents", "getmeta", "searchagenthistory",
            // Reads of a path the caller names (also over a saved connection): the window's own.
            "readeditorfile", "listeditordir", "fs.list", "fs.places", "fileprobe", "filesearchtext", "filesearchbytes",
            "fileindexlines", "widgets.readfile",
        ] {
            assert!(!OPEN.contains(&name), "{name} must be the window's alone");
        }
    }
}
