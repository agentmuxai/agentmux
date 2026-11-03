// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `layout.otherBuilds` and `layout.restoreOtherBuild` handlers, called without the RPC engine.

use super::*;
use crate::server::tests::test_state;

fn scratch_roots(tag: &str) -> (std::path::PathBuf, ScanRoots) {
    let dir = std::env::temp_dir().join(format!("agentmux-other-builds-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("channels")).unwrap();
    let roots = ScanRoots { channels_root: dir.join("channels"), own_db_dir: None };
    (dir, roots)
}

fn list_req(window_id: &str, offer_only: Option<bool>) -> CommandOtherBuildLayoutsData {
    CommandOtherBuildLayoutsData { window_id: window_id.into(), offer_only }
}

#[tokio::test]
async fn an_offer_only_ask_from_a_window_with_nothing_to_offer_over_reads_nothing() {
    let state = test_state();
    let result = list_other_builds(&state, list_req("no-such-window", Some(true)), || {
        panic!("the roots are not even asked for")
    })
    .await
    .unwrap();
    assert!(!result.offer);
    assert!(result.layouts.is_empty());
}

#[tokio::test]
async fn without_data_folders_the_list_is_empty_and_not_an_error() {
    let state = test_state();
    let result = list_other_builds(&state, list_req("no-such-window", None), || None).await.unwrap();
    assert!(!result.offer);
    assert!(result.layouts.is_empty());
}

#[tokio::test]
async fn a_channels_root_with_no_builds_lists_nothing() {
    let state = test_state();
    let (dir, roots) = scratch_roots("empty");
    let result = list_other_builds(&state, list_req("no-such-window", None), || Some(roots)).await.unwrap();
    assert!(result.layouts.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn restoring_without_data_folders_says_so() {
    let state = test_state();
    let req = CommandRestoreOtherBuildLayoutData { window_id: "w".into(), channel: "stable".into(), version: "1.0.0".into() };
    let err = restore_other_build(&state, req, None).await.unwrap_err();
    assert_eq!(err, "layout.restoreOtherBuild: this AgentMux's data folders are unknown");
}

#[tokio::test]
async fn restoring_a_build_that_is_not_there_names_the_command_in_the_error() {
    let state = test_state();
    let (dir, roots) = scratch_roots("missing");
    let req = CommandRestoreOtherBuildLayoutData { window_id: "w".into(), channel: "stable".into(), version: "9.9.9".into() };
    let err = restore_other_build(&state, req, Some(roots)).await.unwrap_err();
    assert!(err.starts_with("layout.restoreOtherBuild: "), "{err}");
    let _ = std::fs::remove_dir_all(dir);
}
