// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

#[test]
fn a_grant_is_per_agent_account_and_service_and_off_by_default() {
    let (s, _d) = store();
    assert_eq!(s.google_grant_get("a", "acct", "calendar").unwrap(), None);
    s.google_grant_set("a", "acct", "calendar", Some(GrantLevel::Read), 1).unwrap();
    assert_eq!(s.google_grant_get("a", "acct", "calendar").unwrap(), Some(GrantLevel::Read));
    assert_eq!(s.google_grant_get("a", "acct", "gmail").unwrap(), None, "per service");
    assert_eq!(s.google_grant_get("b", "acct", "calendar").unwrap(), None, "per agent");
    s.google_grant_set("a", "acct", "calendar", Some(GrantLevel::Write), 2).unwrap();
    assert_eq!(s.google_grant_get("a", "acct", "calendar").unwrap(), Some(GrantLevel::Write));
    assert!(GrantLevel::Write > GrantLevel::Read, "write includes read");
    s.google_grant_set("a", "acct", "calendar", None, 3).unwrap();
    assert_eq!(s.google_grant_get("a", "acct", "calendar").unwrap(), None, "off removes it");
    assert!(s.google_grant_set("a", "acct", "photos", Some(GrantLevel::Read), 4).is_err());
}

#[test]
fn grants_list_by_account_and_by_agent_and_go_with_the_account() {
    let (s, _d) = store();
    s.google_grant_set("a", "acct", "calendar", Some(GrantLevel::Read), 1).unwrap();
    s.google_grant_set("b", "acct", "drive", Some(GrantLevel::Write), 1).unwrap();
    s.google_grant_set("a", "other", "gmail", Some(GrantLevel::Read), 1).unwrap();
    assert_eq!(s.google_grants_for_account("acct").unwrap().len(), 2);
    assert_eq!(s.google_grants_for_agent("a").unwrap().len(), 2);
    s.google_call_log("acct", &GoogleCall { at: 5, agent_id: "a".into(), service: "calendar".into(), action: "list_events".into(), outcome: "ok".into(), ..Default::default() })
        .unwrap();
    s.google_account_forget("acct").unwrap();
    assert!(s.google_grants_for_account("acct").unwrap().is_empty());
    assert!(s.google_calls("acct", 10).unwrap().is_empty());
    assert_eq!(s.google_grants_for_agent("a").unwrap().len(), 1, "another account's grant stays");
}

#[test]
fn the_call_log_is_newest_first_and_keeps_thirty_days() {
    let (s, _d) = store();
    let call = |at: i64, action: &str| GoogleCall { at, agent_id: "a".into(), service: "calendar".into(), action: action.into(), outcome: "ok".into(), ..Default::default() };
    s.google_call_log("acct", &call(1, "old")).unwrap();
    s.google_call_log("acct", &call(1000, "list_events")).unwrap();
    s.google_call_log("acct", &call(2000, "free_busy")).unwrap();
    let actions: Vec<String> = s.google_calls("acct", 10).unwrap().into_iter().map(|c| c.action).collect();
    assert_eq!(actions, ["free_busy", "list_events", "old"]);
    s.google_call_log("acct", &call(1 + CALLS_RETENTION_MS + 1, "new")).unwrap();
    let actions: Vec<String> = s.google_calls("acct", 10).unwrap().into_iter().map(|c| c.action).collect();
    assert!(!actions.contains(&"old".to_string()), "{actions:?}");
}
