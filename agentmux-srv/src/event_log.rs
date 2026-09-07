// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// srv-side event log — a thin binding over the shared implementation in
// `agentmux_common::event_log`.
//
// This file used to be a 415-line byte-for-byte mirror of the launcher's
// event log, differing only in which logging sink warnings went to. Its own
// header carried the to-do ("Phase E.7 cleanup: lift the shared parts into
// agentmux-common and unify launcher + srv event logs. (reagent P2 #610.)");
// this is that lift. See `agentmux-common/src/event_log.rs` for the design.
//
// Call sites are unchanged: `event_log::EventLog` and
// `event_log::run_disk_writer(log, rx)` resolve exactly as before. The only
// srv-specific decision — warnings go to `tracing::warn!` under the
// `event-log` target — is bound here, once.

pub use agentmux_common::event_log::EventLog;

/// srv's disk writer: the shared writer with srv's warning sink bound.
pub async fn run_disk_writer(
    log: std::sync::Arc<EventLog>,
    events_rx: tokio::sync::broadcast::Receiver<agentmux_common::ipc::Event>,
) {
    let warn: agentmux_common::event_log::WarnSink =
        std::sync::Arc::new(|m: &str| tracing::warn!(target: "event-log", "{}", m));
    agentmux_common::event_log::run_disk_writer(log, events_rx, warn).await
}
