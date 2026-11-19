// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A shared sign-in for a site's HTTP prompt (spec A11): where it answers, what it never does, and its "Always".

use super::*;
use crate::backend::browser_profiles_store::agent_profile_id;
use crate::backend::storage::vault::VaultRow;
use crate::backend::vault::signins::{ProfileOwner, Space};
use crate::backend::vault::vaults::{self, VaultAgents};
use crate::backend::vault::{create_in, history, seal, NewEntry, Parts};

const SITE_URL: &str = "https://intranet.example.com";
const PULP: &str = "uid-pulp-0001";
const OTHER: &str = "uid-other-0002";
const REASON: &str = "read the build status behind the sign-in prompt";

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

/// A sign-in for the prompt of realm "Admin" on the site, saved for Personal.
fn prompt_login(s: &Store) -> String {
    login_in(s, "personal", &[("realm", "Admin")])
}

fn own(uid: &str) -> String {
    format!("profile:{}", agent_profile_id(uid))
}

fn owner(name: &str, uid: &str) -> ProfileOwner {
    ProfileOwner { names: vec![name.to_string(), uid.to_string()], uid: uid.to_string() }
}

fn space(profile: &str, realm: &str) -> Space {
    Space { profile: profile.to_string(), origin: SITE_URL.to_string(), realm: realm.to_string(), proxy: false }
}

fn found(s: &Store, space: &Space, who: &ProfileOwner) -> Option<String> {
    signins::find_shared(s, space, who).unwrap().map(|(row, _)| row.id)
}

fn set_field(s: &Store, id: &str, key: &str, value: Option<&str>) {
    let row = s.vault_get(id).unwrap().unwrap();
    let mut f = fields_of(&row);
    match value {
        Some(v) => f.insert(key.to_string(), v.to_string()),
        None => f.remove(key),
    };
    let changed = VaultRow { fields: serde_json::to_string(&f).unwrap(), ..row.clone() };
    assert!(s.vault_update_kept(&changed, &row.sealed, &row.fields, None).unwrap());
}

fn ask_of(s: &Store, id: &str) -> Ask {
    match plan_ask(s, id, "pulp", PULP, Some(REASON)).unwrap() {
        Asked::Ask(ask) => ask,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a11_a_shared_prompt_sign_in_answers_in_the_agents_own_profile_and_nowhere_else() {
    let s = store();
    let id = prompt_login(&s);
    let pulp = owner("pulp", PULP);
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &pulp), None, "not shared: the agent's profile has nothing to answer with");
    assert!(add(&s, &id, PULP).unwrap());
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &pulp), Some(id.clone()));
    assert_eq!(signins::find_shared(&s, &space(&own(PULP), "Admin"), &pulp).unwrap().unwrap().1, "alex");
    // Only that prompt: another realm, another site and a proxy's prompt get nothing.
    assert_eq!(found(&s, &space(&own(PULP), "Staff"), &pulp), None);
    assert_eq!(found(&s, &space(&own(PULP), ""), &pulp), None);
    assert_eq!(found(&s, &Space { origin: "https://elsewhere.example.net".into(), ..space(&own(PULP), "Admin") }, &pulp), None);
    assert_eq!(found(&s, &Space { proxy: true, ..space(&own(PULP), "Admin") }, &pulp), None);
    // Only the agent's own profile: not Personal, not a named profile, not another agent's.
    for profile in ["personal", "profile:work", own(OTHER).as_str()] {
        assert_eq!(found(&s, &space(profile, "Admin"), &pulp), None, "{profile}");
    }
    assert_eq!(found(&s, &space(&own(OTHER), "Admin"), &owner("other", OTHER)), None, "another agent is not given what is shared with pulp");
    // Shared with every agent, each one's own profile answers.
    add(&s, &id, EVERY_AGENT).unwrap();
    assert_eq!(found(&s, &space(&own(OTHER), "Admin"), &owner("other", OTHER)), Some(id.clone()));
    // The share changed nothing else: the entry is still Personal's, and the person's own tabs find it as before.
    assert_eq!(fields_of(&s.vault_get(&id).unwrap().unwrap()).get("profile").map(String::as_str), Some("personal"));
    assert!(signins::find(&s, &space("personal", "Admin")).unwrap().is_some());
}

