// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::store::IdentityAccount;
use crate::backend::transcript::withhold;

fn account(id: &str, provider: &str, dir: &Path) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: id.to_string(),
        provider: provider.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: dir.to_string_lossy().into_owned() },
        context: serde_json::json!({}),
        status: "ok".to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

fn claude_file(access: &str, refresh: &str) -> String {
    let expires = agentmux_common::time::now_ms() + 3_600_000;
    serde_json::json!({ "claudeAiOauth": { "accessToken": access, "refreshToken": refresh, "expiresAt": expires } }).to_string()
}

/// How long a test waits for a value to be withheld: far under the 15 s account tick it must beat.
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

fn start_on(accounts: &[IdentityAccount]) -> Arc<Store> {
    let store = Arc::new(Store::open_in_memory().unwrap());
    for a in accounts {
        store.identity_upsert(a).unwrap();
    }
    start(FsWatchPool::new(), store.clone());
    store
}

#[test]
fn the_sign_in_file_of_each_provider_is_where_its_probe_reads() {
    let dir = Path::new("cfg");
    assert_eq!(sign_in_file("claude", dir), Some(dir.join(".credentials.json")));
    assert_eq!(sign_in_file("codex", dir), Some(dir.join("auth.json")));
    assert_eq!(sign_in_file("grok", dir), Some(dir.join("auth.json")));
    assert_eq!(sign_in_file("muse", dir), Some(dir.join("muse").join("auth.json")));
    assert_eq!(sign_in_file("github", dir), None);
}

#[tokio::test]
async fn a_value_written_into_a_sign_in_file_is_withheld_before_any_account_tick() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(".credentials.json");
    std::fs::write(&file, claude_file("canary-watch-access-0001", "canary-watch-refresh-0001")).unwrap();
    let _store = start_on(&[account("watch-a", "claude", tmp.path())]);

    let body = claude_file("canary-watch-access-0001", "canary-watch-refresh-0001");
    assert!(withheld_after("canary-watch-refresh-0001", || std::fs::write(&file, &body).unwrap()).await);
    assert_eq!(withhold::text("canary-watch-access-0001"), withhold::MARK);
}

#[tokio::test]
async fn a_sign_in_file_replaced_by_a_renamed_temp_file_is_withheld() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(".credentials.json"), claude_file("canary-replace-access-0001", "canary-replace-refresh-0001")).unwrap();
    let _store = start_on(&[account("watch-b", "claude", tmp.path())]);

    let (file, temp) = (tmp.path().join(".credentials.json"), tmp.path().join(".credentials.json.tmp"));
    let body = claude_file("canary-replace-access-0002", "canary-replace-refresh-0002");
    let replaced = withheld_after("canary-replace-refresh-0002", || {
        std::fs::write(&temp, &body).unwrap();
        std::fs::rename(&temp, &file).unwrap();
    })
    .await;
    assert!(replaced, "a refreshed value is withheld when the file is replaced");
    assert_eq!(withhold::text("canary-replace-access-0002"), withhold::MARK);
}

#[tokio::test]
async fn the_sign_in_files_of_other_providers_are_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("auth.json");
    std::fs::write(&file, "{}").unwrap();
    let _store = start_on(&[account("watch-c", "codex", tmp.path())]);
    let body = serde_json::json!({ "tokens": { "refresh_token": "canary-watch-codex-0001" } }).to_string();
    assert!(withheld_after("canary-watch-codex-0001", || std::fs::write(&file, &body).unwrap()).await);
}

#[tokio::test]
async fn an_account_added_after_the_start_is_followed_and_one_deleted_is_let_go() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(".credentials.json");
    std::fs::write(&file, "{}").unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let pool = FsWatchPool::new();
    let mut followed = HashMap::new();
    reconcile(&pool, wanted(&store), &mut followed);
    assert!(followed.is_empty());

    store.identity_upsert(&account("watch-d", "claude", tmp.path())).unwrap();
    reconcile(&pool, wanted(&store), &mut followed);
    assert_eq!(followed.len(), 1, "followed once the account exists");
    reconcile(&pool, wanted(&store), &mut followed);
    assert_eq!(followed.len(), 1, "read again, still one");

    store.identity_delete("watch-d").unwrap();
    reconcile(&pool, wanted(&store), &mut followed);
    assert!(followed.is_empty(), "let go once the account is deleted");
}
