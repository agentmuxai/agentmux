// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let f = Fixture { dir: tempfile::tempdir().unwrap() };
        drop(f.shared());
        drop(f.channel());
        f
    }
    fn ctx(&self) -> MigrationContext {
        MigrationContext {
            home: self.dir.path().to_path_buf(),
            data_dir: self.dir.path().to_path_buf(),
            shared_store_path: self.dir.path().join("store.db"),
            channel_store_path: self.dir.path().join("objects.db"),
        }
    }
    fn shared(&self) -> Store {
        Store::open_shared(&self.dir.path().join("store.db")).unwrap()
    }
    fn channel(&self) -> Store {
        Store::open(&self.dir.path().join("objects.db")).unwrap()
    }
    fn providers(&self, name: &str) -> PathBuf {
        self.dir.path().join("providers").join(name)
    }
    fn run(&self) {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        M0048SeedGrokMuseAccounts.up(&self.ctx()).unwrap();
    }
}

fn agent(store: &Store, id: &str, provider: &str) {
    let mut def = test_agent_def(id, id, "claude", "host", 1, "");
    def.provider = provider.to_string();
    store.agent_def_insert(&mut def).unwrap();
}

fn sign_grok_in(dir: &Path, body: serde_json::Value) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("auth.json"), body.to_string()).unwrap();
}

fn fresh_login() -> serde_json::Value {
    serde_json::json!({ "scope": { "key": "k", "create_time": chrono::Utc::now().to_rfc3339() } })
}

fn account_of(shared: &Store, agent_id: &str, provider: &str) -> Option<String> {
    shared.agent_identity_list_for_agent(agent_id).unwrap().into_iter().find(|l| l.provider == provider).map(|l| l.account_id)
}

#[test]
fn agents_on_a_shared_login_get_one_account_with_its_own_copy_of_it() {
    let f = Fixture::new();
    let (channel, shared) = (f.channel(), f.shared());
    agent(&channel, "a", "grok");
    agent(&channel, "b", "grok");
    agent(&channel, "c", "claude");
    sign_grok_in(&f.providers("grok"), fresh_login());
    std::fs::write(f.providers("grok").join("config.toml"), "x = 1").unwrap();

    f.run();

    let account_id = account_of(&shared, "a", "grok").expect("a is linked");
    assert_eq!(account_of(&shared, "b", "grok"), Some(account_id.clone()), "one login, one account");
    assert_eq!(account_of(&shared, "c", "grok"), None);
    let account = channel.identity_get(&account_id).unwrap().unwrap();
    assert_eq!((account.provider.as_str(), account.kind.as_str(), account.status.as_str()), ("grok", "oauth", "valid"));
    let SecretRef::OAuthConfigDir { dir } = account.secret_ref else { panic!("an OAuth account has a folder") };
    let dir = PathBuf::from(dir);
    assert!(dir.starts_with(f.dir.path().join("identities")), "inside the identities root, so deleting the account removes it");
    assert!(dir.join("auth.json").exists() && dir.join("config.toml").exists(), "the whole folder is carried over");
    assert!(f.providers("grok").join("auth.json").exists(), "the shared folder is left as it was");
}

#[test]
fn a_provider_with_no_login_in_the_shared_folder_is_left_alone() {
    let f = Fixture::new();
    let (channel, shared) = (f.channel(), f.shared());
    agent(&channel, "a", "grok");
    agent(&channel, "m", "muse");
    sign_grok_in(&f.providers("grok"), serde_json::json!({}));

    f.run();

    assert_eq!(account_of(&shared, "a", "grok"), None);
    assert_eq!(account_of(&shared, "m", "muse"), None);
    assert!(!f.dir.path().join("identities").exists(), "nothing was made");
}

#[test]
fn a_login_the_cli_can_refresh_is_carried_over_marked_expired() {
    let f = Fixture::new();
    let (channel, shared) = (f.channel(), f.shared());
    agent(&channel, "a", "grok");
    let long_ago = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
    sign_grok_in(&f.providers("grok"), serde_json::json!({ "s": { "key": "k", "create_time": long_ago, "refresh_token": "r" } }));

    f.run();

    let account_id = account_of(&shared, "a", "grok").expect("linked");
    assert_eq!(channel.identity_get(&account_id).unwrap().unwrap().status, "expired");
}

