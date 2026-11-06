// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::vault::seal::{self, Parts};
use crate::backend::vault::{command_target, create_in, fill_target, list_for_agent, list_for_pane, NewEntry};

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn key(label: &str) -> NewEntry {
    NewEntry {
        kind: "api-key".into(),
        label: label.into(),
        fields: Default::default(),
        tags: vec![],
        parts: Parts([("value".to_string(), "sk-test-1234".to_string())].into()),
    }
}

fn named(ids: &[&str]) -> VaultAgents {
    VaultAgents::Named(ids.iter().map(|s| s.to_string()).collect())
}

#[test]
fn personal_is_there_from_the_start_and_open_to_every_agent() {
    let s = store();
    let all = list(&s).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].id.as_str(), all[0].name.as_str(), &all[0].agents), ("personal", "Personal", &VaultAgents::All));
    let id = crate::backend::vault::create(&s, key("GitHub")).unwrap();
    assert_eq!(s.vault_get(&id).unwrap().unwrap().vault_id, "personal", "an entry made without a Vault goes to Personal");
}

#[test]
fn an_agent_sees_and_uses_only_the_entries_of_vaults_it_may_use() {
    let s = store();
    let work = create(&s, "Work", named(&["builder"])).unwrap();
    let private = create(&s, "Mine", VaultAgents::None).unwrap();
    let open = crate::backend::vault::create(&s, key("Open")).unwrap();
    let w = create_in(&s, key("Work token"), &work.id).unwrap();
    let p = create_in(&s, key("My token"), &private.id).unwrap();

    let ids = |agent: Option<&str>| list_for_agent(&s, agent).unwrap().into_iter().map(|e| e.id).collect::<Vec<_>>();
    let mut builder = ids(Some("Builder"));
    builder.sort();
    let mut want = vec![open.clone(), w.clone()];
    want.sort();
    assert_eq!(builder, want, "a named agent sees Personal and its own Vault, whatever the case of its id");
    assert_eq!(ids(Some("other")), [open.clone()], "another agent sees only Personal");
    assert_eq!(ids(None), [open.clone()], "an agent that did not prove who it is sees only the open Vaults");
    assert!(!ids(Some("builder")).contains(&p), "nobody but the person sees a Vault for none");

    assert!(fill_target(&s, &w, Some("builder")).is_ok());
    let refused = fill_target(&s, &w, Some("other")).err().unwrap();
    assert_eq!(refused, format!("no Vault entry {w:?}: Vault's list action shows the entries"), "refused as an entry that isn't there");
    assert!(command_target(&s, &p, Some("builder")).is_err());
    assert!(command_target(&s, &p, None).is_err());
    assert!(command_target(&s, &open, None).is_ok());
}

#[test]
fn the_pane_lists_every_entry_with_its_vault() {
    let s = store();
    let v = create(&s, "Work", VaultAgents::None).unwrap();
    let id = create_in(&s, key("Work token"), &v.id).unwrap();
    let (entries, _) = list_for_pane(&s).unwrap();
    assert_eq!(entries.iter().find(|e| e.id == id).unwrap().vault_id, v.id);
    assert_eq!(list(&s).unwrap().iter().find(|i| i.id == v.id).unwrap().entries, 1);
}

#[test]
fn moving_an_entry_keeps_its_id_and_grants_and_is_recorded() {
    let s = store();
    let v = create(&s, "Work", VaultAgents::None).unwrap();
    let id = crate::backend::vault::create(&s, key("GitHub")).unwrap();
    let created = s.vault_get(&id).unwrap().unwrap().created_at;
    s.vault_grant_add(&id, "https://github.com", 1, created).unwrap();
    move_entry(&s, &id, &v.id).unwrap();
    assert_eq!(s.vault_get(&id).unwrap().unwrap().vault_id, v.id);
    assert!(s.vault_grant_check(&id, "https://github.com").unwrap(), "the grant stays");
    let log = s.vault_activity_list(10).unwrap();
    assert!(log.iter().any(|a| a.action == "moved to another Vault" && a.target == "Personal -> Work" && a.entry_id == id));
    assert!(move_entry(&s, &id, "vault-nope").is_err());
    assert!(move_entry(&s, "no-such", &v.id).is_err());
}

