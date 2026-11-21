// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::obj::MetaMapType;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::Skill;
use serde_json::json;

fn store() -> Store {
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn skill(id: &str, name: &str) -> Skill {
    Skill {
        id: id.to_string(),
        name: name.to_string(),
        trigger: String::new(),
        skill_type: "agent-skill".to_string(),
        description: format!("when to use {name}"),
        content: format!("steps of {name}"),
        is_global: true,
        created_at: 1,
        updated_at: 1,
    }
}

fn meta_for(uid: &str, dir: &std::path::Path) -> MetaMapType {
    let mut meta = MetaMapType::new();
    meta.insert("agentId".to_string(), json!(uid));
    meta.insert("cmd:cwd".to_string(), json!(dir.to_str().unwrap()));
    meta
}

fn agent(s: &Store, uid: &str, agent_type: &str) {
    let mut def = test_agent_def(uid, uid, "claude", agent_type, 1, "");
    def.slug = uid.to_string();
    s.agent_def_insert(&mut def).unwrap();
}

fn skill_md(dir: &std::path::Path, slug: &str) -> std::path::PathBuf {
    dir.join(".claude").join("skills").join(slug).join("SKILL.md")
}

#[test]
fn a_resume_writes_the_skills_bound_since_the_first_launch() {
    let s = store();
    agent(&s, "a1", "host");
    s.skill_upsert_unique_global(&skill("s1", "tidy-disk")).unwrap();
    s.skill_bind(&s, "a1", "s1").unwrap();
    let dir = tempfile::tempdir().unwrap();

    assert_eq!(refresh_for_block(&meta_for("a1", dir.path()), &s, &s), 1);
    let text = std::fs::read_to_string(skill_md(dir.path(), "tidy-disk")).unwrap();
    assert!(text.contains("steps of tidy-disk"), "{text}");
    // Nothing but skill files: the startup file and .mcp.json are not rewritten on a resume.
    assert!(!dir.path().join("CLAUDE.md").exists());
    assert!(!dir.path().join(".mcp.json").exists());
    // A second start with nothing changed writes nothing.
    assert_eq!(refresh_for_block(&meta_for("a1", dir.path()), &s, &s), 0);
}

#[test]
fn a_skill_that_reaches_every_agent_arrives_without_a_bind() {
    let s = store();
    agent(&s, "a1", "host");
    s.skill_upsert_unique_global(&skill("s1", "for-everyone")).unwrap();
    s.skill_set_reach("s1", true, true, 1).unwrap();
    let dir = tempfile::tempdir().unwrap();

    refresh_for_block(&meta_for("a1", dir.path()), &s, &s);
    assert!(skill_md(dir.path(), "for-everyone").exists());
}

#[test]
fn an_unbound_skill_is_removed_and_the_agents_own_files_stay() {
    let s = store();
    agent(&s, "a1", "host");
    s.skill_upsert_unique_global(&skill("s1", "going-away")).unwrap();
    s.skill_bind(&s, "a1", "s1").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mine = dir.path().join(".claude").join("skills").join("mine").join("SKILL.md");
    std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
    std::fs::write(&mine, "written by the user").unwrap();

    refresh_for_block(&meta_for("a1", dir.path()), &s, &s);
    assert!(skill_md(dir.path(), "going-away").exists());
    s.skill_unbind("a1", "s1").unwrap();
    refresh_for_block(&meta_for("a1", dir.path()), &s, &s);

    assert!(!skill_md(dir.path(), "going-away").exists());
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "written by the user");
}

#[test]
fn a_container_agent_gets_its_skills_the_same_way() {
    let s = store();
    agent(&s, "c1", "container");
    s.skill_upsert_unique_global(&skill("s1", "in-the-box")).unwrap();
    s.skill_bind(&s, "c1", "s1").unwrap();
    let dir = tempfile::tempdir().unwrap();

    refresh_for_block(&meta_for("c1", dir.path()), &s, &s);
    assert!(skill_md(dir.path(), "in-the-box").exists());
}

#[test]
fn a_block_that_is_not_an_agents_or_has_no_folder_is_left_alone() {
    let s = store();
    agent(&s, "a1", "host");
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(refresh_for_block(&MetaMapType::new(), &s, &s), 0);
    assert_eq!(refresh_for_block(&meta_for("no-such-agent", dir.path()), &s, &s), 0);
    assert_eq!(refresh_for_block(&meta_for("a1", &dir.path().join("gone")), &s, &s), 0);
    assert!(!dir.path().join(".claude").exists());
}

#[test]
fn the_launched_name_fills_the_skill_templates_and_a_home_relative_folder_is_expanded() {
    let s = store();
    agent(&s, "a1", "host");
    let mut sk = skill("s1", "who-am-i");
    sk.content = "I am {{AGENT}}".to_string();
    s.skill_upsert_unique_global(&sk).unwrap();
    s.skill_bind(&s, "a1", "s1").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut meta = meta_for("a1", dir.path());
    meta.insert("agentName".to_string(), json!("Instance Two"));

    refresh_for_block(&meta, &s, &s);
    let text = std::fs::read_to_string(skill_md(dir.path(), "who-am-i")).unwrap();
    assert!(text.contains("I am Instance Two"), "{text}");

    // `~` is the user's home, as `agent.open` leaves a default folder.
    let home = crate::backend::base::expand_home_dir_safe("~");
    let rel = format!("~/.agentmux-skill-refresh-test-{}", std::process::id());
    let under_home = crate::backend::base::expand_home_dir_safe(&rel);
    if home.is_dir() && std::fs::create_dir_all(&under_home).is_ok() {
        let mut meta = meta_for("a1", &under_home);
        meta.insert("cmd:cwd".to_string(), json!(rel));
        refresh_for_block(&meta, &s, &s);
        let wrote = skill_md(&under_home, "who-am-i").exists();
        let _ = std::fs::remove_dir_all(&under_home);
        assert!(wrote, "a folder written as {rel} is refreshed");
    }
}

#[test]
fn a_resume_of_a_host_agent_in_a_folder_a_container_agent_can_write_keeps_its_skills_outside_it() {
    let s = store();
    agent(&s, "a1", "host");
    s.skill_upsert_unique_global(&skill("s1", "tidy-disk")).unwrap();
    s.skill_bind(&s, "a1", "s1").unwrap();
    let (data, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let work_dir = dir.path().to_string_lossy().into_owned();
    crate::backend::mcp_config_file::with_data_home(data.path(), || {
        crate::backend::mcp_folder_rule::mark(&work_dir);
        assert_eq!(refresh_for_block(&meta_for("a1", dir.path()), &s, &s), 0);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing is written into the folder");
        let own = crate::backend::host_agent_files::dir_of(&work_dir).unwrap();
        let text = std::fs::read_to_string(crate::backend::host_agent_files::plugin_dir(&own).join("skills/tidy-disk/SKILL.md")).unwrap();
        assert!(text.contains("steps of tidy-disk"), "{text}");
        // A skill unbound since is gone from the plugin the next start.
        s.skill_unbind("a1", "s1").unwrap();
        refresh_for_block(&meta_for("a1", dir.path()), &s, &s);
        assert!(!crate::backend::host_agent_files::plugin_dir(&own).join("skills/tidy-disk").exists());
    });
}