#[test]
fn muse_is_carried_over_from_its_own_folder_under_the_shared_one() {
    let f = Fixture::new();
    let (channel, shared) = (f.channel(), f.shared());
    agent(&channel, "m", "muse");
    std::fs::create_dir_all(f.providers("muse").join("muse")).unwrap();
    std::fs::write(f.providers("muse").join("muse").join("auth.json"), r#"{"token":"t"}"#).unwrap();

    f.run();

    let account_id = account_of(&shared, "m", "muse").expect("linked");
    let SecretRef::OAuthConfigDir { dir } = channel.identity_get(&account_id).unwrap().unwrap().secret_ref else { panic!() };
    assert!(PathBuf::from(dir).join("muse").join("auth.json").exists());
}

#[test]
fn an_agent_with_an_account_keeps_it_and_a_second_run_changes_nothing() {
    let f = Fixture::new();
    let (channel, shared) = (f.channel(), f.shared());
    agent(&channel, "linked", "grok");
    agent(&channel, "new", "grok");
    let mine = IdentityAccount {
        id: "mine".to_string(),
        name: "mine".to_string(),
        provider: "grok".to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: f.dir.path().join("mine").to_string_lossy().into_owned() },
        context: serde_json::json!({}),
        status: "valid".to_string(),
        created_at: 1,
        updated_at: 1,
    };
    shared.identity_upsert(&mine).unwrap();
    shared.agent_identity_link("linked", "mine", "grok").unwrap();
    sign_grok_in(&f.providers("grok"), fresh_login());

    f.run();
    let seeded = account_of(&shared, "new", "grok").expect("the agent without one is linked");
    assert_ne!(seeded, "mine");
    assert_eq!(account_of(&shared, "linked", "grok"), Some("mine".to_string()));

    f.run();
    assert_eq!(account_of(&shared, "new", "grok"), Some(seeded), "no second account");
    assert_eq!(std::fs::read_dir(f.dir.path().join("identities")).unwrap().count(), 1);
}

fn forked(store: &Store, id: &str, parent: &str, seeded: bool) {
    let mut def = test_agent_def(id, id, "claude", "host", 1, "");
    def.provider = "grok".to_string();
    def.parent_id = parent.to_string();
    def.is_seeded = seeded as i64;
    store.agent_def_insert(&mut def).unwrap();
}

fn account_row(shared: &Store, id: &str) {
    let row = IdentityAccount {
        id: id.to_string(),
        name: id.to_string(),
        provider: "grok".to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: format!("/accounts/{id}") },
        context: serde_json::json!({}),
        status: "valid".to_string(),
        created_at: 1,
        updated_at: 1,
    };
    shared.identity_upsert(&row).unwrap();
}

#[test]
fn the_accounts_an_agent_resolves_through_decide_who_is_seeded() {
    let f = Fixture::new();
    let (channel, shared) = (f.channel(), f.shared());
    account_row(&shared, "template-account");
    account_row(&shared, "fork-source-account");
    // A seeded template with an account: the child inherits it, so it is left alone.
    forked(&channel, "template", "", true);
    shared.agent_identity_link("template", "template-account", "grok").unwrap();
    forked(&channel, "from-template", "template", false);
    // An ordinary fork source is not a template: its account is not inherited, so the child needs one.
    forked(&channel, "source", "", false);
    shared.agent_identity_link("source", "fork-source-account", "grok").unwrap();
    forked(&channel, "from-source", "source", false);
    // Links for another provider mean the template's are not used either.
    forked(&channel, "other-links", "template", false);
    shared.agent_identity_link("other-links", "template-account", "claude").unwrap();
    sign_grok_in(&f.providers("grok"), fresh_login());

    f.run();

    assert_eq!(account_of(&shared, "from-template", "grok"), None, "inherits the template's account");
    let seeded = account_of(&shared, "from-source", "grok").expect("a fork source's account does not carry over");
    assert_eq!(account_of(&shared, "other-links", "grok"), Some(seeded.clone()));
    assert_ne!(seeded, "fork-source-account");
}
