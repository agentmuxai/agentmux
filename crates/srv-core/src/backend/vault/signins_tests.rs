// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;

use super::super::{create, list_for_pane, seal, versions, NewEntry, Parts};
use super::*;
use crate::identity::browser_credential_store::BrowserCredential;

const WORK: &str = "profile:p-0123456789ab";

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn space(profile: &str, realm: &str) -> Space {
    Space { profile: profile.into(), origin: "https://intranet.example.com".into(), realm: realm.into(), proxy: false }
}

fn password(s: &Store, sp: &Space) -> String {
    fill(s, sp).unwrap().map(|(_, p)| p.to_string()).unwrap_or_default()
}

fn fields(s: &Store, id: &str) -> BTreeMap<String, String> {
    fields_of(&s.vault_get(id).unwrap().unwrap())
}

#[test]
fn a_tab_keeps_sign_ins_under_its_profile_and_incognito_keeps_none() {
    assert_eq!(profile_for_tab(None).as_deref(), Some(PERSONAL));
    assert_eq!(profile_for_tab(Some(WORK)).as_deref(), Some(WORK));
    assert_eq!(profile_for_tab(Some("incognito:aaaaaaaa-1111")), None);
    assert_eq!(profile_for_tab(Some("profile:Not An Id")), None);
}

#[test]
fn a_saved_http_sign_in_is_a_login_offered_only_in_its_profile_realm_and_kind() {
    let s = store();
    let id = save(&s, &space(PERSONAL, "Admin"), "alex", "correct horse").unwrap();
    let f = fields(&s, &id);
    assert_eq!((f["site"].as_str(), f["realm"].as_str(), f["profile"].as_str(), f["username"].as_str()), ("https://intranet.example.com", "Admin", PERSONAL, "alex"));
    assert_eq!(s.vault_get(&id).unwrap().unwrap().label, "intranet.example.com (Admin)");
    assert_eq!(find(&s, &space(PERSONAL, "Admin")).unwrap().map(|(_, u)| u).as_deref(), Some("alex"));
    assert!(find(&s, &space(WORK, "Admin")).unwrap().is_none(), "another profile's tab isn't offered it");
    assert!(find(&s, &space(PERSONAL, "Other")).unwrap().is_none(), "nor another realm");
    assert!(find(&s, &Space { proxy: true, ..space(PERSONAL, "Admin") }).unwrap().is_none(), "nor the proxy's prompt");
}

#[test]
fn saving_again_replaces_the_password_and_keeps_the_old_one_as_a_version() {
    let s = store();
    let sp = space(PERSONAL, "");
    let id = save(&s, &sp, "alex", "correct horse").unwrap();
    assert_eq!(save(&s, &sp, "alex", "correct horse").unwrap(), id);
    assert!(versions::list(&s, &id).unwrap().is_empty(), "the same password again changes nothing");
    assert_eq!(save(&s, &sp, "alex", "battery staple").unwrap(), id);
    assert_eq!(password(&s, &sp), "battery staple");
    assert_eq!(versions::list(&s, &id).unwrap().len(), 1);
}

#[test]
fn another_user_gets_an_entry_of_its_own_and_the_newest_is_offered() {
    let s = store();
    let sp = space(PERSONAL, "");
    let a = save(&s, &sp, "alex", "correct horse").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let b = save(&s, &sp, "sam", "battery staple").unwrap();
    assert_ne!(a, b);
    assert_eq!(fill(&s, &sp).unwrap().map(|(u, _)| u).as_deref(), Some("sam"));
}

#[test]
fn a_refused_sign_in_is_kept_but_not_offered_until_a_new_password_is_saved() {
    let s = store();
    let sp = space(PERSONAL, "");
    let id = save(&s, &sp, "alex", "correct horse").unwrap();
    refused(&s, &sp).unwrap();
    assert!(s.vault_get(&id).unwrap().is_some(), "it may be the person's only copy");
    assert!(find(&s, &sp).unwrap().is_none());
    assert_eq!(save(&s, &sp, "alex", "battery staple").unwrap(), id);
    assert_eq!(password(&s, &sp), "battery staple");
    assert!(!s.vault_get(&id).unwrap().unwrap().tags.contains(REFUSED));
}

#[test]
fn a_sign_in_is_logged_and_noted_as_a_use_without_the_password() {
    let s = store();
    let sp = space(PERSONAL, "");
    let id = save(&s, &sp, "alex", "correct horse").unwrap();
    assert_eq!(password(&s, &sp), "correct horse");
    let log = super::super::activity(&s, 10).unwrap();
    assert_eq!((log[0].action.as_str(), log[0].target.as_str()), (SIGNED_IN, "https://intranet.example.com"));
    assert!(!serde_json::to_string(&log).unwrap().contains("correct horse"));
    assert!(s.vault_get(&id).unwrap().unwrap().last_used_at > 0);
}

fn login(site: Option<&str>, profile: Option<&str>) -> NewEntry {
    let mut fields = BTreeMap::from([("username".to_string(), "alex".to_string())]);
    if let Some(site) = site {
        fields.insert("site".into(), site.into());
    }
    if let Some(p) = profile {
        fields.insert("profile".into(), p.into());
    }
    NewEntry { kind: "login".into(), label: "Intranet".into(), fields, tags: vec![], parts: Parts([("password".to_string(), "correct horse".to_string())].into()) }
}

