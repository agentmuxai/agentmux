// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::vault::vaults::{self, VaultAgents};
use crate::backend::vault::{create_in, seal, signins, NewEntry, Parts};

const SITE_URL: &str = "https://intranet.example.com";
/// Agent "pulp"'s own browser profile and its permanent uid.
const PULP_PROFILE: &str = "profile:agent-pulp-0123abcd";
const PULP: &str = "uid-pulp-0001";
const OTHER: &str = "uid-other-0002";

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn login_in(s: &Store, vault: &str, extra: &[(&str, &str)]) -> String {
    let mut fields: BTreeMap<String, String> = [("username", "alex"), ("site", SITE_URL)].map(|(k, v)| (k.to_string(), v.to_string())).into();
    fields.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let entry = NewEntry { kind: "login".into(), label: "Intranet".into(), fields, tags: vec![], parts: Parts([("password".to_string(), "pw".to_string())].into()) };
    create_in(s, entry, vault).unwrap()
}

fn login(s: &Store) -> String {
    login_in(s, "personal", &[])
}

fn fields(s: &Store, id: &str) -> BTreeMap<String, String> {
    fields_of(&s.vault_get(id).unwrap().unwrap())
}

fn offered(s: &Store, agent: &str, own: Option<&str>) -> Vec<String> {
    signins::offered_for_agent(s, PULP_PROFILE, &[SITE_URL.to_string()], agent, own).unwrap().into_iter().map(|f| f.0).collect()
}

fn always_key(uid: &str) -> String {
    format!("{}{PULP_PROFILE}|{SITE_URL}", grant_prefix(uid))
}

fn grant(s: &Store, id: &str, key: &str) {
    s.vault_grant_add(id, key, 1, s.vault_get(id).unwrap().unwrap().created_at).unwrap();
}

#[test]
fn a1_a_share_leaves_profile_site_and_vault_as_they_were() {
    let s = store();
    let id = login(&s);
    let before = s.vault_get(&id).unwrap().unwrap();
    assert!(add(&s, &id, PULP).unwrap());
    assert!(!add(&s, &id, PULP).unwrap(), "sharing twice changes nothing");
    let after = s.vault_get(&id).unwrap().unwrap();
    let (was, now) = (fields_of(&before), fields_of(&after));
    for kept in ["site", "profile", "username"] {
        assert_eq!(was.get(kept), now.get(kept), "{kept} is as it was");
    }
    assert_eq!(after.vault_id, before.vault_id, "the Vault is as it was");
    assert_eq!(after.sealed, before.sealed, "the secret is not touched");
    assert_eq!(listed(&now), [PULP]);
}

#[test]
fn a1_only_a_login_saved_for_a_site_can_be_shared_and_only_by_a_real_uid() {
    let s = store();
    let id = login(&s);
    for bad in ["", "a,b", "two words", "../x", &"u".repeat(65)] {
        assert!(add(&s, &id, bad).is_err(), "{bad:?} names no agent");
    }
    let key = create_in(&s, NewEntry { kind: "api-key".into(), label: "Key".into(), fields: BTreeMap::new(), tags: vec![], parts: Parts([("value".to_string(), "v".to_string())].into()) }, "personal").unwrap();
    assert!(add(&s, &key, PULP).unwrap_err().contains("only a login"));
    let http = login_in(&s, "personal", &[("realm", "Admin")]);
    assert!(add(&s, &http, PULP).unwrap_err().contains("HTTP prompt"));
}

#[test]
fn a3_every_agent_names_each_agent_and_a_named_one_only_itself() {
    let s = store();
    let id = login(&s);
    add(&s, &id, PULP).unwrap();
    assert!(names(&fields(&s, &id), PULP));
    assert!(!names(&fields(&s, &id), OTHER));
    add(&s, &id, EVERY_AGENT).unwrap();
    assert!(names(&fields(&s, &id), OTHER));
    assert!(!names(&fields(&s, &id), ""), "an empty uid is no agent");
}

#[test]
fn a4_a_shared_login_is_offered_in_the_agents_own_profile_tab_only() {
    let s = store();
    let id = login(&s);
    assert!(offered(&s, "pulp", Some(PULP)).is_empty(), "an unshared Personal login is not offered, even in its own profile");
    add(&s, &id, PULP).unwrap();
    assert_eq!(offered(&s, "pulp", Some(PULP)), [id.clone()]);
    assert!(offered(&s, "pulp", None).is_empty(), "no uid is given for another profile's tab or an Incognito one");
    assert!(offered(&s, "other", Some(OTHER)).is_empty(), "another agent's own profile is not offered what is shared with pulp");
    // The key button the person presses is as before: Personal's tabs, their own logins.
    assert_eq!(signins::offered_for(&s, "personal", &[SITE_URL.to_string()]).unwrap().len(), 1);
    assert!(signins::offered_for(&s, PULP_PROFILE, &[SITE_URL.to_string()]).unwrap().is_empty(), "a share adds nothing to what a person is offered");
}

#[test]
fn a4_a_share_is_for_the_page_it_is_saved_for_and_not_a_refused_or_unconfirmed_login() {
    let s = store();
    let id = login(&s);
    add(&s, &id, PULP).unwrap();
    assert!(signins::offered_for_agent(&s, PULP_PROFILE, &["https://elsewhere.example.net".to_string()], "pulp", Some(PULP)).unwrap().is_empty());
    let row = s.vault_get(&id).unwrap().unwrap();
    let refused = VaultRow { tags: serde_json::to_string(&[signins::REFUSED]).unwrap(), ..row.clone() };
    assert!(s.vault_update_kept(&refused, &row.sealed, None).unwrap());
    assert!(offered(&s, "pulp", Some(PULP)).is_empty(), "the site refused it");
}

