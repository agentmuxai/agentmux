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
//! use `let _ = self.tx.send(...)` so nothing blows up.

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
#[derive(Clone)]
pub struct StartupEventSink {
    tx: std::sync::mpsc::Sender<StartupEvent>,
}

impl StartupEventSink {
    /// Create a linked `(sink, receiver)` pair.  Pass the receiver to
    /// `splash::spawn_splash`; keep the sink in the launcher startup path.
    pub fn new() -> (Self, std::sync::mpsc::Receiver<StartupEvent>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { tx }, rx)
    }

    pub fn stage_begin(&self, stage: &'static str, label: &'static str) {
        let _ = self.tx.send(StartupEvent::StageBegin { stage, label });
    }

    pub fn stage_end(
        &self,
        stage: &'static str,
        duration_ms: u64,
        status: StartupStatus,
        detail: Option<String>,
    ) {
        let _ = self.tx.send(StartupEvent::StageEnd {
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
        let _ = self.tx.send(StartupEvent::SubBegin {
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
        let _ = self.tx.send(StartupEvent::SubEnd {
            stage,
            id: id.into(),
            duration_ms,
            status,
            detail,
        });
    }
}
