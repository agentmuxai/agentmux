// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const NOW: i64 = 1_000_000;

fn grant(tier: &str, agents: &str, extra: &str) -> String {
    format!(r#"{{ "tier": "{tier}", "mint": {{ "kind": "command", "argv": ["/opt/escalate", "--reason", "{{reason}}"] }}, "agents": [{agents}]{extra} }}"#)
}

fn file(grants: &[(&str, String)]) -> (Policy, String) {
    let body: Vec<String> = grants.iter().map(|(n, g)| format!("\"{n}\": {g}")).collect();
    let text = format!(r#"{{ "version": 1, "grants": {{ {} }} }}"#, body.join(", "));
    (Policy::parse(&text).unwrap(), sha256_hex(&text))
}

fn deploy_for(agents: &str) -> String {
    grant("write", agents, r#", "max_mints_per_hour": 4"#)
}

fn base() -> (Policy, String) {
    file(&[("read", grant("read", r#""*""#, "")), ("deploy", deploy_for(r#""jcont""#))])
}

fn baselined() -> Record {
    let (p, sha) = base();
    sync(None, &p, &sha, NOW).0
}

fn kinds(events: &[Event]) -> Vec<(&'static str, Option<&str>, bool)> {
    events.iter().map(|e| (e.kind, e.grant.as_deref(), e.held)).collect()
}

#[test]
fn the_first_run_records_the_file_as_the_baseline_and_says_so() {
    let (p, sha) = base();
    let (rec, events) = sync(None, &p, &sha, NOW);
    assert_eq!(kinds(&events), vec![("baseline", None, false)]);
    assert!(events[0].text.contains("baseline") && events[0].text.contains("2 grants (1 write)"), "{}", events[0].text);
    assert!(rec.grants.values().all(|e| e.state() == State::Honoured && e.allowed_by.as_deref() == Some(BASELINE_BY)));
    assert!(effective(&p, &rec).grants.values().all(|g| g.hold.is_none()), "nothing is held at the baseline");
    assert_eq!(rec.changes.len(), 1);
}

#[test]
fn a_file_that_has_not_changed_records_nothing() {
    let rec = baselined();
    let (p, sha) = base();
    let (again, events) = sync(Some(rec.clone()), &p, &sha, NOW + 5);
    assert!(events.is_empty());
    assert_eq!(again, rec, "not even the change list moves");
}

#[test]
fn an_edit_that_changes_no_grant_records_nothing_but_remembers_the_new_text() {
    let rec = baselined();
    let (p, _) = base();
    let (again, events) = sync(Some(rec.clone()), &p, "another text, the same grants", NOW + 5);
    assert!(events.is_empty());
    assert_eq!(again.file_sha256, "another text, the same grants");
    assert_eq!(again.grants, rec.grants);
    assert_eq!(again.changes, rec.changes);
}

#[test]
fn an_added_write_grant_is_recorded_and_held() {
    let rec = baselined();
    let (p, sha) = file(&[("read", grant("read", r#""*""#, "")), ("deploy", deploy_for(r#""jcont""#)), ("admin", grant("write", r#""pulse""#, ""))]);
    let (rec, events) = sync(Some(rec), &p, &sha, NOW + 10);
    assert_eq!(kinds(&events), vec![("added", Some("admin"), true)]);
    assert!(events[0].text.contains("held until"), "{}", events[0].text);
    assert_eq!(rec.grants["admin"].state(), State::Held);
    let eff = effective(&p, &rec);
    assert_eq!(eff.grants["admin"].hold, Some(Hold::Awaiting));
    assert_eq!(eff.grants["deploy"].hold, None, "the grants that were there are untouched");
    assert!(!eff.open_to("pulse").contains(&"admin"), "a held grant is open to nobody");
    assert_eq!(rec.grants["admin"].added_ms, NOW + 10);
    assert_eq!(rec.grants["admin"].added_change, events[0].id);
}

#[test]
fn a_new_grant_that_is_switched_off_waits_for_nothing_but_switching_it_on_is_a_widening() {
    let rec = baselined();
    let with = |third: String| file(&[("read", grant("read", r#""*""#, "")), ("deploy", deploy_for(r#""jcont""#)), ("prod", third)]);
    let (p, sha) = with(grant("write", r#""jcont""#, r#", "disabled": true"#));
    let (rec, events) = sync(Some(rec), &p, &sha, NOW + 1);
    assert_eq!(kinds(&events), vec![("added", Some("prod"), false)]);
    assert_eq!(rec.grants["prod"].state(), State::Honoured);
    let (p, sha) = with(grant("write", r#""jcont""#, ""));
    let (rec, events) = sync(Some(rec), &p, &sha, NOW + 2);
    assert_eq!(kinds(&events), vec![("widened", Some("prod"), true)]);
    assert!(events[0].text.contains("switched on"), "{}", events[0].text);
    assert_eq!(rec.grants["prod"].state(), State::Held);
}

fn sum() -> Summary {
    Summary { tier: "read".into(), agents: vec!["a".into(), "b".into()], disabled: false, needs_reason: true, max_seconds: 1800, max_mints_per_hour: 4, recipe: "r1".into() }
}

#[test]
fn each_way_a_grant_gets_wider_is_named() {
    let wider: Vec<(Summary, &str)> = vec![
        (Summary { tier: "write".into(), ..sum() }, "tier"),
        (Summary { agents: vec!["a".into(), "b".into(), "c".into()], ..sum() }, "agents"),
        (Summary { agents: vec!["*".into()], ..sum() }, "agents"),
        (Summary { recipe: "r2".into(), ..sum() }, "recipe"),
        (Summary { needs_reason: false, ..sum() }, "no reason needed"),
        (Summary { max_seconds: 3600, ..sum() }, "session length"),
        (Summary { max_mints_per_hour: 5, ..sum() }, "hourly cap"),
    ];
    for (now, what) in wider {
        assert_eq!(compare(&sum(), &now), Delta::Wider(vec![what]), "{what}");
    }
    let off = Summary { disabled: true, ..sum() };
    assert_eq!(compare(&off, &sum()), Delta::Wider(vec!["switched on"]));
}

#[test]
fn each_way_a_grant_gets_narrower_is_not_a_widening() {
    let was = Summary { tier: "write".into(), ..sum() };
    let narrower = vec![
        Summary { tier: "read".into(), ..sum() },
        Summary { agents: vec!["a".into()], ..was.clone() },
        Summary { disabled: true, ..was.clone() },
        Summary { max_seconds: 900, ..was.clone() },
        Summary { max_mints_per_hour: 1, ..was.clone() },
    ];
    for now in &narrower {
        assert_eq!(compare(&was, now), Delta::Narrower, "{now:?}");
    }
    assert_eq!(compare(&sum(), &sum()), Delta::Same);
    // Wider in one thing and narrower in another is wider.
    assert_eq!(compare(&sum(), &Summary { agents: vec!["a".into()], max_seconds: 3600, ..sum() }), Delta::Wider(vec!["session length"]));
}

#[test]
fn a_widened_grant_is_held_and_a_narrowed_one_is_in_force_at_once() {
    let rec = baselined();
    let wider = file(&[("read", grant("read", r#""*""#, "")), ("deploy", deploy_for(r#""jcont", "lazo""#))]);
    let (rec, events) = sync(Some(rec), &wider.0, &wider.1, NOW + 1);
    assert_eq!(kinds(&events), vec![("widened", Some("deploy"), true)]);
    assert!(events[0].text.contains("agents"), "{}", events[0].text);
    assert_eq!(effective(&wider.0, &rec).grants["deploy"].hold, Some(Hold::Awaiting));
    // Narrower than what was allowed (not than the held file): in force, and what was allowed is now this.
    let narrower = file(&[("read", grant("read", r#""*""#, "")), ("deploy", grant("write", r#""jcont""#, r#", "max_mints_per_hour": 2"#))]);
    let (rec, events) = sync(Some(rec), &narrower.0, &narrower.1, NOW + 2);
    assert_eq!(kinds(&events), vec![("narrowed", Some("deploy"), false)]);
    assert_eq!(rec.grants["deploy"].state(), State::Honoured);
    assert_eq!(effective(&narrower.0, &rec).grants["deploy"].hold, None);
}

#[test]
fn a_removed_grant_is_recorded_and_leaves_no_allowance_behind() {
    let rec = baselined();
    let only_read = file(&[("read", grant("read", r#""*""#, ""))]);
    let (rec, events) = sync(Some(rec), &only_read.0, &only_read.1, NOW + 1);
    assert_eq!(kinds(&events), vec![("removed", Some("deploy"), false)]);
    assert!(!rec.grants.contains_key("deploy"));
    assert!(effective(&only_read.0, &rec).grants.get("deploy").is_none(), "it stops working at once");
    let (rec, events) = sync(Some(rec), &base().0, &base().1, NOW + 2);
    assert_eq!(kinds(&events), vec![("added", Some("deploy"), true)]);
    assert_eq!(rec.grants["deploy"].state(), State::Held);
}

#[test]
fn a_change_of_the_default_grant_is_recorded() {
    let with_default = r#"{ "version": 1, "default_grant": "read", "grants": { "read": { "tier": "read", "mint": { "kind": "host_profile", "profile": "p" }, "agents": ["*"] } } }"#;
    let without = with_default.replace(r#""default_grant": "read", "#, "");
    let first = sync(None, &Policy::parse(with_default).unwrap(), &sha256_hex(with_default), NOW).0;
    let (_, events) = sync(Some(first), &Policy::parse(&without).unwrap(), &sha256_hex(&without), NOW + 1);
    assert_eq!(kinds(&events), vec![("default", None, false)]);
}

fn held_deploy() -> (Record, String) {
    let (p, sha) = file(&[("read", grant("read", r#""*""#, "")), ("deploy", deploy_for(r#""jcont", "lazo""#))]);
    let (rec, events) = sync(Some(baselined()), &p, &sha, NOW + 1);
    (rec, events[0].id.clone())
}

fn answer<'a>(change: &'a str, allow: bool, reason: &'a str) -> Decision<'a> {
    Decision { grant: "deploy", change, allow, by: "loap", reason }
}

#[test]
fn allowing_a_held_grant_needs_a_reason_and_the_change_that_was_read() {
    let (mut rec, change) = held_deploy();
    assert!(decide(&mut rec, &answer(&change, true, "  "), NOW + 2).unwrap_err().contains("reason"));
    assert!(decide(&mut rec, &answer("c0-1", true, "the lead asked"), NOW + 2).unwrap_err().contains("changed since you read it"));
    assert_eq!(rec.grants["deploy"].state(), State::Held, "a refused answer changes nothing");
    let event = decide(&mut rec, &answer(&change, true, "Lazo deploys the staging site"), NOW + 3).unwrap();
    assert_eq!((event.kind, event.by.as_deref(), event.held), ("allowed", Some("loap"), false));
    let e = &rec.grants["deploy"];
    assert_eq!((e.state(), e.allowed_by.as_deref(), e.allowed_ms), (State::Honoured, Some("loap"), Some(NOW + 3)));
    assert_eq!(e.allowed_reason.as_deref(), Some("Lazo deploys the staging site"));
    assert_eq!(e.added_change, format!("c{NOW:x}-1"), "it keeps the change that added it");
    assert_eq!(rec.changes.last().unwrap().kind, "allowed");
    assert!(decide(&mut rec, &answer(&event.id, true, "again"), NOW + 4).unwrap_err().contains("nothing waits"));
}

#[test]
fn refusing_a_held_grant_keeps_it_refused_until_the_file_changes_again() {
    let (mut rec, change) = held_deploy();
    let event = decide(&mut rec, &answer(&change, false, "not wanted"), NOW + 2).unwrap();
    assert_eq!(event.kind, "refused");
    assert_eq!(rec.grants["deploy"].state(), State::Refused);
    let (p, _) = file(&[("read", grant("read", r#""*""#, "")), ("deploy", deploy_for(r#""jcont", "lazo""#))]);
    assert_eq!(effective(&p, &rec).grants["deploy"].hold, Some(Hold::Refused));
    assert!(decide(&mut rec, &answer(&event.id, false, "again"), NOW + 3).unwrap_err().contains("already refused"));
    // Edited again: asked about again.
    let (p, sha) = file(&[("read", grant("read", r#""*""#, "")), ("deploy", deploy_for(r#""jcont", "lazo", "pulse""#))]);
    let (rec, events) = sync(Some(rec), &p, &sha, NOW + 4);
    assert_eq!(kinds(&events), vec![("widened", Some("deploy"), true)]);
    assert_eq!(rec.grants["deploy"].state(), State::Held);
}

#[test]
fn a_grant_in_force_is_not_one_to_refuse() {
    let mut rec = baselined();
    let change = rec.grants["read"].change.clone();
    let e = decide(&mut rec, &Decision { grant: "read", change: &change, allow: false, by: "loap", reason: "x" }, NOW + 5).unwrap_err();
    assert!(e.contains("in force"), "{e}");
}

#[test]
fn putting_a_held_grant_back_as_it_was_lifts_the_hold() {
    let (rec, _) = held_deploy();
    let (p, sha) = base();
    let (rec, events) = sync(Some(rec), &p, &sha, NOW + 2);
    assert_eq!(kinds(&events), vec![("reverted", Some("deploy"), false)]);
    assert_eq!(rec.grants["deploy"].state(), State::Honoured);
}

#[test]
fn a_record_that_cannot_be_read_holds_every_grant() {
    let (p, _) = base();
    assert!(hold_all(&p).grants.values().all(|g| g.hold == Some(Hold::Awaiting)));
}

#[test]
fn the_record_is_kept_beside_the_file_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let at = path_beside(&dir.path().join("aws-grants.json"));
    assert_eq!(at.file_name().unwrap(), "aws-grants.record.json");
    assert_eq!(read(&at).unwrap(), None, "no file is a first run");
    let rec = baselined();
    write(&at, &rec).unwrap();
    assert_eq!(read(&at).unwrap(), Some(rec.clone()));
    std::fs::write(&at, "{ not json").unwrap();
    assert!(read(&at).unwrap_err().contains("doesn't parse"));
    std::fs::write(&at, serde_json::to_string(&Record { version: 9, ..rec }).unwrap()).unwrap();
    assert!(read(&at).unwrap_err().contains("version 9"));
}

#[test]
fn the_record_never_holds_a_recipe_or_a_credential() {
    let text = serde_json::to_string(&baselined()).unwrap();
    assert!(!text.contains("/opt/escalate") && !text.contains("{reason}"), "{text}");
}

#[test]
fn the_list_says_when_each_grant_was_added_and_by_which_change() {
    let (mut rec, change) = held_deploy();
    let v = view(&rec);
    let deploy = v.grants.iter().find(|g| g.name == "deploy").unwrap();
    assert_eq!((deploy.state, deploy.added_change.clone()), ("held", format!("c{NOW:x}-1")), "added by the baseline, changed since");
    assert_eq!(deploy.change, change);
    assert_eq!(v.grants.iter().find(|g| g.name == "read").unwrap().allowed_by.as_deref(), Some(BASELINE_BY));
    assert_eq!(v.changes[0].kind, "widened", "newest first");
    decide(&mut rec, &answer(&change, true, "ok then"), NOW + 9).unwrap();
    assert_eq!(view(&rec).changes[0].kind, "allowed");
}

#[test]
fn the_change_list_is_bounded() {
    let mut rec = baselined();
    for n in 0..(KEEP_CHANGES + 30) {
        rec.changes.push(ChangeRow { id: format!("c{n}"), at_ms: n as i64, kind: "added".into(), grant: None, text: String::new(), by: None });
    }
    let (rec, _) = finish(rec, Vec::new(), NOW);
    assert_eq!(rec.changes.len(), KEEP_CHANGES);
    assert_eq!(rec.changes.last().unwrap().id, format!("c{}", KEEP_CHANGES + 29), "the newest are kept");
}
