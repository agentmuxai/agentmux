// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::vault::{activity, create, generate_entry, list_for_agent, reveal, seal, update, EntryChange, GenerateRequest, NewEntry};

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

/// The Kanban's queue, in an identity store of its own as srv has it.
fn work() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap(), dir)
}

fn day(text: &str) -> NaiveDate {
    parse(text).unwrap()
}

fn key(label: &str, expires: &str) -> NewEntry {
    NewEntry {
        kind: "api-key".into(),
        label: label.into(),
        fields: [("expires".to_string(), expires.to_string())].into(),
        tags: vec![],
        parts: seal::Parts([("value".to_string(), "sk_live_abcdefgh".to_string())].into()),
    }
}

fn tasks(work: &Store) -> Vec<WorkItem> {
    work.work_queue_list("", 100).unwrap().into_iter().filter(|t| t.kind == TASK_KIND).collect()
}

#[test]
fn a_date_is_yyyy_mm_dd_and_nothing_else() {
    assert_eq!(day("2026-11-12").to_string(), "2026-11-12");
    for bad in ["Dec 2026", "2026-13-01", "2026-2-1", "12/11/2026", ""] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_nearest_reminder_is_the_one_due() {
    assert_eq!(Stage::due(30), None);
    assert_eq!(Stage::due(14), Some(Stage::TwoWeeks));
    assert_eq!(Stage::due(4), Some(Stage::TwoWeeks));
    assert_eq!(Stage::due(3), Some(Stage::ThreeDays));
    assert_eq!(Stage::due(1), Some(Stage::ThreeDays));
    assert_eq!(Stage::due(0), Some(Stage::OnTheDay));
    assert_eq!(Stage::due(-40), Some(Stage::OnTheDay), "an entry already past its date is told of once");
}

#[test]
fn a_bad_date_is_refused_and_logins_and_notes_take_one() {
    let s = store();
    assert!(create(&s, key("Stripe", "next month")).unwrap_err().contains("YYYY-MM-DD"));
    let note = NewEntry {
        kind: "note".into(),
        label: "Recovery codes".into(),
        fields: [("expires".to_string(), "2027-01-31".to_string())].into(),
        tags: vec![],
        parts: seal::Parts([("text".to_string(), "aaaa-bbbb".to_string())].into()),
    };
    create(&s, note).unwrap();
}

#[test]
fn each_reminder_is_sent_once_and_the_first_files_a_rotation_task() {
    let (s, (work, _dir)) = (store(), work());
    let id = create(&s, key("Stripe live", "2026-11-20")).unwrap();
    assert!(send(&s, Some(&work), day("2026-10-20")).unwrap().notices.is_empty(), "a month out: nothing yet");

    let sent = send(&s, Some(&work), day("2026-11-10")).unwrap();
    assert_eq!(sent.notices.len(), 1);
    let (due, text) = &sent.notices[0];
    assert_eq!((due.id.as_str(), due.stage, due.first), (id.as_str(), Stage::TwoWeeks, true));
    assert_eq!(text, "Expires in 10 days, on 2026-11-20.");
    let filed = tasks(&work);
    assert_eq!(filed.len(), 1);
    assert_eq!(filed[0].title, "Rotate Stripe live: it expires on 2026-11-20");
    assert_eq!(filed[0].target_agent, "", "the person made it: the task is for anyone");

    assert!(send(&s, Some(&work), day("2026-11-10")).unwrap().notices.is_empty(), "a restart doesn't repeat it");
    assert!(send(&s, Some(&work), day("2026-11-12")).unwrap().notices.is_empty());

    let sent = send(&s, Some(&work), day("2026-11-18")).unwrap();
    assert_eq!(sent.notices[0].0.stage, Stage::ThreeDays);
    assert!(!sent.notices[0].0.first);
    assert_eq!(sent.notices[0].1, "Expires in 2 days, on 2026-11-20.");
    assert_eq!(tasks(&work).len(), 1, "one task per date");

    let sent = send(&s, Some(&work), day("2026-11-20")).unwrap();
    assert_eq!(sent.notices[0].1, "Expires today, 2026-11-20.");
    assert!(send(&s, Some(&work), day("2026-11-25")).unwrap().notices.is_empty(), "after the day, nothing more");

    let lines: Vec<String> = activity(&s, 50).unwrap().into_iter().filter(|l| l.action == REMINDED).map(|l| l.target).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().any(|t| t.starts_with("expires 2026-11-20, 14 days before; rotation task #")), "{lines:?}");
}

#[test]
fn the_rotation_task_goes_to_the_agent_that_made_the_value() {
    let (s, (work, _dir)) = (store(), work());
    let req = GenerateRequest {
        kind: "api-key".into(),
        label: "Deploy token".into(),
        why: String::new(),
        fields: Default::default(),
        rules: Default::default(),
        expires: Some("2026-12-01".into()),
    };
    let made = generate_entry(&s, req, "agent2-6").unwrap();
    let listed = list_for_agent(&s).unwrap();
    assert_eq!(listed[0].expires.as_deref(), Some("2026-12-01"), "an agent sees the date, no secret");
    send(&s, Some(&work), day("2026-11-30")).unwrap();
    let filed = tasks(&work);
    assert_eq!(filed[0].target_agent, "agent2-6");
    assert!(filed[0].payload.contains(&made.id));
    let line = activity(&s, 10).unwrap().into_iter().find(|l| l.action == REMINDED).unwrap();
    assert!(line.target.ends_with(" for agent2-6"), "{}", line.target);
}

