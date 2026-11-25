// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where an agent may say a login is offered, and what it leaves for the person to look at
//! (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md R4, R7 to R10).

use std::collections::BTreeMap;

use super::inspect::{update_details_for, UpdateRequest};
use super::uses::{goes_ahead, UseRequest};
use super::*;

const OWN: &str = "profile:agent-builder-0123abcd";
const REASON: &str = "point the dashboard login at its new address for task 31010";

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn asked(id: &str, fields: &[(&str, &str)]) -> UpdateRequest {
    UpdateRequest { id: id.into(), label: None, fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), tags: None, reason: Some(REASON.into()) }
}

/// A login the agent `builder` made for its own profile.
fn agents_login(s: &Store, site: &str) -> String {
    let req = GenerateRequest {
        kind: "login".into(),
        label: "Dashboard".into(),
        why: String::new(),
        reason: Some("an account for the dashboard task 31010".into()),
        fields: [("site".to_string(), site.to_string()), ("username".to_string(), "builder@example.com".to_string())].into(),
        rules: Default::default(),
        expires: None,
        vault: None,
    };
    generate_entry_in(s, req, "builder", Some(OWN)).unwrap().id
}

/// A login the person made, with a site.
fn persons_login(s: &Store) -> String {
    let fields: BTreeMap<String, String> = [("site".to_string(), "https://intranet.example.com".to_string()), ("username".to_string(), "alex".to_string())].into();
    create(s, NewEntry { kind: "login".into(), label: "Intranet".into(), fields, tags: vec![], parts: Parts([("password".to_string(), "pw-persons-own".to_string())].into()) }).unwrap()
}

fn fields(s: &Store, id: &str) -> BTreeMap<String, String> {
    signins::fields_of(&s.vault_get(id).unwrap().unwrap())
}

fn use_of(s: &Store, id: &str) -> UseRequest {
    let mut u = UseRequest { entry_id: id.into(), kind: "login".into(), offer_always: true, ..Default::default() };
    agent_set::apply(s, &mut u);
    u
}

#[test]
fn on_a_login_it_made_an_agent_sets_the_site_at_once_and_the_place_is_unseen_until_the_person_answers() {
    let s = store();
    let id = agents_login(&s, "https://old.example.com");
    assert_eq!(use_of(&s, &id).set_what, "site", "a site the agent gave at creation is unseen already");
    agent_set::clear(&s, &id);
    assert!(goes_ahead(&use_of(&s, &id)), "seen, it is used as any other");

    let done = update_details_for(&s, &asked(&id, &[("site", "https://new.example.com/")]), "builder", Some(OWN)).unwrap();
    assert_eq!((done.changed.as_slice(), done.proposed.is_empty()), (["field site".to_string()].as_slice(), true));
    assert_eq!(fields(&s, &id).get("site").map(String::as_str), Some("https://new.example.com"), "kept as an origin");
    let u = use_of(&s, &id);
    assert_eq!((u.set_by.as_str(), u.set_what.as_str(), u.offer_always, goes_ahead(&u)), ("builder", "site", false, false), "a use at that place never goes ahead on silence and offers no Always");
    let line = history::history_for(&s, &id, history::Asker::Person, 20).unwrap().into_iter().find(|r| r.action == inspect::CHANGED_DETAILS).unwrap();
    assert_eq!((line.actor.as_str(), line.target.as_str(), line.reason.as_str()), ("agent:builder", "field site", REASON));
}

#[test]
fn an_agent_sets_realm_and_proxy_on_its_own_login_and_its_own_profile_only_where_there_is_none() {
    let s = store();
    let id = agents_login(&s, "https://old.example.com");
    update_details_for(&s, &asked(&id, &[("realm", "Staging"), ("proxy", "yes")]), "builder", Some(OWN)).unwrap();
    assert_eq!((fields(&s, &id).get("realm").map(String::as_str), fields(&s, &id).get("proxy").map(String::as_str)), (Some("Staging"), Some("yes")));

    // It has its own profile already: naming it is no change, another is a share.
    assert!(update_details_for(&s, &asked(&id, &[("profile", "personal")]), "builder", Some(OWN)).unwrap_err().contains("Vault share"));
    assert!(update_details_for(&s, &asked(&id, &[("profile", "profile:somebody-else-77")]), "builder", Some(OWN)).unwrap_err().contains("Vault share"));

    // A login of its own with no site has no profile: it may be given its own, and never another.
    let bare = {
        let mut req = GenerateRequest { kind: "login".into(), label: "Bare".into(), why: String::new(), reason: Some("an account for task 31010".into()), fields: Default::default(), rules: Default::default(), expires: None, vault: None };
        req.fields.clear();
        generate_entry_in(&s, req, "builder", Some(OWN)).unwrap().id
    };
    assert!(update_details_for(&s, &asked(&bare, &[("site", "https://x.example.com"), ("profile", "personal")]), "builder", Some(OWN)).is_err());
    update_details_for(&s, &asked(&bare, &[("site", "https://x.example.com"), ("profile", OWN)]), "builder", Some(OWN)).unwrap();
    assert_eq!(fields(&s, &bare).get("profile").map(String::as_str), Some(OWN));
}