#[test]
fn a5_a_login_in_a_vault_the_agent_may_not_use_is_not_offered_shared_or_not() {
    let s = store();
    let closed = vaults::create(&s, "Work", VaultAgents::Named(vec!["builder".into()])).unwrap();
    let id = login_in(&s, &closed.id, &[]);
    add(&s, &id, PULP).unwrap();
    assert!(offered(&s, "pulp", Some(PULP)).is_empty(), "the Vault is closed to pulp");
    assert_eq!(offered(&s, "builder", Some(PULP)), [id], "open to the agent it is named for");
}

#[test]
fn a7_removing_a_share_ends_the_always_under_it_and_only_that_agents() {
    let s = store();
    let id = login(&s);
    add(&s, &id, PULP).unwrap();
    add(&s, &id, OTHER).unwrap();
    grant(&s, &id, &always_key(PULP));
    grant(&s, &id, &always_key(OTHER));
    assert!(remove(&s, &id, PULP).unwrap());
    assert!(!s.vault_grant_check(&id, &always_key(PULP)).unwrap(), "pulp's Always ended with its share");
    assert!(s.vault_grant_check(&id, &always_key(OTHER)).unwrap(), "another agent's stays");
    assert_eq!(listed(&fields(&s, &id)), [OTHER]);
    assert!(!remove(&s, &id, PULP).unwrap(), "removing twice is no error and changes nothing");
}

#[test]
fn a7_removing_every_agent_ends_the_always_of_those_not_named() {
    let s = store();
    let id = login(&s);
    add(&s, &id, EVERY_AGENT).unwrap();
    add(&s, &id, PULP).unwrap();
    grant(&s, &id, &always_key(PULP));
    grant(&s, &id, &always_key(OTHER));
    remove(&s, &id, EVERY_AGENT).unwrap();
    assert!(s.vault_grant_check(&id, &always_key(PULP)).unwrap(), "pulp is still named");
    assert!(!s.vault_grant_check(&id, &always_key(OTHER)).unwrap());
}

#[test]
fn a9_a_deleted_agent_loses_its_shares_and_always_and_the_entry_stays() {
    let s = store();
    let (one, two) = (login(&s), login_in(&s, "personal", &[("site", "https://other.example.com")]));
    add(&s, &one, PULP).unwrap();
    add(&s, &one, OTHER).unwrap();
    add(&s, &two, PULP).unwrap();
    grant(&s, &one, &always_key(PULP));
    grant(&s, &one, &always_key(OTHER));
    assert_eq!(forget_agent(&s, PULP).unwrap(), 2);
    assert_eq!(listed(&fields(&s, &one)), [OTHER], "the other agent's share stays");
    assert!(!fields(&s, &two).contains_key(SHARED), "the last name goes and so does the field");
    assert!(!s.vault_grant_check(&one, &always_key(PULP)).unwrap());
    assert!(s.vault_grant_check(&one, &always_key(OTHER)).unwrap());
    assert!(s.vault_get(&one).unwrap().is_some() && s.vault_get(&two).unwrap().is_some(), "the entries are not deleted");
}

#[test]
fn a9_deleting_the_entry_takes_its_shares_with_it() {
    let s = store();
    let id = login(&s);
    add(&s, &id, PULP).unwrap();
    crate::backend::vault::delete(&s, &id).unwrap();
    assert!(s.vault_get(&id).unwrap().is_none());
    assert!(signins::offered_for_agent(&s, PULP_PROFILE, &[SITE_URL.to_string()], "pulp", Some(PULP)).unwrap().is_empty());
}

#[test]
fn a10_a_share_names_the_uid_so_a_new_agent_with_the_old_name_gets_none() {
    let s = store();
    let id = login(&s);
    add(&s, &id, PULP).unwrap();
    // pulp renamed: the uid is the same, the share holds.
    assert_eq!(offered(&s, "pulp-renamed", Some(PULP)), [id.clone()]);
    // pulp deleted and another agent made with the name "pulp": a different uid.
    forget_agent(&s, PULP).unwrap();
    assert!(offered(&s, "pulp", Some("uid-pulp-9999")).is_empty());
}

#[test]
fn a_share_is_shown_to_the_agent_as_shared_with_you_and_never_with_whom() {
    let s = store();
    let id = login(&s);
    add(&s, &id, PULP).unwrap();
    add(&s, &id, OTHER).unwrap();
    let mine = crate::backend::vault::list_for_agent_as(&s, Some("pulp"), Some(PULP)).unwrap();
    assert!(mine[0].shared_with_you);
    assert!(!crate::backend::vault::list_for_agent_as(&s, Some("nobody"), Some("uid-nobody")).unwrap()[0].shared_with_you);
    assert!(!crate::backend::vault::list_for_agent(&s, Some("pulp")).unwrap()[0].shared_with_you, "without a uid nothing is said");
    let key = seal::key(false);
    let shown = crate::backend::vault::inspect::show_as(&s, &id, (Some("pulp"), Some(PULP)), &key, chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap()).unwrap();
    assert!(shown.shared_with_you);
    let said = format!("{shown:?}{}", serde_json::to_string(&mine).unwrap());
    assert!(!said.contains(OTHER) && !said.contains(PULP), "who it is shared with is never in what an agent reads: {said}");
}
