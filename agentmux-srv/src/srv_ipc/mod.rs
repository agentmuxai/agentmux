// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1b — srv-side named-pipe IPC server.
//
// Mirrors agentmux-launcher's IPC plumbing:
//   * Per-data-dir pipe (path passed via AGENTMUX_SRV_PIPE_PATH env
//     by the launcher, who owns the data-dir hash).
//   * `tokio::sync::broadcast` event bus.
//   * In-memory event log (`crate::event_log::EventLog`) feeding D.3
//     `GetEvents` replay.
//   * Per-connection fanout task: subscribe to the bus before reading
//     commands so events emitted while the read loop awaits aren't lost.
//
// E.1b laid the plumbing; E.2 added workspace arms; tab/block/
// layout arms arrive in E.2b through E.4. See
// `agentmux-srv::reducer` for the current command surface.

pub mod server;

pub use server::{run_srv_ipc_server, ServerCtx};
