// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `Store::muxbus_load` returns, and what it leaves in the secret store and
//! the SQL row, for each place a sign-in can be found: the single blob, split
//! entries beside it or alone, the old plaintext SQL columns, and the host-wide
//! session a channel adopts once.
//!
//! Written before `muxbus_load_tokens` was split into named steps, to hold its
//! behaviour still. The secrets live in the file backend (`use_file_store`), never
//! the OS keychain. macOS and Linux paths only: Windows reads the split layout
//! first, so its branches are not exercised here.

use super::*;
use crate::test_support::ISOLATED_AUTH_ENV_LOCK;

use std::sync::atomic::{AtomicUsize, Ordering};

/// Puts the process env back as the test found it.
struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

/// One channel's sign-in in its own namespace and SQL row, in a fresh store.
struct Rig {
    store: Store,
    ns: String,
    user_sub: String,
    _tmp: tempfile::TempDir,
    _env: EnvRestore,
    _lock: std::sync::MutexGuard<'static, ()>,
}

fn file_store() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let _ = secret_store::use_file_store(dir.path().join("secrets"));
    assert_eq!(secret_store::backend_name(), "file", "these tests never touch the OS keychain");
}

fn rig() -> Rig {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let lock = ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    file_store();
    let tmp = tempfile::tempdir().unwrap();
    let keys = ["AGENTMUX_CHANNEL", "AGENTMUX_HOME_OVERRIDE", "AGENTMUX_SHARED_DIR", "AGENTMUX_INSTANCE_DIR", "AGENTMUX_ISOLATED_AUTH"];
    let env = EnvRestore(keys.iter().map(|k| (*k, std::env::var_os(k))).collect());
    let channel = format!("loadtest-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    std::env::set_var("AGENTMUX_CHANNEL", &channel);
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
    for k in ["AGENTMUX_SHARED_DIR", "AGENTMUX_INSTANCE_DIR", "AGENTMUX_ISOLATED_AUTH"] {
        std::env::remove_var(k);
    }
    let ns = keychain_namespace();
    assert_eq!(ns, format!("muxbus:channel:{channel}"));
    Rig { store: Store::open_in_memory().unwrap(), ns, user_sub: "user-1".to_string(), _tmp: tmp, _env: env, _lock: lock }
}

/// An id token that names `sub` (the claims are not verified).
fn jwt(sub: &str) -> String {
    use base64::Engine;
    let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!("{}.{}.sig", b64(r#"{"alg":"none"}"#), b64(&format!(r#"{{"sub":"{sub}"}}"#)))
}

fn tokens(tag: &str, sub: &str) -> MuxBusTokens {
    MuxBusTokens { access_token: format!("access-{tag}"), refresh_token: format!("refresh-{tag}"), id_token: jwt(sub) }
}

fn same(a: &MuxBusTokens, b: &MuxBusTokens) -> bool {
    a.access_token == b.access_token && a.refresh_token == b.refresh_token && a.id_token == b.id_token
}

impl Rig {
    /// A saved sign-in: the SQL row and, on this platform, the blob.
    fn save(&self, t: &MuxBusTokens) {
        self.store
            .muxbus_save(&MuxBusCredentials {
                cognito_domain: "login.example".into(),
                client_id: "client".into(),
                access_token: t.access_token.clone(),
                refresh_token: t.refresh_token.clone(),
                id_token: t.id_token.clone(),
                expires_at: 4_000_000_000,
                user_email: "person@example.com".into(),
                user_sub: self.user_sub.clone(),
            })
            .unwrap();
    }
    fn blob(&self) -> Option<MuxBusTokens> {
        secret_store::get_optional(&blob_key(&self.ns)).unwrap().map(|b| serde_json::from_str::<MuxBusBlob>(&b).unwrap().tokens)
    }
    fn blob_generation(&self) -> String {
        let b = secret_store::get_optional(&blob_key(&self.ns)).unwrap().unwrap();
        serde_json::from_str::<MuxBusBlob>(&b).unwrap().generation
    }
    fn remove_blob(&self) {
        secret_store::delete(&blob_key(&self.ns)).unwrap();
    }
    fn split(&self) -> Option<MuxBusTokens> {
        read_split_tokens(&self.ns).unwrap().map(|(t, _)| t)
    }
    fn put_split(&self, t: &MuxBusTokens) {
        write_split_tokens(&self.ns, t).unwrap();
    }
    fn sql_plaintext(&self) -> (String, String, String) {
        let conn = self.store.conn.lock().unwrap();
        conn.query_row("SELECT access_token, refresh_token, id_token FROM db_muxbus_credentials WHERE id = ?1", params![current_row_id()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
    }
    /// The row as an install from before keychain storage left it.
    fn set_sql_plaintext(&self, t: &MuxBusTokens) {
        let conn = self.store.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_muxbus_credentials SET access_token = ?1, refresh_token = ?2, id_token = ?3 WHERE id = ?4",
            params![t.access_token, t.refresh_token, t.id_token, current_row_id()],
        )
        .unwrap();
    }
    /// `muxbus_load` with the migration writes on, the way a normal load runs.
    fn load(&self) -> MuxBusCredentials {
        self.store.muxbus_load().unwrap().expect("a row")
    }
    /// The side-effect-free read `muxbus_is_fresh` uses.
    fn peek(&self) -> MuxBusCredentials {
        self.store.muxbus_load_impl(false).unwrap().expect("a row")
    }
}

fn got(c: &MuxBusCredentials) -> MuxBusTokens {
    MuxBusTokens { access_token: c.access_token.clone(), refresh_token: c.refresh_token.clone(), id_token: c.id_token.clone() }
}

/// Clears the host-wide set around a test that writes it.
struct GlobalSet;

impl GlobalSet {
    fn put(t: &MuxBusTokens) -> Self {
        Self::clear();
        write_single_blob(GLOBAL_KEYCHAIN_NS, t).unwrap();
        Self
    }
    fn clear() {
        delete_split_tokens(GLOBAL_KEYCHAIN_NS);
        secret_store::delete(&blob_key(GLOBAL_KEYCHAIN_NS)).unwrap();
    }
    fn blob() -> Option<MuxBusTokens> {
        secret_store::get_optional(&blob_key(GLOBAL_KEYCHAIN_NS)).unwrap().map(|b| serde_json::from_str::<MuxBusBlob>(&b).unwrap().tokens)
    }
}

impl Drop for GlobalSet {
    fn drop(&mut self) {
        Self::clear();
    }
}

#[test]
fn no_row_means_not_signed_in() {
    let r = rig();
    assert!(r.store.muxbus_load().unwrap().is_none());
    assert!(r.store.muxbus_load_impl(false).unwrap().is_none());
}

#[test]
fn a_saved_sign_in_is_read_from_its_blob_and_the_blob_is_left_as_it_was() {
    let r = rig();
    let t = tokens("a", "user-1");
    r.save(&t);
    let generation = r.blob_generation();
    for c in [r.peek(), r.load()] {
        assert!(same(&got(&c), &t));
        assert_eq!((c.cognito_domain.as_str(), c.client_id.as_str(), c.user_sub.as_str()), ("login.example", "client", "user-1"));
        assert_eq!(c.expires_at, 4_000_000_000);
    }
    assert_eq!(r.blob_generation(), generation, "a plain read does not rewrite the blob");
    assert_eq!(r.sql_plaintext(), (String::new(), String::new(), String::new()));
}

#[test]
fn a_corrupted_blob_is_an_error_not_a_sign_out() {
    let r = rig();
    r.save(&tokens("a", "user-1"));
    secret_store::put(&blob_key(&r.ns), "{ not json").unwrap();
    for load in [r.store.muxbus_load(), r.store.muxbus_load_impl(false)] {
        let err = load.err().expect("an error").to_string();
        assert!(err.contains("stored keychain blob is corrupted"), "{err}");
    }
}

#[test]
fn split_entries_fresher_than_the_blob_win_and_a_normal_load_collapses_them_into_the_blob() {
    let r = rig();
    let old = tokens("old", "user-1");
    let fresh = tokens("fresh", "user-1");
    r.save(&old);
    std::thread::sleep(std::time::Duration::from_millis(5));
    r.put_split(&fresh);

    // The side-effect-free read returns them and changes nothing.
    assert!(same(&got(&r.peek()), &fresh));
    assert!(same(&r.blob().unwrap(), &old));
    assert!(r.split().is_some());

    // A normal load finishes the interrupted migration: one blob, no split entries.
    assert!(same(&got(&r.load()), &fresh));
    assert!(same(&r.blob().unwrap(), &fresh));
    assert!(r.split().is_none());
    assert!(same(&got(&r.load()), &fresh), "and the next load reads the blob alone");
}

#[test]
fn a_blob_at_least_as_fresh_as_the_split_entries_wins_and_a_normal_load_removes_the_leftovers() {
    let r = rig();
    let stale = tokens("stale", "user-1");
    let current = tokens("current", "user-1");
    r.put_split(&stale);
    std::thread::sleep(std::time::Duration::from_millis(5));
    r.save(&current);

    assert!(same(&got(&r.peek()), &current));
    assert!(r.split().is_some(), "the side-effect-free read leaves the leftovers");

    assert!(same(&got(&r.load()), &current));
    assert!(r.split().is_none(), "a normal load finishes the cleanup");
    assert!(same(&r.blob().unwrap(), &current));
}

#[test]
fn split_entries_alone_are_read_and_a_normal_load_collapses_them_into_a_blob() {
    let r = rig();
    let t = tokens("chunked", "user-1");
    r.save(&tokens("placeholder", "user-1"));
    r.remove_blob();
    r.put_split(&t);

    assert!(same(&got(&r.peek()), &t));
    assert!(r.blob().is_none() && r.split().is_some(), "the side-effect-free read writes nothing");

    assert!(same(&got(&r.load()), &t));
    assert!(same(&r.blob().unwrap(), &t));
    assert!(r.split().is_none());
}

#[test]
fn torn_split_entries_with_a_whole_refresh_token_give_back_just_that_token() {
    let r = rig();
    let t = tokens("torn", "user-1");
    r.save(&tokens("placeholder", "user-1"));
    r.remove_blob();
    r.put_split(&t);
    // The access token's stamp is from another save.
    let access = field_key(&r.ns, FIELD_ACCESS);
    secret_store::put(&generation_key(&access), "1").unwrap();

    for c in [r.peek(), r.load()] {
        assert_eq!(c.refresh_token, t.refresh_token);
        assert!(c.access_token.is_empty() && c.id_token.is_empty());
    }
    assert!(r.blob().is_none(), "nothing is written for a torn read");
}

#[test]
fn torn_split_entries_with_no_whole_refresh_token_read_as_signed_out() {
    let r = rig();
    GlobalSet::clear();
    r.save(&tokens("placeholder", "user-1"));
    r.remove_blob();
    r.put_split(&tokens("torn", "user-1"));
    let access = field_key(&r.ns, FIELD_ACCESS);
    secret_store::put(&generation_key(&access), "1").unwrap();
    secret_store::delete(&count_key(&field_key(&r.ns, FIELD_REFRESH))).unwrap();

    let c = r.load();
    assert!(c.access_token.is_empty() && c.refresh_token.is_empty() && c.id_token.is_empty());
}

#[test]
fn the_old_plaintext_columns_are_read_and_a_normal_load_moves_them_into_the_blob() {
    let r = rig();
    let t = tokens("plain", "user-1");
    r.save(&tokens("placeholder", "user-1"));
    r.remove_blob();
    r.set_sql_plaintext(&t);

    assert!(same(&got(&r.peek()), &t));
    assert!(r.blob().is_none());
    assert_eq!(r.sql_plaintext().0, t.access_token, "the side-effect-free read leaves the columns");

    assert!(same(&got(&r.load()), &t));
    assert!(same(&r.blob().unwrap(), &t));
    assert_eq!(r.sql_plaintext(), (String::new(), String::new(), String::new()), "the columns are blanked");
    assert!(same(&got(&r.load()), &t), "and the next load reads the blob");
}

#[test]
fn a_channel_with_no_tokens_of_its_own_adopts_the_host_wide_session_of_the_same_account_once() {
    let r = rig();
    let host = tokens("host", "user-1");
    let _global = GlobalSet::put(&host);
    r.save(&tokens("placeholder", "user-1"));
    r.remove_blob();

    assert!(same(&got(&r.peek()), &host));
    assert!(r.blob().is_none(), "the side-effect-free read copies nothing");

    assert!(same(&got(&r.load()), &host));
    assert!(same(&r.blob().unwrap(), &host), "copied into the channel's own namespace");
    assert!(same(&GlobalSet::blob().unwrap(), &host), "the host-wide set is never changed");
}

#[test]
fn the_host_wide_session_of_another_account_is_not_adopted() {
    let r = rig();
    let _global = GlobalSet::put(&tokens("other", "someone-else"));
    r.save(&tokens("placeholder", "user-1"));
    r.remove_blob();

    let c = r.load();
    assert!(c.access_token.is_empty() && c.refresh_token.is_empty() && c.id_token.is_empty());
    assert!(r.blob().is_none());
}

#[test]
fn a_row_with_tokens_nowhere_reads_as_signed_out() {
    let r = rig();
    r.save(&tokens("placeholder", "user-1"));
    r.remove_blob();
    GlobalSet::clear();

    let c = r.load();
    assert!(c.access_token.is_empty() && c.refresh_token.is_empty() && c.id_token.is_empty());
    assert_eq!(c.user_sub, "user-1", "the row's metadata is still returned");
}
