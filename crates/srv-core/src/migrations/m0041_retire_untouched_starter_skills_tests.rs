// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The starter-skill cleanup deletes only an untouched, unbound starter skill.

use super::*;

/// The id the seed gave the "tdd" starter skill.
const TDD_ID: &str = "47153132-3a8b-5e13-a68a-e4e1aa1d038c";

/// The shipped text of one starter skill, kept here because the cleanup
/// recognises the six by hash alone.
const TDD_NAME: &str = "Test-Driven Development";
const TDD_DESCRIPTION: &str = "Use when implementing new behavior or fixing a bug — write the test that proves it first.";
const TDD_CONTENT: &str = "# Test-Driven Development\n\nInformed by the real-world, widely-used `test-driven-development` skill from obra/superpowers (github.com/obra/superpowers), one of the most popular Claude Code skill collections in the community.\n\n**Core principle:** write a test that fails for the right reason before writing the code that makes it pass. This forces you to know exactly what \"done\" looks like before you start, and leaves behind a regression test instead of just a fix.\n\n**When to use:** new features, bug fixes (write a test that reproduces the bug BEFORE fixing it — otherwise you can't prove the fix worked or prevent it regressing), and any change to logic that has edge cases.\n\n## The cycle\n\n1. **Red** — write a test for the behavior you want, and confirm it actually fails (for the right reason, not a typo or setup error).\n2. **Green** — write the minimum code to make it pass. Resist the urge to build more than the test demands.\n3. **Refactor** — clean up now that you have a passing test as a safety net.\n\n## What makes a good test\n\n- Covers the failure path and edge cases, not just the happy path — the happy path is the case least likely to break.\n- Fast and deterministic — no sleeps, no flaky timing assumptions, no dependence on execution order.\n- Tests behavior, not implementation — a test that breaks every time you refactor without changing behavior is testing the wrong thing.\n- One assertion of intent per test — a test with an unclear failure message is a test that wastes the next person's time.\n\nIf writing the test first feels awkward because the design isn't clear yet, that discomfort is useful signal — it usually means the design itself isn't settled, not that testing should be skipped.";

fn shipped_tdd() -> Skill {
    Skill {
        id: TDD_ID.to_string(),
        name: TDD_NAME.to_string(),
        trigger: "tdd".to_string(),
        skill_type: "prompt".to_string(),
        description: TDD_DESCRIPTION.to_string(),
        content: TDD_CONTENT.to_string(),
        is_global: true,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    }
}

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

/// The shared catalog as it really is: the identity-store schema, which holds
/// the skills and none of the ref tables.
fn identity_store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    std::mem::forget(dir);
    store
}

fn put(store: &Store, skill: &Skill) {
    store.skill_insert_raw(skill).unwrap();
}

fn bind_agent(store: &Store, skill_id: &str) {
    let conn = store.conn().lock().unwrap();
    // The ref tables key on agents and bundles this fixture does not create.
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    conn.execute("INSERT INTO db_agent_skills_ref (agent_id, skill_id) VALUES ('agent-1', ?1)", params![skill_id])
        .unwrap();
}

fn bind_bundle(store: &Store, skill_id: &str) {
    let conn = store.conn().lock().unwrap();
    // The ref tables key on agents and bundles this fixture does not create.
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    conn.execute("INSERT INTO db_bundle_skills_ref (bundle_id, skill_id) VALUES ('bundle-1', ?1)", params![skill_id])
        .unwrap();
}

fn run(mstore: &Store, identity: &Store) -> usize {
    retire_untouched_starters(mstore, Some(identity), &[]).unwrap()
}

#[test]
fn the_frozen_id_and_hash_match_the_shipped_skill() {
    assert_eq!(frozen_starter_skill_id("tdd"), TDD_ID);
    let (_, _, hash) = FROZEN_STARTERS.iter().find(|(_, t, _)| *t == "tdd").unwrap();
    assert_eq!(&fingerprint(&shipped_tdd()), hash);
}

#[test]
fn removes_an_untouched_unbound_starter_from_the_catalog_and_the_channel_copy() {
    let (mstore, identity) = (store(), identity_store());
    put(&identity, &shipped_tdd());
    put(&mstore, &shipped_tdd());

    assert_eq!(run(&mstore, &identity), 1);

    assert!(identity.skill_get(TDD_ID).unwrap().is_none());
    assert!(mstore.skill_get(TDD_ID).unwrap().is_none());
}

#[test]
fn removes_a_channel_copy_when_there_is_no_catalog_yet() {
    let mstore = store();
    put(&mstore, &shipped_tdd());

    assert_eq!(retire_untouched_starters(&mstore, None, &[]).unwrap(), 1);
    assert!(mstore.skill_get(TDD_ID).unwrap().is_none());
}

#[test]
fn keeps_one_whose_content_was_edited() {
    let (mstore, identity) = (store(), identity_store());
    let mut edited = shipped_tdd();
    edited.content.push_str("\nAlso: run the formatter.");
    put(&identity, &edited);

    assert_eq!(run(&mstore, &identity), 0);
    assert_eq!(identity.skill_get(TDD_ID).unwrap().unwrap().content, edited.content);
}

#[test]
fn keeps_one_that_was_renamed() {
    let (mstore, identity) = (store(), identity_store());
    let mut renamed = shipped_tdd();
    renamed.name = "How we test".to_string();
    put(&identity, &renamed);

    assert_eq!(run(&mstore, &identity), 0);
    assert!(identity.skill_get(TDD_ID).unwrap().is_some());
}