#[test]
fn a_site_is_one_exact_origin_and_never_a_path_a_query_user_information_or_a_wildcard() {
    let s = store();
    let id = agents_login(&s, "https://old.example.com");
    for bad in ["https://x.example.com/login", "https://x.example.com/?a=1", "https://user:pw@x.example.com", "https://*.example.com", "x.example.com", "ftp://x.example.com"] {
        let err = update_details_for(&s, &asked(&id, &[("site", bad)]), "builder", Some(OWN)).unwrap_err();
        assert!(err.contains("exact origin"), "{bad}: {err}");
    }
    // A development address is a real case: accepted.
    update_details_for(&s, &asked(&id, &[("site", "http://localhost:3000")]), "builder", Some(OWN)).unwrap();
    assert_eq!(fields(&s, &id).get("site").map(String::as_str), Some("http://localhost:3000"));
    // Creation takes the same rule.
    let req = GenerateRequest { kind: "login".into(), label: "Odd".into(), why: String::new(), reason: Some("an account for task 31010".into()), fields: [("site".to_string(), "https://*.example.com".to_string())].into(), rules: Default::default(), expires: None, vault: None };
    assert!(generate_entry_in(&s, req, "builder", Some(OWN)).unwrap_err().contains("exact origin"));
}

#[test]
fn on_any_other_login_the_change_is_a_proposal_and_the_login_keeps_its_place() {
    let s = store();
    let id = persons_login(&s);
    let before = fields(&s, &id);
    let done = update_details_for(&s, &asked(&id, &[("site", "https://intranet2.example.com")]), "builder", Some(OWN)).unwrap();
    assert!(done.changed.is_empty() && done.proposed == ["field site"], "{done:?}");
    assert!(done.note.as_deref().unwrap().contains("takes effect when the person accepts it"));
    assert_eq!(fields(&s, &id), before, "nothing changed: the old site keeps offering the login");
    assert!(!agent_set::is_unseen(&s, &id), "nothing was set, so nothing is unseen");
    let waiting = proposals::list(&s).unwrap();
    assert_eq!((waiting.len(), waiting[0].field.as_str(), waiting[0].value.as_str(), waiting[0].agent.as_str()), (1, "site", "https://intranet2.example.com", "builder"));
    let line = history::history_for(&s, &id, history::Asker::Person, 20).unwrap().into_iter().find(|r| r.action == proposals::PROPOSED).unwrap();
    assert_eq!((line.actor.as_str(), line.target.as_str(), line.reason.as_str()), ("agent:builder", "field site", REASON));

    // A later proposal for the same field replaces it; another field is a second one; profile is a share and asks.
    update_details_for(&s, &asked(&id, &[("site", "https://intranet3.example.com"), ("realm", "Corp")]), "builder", Some(OWN)).unwrap();
    let all = proposals::list(&s).unwrap();
    assert_eq!(all.iter().map(|p| (p.field.as_str(), p.value.as_str())).collect::<Vec<_>>(), [("realm", "Corp"), ("site", "https://intranet3.example.com")]);
    assert!(update_details_for(&s, &asked(&id, &[("profile", OWN)]), "builder", Some(OWN)).unwrap_err().contains("Vault share"));

    // A change made alongside is made, and the place is only proposed.
    let both = update_details_for(&s, &UpdateRequest { label: Some("Corporate intranet".into()), ..asked(&id, &[("proxy", "yes")]) }, "builder", Some(OWN)).unwrap();
    assert_eq!((both.changed.as_slice(), both.proposed.as_slice()), (["label".to_string()].as_slice(), ["field proxy".to_string()].as_slice()));
    assert!(fields(&s, &id).get("proxy").is_none());
}

