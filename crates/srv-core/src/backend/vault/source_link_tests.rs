// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry's machine copy: which places the link takes, who may change it, what a check answers and records, and
//! that the link stays with the entry through an export, an import, a move and a restored value. Every place named
//! here is invented.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;
use crate::backend::storage::vault::{VaultActivityRow, PERSONAL_VAULT};
use crate::backend::vault::import;
use crate::backend::vault::inspect::{self, UpdateRequest};
use crate::backend::vault::kdbx_export;
use crate::backend::vault::seal::Parts;
use crate::backend::vault::transfer::{self, Duplicates};
use crate::backend::vault::vaults::VaultAgents;
use crate::backend::vault::{create, list_for_agent, list_for_pane, update, versions, EntryChange, NewEntry};
use crate::identity::aws_broker::mint::tests::Fake;

const VALUE: &str = "linked-sentinel-6204-kv";
const OTHER_VALUE: &str = "linked-sentinel-9917-xw";
const REASON: &str = "check the staging copy before the deploy in task 28071";
const PASS: &str = "correct horse battery staple";

fn store() -> Arc<Store> {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    Arc::new(s)
}

fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// An AWS secret's key, in a region.
fn aws() -> Vec<(&'static str, &'static str)> {
    vec![(KIND, AWS_SECRET), (LOCATION, "staging/app"), (KEY, "db.password"), (REGION, "eu-west-1")]
}

fn add(s: &Store, kind: &str, part: &str, label: &str, f: &[(&str, &str)]) -> String {
    create(s, NewEntry { kind: kind.into(), label: label.into(), fields: fields(f), tags: vec![], parts: Parts([(part.to_string(), VALUE.to_string())].into()) }).unwrap()
}

fn key_entry(s: &Store, f: &[(&str, &str)]) -> String {
    add(s, "api-key", "value", "Staging database", f)
}

fn link_of(s: &Store, id: &str) -> Option<Link> {
    Link::of_row(&s.vault_get(id).unwrap().unwrap())
}