#[test]
fn keeps_one_bound_to_an_agent() {
    let (mstore, identity) = (store(), identity_store());
    put(&identity, &shipped_tdd());
    put(&mstore, &shipped_tdd());
    bind_agent(&mstore, TDD_ID);

    assert_eq!(run(&mstore, &identity), 0);
    assert!(identity.skill_get(TDD_ID).unwrap().is_some());
    assert!(mstore.skill_get(TDD_ID).unwrap().is_some());
}

#[test]
fn keeps_one_bound_to_a_bundle() {
    let (mstore, identity) = (store(), identity_store());
    put(&identity, &shipped_tdd());
    bind_bundle(&mstore, TDD_ID);

    assert_eq!(run(&mstore, &identity), 0);
    assert!(identity.skill_get(TDD_ID).unwrap().is_some());
}

#[test]
fn keeps_one_bound_under_the_random_id_an_older_channel_still_has() {
    let (mstore, identity) = (store(), identity_store());
    put(&identity, &shipped_tdd());
    let mut legacy = shipped_tdd();
    legacy.id = "random-legacy-id".to_string();
    put(&mstore, &legacy);
    bind_agent(&mstore, "random-legacy-id");

    assert_eq!(run(&mstore, &identity), 0);
    assert!(identity.skill_get(TDD_ID).unwrap().is_some());
}

#[test]
fn leaves_a_skill_the_user_made_with_a_starter_name() {
    let (mstore, identity) = (store(), identity_store());
    let mut theirs = shipped_tdd();
    theirs.id = "user-made".to_string();
    put(&identity, &theirs);

    assert_eq!(run(&mstore, &identity), 0);
    assert!(identity.skill_get("user-made").unwrap().is_some());
}

#[test]
fn running_twice_changes_nothing_more() {
    let (mstore, identity) = (store(), identity_store());
    put(&identity, &shipped_tdd());
    put(&mstore, &shipped_tdd());
    let mut kept = shipped_tdd();
    kept.id = "kept".to_string();
    kept.name = "Mine".to_string();
    put(&identity, &kept);

    assert_eq!(run(&mstore, &identity), 1);
    assert_eq!(run(&mstore, &identity), 0);

    assert!(identity.skill_get(TDD_ID).unwrap().is_none());
    assert!(identity.skill_get("kept").unwrap().is_some());
}

fn sibling_db(dir: &Path, bound: bool) -> PathBuf {
    let path = dir.join("objects.db");
    let sibling = Store::open(&path).unwrap();
    if bound {
        bind_agent(&sibling, TDD_ID);
    }
    path
}

#[test]
fn keeps_one_another_channel_binds() {
    let dir = tempfile::tempdir().unwrap();
    let sibling = sibling_db(dir.path(), true);
    let (mstore, identity) = (store(), identity_store());
    put(&identity, &shipped_tdd());

    assert_eq!(retire_untouched_starters(&mstore, Some(&identity), &[sibling]).unwrap(), 0);
    assert!(identity.skill_get(TDD_ID).unwrap().is_some());
}

#[test]
fn removes_one_when_another_channel_binds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let sibling = sibling_db(dir.path(), false);
    let (mstore, identity) = (store(), identity_store());
    put(&identity, &shipped_tdd());

    assert_eq!(retire_untouched_starters(&mstore, Some(&identity), &[sibling]).unwrap(), 1);
}

#[test]
fn keeps_one_when_another_channel_cannot_be_read() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gone.db");
    let (mstore, identity) = (store(), identity_store());
    put(&identity, &shipped_tdd());

    assert_eq!(retire_untouched_starters(&mstore, Some(&identity), &[missing]).unwrap(), 0);
    assert!(identity.skill_get(TDD_ID).unwrap().is_some());
}

#[test]
fn up_does_nothing_before_the_channel_store_exists() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = MigrationContext {
        home: dir.path().to_path_buf(),
        data_dir: dir.path().to_path_buf(),
        shared_store_path: dir.path().join("store.db"),
        channel_store_path: dir.path().join("absent.db"),
    };
    M0041RetireUntouchedStarterSkills.up(&ctx).unwrap();
    assert!(!ctx.channel_store_path.exists());
}

#[test]
fn the_six_frozen_ids_are_the_ones_the_seed_minted() {
    let minted = [
        ("systematic-debugging", "501c8a98-50fa-5dda-a2c8-e852f684b8e7"),
        ("tdd", "47153132-3a8b-5e13-a68a-e4e1aa1d038c"),
        ("code-review", "53c0c3cd-815e-5eaf-a531-6f43fcd39175"),
        ("commit-hygiene", "f708fa57-ccc7-5e9d-a740-5f66d1a0150b"),
        ("verification-before-completion", "0f5fb306-c780-52c4-82f6-6dc29cd095d5"),
        ("security-basics", "daedba7a-f268-58a5-9c9e-3481b190d040"),
    ];
    assert_eq!(minted.len(), FROZEN_STARTERS.len());
    for (trigger, id) in minted {
        assert_eq!(frozen_starter_skill_id(trigger), id);
        assert!(FROZEN_STARTERS.iter().any(|(_, t, _)| *t == trigger));
    }
}
