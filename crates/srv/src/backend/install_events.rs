// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `install_chunk` event an install publishes (`server/install_handlers.rs`,
//! `server/system_install_handlers.rs`), kept below the server so `mps_events` can name it.

use serde::Serialize;

/// `install_chunk`: one line of an install's output, or its end. Scoped `install:<sessionId>`;
/// `system.install` sends the same events.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(untagged)]
pub enum InstallChunkEvent {
    Line {
        #[serde(rename = "sessionId")]
        session_id: String,
        line: String,
        stream: InstallStream,
    },
    Done {
        #[serde(rename = "sessionId")]
        session_id: String,
        op: InstallDoneOp,
        ok: bool,
        /// Why it failed: a message, or an `AgentMuxError` in its wire form. `null` when it didn't.
        #[ts(type = "unknown")]
        error: serde_json::Value,
    },
}

impl InstallChunkEvent {
    pub fn line(session_id: &str, line: String, stream: InstallStream) -> Self {
        Self::Line { session_id: session_id.to_string(), line, stream }
    }
    pub fn done(session_id: &str, ok: bool, error: serde_json::Value) -> Self {
        Self::Done { session_id: session_id.to_string(), op: InstallDoneOp::Done, ok, error }
    }
}

#[derive(Debug, Clone, Copy, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum InstallStream {
    Stdout,
    Stderr,
}

/// The `op` of the last event of an install.
#[derive(Debug, Clone, Copy, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum InstallDoneOp {
    Done,
}
