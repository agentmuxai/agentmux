// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Each step alone, as a table: apply gives state X, applying again keeps X
//! (idempotent), and the undo gives back the before-image, file bytes and
//! database rows alike.

use super::super::backup;
use super::super::journal::{Step, STEPS};
use super::super::steps::{self, Agent, Ctx};
use super::fixture::*;

fn loap2() -> Agent {
    Agent { uid: LOAP2.into(), name: "Loap #2".into(), old: "loap-loap-2".into(), new: "loap-2".into() }
}

#[test]
fn each_step_is_idempotent_and_undoes_to_its_before_image() {
    for step in STEPS {
        let fx = Fx::standard();
        let installs = fx.home.installs().unwrap();
        let a = loap2();
        let run_dir = fx.home.backups().join("slug-migration-test");
        let entries = {
            let ctx = Ctx { home: &fx.home, installs: &installs, run_dir: run_dir.clone(), backups: &[], retired_at: 7 };
            backup::take(&run_dir, &super::super::run::backup_list(&ctx, std::slice::from_ref(&a)).unwrap()).unwrap()
        };
        let ctx = Ctx { home: &fx.home, installs: &installs, run_dir, backups: &entries, retired_at: 7 };
        let original = fx.snapshot();
        let before = steps::before(&ctx, &a, step).unwrap();
        let changes = steps::apply(&ctx, &a, step, &before).unwrap();
        let x = fx.snapshot();
        if step != Step::S5 {
            assert_ne!(x, original, "{step:?} changed something: {changes:?}");
        }
        let again = steps::apply(&ctx, &a, step, &before).unwrap();
        assert_eq!(fx.snapshot(), x, "{step:?} applied twice is the same state");
        if step != Step::S5 {
            assert!(again.is_empty(), "{step:?} reports no new change: {again:?}");
        }
        steps::undo(&ctx, &a, step, &before).unwrap();
        let back = fx.snapshot();
        let diff: Vec<_> = original.iter().filter(|(k, v)| back.get(*k) != Some(*v)).map(|(k, _)| k).collect();
        let extra: Vec<_> = back.keys().filter(|k| !original.contains_key(*k)).collect();
        assert!(diff.is_empty() && extra.is_empty(), "{step:?} undo left {diff:?} changed and {extra:?} new");
    }
}

/// S1 writes the slug through the record lock, not `upsert`, which would
/// keep the old one; the old one is recorded lowercased with the run's time.
#[test]
fn s1_renames_the_shared_definition_and_keeps_the_old_slug() {
    let fx = Fx::standard();
    let installs = fx.home.installs().unwrap();
    let ctx = Ctx { home: &fx.home, installs: &installs, run_dir: Default::default(), backups: &[], retired_at: 1234 };
    let a = loap2();
    let before = steps::before(&ctx, &a, Step::S1).unwrap();
    steps::apply(&ctx, &a, Step::S1, &before).unwrap();
    let rec = read_json(&fx.home.definitions().join(format!("{LOAP2}.json")));
    assert_eq!(rec["data"]["slug"], "loap-2");
    assert_eq!(rec["data"]["former_slugs"], serde_json::json!([{ "slug": "loap-loap-2", "retired_at": 1234 }]));
    // A later mirror write from a channel that still holds the old slug can't put it back (P0.2).
    let store = fx.store(&fx.home.root.join("channels/stable/versions/0.59.1/data"));
    store.set_def_registry(fx.defs.clone());
    let mut def = store.agent_def_get(LOAP2).unwrap().unwrap();
    def.description = "edited".into();
    store.agent_def_update(&mut def).unwrap();
    assert_eq!(read_json(&fx.home.definitions().join(format!("{LOAP2}.json")))["data"]["slug"], "loap-2");
}

/// S3 rotates: the old name's keys are gone everywhere, the old name is
/// tombstoned, and another agent's keys are untouched.
#[test]
fn s3_deletes_only_the_old_names_keys_and_tombstones_it() {
    let fx = Fx::standard();
    let installs = fx.home.installs().unwrap();
    let ctx = Ctx { home: &fx.home, installs: &installs, run_dir: Default::default(), backups: &[], retired_at: 1 };
    let a = loap2();
    let before = steps::before(&ctx, &a, Step::S3).unwrap();
    steps::apply(&ctx, &a, Step::S3, &before).unwrap();
    for db in ctx.objects_dbs() {
        assert_eq!(query::<String>(&db, "SELECT agent_id FROM db_agent_jekt_keys WHERE agent_id = 'loap-loap-2'"), None);
        assert_eq!(
            query::<String>(&db, "SELECT deleted_uid FROM db_agent_key_tombstones WHERE name = 'loap-loap-2'").as_deref(),
            Some(LOAP2)
        );
    }
    let stable_a = fx.home.root.join("channels/stable/versions/0.59.1/data/db/objects.db");
    assert_eq!(query::<String>(&stable_a, "SELECT agent_id FROM db_agent_lan_keys WHERE agent_id = 'naki'").as_deref(), Some("naki"));
}

