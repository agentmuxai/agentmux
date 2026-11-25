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

/// A hand-off's reason reaches the person whole: a long one is refused with the limit, never cut.
#[test]
fn a_handoff_reason_is_carried_whole_or_refused_with_the_limit() {
    use super::{handoff_reason, HANDOFF_REASON_MAX};
    let long_but_ok = "Upload the file on the form, then press Done. ".repeat(12);
    assert!(long_but_ok.chars().count() > 300 && long_but_ok.chars().count() <= HANDOFF_REASON_MAX);
    assert_eq!(handoff_reason(&long_but_ok).unwrap(), long_but_ok, "past the old 300-character cut, carried whole");
    let exactly = "x".repeat(HANDOFF_REASON_MAX);
    assert_eq!(handoff_reason(&exactly).unwrap(), exactly);
    let over = "x".repeat(HANDOFF_REASON_MAX + 1);
    let why = handoff_reason(&over).unwrap_err();
    assert!(why.contains(&(HANDOFF_REASON_MAX + 1).to_string()) && why.contains(&HANDOFF_REASON_MAX.to_string()) && why.contains("not cut"), "{why}");
    assert!(handoff_reason("").unwrap_err().contains("what the user should do"));
    // Characters, not bytes: a reason in another script is not refused early.
    assert!(handoff_reason(&"é".repeat(HANDOFF_REASON_MAX)).is_ok());
}
