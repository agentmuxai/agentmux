// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.2: launcher-owned named-pipe IPC server.
//
// Per `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §3.2 and §5,
// the launcher hosts the canonical state machine and exposes it over a
// pipe per data-dir-scoped namespace. Each subscriber (host, eventually
// frontend renderers, srv) connects, sends `Command` messages, and
// receives `Event` messages back.
//
// B.2 scope (this module): just the wire — types + accept loop +
// per-connection read/write tasks. No reducer, no events emitted yet
// (B.3 wires the reducer; B.4 pipes events back). This commit makes
// the host able to register itself with the launcher and the launcher
// to log incoming Commands. Foundation for everything else in Phase B.

pub mod server;

// Wire types live in agentmux-common::ipc so the host (client) and
// launcher (server) compile against one definition. Re-exported
// here for convenience.
pub use agentmux_common::ipc::{Command, Event};
pub use server::run_ipc_server;

/// Construct the named-pipe path for a given data-dir hash.
/// Format: `\\.\pipe\agentmux-{hash16}\command`.
///
/// Per-data-dir scoping preserves multi-instance support per
/// `CLAUDE.md`: different portable folders / installed versions
/// → different data dirs → different hashes → distinct pipes.
/// Two launchers pointing at the SAME data dir will collide on
/// the pipe name, which is also the single-instance signal
/// Phase B.6 relies on.
pub fn pipe_name(data_dir_hash16: &str) -> String {
    format!("\\\\.\\pipe\\agentmux-{}\\command", data_dir_hash16)
}