#[test]
fn a_vault_with_entries_cannot_be_deleted_and_personal_never() {
    let s = store();
    let v = create(&s, "Work", VaultAgents::All).unwrap();
    let id = create_in(&s, key("Work token"), &v.id).unwrap();
    assert!(remove(&s, &v.id).unwrap_err().contains("still holds entries"));
    move_entry(&s, &id, "personal").unwrap();
    remove(&s, &v.id).unwrap();
    assert!(s.vault_info(&v.id).unwrap().is_none());
    assert!(remove(&s, "personal").is_err());
}

#[test]
fn names_are_unique_without_case_and_checked() {
    let s = store();
    create(&s, "Work", VaultAgents::All).unwrap();
    assert!(create(&s, " work ", VaultAgents::All).unwrap_err().contains("already a Vault"));
    assert!(create(&s, "Personal", VaultAgents::All).is_err());
    assert!(create(&s, "  ", VaultAgents::All).is_err());
    assert!(create(&s, &"x".repeat(61), VaultAgents::All).is_err());
    let v = create(&s, "Side", named(&["a", "A", " b "])).unwrap();
    assert_eq!(v.agents, named(&["a", "b"]), "agent ids are trimmed and deduplicated");
    update(&s, &v.id, "Side projects", VaultAgents::None).unwrap();
    assert_eq!(list(&s).unwrap().iter().find(|i| i.id == v.id).unwrap().name, "Side projects");
    update(&s, &v.id, "Side projects", VaultAgents::All).unwrap();
}

#[test]
fn resolve_finds_by_name_or_id_and_hides_vaults_an_agent_may_not_use() {
    let s = store();
    let work = create(&s, "Work", named(&["builder"])).unwrap();
    assert_eq!(resolve(&s, "work", None).unwrap().id, work.id);
    assert_eq!(resolve(&s, &work.id, Some(Some("builder"))).unwrap().id, work.id);
    let e = resolve(&s, "Work", Some(Some("other"))).unwrap_err();
    let listed = e.split("the Vaults are").nth(1).unwrap_or_default();
    assert!(e.contains("no Vault") && listed.contains("Personal") && !listed.contains("Work"), "{e}");
    assert_eq!(default_for(&s, Some("other")).unwrap(), "personal");
    update(&s, "personal", "Personal", VaultAgents::None).unwrap();
    assert!(default_for(&s, Some("other")).is_err(), "no Vault is open to it");
    assert_eq!(default_for(&s, Some("builder")).unwrap(), work.id);
}

#[test]
fn generate_goes_to_the_named_vault_or_the_agents_first_one() {
    use crate::backend::vault::{generate_entry, GenerateRequest};
    let s = store();
    let work = create(&s, "Work", named(&["builder"])).unwrap();
    let req = |vault: Option<&str>| GenerateRequest {
        kind: "api-key".into(),
        label: format!("Key {}", vault.unwrap_or("default")),
        why: String::new(),
        fields: Default::default(),
        rules: Default::default(),
        expires: None,
        vault: vault.map(str::to_string),
    };
    let made = generate_entry(&s, req(None), "builder").unwrap();
    assert_eq!(s.vault_get(&made.id).unwrap().unwrap().vault_id, "personal", "the first Vault the agent may use, in the person's order");
    let made = generate_entry(&s, req(Some("Work")), "builder").unwrap();
    assert_eq!(s.vault_get(&made.id).unwrap().unwrap().vault_id, work.id);
    assert!(generate_entry(&s, req(Some("Work")), "other").is_err(), "an agent can't make an entry in a Vault it may not use");
}