#[test]
fn accepting_applies_a_proposal_only_if_the_login_is_as_it_was_and_dismissing_drops_it() {
    let s = store();
    let id = persons_login(&s);
    update_details_for(&s, &asked(&id, &[("site", "https://intranet2.example.com"), ("realm", "Corp")]), "builder", Some(OWN)).unwrap();
    proposals::accept(&s, &id, "site").unwrap();
    assert_eq!(fields(&s, &id).get("site").map(String::as_str), Some("https://intranet2.example.com"));
    assert!(!agent_set::is_unseen(&s, &id), "the person accepted it: they have seen it");
    assert!(proposals::list(&s).unwrap().iter().all(|p| p.field != "site"));
    let accepted = history::history_for(&s, &id, history::Asker::Person, 20).unwrap().into_iter().find(|r| r.action == proposals::ACCEPTED).unwrap();
    assert_eq!((accepted.actor.as_str(), accepted.target.as_str()), ("you", "field site"));
    assert!(proposals::accept(&s, &id, "site").is_err(), "it is gone");

    // The accept changed the login, so the other proposal is out of date.
    let err = proposals::accept(&s, &id, "realm").unwrap_err();
    assert!(err.contains("changed after this was proposed"), "{err}");
    assert!(proposals::dismiss(&s, &id, "realm").unwrap());
    assert!(!proposals::dismiss(&s, &id, "realm").unwrap(), "nothing left to dismiss");
    assert!(proposals::list(&s).unwrap().is_empty());
}

#[test]
fn a_proposal_older_than_seven_days_is_gone_and_a_deleted_login_takes_its_proposals() {
    let s = store();
    let id = persons_login(&s);
    update_details_for(&s, &asked(&id, &[("site", "https://intranet2.example.com")]), "builder", Some(OWN)).unwrap();
    let mut p = s.vault_proposals(Some(&id), 0).unwrap().remove(0);
    p.at -= proposals::KEEP_MS + 1;
    s.vault_proposal_put(&p).unwrap();
    assert!(proposals::list(&s).unwrap().is_empty());
    assert!(proposals::accept(&s, &id, "site").is_err());
    update_details_for(&s, &asked(&id, &[("realm", "Corp")]), "builder", Some(OWN)).unwrap();
    s.vault_delete(&id).unwrap();
    assert!(s.vault_proposals(None, 0).unwrap().is_empty());
}

#[test]
fn shared_stays_out_of_update_and_a_site_is_for_a_login_only() {
    let s = store();
    let id = agents_login(&s, "https://old.example.com");
    let err = update_details_for(&s, &asked(&id, &[("shared", "*")]), "builder", Some(OWN)).unwrap_err();
    assert!(err.contains("Vault share"), "{err}");
    let key = create(&s, NewEntry { kind: "api-key".into(), label: "Key".into(), fields: Default::default(), tags: vec![], parts: Parts([("value".to_string(), "k-1".to_string())].into()) }).unwrap();
    assert!(update_details_for(&s, &asked(&key, &[("site", "https://x.example.com")]), "builder", Some(OWN)).unwrap_err().contains("no field"));
}

#[test]
fn a_changed_label_keeps_the_previous_details_for_seven_days_in_the_same_change() {
    let s = store();
    let id = persons_login(&s);
    let row = s.vault_get(&id).unwrap().unwrap();
    update_details_for(&s, &UpdateRequest { label: Some("Corporate intranet".into()), tags: Some(vec!["work".into()]), ..asked(&id, &[("username", "alex2")]) }, "builder", Some(OWN)).unwrap();
    let kept = s.vault_details_kept(&id, row.created_at).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!((kept[0].label.as_str(), kept[0].changed_by.as_str(), kept[0].what.as_str()), ("Intranet", "agent:builder", "label, field username, tags"));
    assert_eq!(serde_json::from_str::<BTreeMap<String, String>>(&kept[0].fields).unwrap().get("username").map(String::as_str), Some("alex"), "the fields as they were");
    assert!(!kept[0].fields.contains("pw-persons-own"), "details are plain fields: never a value");
    assert_eq!(s.vault_details_prune(kept[0].changed_at + 1).unwrap(), 1, "past 7 days it goes");
    assert!(s.vault_details_kept(&id, row.created_at).unwrap().is_empty());
}
