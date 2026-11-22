// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::transcript::withhold;
use crate::identity::resolver::probe_oauth_status;

fn copy_body(access: &str, refresh: &str) -> String {
    let expires = agentmux_common::time::now_ms() + 3_600_000;
    serde_json::json!({ "claudeAiOauth": { "accessToken": access, "refreshToken": refresh, "expiresAt": expires } }).to_string()
}

/// A staging folder `<tree>/<container>/<account>`, with the `meta.json` the staging code writes, and the copy file in it.
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

fn hidden(value: &str) -> bool {
    withhold::text(value) == withhold::MARK
}

#[test]
fn a_staged_copys_values_are_withheld_without_trusting_its_expiry() {
    let tmp = tempfile::tempdir().unwrap();
    let file = folder(tmp.path(), "container-a", "h1", "acct-config-one");
    std::fs::write(&file, copy_body("canary-staged-access-0001", "canary-staged-refresh-0001")).unwrap();
    look(&file);
    assert!(hidden("canary-staged-access-0001") && hidden("canary-staged-refresh-0001"));
    // A replaced value stays withheld (kept), and the expiry the copy states is not what bounds it.
    std::fs::write(&file, copy_body("canary-staged-access-0002", "canary-staged-refresh-0002")).unwrap();
    look(&file);
    assert!(hidden("canary-staged-access-0001") && hidden("canary-staged-access-0002") && hidden("canary-staged-refresh-0002"));
}

#[test]
fn a_container_that_floods_its_copy_cannot_push_out_what_the_accounts_own_file_holds() {
    let account = tempfile::tempdir().unwrap();
    let real = serde_json::json!({ "claudeAiOauth": { "accessToken": "canary-real-access-0001", "refreshToken": "canary-real-refresh-0001", "expiresAt": agentmux_common::time::now_ms() + 3_600_000 } });
    std::fs::write(account.path().join(".credentials.json"), real.to_string()).unwrap();
    probe_oauth_status("claude", account.path().to_str().unwrap(), agentmux_common::time::now_ms());
    assert!(hidden("canary-real-refresh-0001"));

    // The container's copy names that very account in its meta.json and is rewritten with many distinct values.
    let tmp = tempfile::tempdir().unwrap();
    let file = folder(tmp.path(), "container-evil", "h1", account.path().to_str().unwrap());
    for n in 0..40 {
        std::fs::write(&file, copy_body(&format!("canary-junk-access-{n:04}"), &format!("canary-junk-refresh-{n:04}"))).unwrap();
        look(&file);
    }
    assert!(hidden("canary-real-access-0001") && hidden("canary-real-refresh-0001"), "the account's own values stay withheld");
    assert!(!hidden("canary-junk-refresh-0000"), "the copy holds a bounded number of values: the oldest is dropped");
    assert!(hidden("canary-junk-refresh-0039"));
}

#[test]
fn each_staging_folder_has_its_own_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = (folder(tmp.path(), "container-x", "h1", "cfg"), folder(tmp.path(), "container-y", "h1", "cfg"));
    std::fs::write(&a, copy_body("canary-cap-access-xx-0000", "canary-cap-refresh-xx-0000")).unwrap();
    look(&a);
    for n in 0..20 {
        std::fs::write(&b, copy_body(&format!("canary-cap-access-yy-{n:04}"), &format!("canary-cap-refresh-yy-{n:04}"))).unwrap();
        look(&b);
    }
    assert!(hidden("canary-cap-refresh-xx-0000"), "another container's copy is not pushed out");
}

#[test]
fn a_half_written_copy_withholds_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let half = folder(tmp.path(), "container-b", "h2", "acct-config-half");
    std::fs::write(&half, "{ \"claudeAiOauth\": { \"accessToken\": \"canary-half-access-0001").unwrap();
    look(&half);
    assert!(!hidden("canary-half-access-0001"));
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
    assert!(hidden("canary-copy-access-0001"));
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
    assert!(hidden("canary-swap-access-0002"));
}
