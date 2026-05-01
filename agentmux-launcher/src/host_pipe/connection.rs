// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CPD-2 — host_pipe connection state.
//
// Today the launcher's IPC server (`agentmux-launcher::ipc::server`)
// owns the accept loop for the launcher pipe; the host's writer
// half is handed to `HostPipe::set_writer` once the connecting
// client registers as `ClientKind::Host`. There is no dedicated
// accept loop in this module — the existing one in `ipc::server`
// already covers both inbound (host → launcher Commands) and
// outbound (launcher → host Events / Commands via `HostPipe`) on
// the same pipe instance.
//
// This file is reserved for future expansion (connection-state
// telemetry, reconnect-attempt counter for `--diag` surfacing, etc.)
// per the SPEC §3.5 module layout. CPD-3+ may grow it; CPD-2 keeps
// the surface minimal.

#![allow(dead_code)]

use std::time::Instant;

/// Connection-state telemetry. Stashed inside `HostPipe`'s inner
/// mutex (alongside the writer + pending buffer). Kept distinct so
/// future `--diag host_pipe` surfaces have a clean snapshot point
/// without reaching into private fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConnectionTelemetry {
    /// Total times a writer has been installed (set_writer calls).
    /// Increments on every reconnect.
    pub connect_count: u64,
    /// Total times the writer has been cleared (disconnect signals).
    pub disconnect_count: u64,
    /// Total frames written successfully.
    pub frames_written: u64,
    /// Total frames buffered while disconnected.
    pub frames_buffered: u64,
    /// Total frames dropped (overflow + 30s timeout combined).
    pub frames_dropped: u64,
    /// Most recent `host_disconnected_at`, if any. Convenience copy
    /// for telemetry surfaces; the authoritative value is on
    /// `HostPipeInner`.
    pub last_disconnect_at: Option<Instant>,
}
