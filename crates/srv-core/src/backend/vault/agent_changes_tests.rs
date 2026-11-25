// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The strip's data: what agents changed that the person has not looked at, and what each of Seen and Undo does
//! (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md §5).

use std::collections::BTreeMap;

use super::capture::{plan, Outcome};
use super::inspect::{update_details_for, UpdateRequest};
use super::*;
use agentmux_common::api_types::{CaptureFrom, CaptureInto, CaptureRule};
use crate::backend::storage::vault::{VaultActivityRow, VaultRow};

const OWN: &str = "profile:agent-builder-0123abcd";
const REASON: &str = "keep the staging details tidy for task 31020";

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn key(s: &Store, label: &str, value: &str) -> String {
    create(s, NewEntry { kind: "api-key".into(), label: label.into(), fields: Default::default(), tags: vec![], parts: Parts([("value".to_string(), value.to_string())].into()) }).unwrap()
}

fn asked(id: &str, label: Option<&str>, fields: &[(&str, &str)]) -> UpdateRequest {
    UpdateRequest { id: id.into(), label: label.map(String::from), fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), tags: None, reason: Some(REASON.into()) }
}

/// An agent replaces the value of `id` the way a Terminal capture does.
fn capture_over(s: &Store, id: &str, agent: &str, value: &str) {
    let rule = CaptureRule { into: CaptureInto { id: Some(id.into()), ..Default::default() }, from: CaptureFrom::Stdout("all".into()) };
    let mut cap = plan(s, &[rule], agent, "gh auth token").unwrap();
    cap.reason = REASON.into();
    assert!(matches!(cap.finish(s, &format!("{value}\n"), value, false, 0), Outcome::Stored { .. }));
}

fn generated_login(s: &Store) -> String {
    let req = GenerateRequest {
        kind: "login".into(),
        label: "Dashboard".into(),
        why: String::new(),
        reason: Some("an account for the dashboard task 31020".into()),
        fields: [("site".to_string(), "https://dash.example.com".to_string())].into(),
        rules: Default::default(),
        expires: None,
        vault: None,
    };
    generate_entry_in(s, req, "builder", Some(OWN)).unwrap().id
}

#[test]
fn an_entry_an_agent_added_is_listed_unseen_and_seen_clears_it_and_the_mark_on_its_value() {
    let s = store();
    let id = generated_login(&s);
    let all = agent_changes::list(&s, None, false).unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    let c = &all[0];
    assert_eq!((c.kind.as_str(), c.agent.as_str(), c.entry_id.as_str(), c.undo.as_str(), c.seen), ("added", "builder", id.as_str(), "remove", false));
    assert!(c.reason.contains("dashboard task 31020"));
    assert!(agent_set::is_unseen(&s, &id), "the site the agent chose is not trusted yet");

    agent_changes::seen(&s, c.seq).unwrap();
    assert!(agent_changes::list(&s, None, false).unwrap().is_empty());
    assert!(!agent_set::is_unseen(&s, &id), "marking it seen trusts the place, as an answered use does");
    let again = agent_changes::list(&s, Some(&id), true).unwrap();
    assert_eq!((again.len(), again[0].seen), (1, true), "the entry's own history still has it, for Undo");
}

#[test]
fn undo_of_an_added_entry_removes_it_as_the_persons_own_action() {
    let s = store();
    let id = generated_login(&s);
    let seq = agent_changes::list(&s, None, false).unwrap()[0].seq;
    agent_changes::undo(&s, seq).unwrap();
    assert!(s.vault_get(&id).unwrap().is_none());
    let lines = activity(&s, 50).unwrap();
    let deleted = lines.iter().find(|l| l.action == "deleted" && l.entry_id == id).expect("a line for the removal");
    assert_eq!(deleted.actor, "you");
    assert!(agent_changes::list(&s, None, true).unwrap().is_empty(), "nothing left to look at");
}

