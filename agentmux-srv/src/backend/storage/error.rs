// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Error types for the storage layer.


#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,

    #[error("already exists")]
    #[allow(dead_code)]
    AlreadyExists,

    #[error("empty OID")]
    EmptyOID,

    #[error("version mismatch: expected {expected}, got {actual}")]
    #[allow(dead_code)]
    VersionMismatch { expected: i64, actual: i64 },

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// The on-disk database was written by a NEWER AgentMux binary
    /// than the one running now. Refusing to open it is the
    /// channels-design safety lock from
    /// `SPEC_DATA_CHANNELS_2026_05_24.md` §3.3 — a forward-compat
    /// guard that prevents a downgrade from silently corrupting
    /// state the newer schema added. The user must upgrade the
    /// binary or switch channels (e.g.
    /// `AGENTMUX_CHANNEL=experiment` for an empty side dir) to
    /// recover.
    #[error(
        "{db}: this AgentMux is too old to open this data — schema v{found} \
         on disk, this binary speaks v{expected}. Upgrade AgentMux, or set \
         AGENTMUX_CHANNEL=<other> to use a fresh channel."
    )]
    SchemaTooNew {
        db: String,
        found: i64,
        expected: i64,
    },

    #[error("{0}")]
    Other(String),
}
