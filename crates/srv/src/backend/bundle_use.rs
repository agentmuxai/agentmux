// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A count of how often agent bundles are moved as files (export, import), so a decision to keep
//! or drop that transport can rest on use instead of a guess
//! (`docs/reports/REPORT_AGENT_BUNDLES_REFINE_OR_DROP_2026_11_03.md`).
//!
//! One line per call in `bundle-transport-use.jsonl` in the channel's folder: the time and which
//! call, nothing about the bundle, the agent or the person. The running total is logged with each
//! call, so `muxlog` shows it. Best-effort: a failure to write is dropped.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

const FILE_NAME: &str = "bundle-transport-use.jsonl";

/// Note that `op` ("export", "export_for_agent", "import_commit", ...) was called.
pub fn note(op: &str) {
    // A test must not write into the home of whoever runs it.
    #[cfg(any(test, feature = "test-support"))]
    let _ = op;
    #[cfg(not(any(test, feature = "test-support")))]
    if let Some(paths) = agentmux_common::DataPaths::from_env() {
        if let Some(total) = note_at(&paths.instance_dir.join(FILE_NAME), op, agentmux_common::time::now_ms()) {
            tracing::info!(op, total, "bundle transport used");
        }
    }
}

/// Append one line to `path`; the number of calls noted there so far.
fn note_at(path: &Path, op: &str, at_ms: i64) -> Option<u64> {
    let line = serde_json::json!({ "at_ms": at_ms, "op": op }).to_string();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
    writeln!(file, "{line}").ok()?;
    Some(counts(path).values().sum())
}

/// How many times each call was noted in `path`.
pub fn counts(path: &Path) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else { return counts };
    for line in text.lines() {
        if let Some(op) = serde_json::from_str::<serde_json::Value>(line).ok().and_then(|v| v.get("op")?.as_str().map(str::to_string)) {
            *counts.entry(op).or_insert(0) += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_call_and_gives_the_running_total() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert_eq!(note_at(&path, "export", 1), Some(1));
        assert_eq!(note_at(&path, "import_commit", 2), Some(2));
        assert_eq!(note_at(&path, "export", 3), Some(3));
        let by_op = counts(&path);
        assert_eq!(by_op.get("export"), Some(&2));
        assert_eq!(by_op.get("import_commit"), Some(&1));
        assert_eq!(counts(&dir.path().join("none.jsonl")).len(), 0);
    }

    #[test]
    fn a_line_that_is_not_json_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "not json\n{\"at_ms\":1,\"op\":\"export\"}\n").unwrap();
        assert_eq!(counts(&path).get("export"), Some(&1));
    }

    #[test]
    fn an_unwritable_place_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(note_at(&dir.path().join("no-such-dir").join(FILE_NAME), "export", 1), None);
    }
}