#[test]
fn a_value_an_agent_replaced_on_an_entry_the_person_made_comes_first_and_undo_puts_the_old_value_back() {
    let s = store();
    let mine = key(&s, "GitHub", "ghp_persons_old_000");
    update_details_for(&s, &asked(&mine, Some("GitHub CI"), &[]), "tidier", Some(OWN)).unwrap();
    capture_over(&s, &mine, "builder", "ghp_agents_new_111");
    let all = agent_changes::list(&s, None, false).unwrap();
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!((all[0].kind.as_str(), all[0].replaces_yours, all[0].undo.as_str()), ("new value", true, "restore value"), "the value of an entry you made sorts first");
    assert_eq!((all[1].kind.as_str(), all[1].agent.as_str()), ("changed details", "tidier"));

    agent_changes::undo(&s, all[0].seq).unwrap();
    assert_eq!(reveal(&s, &mine, "value", "revealed").unwrap().as_str(), "ghp_persons_old_000");
    let after = agent_changes::list(&s, None, false).unwrap();
    assert_eq!(after.len(), 1, "an undone change is looked at");
    assert!(!agent_set::is_unseen(&s, &mine), "the value the agent set is gone, so the use card has nothing to say about it");
}

#[test]
fn undo_of_changed_details_puts_the_earlier_label_fields_and_tags_back_and_is_recorded_as_the_persons() {
    let s = store();
    let id = key(&s, "Stripe", "sk_value_000000");
    update_details_for(&s, &asked(&id, Some("Stripe live"), &[("notes", "moved")]), "builder", Some(OWN)).unwrap();
    let change = agent_changes::list(&s, None, false).unwrap().remove(0);
    assert_eq!((change.kind.as_str(), change.undo.as_str(), change.target.as_str()), ("changed details", "restore details", "label, field notes"));
    agent_changes::undo(&s, change.seq).unwrap();
    let row = s.vault_get(&id).unwrap().unwrap();
    assert_eq!(row.label, "Stripe");
    assert!(!signins::fields_of(&row).contains_key("notes"));
    let lines = history::history_for(&s, &id, history::Asker::Person, 20).unwrap();
    let put_back = lines.iter().find(|l| l.action == agent_changes::RESTORED_DETAILS).unwrap();
    assert_eq!((put_back.actor.as_str(), put_back.outcome.as_str()), ("you", "done"));
    assert_eq!(reveal(&s, &id, "value", "revealed").unwrap().as_str(), "sk_value_000000", "the value was never touched");

    // Past its 7 days there is nothing to put back.
    let again = key(&s, "Mail", "sk_mail_000000");
    update_details_for(&s, &asked(&again, Some("Mail prod"), &[]), "builder", Some(OWN)).unwrap();
    s.vault_details_prune(now_ms() + 1).unwrap();
    let late = agent_changes::list(&s, Some(&again), false).unwrap().remove(0);
    assert_eq!(late.undo, "");
    assert!(agent_changes::undo(&s, late.seq).is_err());
}

#[test]
fn a_proposal_is_listed_first_with_what_it_would_change_and_leaves_when_accepted_or_dismissed() {
    let s = store();
    let fields: BTreeMap<String, String> = [("site".to_string(), "https://intranet.example.com".to_string())].into();
    let login = create(&s, NewEntry { kind: "login".into(), label: "Intranet".into(), fields, tags: vec![], parts: Parts([("password".to_string(), "pw-persons".to_string())].into()) }).unwrap();
    let _other = generated_login(&s);
    update_details_for(&s, &asked(&login, None, &[("site", "https://intranet2.example.com"), ("realm", "Corp")]), "builder", Some(OWN)).unwrap();
    let all = agent_changes::list(&s, None, false).unwrap();
    assert_eq!(all.iter().map(|c| c.kind.as_str()).collect::<Vec<_>>(), ["proposal", "proposal", "added"], "{all:?}");
    let site = all.iter().find(|c| c.proposal.as_ref().is_some_and(|p| p.field == "site")).unwrap();
    let p = site.proposal.as_ref().unwrap();
    assert_eq!((p.value.as_str(), p.current.as_str(), site.entry_id.as_str()), ("https://intranet2.example.com", "https://intranet.example.com", login.as_str()));
    assert_eq!(site.undo, "", "a proposal has Accept and Dismiss, not Undo");

    // A proposal the agent replaced after it was shown is refused, and the replacement stays waiting.
    let shown = site.proposal.as_ref().unwrap().at;
    assert_eq!(proposals::accept_shown(&s, &login, "site", Some(shown - 1)).unwrap_err(), proposals::REPLACED);
    assert_eq!(proposals::dismiss_shown(&s, &login, "realm", Some(0)).unwrap_err(), proposals::REPLACED);
    proposals::dismiss(&s, &login, "realm").unwrap();
    let rest = agent_changes::list(&s, None, false).unwrap();
    assert_eq!(rest.iter().filter(|c| c.kind == "proposal").count(), 1, "a dismissed proposal is gone from the list");
    proposals::accept_shown(&s, &login, "site", Some(shown)).unwrap();
    assert!(agent_changes::list(&s, None, false).unwrap().iter().all(|c| c.kind != "proposal"));
}