#[test]
fn a11_a_form_login_never_answers_a_prompt_and_a_prompt_sign_in_is_never_offered_for_a_form() {
    let s = store();
    let form = login_in(&s, "personal", &[]);
    add(&s, &form, PULP).unwrap();
    let pulp = owner("pulp", PULP);
    // A prompt that names no realm reads like a login with no realm: a shared form login still never answers it.
    assert_eq!(found(&s, &space(&own(PULP), ""), &pulp), None, "a share of a form login does nothing new");
    let prompt = login_in(&s, "personal", &[("realm", "Admin"), ("username", "root")]);
    add(&s, &prompt, PULP).unwrap();
    let offered: Vec<String> = signins::offered_for_agent(&s, &own(PULP), &[SITE_URL.to_string()], "pulp", Some(PULP)).unwrap().into_iter().map(|f| f.0).collect();
    assert_eq!(offered, [form], "a form is offered the form login only");
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &pulp), Some(prompt));
}

#[test]
fn a11_a_closed_vault_a_refused_sign_in_and_a_removed_share_answer_nothing() {
    let s = store();
    let closed = vaults::create(&s, "Work", VaultAgents::Named(vec!["builder".into()])).unwrap();
    let id = login_in(&s, &closed.id, &[("realm", "Admin")]);
    add(&s, &id, PULP).unwrap();
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &owner("pulp", PULP)), None, "the Vault is closed to pulp (A5)");
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &owner("builder", PULP)), Some(id.clone()), "open to the agent it names");

    let open = prompt_login(&s);
    add(&s, &open, PULP).unwrap();
    let pulp = owner("pulp", PULP);
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &pulp), Some(open.clone()));
    // The site refused it: it is kept and marked, and answers no prompt until a new password is saved.
    signins::refused_shared(&s, &space(&own(PULP), "Admin"), &pulp).unwrap();
    assert!(signins::tags_of(&s.vault_get(&open).unwrap().unwrap()).contains(&signins::REFUSED.to_string()));
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &pulp), None);
    assert!(names(&fields_of(&s.vault_get(&open).unwrap().unwrap()), PULP), "marking it left the share as it was");

    let again = login_in(&s, "personal", &[("realm", "Admin"), ("username", "second")]);
    add(&s, &again, PULP).unwrap();
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &pulp), Some(again.clone()));
    assert!(remove(&s, &again, PULP).unwrap());
    assert_eq!(found(&s, &space(&own(PULP), "Admin"), &pulp), None, "removed before the next use (A7)");
}

#[test]
fn a11_the_card_shows_the_realm_and_a_login_that_changed_meanwhile_is_not_shared() {
    let s = store();
    let id = prompt_login(&s);
    let ask = ask_of(&s, &id);
    assert_eq!((ask.prompt, ask.realm.as_str(), ask.proxy, ask.site.as_str()), (true, "Admin", false, SITE_URL));
    assert!(!ask.user.contains("alex"), "the user name is masked: {}", ask.user);
    let form = login_in(&s, "personal", &[("username", "form-user")]);
    assert!(!ask_of(&s, &form).prompt);
    // The realm is edited while the person is asked: their yes was for another prompt.
    set_field(&s, &id, "realm", Some("Staff"));
    assert!(grant_asked(&s, &ask, PULP).unwrap_err().contains("changed while the person was asked"));
    assert!(!names(&fields_of(&s.vault_get(&id).unwrap().unwrap()), PULP));
    // A proxy's sign-in says so on the card.
    let proxy = login_in(&s, "personal", &[("proxy", "yes"), ("username", "px")]);
    assert_eq!((ask_of(&s, &proxy).prompt, ask_of(&s, &proxy).proxy), (true, true));
}