/// What the CLI prints for a JSON secret whose `db.password` is `value`.
fn secret_with(value: &str) -> String {
    serde_json::to_string(&format!(r#"{{"db":{{"password":"{value}"}},"user":"app"}}"#)).unwrap()
}

fn rows(s: &Store) -> Vec<VaultActivityRow> {
    s.vault_activity_all().unwrap()
}

/// Neither value, in any history row or answer.
fn nothing_leaked(s: &Store, texts: &[String]) {
    for row in rows(s) {
        let row = format!("{row:?}");
        assert!(!row.contains(VALUE) && !row.contains(OTHER_VALUE), "a history row holds a value: {row}");
    }
    for t in texts {
        assert!(!t.contains(VALUE) && !t.contains(OTHER_VALUE), "a value is in {t}");
    }
}

fn asked(id: &str, f: &[(&str, &str)]) -> UpdateRequest {
    UpdateRequest { id: id.into(), label: None, fields: fields(f), tags: None, reason: Some(REASON.into()) }
}

#[test]
fn a_link_is_a_known_kind_of_place_written_the_way_that_place_is() {
    for ok in [
        aws(),
        // A secret's full name has colons, as an ARN does.
        vec![(KIND, AWS_SECRET), (LOCATION, "staging/app-AbCdEf:with:colons"), (PROFILE, "work-read")],
        vec![(KIND, GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "DEPLOY_TOKEN")],
        vec![(KIND, GITHUB_SECRET), (LOCATION, "example-org/example-app@release"), (KEY, "STORE_KEY")],
        vec![(KIND, OTHER), (LOCATION, "the build server's settings page"), (KEY, "Deploy key")],
        vec![(KIND, OTHER), (LOCATION, "a sealed envelope in the office safe")],
        vec![],
    ] {
        assert_eq!(validate(&fields(&ok)), Ok(()), "{ok:?}");
    }
    for (bad, says) in [
        (vec![(LOCATION, "staging/app")], "needs source_kind"),
        (vec![(KIND, "ftp"), (LOCATION, "x")], "isn't a kind of machine copy"),
        (vec![(KIND, AWS_SECRET)], "needs source_location"),
        (vec![(KIND, AWS_SECRET), (LOCATION, "staging/app; rm -rf")], "name or ARN"),
        (vec![(KIND, AWS_SECRET), (LOCATION, "--profile")], "name or ARN"),
        (vec![(KIND, AWS_SECRET), (LOCATION, "staging/app"), (KEY, "db password")], "dotted path"),
        (vec![(KIND, AWS_SECRET), (LOCATION, "staging/app"), (REGION, "europe")], "AWS region"),
        (vec![(KIND, AWS_SECRET), (LOCATION, "staging/app"), (PROFILE, "a b")], "AWS profile"),
        (vec![(KIND, GITHUB_SECRET), (LOCATION, "example-app"), (KEY, "TOKEN")], "owner/repo"),
        (vec![(KIND, GITHUB_SECRET), (LOCATION, "example-org/example-app")], "the secret's name"),
        (vec![(KIND, GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "TOKEN"), (REGION, "eu-west-1")], "aws-secret only"),
        (vec![(KIND, OTHER), (LOCATION, "line one\nline two")], "one line"),
    ] {
        let err = validate(&fields(&bad)).unwrap_err();
        assert!(err.contains(says), "{bad:?}: {err}");
    }
}

#[test]
fn every_kind_of_entry_takes_the_link_and_no_other_unknown_field() {
    let s = store();
    for (kind, part) in [("api-key", "value"), ("login", "password"), ("note", "text"), ("ssn", "value")] {
        let id = add(&s, kind, part, &format!("A {kind}"), &aws());
        assert_eq!(link_of(&s, &id).unwrap().text(), "aws-secret:staging/app#db.password", "{kind}");
    }
    let refused = create(&s, NewEntry { kind: "note".into(), label: "N".into(), fields: fields(&[("source_owner", "x")]), tags: vec![], parts: Parts([("text".to_string(), "t".to_string())].into()) });
    assert!(refused.unwrap_err().contains("has no field"), "a name that only looks like the link's is still unknown");
    let bad = create(&s, NewEntry { kind: "note".into(), label: "N".into(), fields: fields(&[(KIND, "ftp"), (LOCATION, "x")]), tags: vec![], parts: Parts([("text".to_string(), "t".to_string())].into()) });
    assert!(bad.unwrap_err().contains("isn't a kind of machine copy"));
}

#[test]
fn an_agents_list_names_the_place_and_never_the_login_it_is_read_under() {
    let s = store();
    let mut with_profile = aws();
    with_profile.push((PROFILE, "work-read"));
    let linked = key_entry(&s, &with_profile);
    add(&s, "note", "text", "Plain note", &[]);
    let list = list_for_agent(&s, Some("builder")).unwrap();
    let entry = list.iter().find(|e| e.id == linked).unwrap();
    assert_eq!(entry.source, Some(AgentLink { kind: AWS_SECRET.into(), location: "staging/app".into(), key: "db.password".into(), checked: None, ..Default::default() }));
    assert!(list.iter().find(|e| e.label == "Plain note").unwrap().source.is_none());
    let shown = serde_json::to_string(&list).unwrap();
    assert!(shown.contains(r#""source":{"kind":"aws-secret","location":"staging/app","key":"db.password"}"#), "{shown}");
    assert!(!shown.contains("work-read") && !shown.contains(VALUE), "{shown}");

    // `show` gives the entry's plain fields, and the login is not one of them there either.
    let one = inspect::show(&s, &linked, Some("builder"), &Ok(seal::test_key()), crate::backend::vault::expiry::today()).unwrap();
    assert_eq!(one.fields, fields(&aws()));
    let shown = serde_json::to_string(&one).unwrap();
    assert!(!shown.contains("work-read") && !shown.contains(PROFILE) && !shown.contains(VALUE), "{shown}");
    assert_eq!(link_of(&s, &linked).unwrap().profile, "work-read", "the entry keeps it");
}

#[tokio::test]
async fn a_check_says_same_or_different_names_the_place_in_the_history_and_holds_no_value() {
    let s = store();
    let id = key_entry(&s, &aws());
    let plan = plan_for_agent(&s, &id, "builder", Some(REASON)).unwrap();
    let runner = Fake::ok(secret_with(VALUE));
    let same = check(&s, &runner, &plan, Via::Host).await;
    assert_eq!((same.outcome, same.why.as_str(), same.source.as_str(), same.id.as_str()), (SAME, "", "aws-secret:staging/app#db.password", id.as_str()));
    // The place and the region the link names are what the read was given.
    let args = runner.seen.lock().unwrap()[0].1.clone();
    let names = |value: &str| args.iter().any(|a| a == value);
    assert!(names("staging/app") && args.windows(2).any(|w| w[0] == "--region" && w[1] == "eu-west-1"), "{args:?}");

    let different = check(&s, &Fake::ok(secret_with(OTHER_VALUE)), &plan, Via::Host).await;
    assert_eq!((different.outcome, different.why.as_str()), (DIFFERENT, ""));
    // The same length and a shared start are still different, and nothing says how close it was.
    let near = format!("{}x", &VALUE[..VALUE.len() - 1]);
    assert_eq!(check(&s, &Fake::ok(secret_with(&near)), &plan, Via::Host).await.outcome, DIFFERENT);

    let lines: Vec<_> = rows(&s).into_iter().filter(|r| r.action == CHECKED).collect();
    assert_eq!(lines.iter().map(|r| r.outcome.as_str()).collect::<Vec<_>>(), [SAME, DIFFERENT, DIFFERENT]);
    assert!(lines.iter().all(|r| r.target == "aws-secret:staging/app#db.password" && r.actor == "builder" && r.reason == REASON && r.entry_id == id));
    nothing_leaked(&s, &[serde_json::to_string(&same).unwrap(), serde_json::to_string(&different).unwrap()]);
}

#[tokio::test]
async fn a_place_that_refuses_or_cant_be_read_back_is_the_answer_not_an_error() {
    let s = store();
    let id = key_entry(&s, &aws());
    let plan = plan_for_agent(&s, &id, "builder", Some(REASON)).unwrap();
    let denied = "An error occurred (AccessDeniedException) when calling the GetSecretValue operation: User is not authorized to perform that action";
    let refused = check(&s, &Fake::failing(denied), &plan, Via::Host).await;
    assert_eq!((refused.outcome, refused.why.as_str()), (COULD_NOT_READ, "AWS refused: this identity can't read that secret"));
    let no_key = check(&s, &Fake::ok(serde_json::to_string(r#"{"user":"app"}"#).unwrap()), &plan, Via::Host).await;
    assert_eq!((no_key.outcome, no_key.why.as_str()), (COULD_NOT_READ, "that secret has no key db.password"));

    // A CI secret is written, never read: the check says so and runs nothing.
    let ci = key_entry(&s, &[(KIND, GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "DEPLOY_TOKEN")]);
    let runner = Fake::ok(secret_with(VALUE));
    let unread = check(&s, &runner, &plan_for_agent(&s, &ci, "builder", Some(REASON)).unwrap(), Via::Host).await;
    assert_eq!((unread.outcome, unread.why.as_str(), runner.runs()), (COULD_NOT_READ, "this kind of place can't be read back from here", 0));

    let outcomes: Vec<String> = rows(&s).into_iter().filter(|r| r.action == CHECKED).map(|r| r.outcome).collect();
    assert_eq!(outcomes, ["could not read: AWS refused: this identity can't read that secret", "could not read: that secret has no key db.password", "could not read: this kind of place can't be read back from here"]);
    nothing_leaked(&s, &[]);
}

#[tokio::test]
async fn an_entry_deleted_or_remade_while_its_copy_was_read_is_not_compared() {
    let s = store();
    let id = key_entry(&s, &aws());
    let plan = plan_for_person(&s, &id).unwrap();
    crate::backend::vault::delete(&s, &id).unwrap();
    let gone = check(&s, &Fake::ok(secret_with(VALUE)), &plan, Via::Host).await;
    assert_eq!((gone.outcome, gone.why.as_str()), (COULD_NOT_READ, "the entry was deleted while its machine copy was read"));
}

#[tokio::test]
async fn agents_get_six_checks_of_an_entry_an_hour_between_them_and_the_person_is_not_counted() {
    let s = store();
    let id = key_entry(&s, &aws());
    let runner = Fake::ok(secret_with(VALUE));
    check(&s, &runner, &plan_for_person(&s, &id).unwrap(), Via::Host).await;
    for n in 0..AGENT_CHECKS_PER_HOUR {
        let agent = if n % 2 == 0 { "builder" } else { "deployer" };
        check(&s, &runner, &plan_for_agent(&s, &id, agent, Some(REASON)).unwrap(), Via::Host).await;
    }
    for agent in ["builder", "deployer", "a-third-agent"] {
        let err = plan_for_agent(&s, &id, agent, Some(REASON)).unwrap_err();
        assert!(err.contains("6 times by agents in the last hour"), "{agent}: {err}");
    }
    assert!(plan_for_person(&s, &id).is_ok(), "the person's own check is not limited");
    let other = key_entry(&s, &aws());
    assert!(plan_for_agent(&s, &other, "builder", Some(REASON)).is_ok(), "the limit is per entry");
}

#[tokio::test]
async fn checks_asked_for_together_count_toward_the_limit_before_any_has_finished() {
    let s = store();
    let id = key_entry(&s, &aws());
    let plan = |agent: &str| plan_for_agent(&s, &id, agent, Some(REASON));
    // Six checks allowed and none run yet. Counted by the history's lines alone, a seventh would be let in too.
    let mut planned: Vec<CheckPlan> = (0..AGENT_CHECKS_PER_HOUR).map(|_| plan("builder").unwrap()).collect();
    let err = plan("deployer").unwrap_err();
    assert!(err.contains("0 check(s) by agents in the last hour and 6 more running"), "{err}");
    assert!(plan_for_person(&s, &id).is_ok(), "the person's own check is not limited");
    assert!(plan_for_agent(&s, &key_entry(&s, &aws()), "builder", Some(REASON)).is_ok(), "the limit is per entry");

    // A check given up before it ran frees its place. One that ran keeps it, as its line in the history.
    drop(planned.pop());
    let ran = plan("deployer").unwrap();
    assert!(plan("builder").is_err());
    check(&s, &Fake::ok(secret_with(VALUE)), &ran, Via::Host).await;
    drop(ran);
    let err = plan("builder").unwrap_err();
    assert!(err.contains("1 check(s) by agents in the last hour and 5 more running"), "{err}");
    drop(planned);
    let rest: Vec<CheckPlan> = (0..AGENT_CHECKS_PER_HOUR - 1).map(|_| plan("builder").unwrap()).collect();
    assert!(plan("builder").is_err());
    // The check runs on a clone of its plan, which shares the place: dropping one of the two frees nothing.
    drop(rest[0].clone());
    assert!(plan("builder").is_err());
    drop(rest);
    assert!(plan("builder").is_ok());
}

#[test]
fn a_check_needs_a_link_a_reason_and_an_entry_the_agent_may_use() {
    let s = store();
    let plain = key_entry(&s, &[]);
    assert!(plan_for_agent(&s, &plain, "builder", Some(REASON)).unwrap_err().contains("has no machine copy recorded"));
    let id = key_entry(&s, &aws());
    assert!(plan_for_agent(&s, &id, "builder", None).unwrap_err().contains("a reason is required"));
    let private = crate::backend::vault::vaults::create(&s, "Private", VaultAgents::None).unwrap();
    crate::backend::vault::vaults::move_entry(&s, &id, &private.id).unwrap();
    assert_eq!(plan_for_agent(&s, &id, "builder", Some(REASON)).unwrap_err(), format!("no entry {id}"));
    assert!(plan_for_person(&s, &id).is_ok());
}

#[tokio::test]
async fn the_newest_check_is_shown_until_the_link_names_another_place() {
    let s = store();
    let id = key_entry(&s, &aws());
    check(&s, &Fake::ok(secret_with(OTHER_VALUE)), &plan_for_person(&s, &id).unwrap(), Via::Host).await;
    let (pane, _) = list_for_pane(&s).unwrap();
    let shown = pane[0].source_checked.as_ref().unwrap();
    assert!(shown.at > 0);
    assert_eq!(shown.outcome, DIFFERENT);
    assert_eq!(list_for_agent(&s, Some("builder")).unwrap()[0].source.as_ref().unwrap().checked.as_ref().unwrap().outcome, DIFFERENT);

    // The person points the link somewhere else: the old place's result says nothing about the new one.
    let mut moved = fields(&aws());
    moved.insert(LOCATION.into(), "production/app".into());
    update(&s, EntryChange { id: id.clone(), label: "Staging database".into(), fields: moved, tags: vec![], parts: Parts::default(), clear: vec![] }).unwrap();
    let (pane, _) = list_for_pane(&s).unwrap();
    assert!(pane[0].source_checked.is_none());
    assert!(list_for_agent(&s, Some("builder")).unwrap()[0].source.as_ref().unwrap().checked.is_none());
}

#[tokio::test]
async fn a_check_made_before_the_entry_last_changed_is_not_its_newest_check() {
    let s = store();
    let mut place = aws();
    place.push((PROFILE, "work-read"));
    let id = key_entry(&s, &place);
    let shown = |s: &Store| list_for_pane(s).unwrap().0[0].source_checked.clone().map(|c| c.outcome);
    let to_agents = |s: &Store| list_for_agent(s, Some("builder")).unwrap()[0].source.as_ref().unwrap().checked.clone().map(|c| c.outcome);
    let change = |f: BTreeMap<String, String>, parts: Parts| {
        // A change is later than the check before it, as it is outside a test.
        std::thread::sleep(std::time::Duration::from_millis(3));
        update(&s, EntryChange { id: id.clone(), label: "Staging database".into(), fields: f, tags: vec![], parts, clear: vec![] }).unwrap();
    };
    let same = Fake::ok(secret_with(VALUE));
    check(&s, &same, &plan_for_person(&s, &id).unwrap(), Via::Host).await;
    assert_eq!((shown(&s).as_deref(), to_agents(&s).as_deref()), (Some(SAME), Some(SAME)));

    // Only the region changes. The history names the place the same way, and another secret is read from now on.
    let mut f = fields(&place);
    f.insert(REGION.into(), "us-east-1".into());
    change(f.clone(), Parts::default());
    assert_eq!(link_of(&s, &id).unwrap().text(), "aws-secret:staging/app#db.password");
    assert_eq!((shown(&s), to_agents(&s)), (None, None));
    check(&s, &same, &plan_for_person(&s, &id).unwrap(), Via::Host).await;
    assert_eq!(shown(&s).as_deref(), Some(SAME));

    // Only the profile, which no history line names.
    f.insert(PROFILE.into(), "work-admin".into());
    change(f.clone(), Parts::default());
    assert_eq!((shown(&s), to_agents(&s)), (None, None));
    check(&s, &same, &plan_for_person(&s, &id).unwrap(), Via::Host).await;
    assert_eq!(shown(&s).as_deref(), Some(SAME));

    // The entry's own value: the "same" before it was a comparison with the old one.
    change(f, Parts([("value".to_string(), OTHER_VALUE.to_string())].into()));
    assert_eq!((shown(&s), to_agents(&s)), (None, None));
    check(&s, &same, &plan_for_person(&s, &id).unwrap(), Via::Host).await;
    assert_eq!((shown(&s).as_deref(), to_agents(&s).as_deref()), (Some(DIFFERENT), Some(DIFFERENT)));
}

#[tokio::test]
async fn a_check_whose_link_changed_while_it_was_read_says_nothing_about_the_new_one() {
    let s = store();
    let mut place = aws();
    place.push((PROFILE, "work-read"));
    let id = key_entry(&s, &place);
    let shown = |s: &Store| list_for_pane(s).unwrap().0[0].source_checked.clone().map(|c| c.outcome);
    let to_agents = |s: &Store| list_for_agent(s, Some("builder")).unwrap()[0].source.as_ref().unwrap().checked.clone().map(|c| c.outcome);
    let change = |name: &str, value: &str| {
        let mut f: BTreeMap<String, String> = serde_json::from_str(&s.vault_get(&id).unwrap().unwrap().fields).unwrap();
        f.insert(name.into(), value.into());
        update(&s, EntryChange { id: id.clone(), label: "Staging database".into(), fields: f, tags: vec![], parts: Parts::default(), clear: vec![] }).unwrap();
    };
    let changed = "could not read: where the entry's machine copy is was changed while it was read";

    // Planned for one region and finished after the link names another: the history names both places the same way,
    // and the line is written after the change. The copy that was read holds the entry's value, and that is not said.
    for (name, value) in [(REGION, "us-east-1"), (PROFILE, "work-admin")] {
        let stale = plan_for_agent(&s, &id, "builder", Some(REASON)).unwrap();
        change(name, value);
        assert_eq!(link_of(&s, &id).unwrap().text(), stale.link.text());
        let found = check(&s, &Fake::ok(secret_with(VALUE)), &stale, Via::Host).await;
        assert_eq!(found.outcome, COULD_NOT_READ, "{name}");
        assert!(found.why.contains("was changed while it was read"), "{}", found.why);
        assert!(shown(&s).unwrap().starts_with(changed) && to_agents(&s).unwrap().starts_with(changed), "{name}: {:?}", shown(&s));
    }

    // Nor is a refusal under the old login the new one's.
    let stale = plan_for_person(&s, &id).unwrap();
    change(PROFILE, "work-read");
    let refused = check(&s, &Fake::failing("An error occurred (AccessDeniedException) when calling the GetSecretValue operation"), &stale, Via::Host).await;
    assert!(refused.why.contains("was changed while it was read"), "{}", refused.why);

    // A check of the link as it is now answers for it.
    check(&s, &Fake::ok(secret_with(VALUE)), &plan_for_person(&s, &id).unwrap(), Via::Host).await;
    assert_eq!((shown(&s).as_deref(), to_agents(&s).as_deref()), (Some(SAME), Some(SAME)));
    nothing_leaked(&s, &[]);
}

#[test]
fn an_agent_changes_the_link_only_as_the_person_allowed_it() {
    let s = store();
    let id = key_entry(&s, &[("service", "Example")]);
    // Any other detail goes through as before, and asks nobody.
    assert_eq!(inspect::link_change(&s, &asked(&id, &[("service", "Example DB")]), "builder").unwrap(), None);
    inspect::update_details(&s, &asked(&id, &[("service", "Example DB")]), "builder").unwrap();

    let set = asked(&id, &aws());
    let refused = inspect::update_details(&s, &set, "builder").unwrap_err();
    assert!(refused.contains("the person is asked before it changes"), "{refused}");
    assert!(link_of(&s, &id).is_none());

    let change = inspect::link_change(&s, &set, "builder").unwrap().unwrap();
    assert_eq!((change.from.clone(), change.to.as_ref().map(Link::describe)), (None, Some("the AWS secret staging/app, key db.password, region eu-west-1".to_string())));
    assert_eq!((change.label.as_str(), change.reason.as_str()), ("Staging database", REASON));
    let done = inspect::update_details_allowed(&s, &set, "builder", &change).unwrap();
    assert!(done.changed.contains(&"field source_location".to_string()), "{:?}", done.changed);
    assert_eq!(link_of(&s, &id).unwrap().text(), "aws-secret:staging/app#db.password");

    // A change to one part is a change of place, shown with the place it has now.
    let repoint = asked(&id, &[(LOCATION, "production/app")]);
    let change = inspect::link_change(&s, &repoint, "builder").unwrap().unwrap();
    assert_eq!(change.from.as_ref().map(Link::text), Some("aws-secret:staging/app#db.password".to_string()));
    assert_eq!(change.to.as_ref().map(Link::text), Some("aws-secret:production/app#db.password".to_string()));
    // The person moved the link while they were asked: the yes was for another change, so nothing is written.
    let mut by_person = fields(&aws());
    by_person.insert(KEY.into(), "db.user".into());
    update(&s, EntryChange { id: id.clone(), label: "Staging database".into(), fields: by_person, tags: vec![], parts: Parts::default(), clear: vec![] }).unwrap();
    let stale = inspect::update_details_allowed(&s, &repoint, "builder", &change).unwrap_err();
    assert!(stale.contains("changed while the person was asked"), "{stale}");
    assert_eq!(link_of(&s, &id).unwrap().text(), "aws-secret:staging/app#db.user");

    // A new kind of place takes the old kind's region with it: the agent names the new place and nothing else.
    let to_github = asked(&id, &[(KIND, GITHUB_SECRET), (LOCATION, "acme/web@staging"), (KEY, "DEPLOY_TOKEN")]);
    let change = inspect::link_change(&s, &to_github, "builder").unwrap().unwrap();
    assert_eq!(change.to.as_ref().map(Link::describe), Some("the GitHub secret DEPLOY_TOKEN of acme/web@staging".to_string()));
    assert_eq!(change.to.as_ref().map(|l| l.region.as_str()), Some(""));
    // A region the update itself gives to a place that has none is still refused.
    let with_region = asked(&id, &[(KIND, GITHUB_SECRET), (LOCATION, "acme/web"), (KEY, "DEPLOY_TOKEN"), (REGION, "eu-west-1")]);
    let refused = inspect::link_change(&s, &with_region, "builder").unwrap_err();
    assert!(refused.contains("are for an aws-secret only"), "{refused}");
    // And a part changed without naming the kind keeps the region the AWS place has.
    let rekey = inspect::link_change(&s, &asked(&id, &[(KEY, "db.host")]), "builder").unwrap().unwrap();
    assert_eq!(rekey.to.as_ref().map(|l| l.region.as_str()), Some("eu-west-1"));

    // Taking the link off is a change too: an empty kind is enough. A refusal is a line in the entry's history.
    let clear = asked(&id, &[(KIND, "")]);
    let change = inspect::link_change(&s, &clear, "builder").unwrap().unwrap();
    assert!(change.to.is_none() && change.from.is_some());
    let spelled_out = asked(&id, &[(KIND, ""), (LOCATION, ""), (KEY, ""), (REGION, "")]);
    assert_eq!(inspect::link_change(&s, &spelled_out, "builder").unwrap().unwrap(), change);
    inspect::note_link_refused(&s, &change, "builder", "denied");
    let line = rows(&s).into_iter().find(|r| r.outcome == "denied").unwrap();
    assert_eq!((line.action.as_str(), line.target.as_str(), line.actor.as_str(), line.reason.as_str()), (inspect::CHANGED_DETAILS, "machine copy", "builder", REASON));
}

#[test]
fn a_vault_file_brings_the_link_back_and_a_replacing_import_never_drops_one() {
    let s = store();
    let id = key_entry(&s, &aws());
    let dir = tempfile::tempdir().unwrap();
    let linked = dir.path().join("linked.vault");
    transfer::export(&s, None, PASS, &linked).unwrap();

    // Into an empty Vault: the entry comes back with its link.
    s.vault_delete(&id).unwrap();
    let preview = transfer::inspect(&s, &linked, Some(PASS)).unwrap();
    transfer::commit(&s, &preview.token, Duplicates::Skip, PERSONAL_VAULT).unwrap();
    assert_eq!(link_of(&s, &id).unwrap().text(), "aws-secret:staging/app#db.password");

    // A file made before the entry had a link replaces the value and leaves the link where it is.
    let plain = dir.path().join("plain.vault");
    update(&s, EntryChange { id: id.clone(), label: "Staging database".into(), fields: Default::default(), tags: vec![], parts: Parts::default(), clear: vec![] }).unwrap();
    assert!(link_of(&s, &id).is_none());
    transfer::export(&s, None, PASS, &plain).unwrap();
    update(&s, EntryChange { id: id.clone(), label: "Staging database".into(), fields: fields(&aws()), tags: vec![], parts: Parts::default(), clear: vec![] }).unwrap();
    let preview = transfer::inspect(&s, &plain, Some(PASS)).unwrap();
    let done = transfer::commit(&s, &preview.token, Duplicates::Replace, PERSONAL_VAULT).unwrap();
    assert_eq!(done.replaced, 1);
    assert_eq!(link_of(&s, &id).unwrap().text(), "aws-secret:staging/app#db.password", "replace keeps the entry's own link");

    // A file that names a place wins over the entry's, and "keep both" gives the copy the file's link.
    let mut elsewhere = fields(&aws());
    elsewhere.insert(LOCATION.into(), "production/app".into());
    update(&s, EntryChange { id: id.clone(), label: "Staging database".into(), fields: elsewhere, tags: vec![], parts: Parts::default(), clear: vec![] }).unwrap();
    let preview = transfer::inspect(&s, &linked, Some(PASS)).unwrap();
    transfer::commit(&s, &preview.token, Duplicates::Replace, PERSONAL_VAULT).unwrap();
    assert_eq!(link_of(&s, &id).unwrap().location, "staging/app");
    let preview = transfer::inspect(&s, &linked, Some(PASS)).unwrap();
    let done = transfer::commit(&s, &preview.token, Duplicates::KeepBoth, PERSONAL_VAULT).unwrap();
    assert_eq!(done.added, 1);
    let copy = s.vault_list().unwrap().into_iter().find(|r| r.id != id).unwrap();
    assert_eq!(Link::of_row(&copy).unwrap().text(), "aws-secret:staging/app#db.password");
}

#[test]
fn a_keepass_export_carries_the_link_as_plain_fields_and_the_importer_reads_it_back() {
    let s = store();
    key_entry(&s, &aws());
    add(&s, "login", "password", "Example site", &[("site", "https://example.com"), (KIND, GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "SITE_PASSWORD")]);
    add(&s, "note", "text", "Recovery codes", &[(KIND, OTHER), (LOCATION, "a sealed envelope in the office safe")]);
    let entries = transfer::gather(&s, None).unwrap();
    let bytes = kdbx_export::build(&entries, PASS, keepass::config::KdfConfig::Aes { rounds: 100 }).unwrap();
    let back = match import::read_with(&bytes, "x.kdbx", None, Some(PASS)) {
        Ok(read) => read,
        Err(_) => panic!("the export doesn't read back"),
    };
    let link = |label: &str| Link::of(&back.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label}")).fields).map(|l| l.text());
    assert_eq!(link("Staging database").as_deref(), Some("aws-secret:staging/app#db.password"));
    assert_eq!(link("Example site").as_deref(), Some("github-secret:example-org/example-app#SITE_PASSWORD"));
    assert_eq!(link("Recovery codes").as_deref(), Some("other:a sealed envelope in the office safe"));
    // The link is fields, not note text beside the entry.
    assert!(back.entries.iter().all(|e| !e.parts.values().any(|v| v.contains("Machine copy"))), "{:?}", back.entries.iter().map(|e| &e.label).collect::<Vec<_>>());
    assert_eq!(back.entries.len(), 3);
}

#[test]
fn a_move_to_another_vault_and_a_restored_value_keep_the_link() {
    let s = store();
    let id = key_entry(&s, &aws());
    let work = crate::backend::vault::vaults::create(&s, "Work", VaultAgents::All).unwrap();
    crate::backend::vault::vaults::move_entry(&s, &id, &work.id).unwrap();
    assert_eq!(link_of(&s, &id).unwrap().text(), "aws-secret:staging/app#db.password");

    let new_value = Parts([("value".to_string(), OTHER_VALUE.to_string())].into());
    update(&s, EntryChange { id: id.clone(), label: "Staging database".into(), fields: fields(&aws()), tags: vec![], parts: new_value, clear: vec![] }).unwrap();
    let row = s.vault_get(&id).unwrap().unwrap();
    let earlier = s.vault_versions(&id, row.created_at).unwrap();
    versions::restore(&s, &id, earlier[0].seq).unwrap();
    assert_eq!(link_of(&s, &id).unwrap().text(), "aws-secret:staging/app#db.password");
    assert_eq!(seal::open(&seal::test_key(), &id, &s.vault_get(&id).unwrap().unwrap().sealed).unwrap().get("value"), Some(VALUE));
}
