// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Launcher-side event log — a thin binding over the shared implementation
// in `agentmux_common::event_log`.
//
// This file used to be the 415-line original (Phase D.2) that srv later
// mirrored byte-for-byte; the two differed only in which logging sink
// warnings went to. See `agentmux-common/src/event_log.rs` for the design
// and the audit that motivated the lift.
//
// Call sites are unchanged: `event_log::EventLog` and
// `event_log::run_disk_writer(log, rx)` resolve exactly as before. The only
// launcher-specific decision — warnings go to `crate::log`, the launcher's
// own rotating file, because the launcher installs no `tracing` subscriber
// — is bound here, once.

pub use agentmux_common::event_log::EventLog;

/// The launcher's disk writer: the shared writer with the launcher's
/// warning sink bound.
pub async fn run_disk_writer(
    log: std::sync::Arc<EventLog>,
    events_rx: tokio::sync::broadcast::Receiver<agentmux_common::ipc::Event>,
) {
    let warn: agentmux_common::event_log::WarnSink =
        std::sync::Arc::new(|m: &str| crate::log(m));
    agentmux_common::event_log::run_disk_writer(log, events_rx, warn).await
}