#[test]
fn a11_always_is_for_the_prompt_shown_and_ends_with_the_share_or_the_agent() {
    let s = store();
    let id = prompt_login(&s);
    let profile = own(PULP);
    let row = |s: &Store| s.vault_get(&id).unwrap().unwrap();
    // A plain yes shares it and gives no Always.
    grant_asked(&s, &ask_of(&s, &id), PULP).unwrap();
    assert!(names(&fields_of(&row(&s)), PULP) && !prompt_always(&s, &row(&s), PULP, &profile));
    remove(&s, &id, PULP).unwrap();
    // "Always" shares it and keeps the answer for this agent, profile, site and realm.
    grant_asked_always(&s, &ask_of(&s, &id), PULP, &profile).unwrap();
    assert!(prompt_always(&s, &row(&s), PULP, &profile));
    assert!(!prompt_always(&s, &row(&s), OTHER, &own(OTHER)), "another agent holds none");
    assert!(!prompt_always(&s, &row(&s), PULP, "profile:work"), "nor does another profile");
    // The realm is edited: the answer was for another prompt, so the next use asks again.
    set_field(&s, &id, "realm", Some("Staff"));
    assert!(!prompt_always(&s, &row(&s), PULP, &profile));
    set_field(&s, &id, "realm", Some("Admin"));
    // Removing the share ends it, and a share made again starts by asking.
    assert!(remove(&s, &id, PULP).unwrap());
    assert!(s.vault_grants().unwrap().get(&id).is_none_or(Vec::is_empty), "no Always is left on the entry");
    add(&s, &id, PULP).unwrap();
    assert!(!prompt_always(&s, &row(&s), PULP, &profile));
    // A deleted agent loses it with its share.
    remove(&s, &id, PULP).unwrap();
    grant_asked_always(&s, &ask_of(&s, &id), PULP, &profile).unwrap();
    assert_eq!(forget_agent(&s, PULP).unwrap(), 1);
    assert!(s.vault_grants().unwrap().get(&id).is_none_or(Vec::is_empty));
    // A form login has no Always with its share: its first sign-in asks for one.
    let form = login_in(&s, "personal", &[("username", "form-user")]);
    assert!(grant_asked_always(&s, &ask_of(&s, &form), PULP, &profile).unwrap_err().contains("HTTP prompt"));
    assert!(!names(&fields_of(&s.vault_get(&form).unwrap().unwrap()), PULP), "and nothing was shared by the refusal");
}

#[test]
fn a11_a_use_is_the_agents_line_in_the_history_and_holds_no_value() {
    let s = store();
    let id = prompt_login(&s);
    add(&s, &id, PULP).unwrap();
    let pulp = owner("pulp", PULP);
    let (row, user, password) = signins::fill_shared(&s, &space(&own(PULP), "Admin"), &pulp, "pulp").unwrap().unwrap();
    assert_eq!((row.id.as_str(), user.as_str(), password.as_str()), (id.as_str(), "alex", "pw"));
    let lines = history::history_for(&s, &id, history::Asker::Person, 50).unwrap();
    let used = lines.iter().find(|l| l.action == signins::SIGNED_IN).expect("the use is in the history");
    assert_eq!((used.actor.as_str(), used.target.as_str(), used.outcome.as_str()), ("pulp", SITE_URL, "done"));
    assert!(!lines.iter().any(|l| [&l.actor, &l.action, &l.target, &l.outcome, &l.reason].iter().any(|v| v.as_str() == "pw")), "no line holds the password");
    assert!(signins::fill_shared(&s, &space(&own(OTHER), "Admin"), &owner("other", OTHER), "other").unwrap().is_none(), "another agent's profile is given nothing");
}
