// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::transcript::withhold;
use crate::identity::resolver::release_sign_in;

fn copy_body(access: &str, refresh: &str) -> String {
    let expires = agentmux_common::time::now_ms() + 3_600_000;
    serde_json::json!({ "claudeAiOauth": { "accessToken": access, "refreshToken": refresh, "expiresAt": expires } }).to_string()
}

/// A staging folder `<tree>/<container>/<account>` that names `config_dir` as its account, and the copy file in it.
fn folder(tree: &Path, container: &str, account: &str, config_dir: &str) -> PathBuf {
    let dir = tree.join(container).join(account);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta.json"), serde_json::json!({ "config_dir": config_dir, "seed": "x" }).to_string()).unwrap();
    dir.join(COPY_FILE)
}

const WAIT: Duration = Duration::from_secs(8);

/// Runs `write` every 100 ms until `value` is withheld, or [`WAIT`] runs out. The writes make sure a change lands once
/// the watch is live.
async fn withheld_after(value: &str, mut write: impl FnMut()) -> bool {
    let end = tokio::time::Instant::now() + WAIT;
    while tokio::time::Instant::now() < end {
        write();
        tokio::time::sleep(Duration::from_millis(100)).await;
        if withhold::text(value) != value {
            return true;
        }
    }
    false
}

#[test]
fn a_staged_copy_is_withheld_under_its_accounts_owners_and_released_with_them() {
    let tmp = tempfile::tempdir().unwrap();
    let (one, other) = ("acct-config-one", "acct-config-two");
    let file = folder(tmp.path(), "container-a", "h1", one);
    let other_file = folder(tmp.path(), "container-a", "h2", other);
    std::fs::write(&file, copy_body("canary-staged-access-0001", "canary-staged-refresh-0001")).unwrap();
    std::fs::write(&other_file, copy_body("canary-staged-access-0002", "canary-staged-refresh-0002")).unwrap();
    look(&file);
    look(&other_file);
    for value in ["canary-staged-access-0001", "canary-staged-refresh-0001", "canary-staged-access-0002"] {
        assert_eq!(withhold::text(value), withhold::MARK, "{value}");
    }
    release_sign_in(one);
    assert_eq!(withhold::text("canary-staged-refresh-0001"), "canary-staged-refresh-0001", "held under the account's own owners");
    assert_eq!(withhold::text("canary-staged-refresh-0002"), withhold::MARK, "another account's stays");
}

#[test]
fn a_copy_with_no_account_or_that_is_half_written_withholds_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let orphan = tmp.path().join("container-b").join("h1");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join(COPY_FILE), copy_body("canary-orphan-access-0001", "canary-orphan-refresh-0001")).unwrap();
    look(&orphan.join(COPY_FILE));
    let half = folder(tmp.path(), "container-b", "h2", "acct-config-half");
    std::fs::write(&half, "{ \"claudeAiOauth\": { \"accessToken\": \"canary-half-access-0001").unwrap();
    look(&half);
    for value in ["canary-orphan-access-0001", "canary-orphan-refresh-0001", "canary-half-access-0001"] {
        assert_eq!(withhold::text(value), value, "{value}");
    }
}

#[tokio::test]
async fn a_value_written_first_to_a_container_copy_is_withheld_with_no_account_file_change() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join(STAGING_ROOT);
    std::fs::create_dir_all(&tree).unwrap();
    start_at(FsWatchPool::new(), tree.clone());

    // The container comes after the start: it is followed with no further step.
    let file = folder(&tree, "container-c", "h1", "acct-config-watch");
    let body = copy_body("canary-copy-access-0001", "canary-copy-refresh-0001");
    assert!(withheld_after("canary-copy-refresh-0001", || std::fs::write(&file, &body).unwrap()).await);
    assert_eq!(withhold::text("canary-copy-access-0001"), withhold::MARK);
}

#[tokio::test]
async fn a_copy_replaced_by_a_renamed_temp_file_is_withheld() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join(STAGING_ROOT);
    std::fs::create_dir_all(&tree).unwrap();
    let file = folder(&tree, "container-d", "h1", "acct-config-replace");
    std::fs::write(&file, copy_body("canary-swap-access-0001", "canary-swap-refresh-0001")).unwrap();
    start_at(FsWatchPool::new(), tree.clone());

    let temp = file.with_file_name(".credentials.agentmux-1-1");
    let body = copy_body("canary-swap-access-0002", "canary-swap-refresh-0002");
    let replaced = withheld_after("canary-swap-refresh-0002", || {
        std::fs::write(&temp, &body).unwrap();
        std::fs::rename(&temp, &file).unwrap();
    })
    .await;
    assert!(replaced, "a refreshed value in a copy replaced by a renamed temp file is withheld");
    assert_eq!(withhold::text("canary-swap-access-0002"), withhold::MARK);
}
