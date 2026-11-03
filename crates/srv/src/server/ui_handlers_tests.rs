// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of `ui_handlers.rs`, kept beside it so the handlers file stays
//! within its size baseline (scripts/check-file-sizes.mjs).

use super::{prune_old_screenshots, BROWSER_DEFAULT_SPLIT, SCREENSHOT_RETENTION};
use crate::server::app_api::pane::resolve_placement;
use crate::server::service::layout_intents::Placement;
use agentmux_common::SplitPosition::Before;

/// A browser an agent opens without saying where goes above the agent, each with half of the
/// space the agent had; an explicit "up" keeps the layout's default size.
#[test]
fn an_agents_browser_opens_above_it_taking_half_its_space() {
    assert_eq!(
        resolve_placement(Some(BROWSER_DEFAULT_SPLIT), Some("agent-block"), false),
        Placement::Split { target_block_id: "agent-block".into(), horizontal: false, position: Before, size_fraction: Some(0.5), focused: false }
    );
    assert!(matches!(
        resolve_placement(Some("up"), Some("agent-block"), false),
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
