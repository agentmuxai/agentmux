// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The planner: §5.1's candidates, the collision check, and that a plan
//! writes nothing.

use super::fixture::*;
use crate::backend::rpc_types::ForkSlugCandidate;

fn by_name<'a>(cands: &'a [ForkSlugCandidate], name: &str) -> &'a ForkSlugCandidate {
    cands.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name} is a candidate"))
}

/// The three real shapes: each gets `fork_slug`'s id, parent first, and the
/// roots are left alone.
#[test]
fn the_three_real_chains_are_candidates_parent_first() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    let plan = super::super::plan(&fx.home, false).unwrap();
    let names: Vec<(&str, &str, &str)> =
        plan.candidates.iter().map(|c| (c.name.as_str(), c.old_slug.as_str(), c.new_slug.as_str())).collect();
    assert_eq!(
        names,
        vec![("Loap #2", "loap-loap-2", "loap-2"), ("Naki #2", "naki-naki-2", "naki-2"), ("Loap #3", "loap-loap-2-loap-3", "loap-3")]
    );
    assert!(plan.candidates.iter().all(|c| c.refused.is_none()), "{:?}", plan.candidates);
    assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
}

/// Agents made from a template keep the template's id as their `parent_id`:
/// they are lineage roots, not forks with a missing parent, so the chains
/// below them are found (the operator's home, v0.59.40).
#[test]
fn template_parents_are_lineage_roots() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.agent(&data, LOAP, "Loap", "loap", "claude", "", true);
    fx.agent(&data, LOAP2, "Loap #2", "loap-loap-2", LOAP, "Loap #2", true);
    fx.agent(&data, LOAP3, "Loap #3", "loap-loap-2-loap-3", LOAP2, "Loap #3", true);
    fx.agent(&data, NAKI, "Naki", "naki", "claude", "", true);
    fx.agent(&data, NAKI2, "Naki #2", "naki-naki-2", NAKI, "Naki #2", true);
    fx.agent(&data, "uid-first", "Solo", "solo", "kimi", "", true);
    let plan = super::super::plan(&fx.home, false).unwrap();
    let names: Vec<(&str, &str, &str)> =
        plan.candidates.iter().map(|c| (c.name.as_str(), c.old_slug.as_str(), c.new_slug.as_str())).collect();
    assert_eq!(
        names,
        vec![("Loap #2", "loap-loap-2", "loap-2"), ("Naki #2", "naki-naki-2", "naki-2"), ("Loap #3", "loap-loap-2-loap-3", "loap-3")]
    );
    assert!(plan.candidates.iter().all(|c| c.refused.is_none()), "{:?}", plan.candidates);
    assert!(plan.skipped.is_empty(), "first-generation agents aren't forks: {:?}", plan.skipped);
}

/// A collision suffix and a missing parent are listed as left alone; a fork
/// that already has its clean id isn't listed at all.
#[test]
fn a_suffix_and_a_missing_parent_are_not_candidates() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.agent(&data, LOAP, "Loap", "loap", "", "", true);
    fx.agent(&data, "uid-suffixed", "Loap #4", "loap-4-2", LOAP, "Loap #4", true);
    fx.agent(&data, "uid-orphan", "Orphan #2", "orphan-orphan-2", "uid-gone", "Orphan #2", true);
    fx.agent(&data, "uid-clean", "Loap #5", "loap-5", LOAP, "Loap #5", true);
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(plan.candidates.is_empty(), "{:?}", plan.candidates);
    let reasons: Vec<(&str, &str)> = plan.skipped.iter().map(|s| (s.name.as_str(), s.reason.as_str())).collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons.iter().any(|(n, r)| *n == "Loap #4" && r.starts_with("not a pre-fix chain")));
    assert!(reasons.iter().any(|(n, r)| *n == "Orphan #2" && r.contains("parent is missing")));
}