#[test]
fn a_capture_is_refused_before_the_command_runs_when_it_cannot_be_kept() {
    use crate::backend::vault::capture::plan;
    use agentmux_common::api_types::{CaptureFrom, CaptureInto, CaptureRule};
    let s = store();
    let work = create(&s, "Work", named(&["builder"])).unwrap();
    let hidden = create_in(&s, key("Work token"), &work.id).unwrap();
    let new_key = |label: &str| CaptureRule {
        into: CaptureInto { kind: Some("api-key".into()), label: Some(label.into()), ..Default::default() },
        from: CaptureFrom::Stdout("all".into()),
    };
    let existing = |id: &str| CaptureRule { into: CaptureInto { id: Some(id.into()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) };

    assert!(plan(&s, &[existing(&hidden)], "other", "true").unwrap_err().contains("there is no entry"), "an entry in a Vault it may not use is not there");
    assert!(plan(&s, &[existing(&hidden)], "builder", "true").is_ok());

    // Nowhere for a new entry to go: refused now, not after a command has issued a key.
    update(&s, "personal", "Personal", VaultAgents::None).unwrap();
    assert!(plan(&s, &[new_key("Fresh")], "other", "true").unwrap_err().contains("no Vault is open"));
    let ok = plan(&s, &[new_key("Fresh")], "builder", "true").unwrap();
    assert!(matches!(&ok.rules[0].into, crate::backend::vault::capture::Target::New { vault_id, .. } if *vault_id == work.id), "it goes to the agent's first Vault");
}

#[test]
fn an_agents_browser_sign_in_chooses_only_from_vaults_it_may_use() {
    use crate::backend::vault::signins;
    let s = store();
    let work = create(&s, "Work", named(&["builder"])).unwrap();
    let site = "https://intranet.example.com";
    let entry = NewEntry {
        kind: "login".into(),
        label: "Intranet".into(),
        fields: [("username", "alex"), ("site", site)].map(|(k, v)| (k.to_string(), v.to_string())).into(),
        tags: vec![],
        parts: Parts([("password".to_string(), "pw".to_string())].into()),
    };
    let id = create_in(&s, entry, &work.id).unwrap();
    // Personal's tabs keep their sign-ins in Work, which only `builder` may use.
    set_signin_vault(&s, "personal", &work.id).unwrap();
    let origins = [site.to_string()];
    let ids = |agent: &str| signins::offered_for_agent(&s, "personal", &origins, agent).unwrap().into_iter().map(|f| f.0).collect::<Vec<_>>();
    assert_eq!(ids("builder"), [id.clone()]);
    assert!(ids("other").is_empty(), "a sign-in in a Vault it may not use is not there");
    assert_eq!(signins::offered_for(&s, "personal", &origins).unwrap().len(), 1, "the key button the person presses lists it");
}

#[test]
fn a_profile_saves_and_offers_sign_ins_in_its_own_vault() {
    use crate::backend::vault::signins::{self, Space};
    let s = store();
    let work = create(&s, "Work", VaultAgents::None).unwrap();
    let site = "https://intranet.example.com".to_string();
    let space = |profile: &str| Space { profile: profile.into(), origin: site.clone(), realm: String::new(), proxy: false };
    let in_vault = |id: &str| s.vault_get(id).unwrap().unwrap().vault_id;

    // Nothing chosen: Personal, as before.
    let first = signins::save(&s, &space("personal"), "alex", "pw-1").unwrap();
    assert_eq!(in_vault(&first), "personal");

    // The work profile saves into Work; Personal's tabs are not offered it, nor it Personal's.
    set_signin_vault(&s, "profile:p-0123456789ab", &work.id).unwrap();
    let second = signins::save(&s, &space("profile:p-0123456789ab"), "sam", "pw-2").unwrap();
    assert_eq!(in_vault(&second), work.id);
    let origins = [site.clone()];
    assert_eq!(signins::offered_for(&s, "profile:p-0123456789ab", &origins).unwrap().into_iter().map(|f| f.0).collect::<Vec<_>>(), [second.clone()]);
    assert_eq!(signins::offered_for(&s, "personal", &origins).unwrap().into_iter().map(|f| f.0).collect::<Vec<_>>(), [first.clone()]);

    // Moving an entry out of the profile's Vault takes it out of the profile's list.
    move_entry(&s, &second, "personal").unwrap();
    assert!(signins::offered_for(&s, "profile:p-0123456789ab", &origins).unwrap().is_empty());

    // A Vault that is gone sends the profile back to Personal; so does choosing Personal.
    move_entry(&s, &second, &work.id).unwrap();
    move_entry(&s, &second, "personal").unwrap();
    remove(&s, &work.id).unwrap();
    assert_eq!(signins::vault_for(&s, "profile:p-0123456789ab").unwrap(), "personal");
    assert_eq!(signins::vault_for(&s, "personal").unwrap(), "personal");
}

#[test]
fn only_a_browser_profile_and_an_existing_vault_can_be_chosen_for_sign_ins() {
    let s = store();
    assert!(set_signin_vault(&s, "somewhere", "personal").unwrap_err().contains("not a browser profile"));
    assert!(set_signin_vault(&s, "personal", "vault-nope").unwrap_err().contains("no Vault"));
    let work = create(&s, "Work", VaultAgents::All).unwrap();
    set_signin_vault(&s, "personal", &work.id).unwrap();
    assert_eq!(signin_vaults(&s).unwrap().get("personal"), Some(&work.id));
    set_signin_vault(&s, "personal", "personal").unwrap();
    assert!(signin_vaults(&s).unwrap().is_empty(), "Personal is the default and keeps no row");
}
