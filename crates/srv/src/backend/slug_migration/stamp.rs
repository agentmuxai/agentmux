// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a running srv can do, written beside its databases at startup
//! (`<data>/srv-features.json`), so a migration can tell whether every
//! AgentMux running on this host understands former slugs before it renames
//! one (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` §4 Phase 2, preconditions).
//!
//! Nothing else said it reliably. The srv lock (`<data>/srv.lock`) says which
//! pid runs on a data directory, but not what it is; a lease names a version
//! only while it holds an agent; the cross-channel registry lists only
//! instances with an agent registered. So each srv now writes its pid and its
//! features here, and a migration accepts an instance only when the stamp's
//! pid is the lock's. An srv older than this file writes none and is refused,
//! which is the safe answer: an instance that predates Phase 1 would resolve
//! nothing addressed to a former slug, and one that predates Phase 0 could
//! write the old slug back.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const STAMP_FILE: &str = "srv-features.json";

/// Phases 0 and 1 of the migration spec: one reader of the slug, no stale
/// write-back, the server's spawn identity, former slugs resolved and
/// reserved. An srv that writes this understands a renamed agent.
pub const FORMER_SLUGS: &str = "former-slugs-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub pid: u32,
    pub version: String,
    #[serde(default)]
    pub features: Vec<String>,
}

impl Stamp {
    pub fn has(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

/// This srv's stamp, written into its data directory. Best effort: a stamp
/// that can't be written only makes a migration refuse while this srv runs.
pub fn write(data_dir: &Path) {
    let stamp = Stamp {
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        features: vec![FORMER_SLUGS.to_string()],
    };
    let result = serde_json::to_vec_pretty(&stamp)
        .map_err(|e| e.to_string())
        .and_then(|bytes| super::home::write_file(&data_dir.join(STAMP_FILE), &bytes));
    if let Err(e) = result {
        tracing::warn!(error = %e, "srv features stamp: not written; a fork slug migration will refuse while this srv runs");
    }
}

pub fn read(path: &Path) -> Option<Stamp> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The pid an srv lock names, if it names one.
pub fn lock_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
