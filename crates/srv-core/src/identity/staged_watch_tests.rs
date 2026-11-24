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

/// Asserts `value` stops being withheld. Dropping a value marks the matcher stale and the next read rebuilds it, but
/// a read that arrives while another thread's rebuild is running (tests in one process share the registry) is served
/// by the matcher in use, which still holds the value until that rebuild is swapped in. So a drop is checked as taking
/// effect within a few seconds, never in the same instant.
fn dropped_soon(value: &str, why: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while hidden(value) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!hidden(value), "{why}");
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
    dropped_soon("canary-junk-refresh-0000", "the copy holds a bounded number of values: the oldest is dropped");
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
fn a_copy_registered_by_another_reader_is_held_under_the_copys_own_owners() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("container-z").join("h1");
    let json: serde_json::Value = serde_json::from_str(&copy_body("canary-reader-access-0001", "canary-reader-refresh-0001")).unwrap();
    withhold_copy(&folder, &json);
    assert!(hidden("canary-reader-access-0001") && hidden("canary-reader-refresh-0001"));
    crate::backend::transcript::withhold::release_prefix(&format!("sign-in:staged:{}:", folder.to_string_lossy()));
    dropped_soon("canary-reader-refresh-0001", "the copy's owners are named by its folder");
}

#[test]
fn an_expiry_in_either_shape_a_copy_is_written_in_is_not_used() {
    let tmp = tempfile::tempdir().unwrap();
    let file = folder(tmp.path(), "container-top", "h1", "cfg");
    for n in 0..40 {
        let far = agentmux_common::time::now_ms() + 86_400_000 + n;
        let top = serde_json::json!({ "accessToken": format!("canary-top-access-{n:04}"), "refreshToken": format!("canary-top-refresh-{n:04}"), "expiresAt": far });
        std::fs::write(&file, top.to_string()).unwrap();
        look(&file);
    }
    dropped_soon("canary-top-access-0000", "the cap holds for the top-level shape too: the oldest is dropped");
    assert!(hidden("canary-top-access-0039") && hidden("canary-top-refresh-0039"));
}

#[tokio::test]
async fn copies_that_exist_when_the_watch_starts_are_withheld_with_no_change() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join(STAGING_ROOT);
    let file = folder(&tree, "container-old", "h1", "cfg");
    std::fs::write(&file, copy_body("canary-start-access-0001", "canary-start-refresh-0001")).unwrap();
    start_at(FsWatchPool::new(), tree);
    let end = tokio::time::Instant::now() + WAIT;
    while !hidden("canary-start-refresh-0001") && tokio::time::Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(hidden("canary-start-refresh-0001") && hidden("canary-start-access-0001"));
}

#[tokio::test]
async fn a_staging_tree_that_does_not_exist_yet_is_created_and_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join(STAGING_ROOT);
    start_at(FsWatchPool::new(), tree.clone());
    let file = folder(&tree, "container-first", "h1", "cfg");
    let body = copy_body("canary-first-access-0001", "canary-first-refresh-0001");
    assert!(withheld_after("canary-first-refresh-0001", || std::fs::write(&file, &body).unwrap()).await);
}

#[test]
fn a_value_too_large_to_be_a_token_is_not_withheld_and_the_rest_of_the_copy_is() {
    let tmp = tempfile::tempdir().unwrap();
    let file = folder(tmp.path(), "container-big", "h1", "cfg");
    let big = format!("canary-big-{}", "x".repeat(crate::identity::resolver::oauth_probe::MAX_SIGN_IN_VALUE));
    std::fs::write(&file, copy_body(&big, "canary-big-refresh-0001")).unwrap();
    look(&file);
    assert!(hidden("canary-big-refresh-0001"));
    assert!(!hidden(&big), "an oversized value is not withheld");
    // A copy over the size a real one has is not read at all.
    let huge = tmp.path().join("container-big").join("h2");
    std::fs::create_dir_all(&huge).unwrap();
    let body = serde_json::json!({ "claudeAiOauth": { "refreshToken": "canary-huge-refresh-0001", "pad": "y".repeat(100_000) } });
    std::fs::write(huge.join(COPY_FILE), body.to_string()).unwrap();
    look(&huge.join(COPY_FILE));
    assert!(!hidden("canary-huge-refresh-0001"));
}

#[tokio::test]
async fn a_credentials_file_outside_the_staging_tree_is_not_registered() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join(STAGING_ROOT);
    let outside = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let pool = FsWatchPool::new();
    let _watched = pool.subscribe_dir(&outside);
    start_at(pool, tree);
    let file = outside.join(COPY_FILE);
    let body = copy_body("canary-outside-access-0001", "canary-outside-refresh-0001");
    for _ in 0..15 {
        std::fs::write(&file, &body).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!hidden("canary-outside-refresh-0001"), "only files under the staging tree are staged copies");
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