#[test]
fn a_login_with_a_site_belongs_to_personal_unless_it_names_a_profile_and_one_without_a_site_to_none() {
    let s = store();
    let a = create(&s, login(Some("https://intranet.example.com"), None)).unwrap();
    assert_eq!(fields(&s, &a)["profile"], PERSONAL);
    let b = create(&s, login(Some("https://intranet.example.com"), Some(WORK))).unwrap();
    assert_eq!(fields(&s, &b)["profile"], WORK);
    let c = create(&s, login(None, Some(WORK))).unwrap();
    assert!(!fields(&s, &c).contains_key("profile"));
    assert!(create(&s, login(Some("https://intranet.example.com"), Some("incognito:aaaaaaaa-1111"))).unwrap_err().contains("isn't a browser profile"));
    let mut proxy = login(Some("https://intranet.example.com"), None);
    proxy.fields.insert("proxy".into(), "no".into());
    assert!(create(&s, proxy).is_err());
}

#[test]
fn a_deleted_profiles_sign_ins_move_to_personal_tagged_or_go_with_it() {
    let s = store();
    let moved = save(&s, &space(WORK, ""), "alex", "correct horse").unwrap();
    save(&s, &space(PERSONAL, ""), "sam", "battery staple").unwrap();
    assert_eq!(counts(&s).unwrap(), BTreeMap::from([("p-0123456789ab".to_string(), 1)]));

    assert_eq!(profile_deleted(&s, "p-0123456789ab", "Work", true).unwrap(), 1);
    assert_eq!(fields(&s, &moved)["profile"], PERSONAL);
    assert!(s.vault_get(&moved).unwrap().unwrap().tags.contains("from deleted profile Work"));
    assert!(counts(&s).unwrap().is_empty());
    assert_eq!(super::super::activity(&s, 1).unwrap()[0].action, "moved to Personal");

    let gone = save(&s, &space("profile:p-ffff", ""), "alex", "correct horse").unwrap();
    assert_eq!(profile_deleted(&s, "p-ffff", "Old", false).unwrap(), 1);
    assert!(s.vault_get(&gone).unwrap().is_none());
    assert_eq!(list_for_pane(&s).unwrap().0.len(), 2, "the others stay");
}

static DELETED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn old_load(identity: &str, origin: &str, _realm: &str, _proxy: bool) -> Result<Option<BrowserCredential>, String> {
    match origin {
        "https://moves.example.com" => Ok(Some(BrowserCredential { username: format!("{identity}-user"), password: "correct horse".into() })),
        "https://locked.example.com" => Err("the keychain is locked".into()),
        _ => Ok(None),
    }
}

fn old_delete(identity: &str, origin: &str, _realm: &str, _proxy: bool) -> Result<(), String> {
    DELETED.lock().unwrap().push(format!("{identity} {origin}"));
    Ok(())
}

#[test]
fn the_keychain_sign_ins_move_in_once_under_personal_tagged_to_check() {
    let s = store();
    for origin in ["https://moves.example.com", "https://locked.example.com", "https://gone.example.com"] {
        s.browser_signin_note("agent-account", origin, "Admin", false, "ag***", 10).unwrap();
    }
    assert_eq!(move_from(&s, old_load, old_delete), 1);
    let sp = Space { profile: PERSONAL.into(), origin: "https://moves.example.com".into(), realm: "Admin".into(), proxy: false };
    let (row, user) = find(&s, &sp).unwrap().unwrap();
    assert_eq!(user, "agent-account-user");
    assert!(row.tags.contains(FROM_KEYCHAIN));
    let deleted: Vec<String> = DELETED.lock().unwrap().iter().filter(|d| d.starts_with("agent-account ")).cloned().collect();
    assert_eq!(deleted, ["agent-account https://moves.example.com"], "only the one that moved leaves the keychain");
    let left: Vec<String> = s.browser_signins_saved().unwrap().into_iter().map(|r| r.1).collect();
    assert_eq!(left, ["https://locked.example.com"], "one the keychain couldn't read waits for the next start");
    assert_eq!(move_from(&s, old_load, old_delete), 0, "nothing moves twice");
}

#[test]
fn a_keychain_copy_never_replaces_what_the_vault_holds_for_that_user() {
    let s = store();
    let sp = Space { profile: PERSONAL.into(), origin: "https://moves.example.com".into(), realm: "Admin".into(), proxy: false };
    let id = save(&s, &sp, "later-user", "battery staple").unwrap();
    s.browser_signin_note("later", "https://moves.example.com", "Admin", false, "la***", 10).unwrap();
    assert_eq!(move_from(&s, old_load, old_delete), 1);
    assert!(DELETED.lock().unwrap().iter().any(|d| d == "later https://moves.example.com"), "its keychain copy still goes");
    assert_eq!(password(&s, &sp), "battery staple");
    assert!(versions::list(&s, &id).unwrap().is_empty());
    assert!(s.browser_signins_saved().unwrap().is_empty());
}