// A key's uses must never push out the line that says a reminder was sent, or
// the one naming who made the value (#5854).
#[test]
fn a_much_used_key_is_still_reminded_once_and_its_maker_still_known() {
    let (s, (work, _dir)) = (store(), work());
    let req = GenerateRequest {
        kind: "api-key".into(),
        label: "Busy token".into(),
        why: String::new(),
        fields: Default::default(),
        rules: Default::default(),
        expires: Some("2026-12-01".into()),
    };
    let made = generate_entry(&s, req, "agent2-6").unwrap();
    for _ in 0..600 {
        crate::backend::vault::record(&s, &made.id, "Busy token", "agent2-6", "ran a command", "deploy", "done");
    }
    send(&s, Some(&work), day("2026-11-20")).unwrap();
    for _ in 0..600 {
        crate::backend::vault::record(&s, &made.id, "Busy token", "agent2-6", "ran a command", "deploy", "done");
    }
    assert!(send(&s, Some(&work), day("2026-11-21")).unwrap().notices.is_empty(), "not sent again");
    let filed = tasks(&work);
    assert_eq!(filed.len(), 1);
    assert_eq!(filed[0].target_agent, "agent2-6");
}

#[test]
fn a_rotation_task_that_couldnt_be_filed_is_filed_at_the_next_reminder() {
    let (s, (work, _dir)) = (store(), work());
    create(&s, key("Stripe", "2026-11-20")).unwrap();
    // No queue to file it in: the reminder is sent, the task isn't.
    let sent = send(&s, None, day("2026-11-10")).unwrap();
    assert!(sent.notices[0].0.first);
    let sent = send(&s, Some(&work), day("2026-11-18")).unwrap();
    assert!(sent.notices[0].0.first, "still no task for this date");
    assert_eq!(tasks(&work).len(), 1);
    assert!(!send(&s, Some(&work), day("2026-11-20")).unwrap().notices[0].0.first);
}

#[test]
fn a_reminder_is_shown_while_agentmux_has_focus() {
    use crate::backend::notify::policy::{NotifyKind, Priority};
    // It is recorded as sent when emitted: a gated-out one would be lost. A pause
    // or quiet hours are waited out by `run` (`holds_back_attention`).
    assert_eq!(NotifyKind::VaultExpiring.priority(), Priority::Attention);
}

#[test]
fn a_few_reminders_show_one_each_and_more_share_one() {
    let (s, (work, _dir)) = (store(), work());
    for label in ["Stripe", "GitHub token", "AWS key"] {
        create(&s, key(label, "2026-11-20")).unwrap();
    }
    let three = send(&s, Some(&work), day("2026-11-10")).unwrap().notices;
    let shown = toasts(&three);
    assert_eq!(shown.len(), 3);
    assert!(shown.iter().all(|t| t.key.starts_with("vault:api-key-") && t.body == "Expires in 10 days, on 2026-11-20."));

    for label in ["Deploy", "Webhook", "Mail", "Maps", "Search"] {
        create(&s, key(label, "2026-11-21")).unwrap();
    }
    let five = send(&s, Some(&work), day("2026-11-11")).unwrap().notices;
    assert_eq!(five.len(), 5, "every reminder is recorded");
    let shown = toasts(&five);
    assert_eq!(shown.len(), 1, "one notification for them all");
    assert_eq!(shown[0].name, "5 entries");
    assert!(shown[0].body.ends_with("and 2 more expire within 14 days, or have expired. The Vault's Expiring soon filter lists them."), "{}", shown[0].body);
}

#[test]
fn a_new_date_starts_the_reminders_again_and_the_change_is_recorded() {
    let (s, (work, _dir)) = (store(), work());
    let id = create(&s, key("Stripe", "2026-11-20")).unwrap();
    send(&s, Some(&work), day("2026-11-19")).unwrap();
    let change = EntryChange {
        id: id.clone(),
        label: "Stripe".into(),
        fields: [("expires".to_string(), "2027-11-20".to_string())].into(),
        tags: vec![],
        parts: Default::default(),
        clear: vec![],
    };
    update(&s, change).unwrap();
    let changed = activity(&s, 10).unwrap().into_iter().find(|l| l.action == "changed").unwrap();
    assert_eq!(changed.target, "expires 2027-11-20");
    let sent = send(&s, Some(&work), day("2027-11-10")).unwrap();
    assert!(sent.notices[0].0.first, "the new date's first reminder");
    assert_eq!(tasks(&work).len(), 2);
}

#[test]
fn an_expired_entry_stays_usable_and_its_uses_say_so() {
    let s = store();
    let id = create(&s, key("Old key", "2020-01-01")).unwrap();
    reveal(&s, &id, "value", "revealed").unwrap();
    let line = activity(&s, 10).unwrap().into_iter().find(|l| l.action == "revealed").unwrap();
    assert_eq!(line.target, "value (expired)");
}

#[test]
fn a_card_and_an_entry_without_a_date_get_no_reminder() {
    let s = store();
    let card = NewEntry {
        kind: "card".into(),
        label: "Visa".into(),
        fields: Default::default(),
        tags: vec![],
        parts: seal::Parts([("number".to_string(), "4111111111111111".to_string()), ("expiry".to_string(), "11/26".to_string())].into()),
    };
    create(&s, card).unwrap();
    create(&s, NewEntry { fields: Default::default(), ..key("No date", "2026-01-01") }).unwrap();
    assert!(due(&s, day("2026-11-01")).unwrap().is_empty());
}
