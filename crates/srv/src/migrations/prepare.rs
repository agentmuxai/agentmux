// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-srv migrate --prepare`: everything a version's data needs
//! before srv opens it, as one step a supervisor runs while srv is stopped.
//!
//! In order: copy the newest older version's databases forward when this
//! version has none yet (`bootstrap/carry_forward.rs`), then the same locked
//! batch the daemon runs (`runner::apply_pending`), whose backup leaves out
//! `objects.db` after a copy-forward. Each part is reported on stderr in the
//! `agentmux_common::srv_stderr` protocol (STEP and MIGRATION lines), so the
//! launcher's native splash can show it; the NDJSON summary on stdout is the
//! one plain `migrate` prints.
//!
//! The upgrade step of `docs/specs/SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md`
//! §10 (S1a). Nothing calls it yet: the update route (S1b) and srv's boot gate
//! (S1c) will.

use std::path::Path;

use super::runner::{apply_pending, emit_summary, write_error_log, STDERR_PROGRESS};
use super::REGISTRY;

pub(super) fn run(data_dir: &Path, home: &Path, shared_store_path: &Path) -> i32 {
    let version = std::env::var("AGENTMUX_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    run_for_version(data_dir, home, shared_store_path, &version)
}

/// [`run`] for an explicit version, so a test needn't set the environment.
pub(super) fn run_for_version(data_dir: &Path, home: &Path, shared_store_path: &Path, version: &str) -> i32 {
    crate::bootstrap::carry_forward::from_previous_version(&data_dir.join("db"), version);
    match apply_pending(REGISTRY, home, shared_store_path, data_dir, Some(&STDERR_PROGRESS)) {
        Ok(outcome) => {
            emit_summary(outcome.applied, outcome.skipped);
            0
        }
        Err(msg) => {
            write_error_log(home, &msg);
            eprintln!("{}", agentmux_common::srv_stderr::migration_failed_line(&msg));
            1
        }
    }
}
