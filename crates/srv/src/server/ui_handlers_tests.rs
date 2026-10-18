// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of `ui_handlers.rs`, kept beside it so the handlers file stays
//! within its size baseline (scripts/check-file-sizes.mjs).

use super::{prune_old_screenshots, SCREENSHOT_RETENTION};

#[test]
fn prune_old_screenshots_deletes_only_stale_pngs() {
    let dir = tempfile::tempdir().unwrap();

    let fresh = dir.path().join("fresh.png");
    std::fs::write(&fresh, b"png").unwrap();

    let stale = dir.path().join("stale.png");
    std::fs::write(&stale, b"png").unwrap();
    let old_time = std::time::SystemTime::now() - (SCREENSHOT_RETENTION * 2);
    let file = std::fs::File::options().write(true).open(&stale).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(old_time))
        .unwrap();

    // Non-PNG files must never be touched, however old.
    let other = dir.path().join("notes.txt");
    std::fs::write(&other, b"keep me").unwrap();
    let file = std::fs::File::options().write(true).open(&other).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(old_time))
        .unwrap();

    prune_old_screenshots(dir.path());

    assert!(fresh.exists(), "fresh screenshot must survive pruning");
    assert!(!stale.exists(), "stale screenshot must be pruned");
    assert!(other.exists(), "non-png files must never be pruned");
}
