// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry's history: what every row says, the versions, who may read it, and the hash chain
//! (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §5, §11).

use super::history::{self, Asker, Head, Why};
use super::vaults::{self, VaultAgents};
use super::{create, create_in, delete, generate_entry, record, record_row, reveal, seal, update, EntryChange, GenerateRequest, NewEntry};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultActivityRow;

const SECRET: &str = "sk_live_history_test_value_0042";

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn key(label: &str, value: &str) -> NewEntry {
    NewEntry {
        kind: "api-key".into(),
        label: label.into(),
        fields: Default::default(),
        tags: vec![],
        parts: seal::Parts([("value".to_string(), value.to_string())].into()),
    }
}

fn change(id: &str, label: &str, value: Option<&str>) -> EntryChange {
    EntryChange {
        id: id.into(),
        label: label.into(),
        fields: Default::default(),
        tags: vec![],
        parts: seal::Parts(value.map(|v| ("value".to_string(), v.to_string())).into_iter().collect()),
        clear: vec![],
    }
}

fn person(s: &Store, id: &str) -> Vec<(String, i64)> {
    let mut rows = history::history_for(s, id, Asker::Person, 100).unwrap();
    rows.reverse();
    rows.into_iter().map(|r| (r.action, r.version)).collect()
}

#[test]
fn every_row_says_who_where_and_under_which_version_of_the_value() {
    let s = store();
    let id = create(&s, key("Stripe", SECRET)).unwrap();
    update(&s, change(&id, "Stripe live", None)).unwrap();
    update(&s, change(&id, "Stripe live", Some("sk_live_second"))).unwrap();
    record(&s, &id, "Stripe live", "builder", "ran a command", "deploy", "done");
    record(&s, &id, "Stripe live", "builder", "ran a command", "deploy", "denied");
    update(&s, change(&id, "Stripe", Some("sk_live_third"))).unwrap();
    record(&s, &id, "Stripe", "builder", "ran a command", "deploy", "done");

    assert_eq!(
        person(&s, &id),
        [
            ("created", 1),
            ("changed", 0),
            ("replaced", 2),
            ("ran a command", 2),
            ("ran a command", 2),
            ("replaced", 3),
            ("ran a command", 3),
        ]
        .map(|(a, v)| (a.to_string(), v)),
        "a new value is the next version; a use is under the version then current; details are 0"
    );
    let rows = history::history_for(&s, &id, Asker::Person, 100).unwrap();
    let by_builder = rows.iter().find(|r| r.actor == "builder").unwrap();
    assert_eq!(by_builder.actor_kind, "agent");
    assert!(rows.iter().filter(|r| r.actor == "you").all(|r| r.actor_kind == "person"));
    assert!(rows.iter().find(|r| r.action == "ran a command").unwrap().is_use);
    assert!(!rows.iter().find(|r| r.action == "replaced").unwrap().is_use);
}

#[test]
fn a_reason_and_references_are_kept_clean_and_clipped() {
    let s = store();
    let id = create(&s, key("Stripe", SECRET)).unwrap();
    let long = format!("rotating after the leak\n  for   task #27968 {}", "x".repeat(400));
    record_row(
        &s,
        VaultActivityRow {
            entry_id: id.clone(),
            entry_label: "Stripe".into(),
            actor: "builder".into(),
            action: "rotated".into(),
            outcome: "done".into(),
            reason: long,
            refs: history::refs_text(&["#27968".into(), "  ".into(), "https://example.com/pr/1".into(), "c".into(), "d".into()]),
            ..Default::default()
        },
    );
    let row = history::history_for(&s, &id, Asker::Person, 1).unwrap().remove(0);
    assert_eq!(row.reason.chars().count(), history::MAX_REASON);
    assert!(row.reason.starts_with("rotating after the leak for task #27968 xxx"), "{}", row.reason);
    assert!(!row.reason.contains('\n'));
    assert_eq!(row.refs, ["#27968", "https://example.com/pr/1", "c"], "an empty one is dropped and three are kept");
    assert_eq!(row.version, 2, "a rotation is the next version");
    assert_eq!(history::refs_of("not json"), Vec::<String>::new());
}

