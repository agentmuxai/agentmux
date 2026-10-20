// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Startup event bus: typed events emitted by each launcher startup stage and
//! consumed by the native splash screen renderer.
//!
//! Transport: `std::sync::mpsc::channel` (unbounded).  `Sender::send` never
//! blocks and is safe to call from async tokio code.  The splash thread drains
//! the receiver with `try_recv` every animation frame (~16 ms).
//!
//! On non-Windows platforms (or when the splash is disabled), the receiver is
//! dropped immediately and subsequent sends silently return `Err` — callers
//! ignore the result of every send so nothing blows up.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupStatus {
    Ok,
    Warn,
    Error,
}

#[derive(Debug)]
pub enum StartupEvent {
    /// A top-level startup stage has begun.
    StageBegin {
        stage: &'static str,
        label: &'static str,
    },
    /// A top-level startup stage has completed.
    StageEnd {
        stage: &'static str,
        duration_ms: u64,
        status: StartupStatus,
        detail: Option<String>,
    },
    /// A sub-item within a stage has begun (e.g. an individual migration).
    SubBegin {
        stage: &'static str,
        id: String,
        label: String,
    },
    /// A sub-item within a stage has completed.
    SubEnd {
        stage: &'static str,
        id: String,
        duration_ms: u64,
        status: StartupStatus,
        /// Short human-readable annotation shown after the time (e.g. "✓").
        detail: Option<String>,
    },
}

/// Clone-able sender half — one per startup stage, cloned as needed.
///
/// Every clone shares one sender, so [`StartupEventSink::reconnect`] moves
/// them all to a new receiver at once: the restart splash takes over the
/// events the launcher IPC server (which holds a clone) forwards from the
/// restarted host.
#[derive(Clone, Debug)]
pub struct StartupEventSink {
    tx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Sender<StartupEvent>>>,
}

impl StartupEventSink {
    /// Create a linked `(sink, receiver)` pair.  Pass the receiver to
    /// `splash::spawn_splash`; keep the sink in the launcher startup path.
    pub fn new() -> (Self, std::sync::mpsc::Receiver<StartupEvent>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { tx: std::sync::Arc::new(std::sync::Mutex::new(tx)) }, rx)
    }

    /// Point this sink and every clone of it at a new receiver, for a splash
    /// opened after the first one closed. The old receiver gets nothing more.
    pub fn reconnect(&self) -> std::sync::mpsc::Receiver<StartupEvent> {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Ok(mut current) = self.tx.lock() {
            *current = tx;
        }
        rx
    }

    fn send(&self, event: StartupEvent) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(event);
        }
    }

    pub fn stage_begin(&self, stage: &'static str, label: &'static str) {
        self.send(StartupEvent::StageBegin { stage, label });
    }

    pub fn stage_end(
        &self,
        stage: &'static str,
        duration_ms: u64,
        status: StartupStatus,
        detail: Option<String>,
    ) {
        self.send(StartupEvent::StageEnd {
            stage,
            duration_ms,
            status,
            detail,
        });
    }

    pub fn sub_begin(
        &self,
        stage: &'static str,
        id: impl Into<String>,
        label: impl Into<String>,
    ) {
        self.send(StartupEvent::SubBegin {
            stage,
            id: id.into(),
            label: label.into(),
        });
    }

    pub fn sub_end(
        &self,
        stage: &'static str,
        id: impl Into<String>,
        duration_ms: u64,
        status: StartupStatus,
        detail: Option<String>,
    ) {
        self.send(StartupEvent::SubEnd {
            stage,
            id: id.into(),
            duration_ms,
            status,
            detail,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_moves_every_clone_to_the_new_receiver() {
        let (sink, first) = StartupEventSink::new();
        let ipc_clone = sink.clone();
        ipc_clone.stage_begin("host", "Host startup");
        assert!(matches!(first.try_recv(), Ok(StartupEvent::StageBegin { stage: "host", .. })));

        let second = sink.reconnect();
        ipc_clone.stage_begin("cef_init", "CEF initialize");
        assert!(matches!(second.try_recv(), Ok(StartupEvent::StageBegin { stage: "cef_init", .. })));
        assert!(first.try_recv().is_err(), "the old receiver gets nothing after a reconnect");
    }
}