/// Undoing S3 after the agent relaunched under its new id drops the keys it
/// minted there, so no later agent of that name inherits them, and puts the
/// old name's back.
#[test]
fn s3_undo_drops_keys_minted_under_the_new_name() {
    let fx = Fx::standard();
    let installs = fx.home.installs().unwrap();
    let a = loap2();
    let run_dir = fx.home.backups().join("slug-migration-test");
    let entries = {
        let ctx = Ctx { home: &fx.home, installs: &installs, run_dir: run_dir.clone(), backups: &[], retired_at: 1 };
        backup::take(&run_dir, &super::super::run::backup_list(&ctx, std::slice::from_ref(&a)).unwrap()).unwrap()
    };
    let ctx = Ctx { home: &fx.home, installs: &installs, run_dir, backups: &entries, retired_at: 1 };
    let before = steps::before(&ctx, &a, Step::S3).unwrap();
    steps::apply(&ctx, &a, Step::S3, &before).unwrap();
    let stable_a = fx.home.root.join("channels/stable/versions/0.59.1/data");
    fx.keys(&stable_a, "loap-2");
    steps::undo(&ctx, &a, Step::S3, &before).unwrap();
    let db = stable_a.join("db/objects.db");
    assert_eq!(query::<String>(&db, "SELECT agent_id FROM db_agent_jekt_keys WHERE agent_id = 'loap-2'"), None);
    assert_eq!(query::<String>(&db, "SELECT hmac_key FROM db_agent_jekt_keys WHERE agent_id = 'loap-loap-2'").as_deref(), Some("hmac-loap-loap-2"));
}

/// S6 gives each name-only row the new name and the uid; S7 rewrites the
/// identity in `cmd:env` and the layout snapshot, keeps every other variable,
/// and retargets the workspace `.mcp.json` without its old key.
#[test]
fn s6_and_s7_rewrite_names_and_keep_everything_else() {
    let fx = Fx::standard();
    let installs = fx.home.installs().unwrap();
    let ctx = Ctx { home: &fx.home, installs: &installs, run_dir: Default::default(), backups: &[], retired_at: 1 };
    let a = loap2();
    for step in [Step::S6, Step::S7] {
        let before = steps::before(&ctx, &a, step).unwrap();
        steps::apply(&ctx, &a, step, &before).unwrap();
    }
    let store_db = fx.home.shared.join("store.db");
    assert_eq!(query::<String>(&store_db, "SELECT target || '/' || target_uid FROM db_cron_jobs WHERE id = 'cron-1'").as_deref(), Some(&*format!("loap-2/{LOAP2}")));
    assert_eq!(query::<String>(&store_db, "SELECT target_uid FROM db_cron_jobs WHERE id = 'cron-2'").as_deref(), Some(""));
    assert_eq!(query::<String>(&store_db, "SELECT target || '/' || target_uid FROM db_jobs WHERE id = 'job-1'").as_deref(), Some(&*format!("loap-2/{LOAP2}")));
    assert_eq!(query::<String>(&store_db, "SELECT target_uid FROM db_jobs WHERE id = 'job-2'").as_deref(), Some(""));
    let queue = fx.home.shared.join("identity-store.db");
    assert_eq!(query::<String>(&queue, "SELECT target_agent_uid FROM db_work_queue WHERE id = 'w-1'").as_deref(), Some(LOAP2));
    let stable_a = fx.home.root.join("channels/stable/versions/0.59.1/data/db/objects.db");
    assert_eq!(query::<String>(&stable_a, "SELECT granted_peer_agent_id FROM db_conversation_trust_grants").as_deref(), Some("loap-2"));
    let stable_b = fx.home.root.join("channels/stable/versions/0.59.2/data/db/objects.db");
    let block: serde_json::Value = serde_json::from_slice(&query::<Vec<u8>>(&stable_b, "SELECT data FROM db_block WHERE oid = 'block-loap2'").unwrap()).unwrap();
    let env = &block["meta"]["cmd:env"];
    assert_eq!(env["AGENTMUX_AGENT_ID"], "loap-2");
    assert_eq!(env["GIT_AUTHOR_EMAIL"], "loap-2@agentmux.local");
    assert_eq!(env["API_TOKEN"], "keep-me");
    let client: serde_json::Value =
        serde_json::from_slice(&query::<Vec<u8>>(&stable_b, "SELECT data FROM db_client WHERE oid = 'client-block-loap2'").unwrap()).unwrap();
    assert_eq!(client["meta"]["session:last_topology"]["windows"][0]["tabs"][0]["blocks"][0]["meta"]["cmd:env"]["AGENTMUX_AGENT_SLUG"], "loap-2");
    let mcp = read_json(&fx.root().join("ws/loap-2/.mcp.json"));
    assert_eq!(mcp["mcpServers"]["agentmux"]["env"]["AGENTMUX_AGENT_ID"], "loap-2");
    assert!(mcp["mcpServers"]["agentmux"]["env"].get("AGENTMUX_JEKT_KEY").is_none(), "the old name's key goes");
    assert_eq!(mcp["mcpServers"]["mine"]["command"], "my-server");
}

/// S8 keeps both folders when the new name's exists, and moves nothing.
#[test]
fn s8_keeps_both_folders_when_the_target_exists() {
    let fx = Fx::standard();
    let config = fx.root().join("channels/stable/config");
    std::fs::create_dir_all(config.join("gh-loap-2")).unwrap();
    let installs = fx.home.installs().unwrap();
    let ctx = Ctx { home: &fx.home, installs: &installs, run_dir: Default::default(), backups: &[], retired_at: 1 };
    let a = loap2();
    let before = steps::before(&ctx, &a, Step::S8).unwrap();
    let changes = steps::apply(&ctx, &a, Step::S8, &before).unwrap();
    assert!(config.join("gh-loap-loap-2").is_dir() && config.join("gh-loap-2").is_dir());
    assert!(changes.iter().any(|c| c.contains("kept beside")), "{changes:?}");
    assert!(fx.home.bashwrap_dir.join("loap-2.cwd").is_file(), "the cwd file still moves");
}