#[test]
fn a_row_never_holds_a_value() {
    let s = store();
    let id = create(&s, key("Stripe", SECRET)).unwrap();
    update(&s, change(&id, "Stripe", Some("sk_live_second_value"))).unwrap();
    reveal(&s, &id, "value", "revealed").unwrap();
    delete(&s, &id).unwrap();
    let all = s.vault_activity_all().unwrap();
    assert!(all.len() >= 4);
    for r in all {
        let text = format!("{r:?}");
        for value in [SECRET, "sk_live_second_value", &SECRET[SECRET.len() - 4..]] {
            // A value's last four characters could be common text, so only the long ones are searched for whole.
            if value.len() > 4 {
                assert!(!text.contains(value), "{text}");
            }
        }
    }
}

#[test]
fn a_deleted_entrys_rows_stay_and_a_reused_id_starts_again() {
    let s = store();
    let id = create(&s, key("Stripe", SECRET)).unwrap();
    update(&s, change(&id, "Stripe", Some("sk_live_second"))).unwrap();
    delete(&s, &id).unwrap();
    assert_eq!(s.vault_activity_history(&id, 0, 100).unwrap().len(), 3, "created, replaced, deleted stay");
    assert!(history::history_for(&s, &id, Asker::Person, 10).is_err(), "no entry to read it through any more");

    std::thread::sleep(std::time::Duration::from_millis(5));
    let again = create(&s, key("Stripe", SECRET)).unwrap();
    assert_eq!(again, id, "the id came round");
    assert_eq!(person(&s, &again), [("created".to_string(), 1)], "the first entry's rows aren't lent to the second, and it is version 1");
}

#[test]
fn an_agent_reads_the_history_only_of_entries_in_vaults_it_may_use() {
    let s = store();
    let open = create(&s, key("Open key", SECRET)).unwrap();
    let work = vaults::create(&s, "Work", VaultAgents::Named(vec!["builder".into()])).unwrap();
    let shut = vaults::create(&s, "Mine", VaultAgents::None).unwrap();
    let worked = create_in(&s, key("Work key", SECRET), &work.id).unwrap();
    let hidden = create_in(&s, key("Hidden key", SECRET), &shut.id).unwrap();

    let asks = |agent: Option<&str>, id: &str| history::history_for(&s, id, Asker::Agent(agent), 10);
    assert_eq!(asks(Some("builder"), &open).unwrap().len(), 1);
    assert_eq!(asks(Some("builder"), &worked).unwrap().len(), 1);
    assert_eq!(asks(Some("other"), &worked).unwrap_err(), format!("no entry {worked}"));
    assert_eq!(asks(None, &worked).unwrap_err(), format!("no entry {worked}"), "an agent that doesn't prove who it is sees what every agent may");
    assert_eq!(asks(Some("builder"), &hidden).unwrap_err(), format!("no entry {hidden}"));
    assert_eq!(asks(Some("builder"), "nothing-here").unwrap_err(), "no entry nothing-here", "the same words for an id that isn't there");
    assert_eq!(history::history_for(&s, &hidden, Asker::Person, 10).unwrap().len(), 1, "the person reads all of it");
}

fn chain_of(s: &Store) -> super::history::VaultChain {
    history::check(s).unwrap()
}

fn busy_store() -> (Store, String) {
    let s = store();
    let id = create(&s, key("Stripe", SECRET)).unwrap();
    for n in 0..4 {
        update(&s, change(&id, "Stripe", Some(&format!("sk_live_{n}")))).unwrap();
    }
    record(&s, &id, "Stripe", "builder", "ran a command", "deploy", "done");
    (s, id)
}

