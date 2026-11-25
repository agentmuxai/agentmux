// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of `ui_handlers.rs`, kept beside it so the handlers file stays
//! within its size baseline (scripts/check-file-sizes.mjs).

use super::{prune_old_screenshots, SCREENSHOT_RETENTION};
use crate::server::app_api::pane::resolve_placement;
use crate::server::service::layout_intents::Placement;
use agentmux_common::SplitPosition::After;

/// A split asked for as `down:half` gives the new pane half of the space the pane had; a plain "down" keeps the layout's
/// default size. (A browser an agent opens without saying where is a tab, not a split: `agent_tab_default`.)
#[test]
fn a_split_asked_for_as_half_takes_half_the_space() {
    assert_eq!(
        resolve_placement(Some("down:half"), Some("agent-block"), false),
        Placement::Split { target_block_id: "agent-block".into(), horizontal: false, position: After, size_fraction: Some(0.5), focused: false }
    );
    assert!(matches!(
        resolve_placement(Some("down"), Some("agent-block"), false),
        Placement::Split { horizontal: false, size_fraction: None, .. }
    ));
}

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