#[test]
fn seen_all_clears_the_strip_and_the_marks_and_the_persons_own_lines_are_never_in_it() {
    let s = store();
    let a = generated_login(&s);
    let b = key(&s, "Mine", "sk_mine_000000");
    capture_over(&s, &b, "builder", "sk_agents_000001");
    assert_eq!(agent_changes::list(&s, None, false).unwrap().len(), 2);
    assert!(history::history_for(&s, &b, history::Asker::Person, 20).unwrap().iter().any(|l| l.actor_kind != "agent"), "the person's own creation line exists and is not a change");
    agent_changes::seen_all(&s).unwrap();
    assert!(agent_changes::list(&s, None, false).unwrap().is_empty());
    assert!(!agent_set::is_unseen(&s, &a) && !agent_set::is_unseen(&s, &b));
    // A later change is new again.
    capture_over(&s, &b, "builder", "sk_agents_000002");
    assert_eq!(agent_changes::list(&s, None, false).unwrap().len(), 1);
    // And a deleted entry has nothing to look at.
    delete(&s, &b).unwrap();
    assert!(agent_changes::list(&s, None, false).unwrap().is_empty());
}

#[test]
fn seen_on_one_change_keeps_the_entrys_mark_while_a_newer_change_is_unseen_and_refuses_a_line_that_is_not_an_agents() {
    let s = store();
    let id = key(&s, "GitHub", "ghp_persons_old_000");
    capture_over(&s, &id, "builder", "ghp_agents_new_111");
    capture_over(&s, &id, "other", "ghp_agents_new_222");
    let all = agent_changes::list(&s, Some(&id), false).unwrap();
    assert_eq!(all.len(), 2, "{all:?}");
    let (newer, older) = (&all[0], &all[1]);
    assert!(newer.seq > older.seq);

    agent_changes::seen(&s, older.seq).unwrap();
    assert!(agent_set::is_unseen(&s, &id), "the newer value is still unseen, so it still waits for a yes");
    assert_eq!(agent_changes::list(&s, Some(&id), false).unwrap().len(), 1);
    agent_changes::seen(&s, newer.seq).unwrap();
    assert!(!agent_set::is_unseen(&s, &id));

    let row = s.vault_get(&id).unwrap().unwrap();
    let mine = s.vault_activity_first(&id, row.created_at).unwrap().unwrap();
    assert_ne!(mine.actor_kind, "agent");
    assert!(agent_changes::seen(&s, mine.seq).is_err(), "the person's own line is not a change by an agent");
}

#[test]
fn seen_all_leaves_a_pending_proposal_in_the_strip_to_accept_or_dismiss() {
    let s = store();
    let early = key(&s, "Early", "sk_early_000000");
    capture_over(&s, &early, "builder", "sk_agents_000001");
    let fields: BTreeMap<String, String> = [("site".to_string(), "https://intranet.example.com".to_string())].into();
    let login = create(&s, NewEntry { kind: "login".into(), label: "Intranet".into(), fields, tags: vec![], parts: Parts([("password".to_string(), "pw-persons".to_string())].into()) }).unwrap();
    update_details_for(&s, &asked(&login, None, &[("site", "https://intranet2.example.com")]), "builder", Some(OWN)).unwrap();
    let late = key(&s, "Late", "sk_late_0000000");
    capture_over(&s, &late, "builder", "sk_agents_000002");

    agent_changes::seen_all(&s).unwrap();
    let left = agent_changes::list(&s, None, false).unwrap();
    assert_eq!(left.iter().map(|c| c.kind.as_str()).collect::<Vec<_>>(), ["proposal"], "{left:?}");
    assert!(!agent_set::is_unseen(&s, &early) && !agent_set::is_unseen(&s, &late), "the changes after the proposal are seen too");
    proposals::accept(&s, &login, "site").unwrap();
    assert!(agent_changes::list(&s, None, false).unwrap().is_empty());
}