#[test]
fn the_chain_holds_and_each_row_carries_the_one_before() {
    let (s, _) = busy_store();
    let chain = chain_of(&s);
    assert!(chain.intact, "{chain:?}");
    assert_eq!((chain.checked, chain.before_chain), (6, 0));
    let rows = s.vault_activity_all().unwrap();
    assert_eq!(rows[0].prev_hash, "");
    for pair in rows.windows(2) {
        assert_eq!(pair[1].prev_hash, pair[0].hash);
        assert_eq!(pair[1].seq, pair[0].seq + 1);
    }
}

#[test]
fn changing_removing_or_slipping_in_a_row_is_found_at_the_right_one() {
    let edits: [(&str, &str, i64); 5] = [
        ("an edited row", "UPDATE db_vault_activity SET actor = 'someone' WHERE seq = 3", 3),
        ("a removed row", "DELETE FROM db_vault_activity WHERE seq = 3", 4),
        (
            "a row slipped in",
            "INSERT INTO db_vault_activity (seq, at, entry_id, action, outcome, hash) VALUES (50, 1, 'x', 'revealed', 'done', 'forged')",
            50,
        ),
        ("a rewritten reason", "UPDATE db_vault_activity SET reason = 'it was fine' WHERE seq = 5", 5),
        ("a renamed entry", "UPDATE db_vault_activity SET entry_label = 'Something else' WHERE seq = 2", 2),
    ];
    for (what, sql, at) in edits {
        let (s, _) = busy_store();
        s.vault_run_sql_for_tests(sql);
        let chain = chain_of(&s);
        assert!(!chain.intact, "{what} should break the chain");
        assert_eq!(chain.broken_at, Some(at), "{what}: {chain:?}");
        assert!(!chain.problem.is_empty());
    }
}

#[test]
fn the_head_in_the_keychain_finds_what_the_table_alone_cannot() {
    let (s, _) = busy_store();
    let rows = s.vault_activity_all().unwrap();
    let last = rows.last().unwrap();
    let head = Head { seq: last.seq, hash: last.hash.clone() };
    let held = history::verify_chain(&rows, Some(&head));
    assert!(held.intact && held.head_checked);
    assert!(!history::verify_chain(&rows, None).head_checked, "without the keychain's copy the chain says the head wasn't compared");
    assert!(history::verify_chain(&rows, Some(&Head { seq: last.seq - 2, hash: rows[rows.len() - 3].hash.clone() })).intact, "the head may lag");

    // The newest rows were deleted: the rest still chains, but the head's row is gone.
    let kept = &rows[..rows.len() - 2];
    assert!(history::verify_chain(kept, None).intact);
    let cut = history::verify_chain(kept, Some(&head));
    assert!((!cut.intact, cut.broken_at) == (true, Some(last.seq)), "{cut:?}");

    // The head's row is there but isn't the one the keychain saw.
    let other = history::verify_chain(&rows, Some(&Head { seq: last.seq, hash: "0".repeat(64) }));
    assert_eq!(other.broken_at, Some(last.seq));
}