/// A target another uid holds in any `objects.db` on disk, even one the
/// shared definitions don't know, refuses that agent with the next free
/// suffix suggested; the other agents still can migrate.
#[test]
fn a_target_held_by_another_uid_is_refused_with_a_suggestion() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    let old = fx.version("local-x", "0.58.0");
    fx.agent(&old, "uid-squatter", "Someone", "loap-3", "", "", false);
    let plan = super::super::plan(&fx.home, false).unwrap();
    let loap3 = by_name(&plan.candidates, "Loap #3");
    assert!(loap3.refused.as_deref().is_some_and(|r| r.contains("uid-squatter")), "{:?}", loap3.refused);
    assert_eq!(loap3.suggestion.as_deref(), Some("loap-3-2"));
    assert!(by_name(&plan.candidates, "Loap #2").refused.is_none());
    assert!(by_name(&plan.candidates, "Naki #2").refused.is_none());
}

/// Another agent's former slug holds a target forever.
#[test]
fn another_agents_former_slug_holds_a_target() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    fx.agent(&data, "uid-old", "Old", "old", "", "", true);
    let path = fx.home.definitions().join("uid-old.json");
    let mut rec = read_json(&path);
    rec["data"]["former_slugs"] = serde_json::json!([{ "slug": "naki-2", "retired_at": 1 }]);
    std::fs::write(&path, serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(by_name(&plan.candidates, "Naki #2").refused.as_deref().is_some_and(|r| r.contains("renamed from it")));
}

/// A display name another agent was renamed from still reaches it by its
/// route, so it holds a target as a former slug does; the candidate's own
/// former display names don't.
#[test]
fn another_agents_former_display_name_holds_a_target() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    fx.agent(&data, "uid-old", "Old", "old", "", "", true);
    let db = data.join("db/objects.db");
    fx.exec(&db, &format!("INSERT INTO db_agent_former_names (agent_id, name, recorded_at) VALUES ('{LOAP3}', 'Loap #3', 1)"));
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(by_name(&plan.candidates, "Loap #3").refused.is_none(), "its own former name is free");
    fx.exec(&db, "INSERT INTO db_agent_former_names (agent_id, name, recorded_at) VALUES ('uid-old', 'Naki #2', 1)");
    let plan = super::super::plan(&fx.home, false).unwrap();
    let naki2 = by_name(&plan.candidates, "Naki #2");
    assert!(naki2.refused.as_deref().is_some_and(|r| r.contains("uid-old") && r.contains("Naki #2")), "{:?}", naki2.refused);
}

/// Another fork whose branch label routes to the target holds it: messages
/// to that label reach that fork, so the target isn't free.
#[test]
fn another_forks_branch_label_holds_a_target() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    fx.agent(&data, "uid-other", "Other", "other-fork", NAKI, "Loap #3", true);
    let plan = super::super::plan(&fx.home, false).unwrap();
    let loap3 = by_name(&plan.candidates, "Loap #3");
    assert!(loap3.refused.as_deref().is_some_and(|r| r.contains("branch label")), "{:?}", loap3.refused);
}

/// A target held only by the agent itself, its own display binding (a
/// live registration of its own pane under `loap-3`), is free.
#[test]
fn a_target_held_only_by_the_same_uid_is_free() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    let ws = fx.root().join("ws");
    fx.pane(&data, LOAP3, "loap-loap-2-loap-3", "block-own", &ws);
    let entry = serde_json::json!({ "agent_id": "Loap-3", "local_url": "", "block_id": "block-own", "pid": std::process::id(), "updated_at": 1 });
    std::fs::create_dir_all(data.join("agents")).unwrap();
    std::fs::write(data.join("agents").join("loap-3.json"), serde_json::to_vec(&entry).unwrap()).unwrap();
    let plan = super::super::plan(&fx.home, false).unwrap();
    let loap3 = by_name(&plan.candidates, "Loap #3");
    assert!(loap3.refused.is_none(), "{:?}", loap3.refused);
    assert_eq!(loap3.running.len(), 1, "but it runs, so an apply would wait: {:?}", loap3.running);

    // Another agent's pane registered under the target holds it.
    let mut other = entry.clone();
    other["block_id"] = "block-other".into();
    fx.pane(&data, NAKI, "naki", "block-other", &ws);
    std::fs::write(data.join("agents").join("loap-3.json"), serde_json::to_vec(&other).unwrap()).unwrap();
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(by_name(&plan.candidates, "Loap #3").refused.is_some());
}