#[test]
fn undoing_changed_details_twice_is_refused_and_keeps_what_the_person_edited_since() {
    let s = store();
    let id = key(&s, "Stripe", "sk_value_000000");
    update_details_for(&s, &asked(&id, Some("Stripe live"), &[("notes", "moved")]), "builder", Some(OWN)).unwrap();
    // The person tags the entry after the agent's change.
    let row = s.vault_get(&id).unwrap().unwrap();
    update(&s, EntryChange { id: id.clone(), label: row.label.clone(), fields: signins::fields_of(&row), tags: vec!["ops".into()], parts: Parts::default(), clear: vec![] }).unwrap();

    let change = agent_changes::list(&s, Some(&id), false).unwrap().into_iter().find(|c| c.kind == "changed details").unwrap();
    agent_changes::undo(&s, change.seq).unwrap();
    let now = s.vault_get(&id).unwrap().unwrap();
    assert_eq!(now.label, "Stripe");
    assert!(!signins::fields_of(&now).contains_key("notes"), "what the agent added is gone");
    assert_eq!(now.tags, r#"["ops"]"#, "the person's later edit stays");

    let listed = agent_changes::list(&s, Some(&id), true).unwrap().into_iter().find(|c| c.seq == change.seq).unwrap();
    assert_eq!(listed.undo, "", "it is not offered again");
    assert!(agent_changes::undo(&s, change.seq).is_err(), "and a second try changes nothing");
    assert_eq!(s.vault_get(&id).unwrap().unwrap().tags, r#"["ops"]"#);
}

#[test]
fn two_quick_changes_by_one_agent_each_undo_to_their_own_earlier_value() {
    let s = store();
    let id = key(&s, "GitHub", "ghp_a_000000000");
    capture_over(&s, &id, "builder", "ghp_b_000000000");
    capture_over(&s, &id, "builder", "ghp_c_000000000");
    let all = agent_changes::list(&s, Some(&id), false).unwrap();
    assert_eq!(all.len(), 2, "{all:?}");
    let first = all.iter().min_by_key(|c| c.seq).unwrap();
    assert_eq!(first.undo, "restore value");
    agent_changes::undo(&s, first.seq).unwrap();
    assert_eq!(reveal(&s, &id, "value", "revealed").unwrap().as_str(), "ghp_a_000000000", "the first change put back its own earlier value, not the second's");
}

#[test]
fn only_a_creating_line_written_with_the_entry_makes_it_added_and_offers_removal() {
    let s = store();
    let id = key(&s, "Synced", "sk_synced_00000");
    capture_over(&s, &id, "builder", "sk_agents_00001");
    let row = s.vault_get(&id).unwrap().unwrap();
    let line = s.vault_activity_agent_changes(0, &["captured"], 10).unwrap().remove(0);
    // An entry with no local creation line (from sync or a bulk import) has an edit like this one for its first line.
    let long_before = VaultRow { created_at: line.at - 60_000, ..row.clone() };
    assert!(!inspect::is_creation(&line, &long_before), "a capture long after the entry was made is an edit, not its creation");
    let edit = VaultActivityRow { action: inspect::CHANGED_DETAILS.into(), ..line.clone() };
    assert!(!inspect::is_creation(&edit, &row), "an edit of details is never a creation");
    // And the person's own entry is never listed as added: its first line is theirs.
    let c = agent_changes::list(&s, Some(&id), false).unwrap().remove(0);
    assert_eq!((c.kind.as_str(), c.undo.as_str()), ("new value", "restore value"), "{c:?}");
}