#[test]
fn rows_made_before_the_chain_are_counted_and_not_checked() {
    let s = store();
    s.vault_run_sql_for_tests(
        "INSERT INTO db_vault_activity (at, entry_id, entry_label, actor, action, target, outcome) VALUES (1, 'old', 'Old', 'you', 'created', '', 'done');
         INSERT INTO db_vault_activity (at, entry_id, entry_label, actor, action, target, outcome) VALUES (2, 'old', 'Old', 'you', 'copied', 'value', 'done');",
    );
    record(&s, "old", "Old", "you", "copied", "value", "done");
    let chain = chain_of(&s);
    assert!(chain.intact, "{chain:?}");
    assert_eq!((chain.before_chain, chain.checked), (2, 1));
    let rows = s.vault_activity_all().unwrap();
    assert_eq!((rows[0].reason.as_str(), rows[0].hash.as_str(), rows[0].version), ("", "", 0), "an old row is left as it was");
    assert_eq!(rows[2].seq, 3);
    assert_eq!(rows[2].version, 1, "old rows' versions were never kept, but their actions count: the old entry's `created` is its first version");

    // A row written after the chain began by an older build sharing the store has no hash: counted, not a change,
    // and the next row of this build still follows the last chained one.
    s.vault_run_sql_for_tests("INSERT INTO db_vault_activity (seq, at, entry_id, action, outcome) VALUES (9, 3, 'x', 'revealed', 'done')");
    let mixed = chain_of(&s);
    assert!(mixed.intact, "{mixed:?}");
    assert_eq!((mixed.checked, mixed.before_chain, mixed.unchained_after), (1, 2, 1));
    record(&s, "old", "Old", "you", "copied", "value", "done");
    let after = chain_of(&s);
    assert!(after.intact, "{after:?}");
    assert_eq!((after.checked, after.unchained_after), (2, 1));
}

#[test]
fn the_actor_kind_tells_a_person_an_agent_and_the_vault_itself() {
    assert_eq!(history::actor_kind("you"), "person");
    assert_eq!(history::actor_kind("Vault"), "srv");
    assert_eq!(history::actor_kind("agent2-6"), "agent");
}

fn generating(why: &str, reason: Option<&str>) -> GenerateRequest {
    GenerateRequest {
        kind: "api-key".into(),
        label: "Webhook secret".into(),
        why: why.into(),
        reason: reason.map(String::from),
        fields: Default::default(),
        rules: Default::default(),
        expires: None,
        vault: None,
    }
}

#[test]
fn a_reason_is_one_line_of_eight_to_three_hundred_characters() {
    assert_eq!(history::require_reason(Some("  deploy the docs
 site,  task 27968 ")).unwrap(), "deploy the docs site, task 27968");
    assert!(history::require_reason(None).unwrap_err().contains("a reason is required"));
    assert!(history::require_reason(Some("   ")).is_err());
    assert!(history::require_reason(Some("too short")).is_ok(), "nine characters is enough");
    assert!(history::require_reason(Some("short")).unwrap_err().contains("8 to 300"));
    assert!(history::require_reason(Some(&"x".repeat(300))).is_ok());
    assert!(history::require_reason(Some(&"x".repeat(301))).unwrap_err().contains("at most 300"));
}

#[test]
fn generate_refuses_a_missing_or_short_reason_before_it_makes_anything() {
    let s = store();
    for req in [generating("", None), generating("", Some("short")), generating("short", Some("   ")), generating("x", None)] {
        let err = generate_entry(&s, req, "builder").unwrap_err();
        assert!(err.contains("a reason is required"), "{err}");
    }
    assert_eq!(s.vault_count().unwrap(), 0, "nothing was made");
    assert!(s.vault_activity_all().unwrap().is_empty(), "nothing was recorded");
}

#[test]
fn generate_keeps_the_reason_in_the_history_and_why_still_works_as_the_older_name() {
    let s = store();
    let a = generate_entry(&s, generating("", Some("a webhook secret for task 27968")), "builder").unwrap();
    let b = generate_entry(&s, generating("the older name still counts", None), "builder").unwrap();
    let reason_of = |id: &str| history::history_for(&s, id, Asker::Person, 5).unwrap().into_iter().find(|r| r.action == "generated").unwrap();
    let (ra, rb) = (reason_of(&a.id), reason_of(&b.id));
    assert_eq!((ra.reason.as_str(), ra.actor.as_str(), ra.actor_kind.as_str(), ra.version), ("a webhook secret for task 27968", "builder", "agent", 1));
    assert_eq!(rb.reason, "the older name still counts");
    assert!(!ra.target.contains("task 27968"), "the reason is its own column, not the target");
}