/// A plan, full or quick, changes nothing on disk; and lists every location
/// type of the fixture home for `loap-loap-2`.
#[test]
fn a_plan_writes_nothing_and_lists_every_location() {
    let fx = Fx::standard();
    let before = fx.snapshot();
    let plan = super::super::plan(&fx.home, false).unwrap();
    let quick = super::super::plan(&fx.home, true).unwrap();
    assert_eq!(fx.snapshot(), before, "nothing changed");
    assert_eq!(quick.candidates.len(), plan.candidates.len());
    assert!(quick.candidates.iter().all(|c| c.locations.is_empty()));
    let loap2 = by_name(&plan.candidates, "Loap #2");
    let mut keys: Vec<&str> = loap2.locations.iter().map(|l| l.key.as_str()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys, vec!["K1", "K10", "K11", "K12", "K13", "K2", "K3", "K4", "K5", "K7", "K9"]);
    assert_eq!(loap2.locations.iter().filter(|l| l.key == "K1").count(), 4, "one row per objects.db");
    let text = serde_json::to_string(&plan).unwrap();
    for secret in ["hmac-loap-loap-2", "lpriv", "wpriv", "secret", "keep-me"] {
        assert!(!text.contains(secret), "the plan never carries a key or a secret ({secret})");
    }
}

/// Another agent whose display name routes to a target holds it, whatever
/// its own slug (`loap-2-2`, a collision suffix): a message to `loap-2` would
/// otherwise reach the migrated agent instead of it. Case-insensitive, and
/// in a channel's rows as in the shared definitions.
#[test]
fn another_agents_display_route_holds_a_target() {
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    fx.agent(&data, "uid-routed", "LOAP 2", "loap-2-2", "", "", true);
    let plan = super::super::plan(&fx.home, false).unwrap();
    let loap2 = by_name(&plan.candidates, "Loap #2");
    assert!(loap2.refused.as_deref().is_some_and(|r| r.contains("uid-routed") && r.contains("routes there")), "{:?}", loap2.refused);
    assert!(by_name(&plan.candidates, "Naki #2").refused.is_none());

    // Known only to an older version's rows, by its instance name.
    let fx = Fx::empty();
    let data = fx.version("stable", "0.59.1");
    fx.chains(&data, true);
    let old = fx.version("local-x", "0.58.0");
    fx.agent(&old, "uid-inst", "Someone", "someone", "", "", false);
    fx.exec(&old.join("db/objects.db"), "UPDATE db_agents SET instance_name = 'Naki-2' WHERE id = 'uid-inst'");
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(by_name(&plan.candidates, "Naki #2").refused.as_deref().is_some_and(|r| r.contains("uid-inst")));
}

/// An objects.db from a version older than `db_agents.slug` holds no slug:
/// it neither blocks the plan nor fails the run (#5342).
#[test]
fn an_objects_db_older_than_agent_slugs_holds_nothing() {
    let fx = Fx::standard();
    let old = fx.root().join("channels/stable/versions/0.40.0/data/db");
    std::fs::create_dir_all(&old).unwrap();
    rusqlite::Connection::open(old.join("objects.db"))
        .unwrap()
        .execute_batch("CREATE TABLE db_agent_definitions (id TEXT PRIMARY KEY, name TEXT);")
        .unwrap();
    let plan = super::super::plan(&fx.home, false).unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    assert!(plan.candidates.iter().all(|c| c.refused.is_none()), "{:?}", plan.candidates);
    let report = super::apply(&fx.home).unwrap();
    assert_eq!(report.failed, None);
    assert_eq!(report.migrated.len(), 3);
}
