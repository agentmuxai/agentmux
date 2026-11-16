// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What is written to an agent, and what it lists, is the skills it holds:
//! a global skill nobody bound to the agent is catalog content only. The
//! count the Skills tab and Memory → Skills show comes from the same binds.

use super::effective_skills_tests::{global_skill, insert_agent_with_bundle, insert_test_bundle, make_store};
use super::*;
use std::collections::HashMap;
use crate::backend::agent_config::{build_config_files, skills_index_text};

fn trigger_skill(id: &str, name: &str, trigger: &str) -> Skill {
    Skill { trigger: trigger.to_string(), ..global_skill(id, name) }
}

/// One agent in a bundle, with two global skills in the catalog.
fn setup() -> Store {
    let store = make_store();
    insert_test_bundle(&store, "bundle-1");
    insert_agent_with_bundle(&store, "agent-1", "bundle-1");
    store.skill_upsert_unique_global(&trigger_skill("g-bound", "Bound Skill", "bound")).unwrap();
    store.skill_upsert_unique_global(&trigger_skill("g-unbound", "Unbound Skill", "unbound")).unwrap();
    store
}

fn names(store: &Store) -> Vec<String> {
    store.effective_skills(store, "agent-1").into_iter().map(|s| s.name).collect()
}

#[test]
fn an_unbound_global_skill_is_not_effective_for_the_agent() {
    let store = setup();
    assert!(names(&store).is_empty());

    store.skill_bind(&store, "agent-1", "g-bound").unwrap();

    assert_eq!(names(&store), ["Bound Skill"]);
}

#[test]
fn a_global_skill_bound_to_the_agents_bundle_is_effective_and_another_is_not() {
    let store = setup();
    store.bundle_skill_bind(&store, &store, "bundle-1", "g-bound").unwrap();

    assert_eq!(names(&store), ["Bound Skill"]);
}

#[test]
fn an_unbound_global_skill_does_not_discard_legacy_skills_or_count_as_an_own_skill() {
    let store = setup();
    store
        .agent_skill_insert(&AgentSkill {
            id: "legacy-1".to_string(),
            agent_id: "agent-1".to_string(),
            name: "Legacy Skill".to_string(),
            trigger: "legacy".to_string(),
            skill_type: "prompt".to_string(),
            description: String::new(),
            content: "legacy content".to_string(),
            created_at: 1_700_000_000_000,
        })
        .unwrap();
    store.skill_bind(&store, "agent-1", "g-bound").unwrap();

    let got = names(&store);
    assert!(got.contains(&"Legacy Skill".to_string()) && got.contains(&"Bound Skill".to_string()), "{got:?}");
    assert!(!got.contains(&"Unbound Skill".to_string()), "{got:?}");
}

/// The files the agent gets, the index it is shown and the count the UI
/// shows all come from the same set.
#[test]
fn written_files_the_index_and_the_count_agree() {
    let store = setup();
    store.skill_bind(&store, "agent-1", "g-bound").unwrap();

    let effective = store.effective_skills(&store, "agent-1");
    let counted = store
        .skill_list_global_for_agent(&store, "agent-1")
        .unwrap()
        .into_iter()
        .filter(|item| item.bound_to_agent)
        .count();
    let used_by: Vec<_> = store
        .skill_list_global(&store)
        .unwrap()
        .into_iter()
        .map(|item| (item.skill.name, item.bound_count))
        .collect();
    assert_eq!(counted, 1);
    assert!(used_by.contains(&("Bound Skill".to_string(), 1)) && used_by.contains(&("Unbound Skill".to_string(), 0)), "{used_by:?}");

    // Claude: one command file per skill, none for the unbound one.
    let files = build_config_files(&HashMap::new(), &effective, "Aria", "agent-1", "aria", "/tmp/aria", "claude");
    let commands: Vec<&str> =
        files.iter().map(|f| f.filename.as_str()).filter(|f| f.starts_with(".claude/commands/")).collect();
    assert_eq!(commands, [".claude/commands/bound.md"]);
    assert_eq!(commands.len(), counted);

    // Another provider lists the skills in its startup file.
    let index = skills_index_text(&effective, "codex");
    assert!(index.contains("/bound") && !index.contains("/unbound"), "{index}");
    let codex = build_config_files(&HashMap::new(), &effective, "Aria", "agent-1", "aria", "/tmp/aria", "codex");
    let agents_md = codex.iter().find(|f| f.filename == "AGENTS.md").unwrap();
    assert!(agents_md.content.contains("/bound") && !agents_md.content.contains("/unbound"));
}

/// Unbinding takes the skill out of what is written.
#[test]
fn unbinding_removes_the_skill_from_what_is_written() {
    let store = setup();
    store.skill_bind(&store, "agent-1", "g-bound").unwrap();
    store.skill_unbind("agent-1", "g-bound").unwrap();

    let files = build_config_files(
        &HashMap::new(),
        &store.effective_skills(&store, "agent-1"),
        "Aria",
        "agent-1",
        "aria",
        "/tmp/aria",
        "claude",
    );
    assert!(files.iter().all(|f| !f.filename.starts_with(".claude/commands/")));
}
