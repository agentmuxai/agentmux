use super::*;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

/// The channel store, where agent refs live: deleting a skill purges its refs there and its catalog row in `s`.
fn channel_store() -> (Store, tempfile::NamedTempFile) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    (Store::open(tmp.path()).unwrap(), tmp)
}

fn one() -> Builtin {
    Builtin { id: "11111111-1111-4111-8111-111111111111", name: "demo", trigger: "demo", description: "A demo skill.", content: "Original text." }
}

#[test]
fn a_fresh_catalog_gets_the_skill_for_every_agent() {
    let (s, _d) = store();
    assert_eq!(seed(&s, &[one()], 1_000), 1);
    let skill = s.skill_get(one().id).unwrap().expect("seeded");
    assert!(skill.is_global);
    assert_eq!((skill.name.as_str(), skill.skill_type.as_str(), skill.content.as_str()), ("demo", "agent-skill", "Original text."));
    let reach = s.skill_reach_map().unwrap();
    assert!(reach[one().id].all_agents, "every agent, no bind needed");
}

#[test]
fn it_is_seeded_once_and_a_second_start_adds_nothing() {
    let (s, _d) = store();
    assert_eq!(seed(&s, &[one()], 1_000), 1);
    assert_eq!(seed(&s, &[one()], 2_000), 0);
    assert_eq!(s.skill_list_all_raw().unwrap().len(), 1);
}

#[test]
fn a_skill_the_user_deleted_is_not_brought_back() {
    let (s, _d) = store();
    let (m, _t) = channel_store();
    seed(&s, &[one()], 1_000);
    assert!(m.skill_delete(&s, one().id).unwrap());
    assert_eq!(seed(&s, &[one()], 2_000), 0);
    assert!(s.skill_get(one().id).unwrap().is_none(), "a delete is final");
    assert!(s.skill_reach_map().unwrap().get(one().id).is_none());
}

#[test]
fn a_skill_the_user_edited_is_not_overwritten_by_a_newer_text() {
    let (s, _d) = store();
    seed(&s, &[one()], 1_000);
    let mut edited = s.skill_get(one().id).unwrap().unwrap();
    edited.content = "My own text.".into();
    edited.updated_at = 1_500;
    s.skill_upsert_unique_global(&edited).unwrap();
    let newer = Builtin { content: "Improved text in a later release.", ..one() };
    assert_eq!(seed(&s, &[newer], 2_000), 0);
    assert_eq!(s.skill_get(one().id).unwrap().unwrap().content, "My own text.");
}

#[test]
fn a_release_that_adds_a_skill_seeds_only_the_new_one() {
    let (s, _d) = store();
    let (m, _t) = channel_store();
    seed(&s, &[one()], 1_000);
    m.skill_delete(&s, one().id).unwrap();
    let second = Builtin { id: "22222222-2222-4222-8222-222222222222", name: "second", trigger: "second", ..one() };
    assert_eq!(seed(&s, &[one(), second], 2_000), 1);
    assert!(s.skill_get(one().id).unwrap().is_none());
    assert!(s.skill_get("22222222-2222-4222-8222-222222222222").unwrap().is_some());
}

#[test]
fn a_global_skill_of_the_same_name_is_left_alone_and_not_retried() {
    let (s, _d) = store();
    let theirs = Skill {
        id: "99999999-9999-4999-8999-999999999999".into(),
        name: "demo".into(),
        trigger: "demo".into(),
        skill_type: "prompt".into(),
        description: "Mine.".into(),
        content: "Their text.".into(),
        is_global: true,
        created_at: 1,
        updated_at: 1,
    };
    s.skill_upsert_unique_global(&theirs).unwrap();
    assert_eq!(seed(&s, &[one()], 1_000), 0);
    assert!(s.skill_get(one().id).unwrap().is_none());
    assert_eq!(s.skill_get(&theirs.id).unwrap().unwrap().content, "Their text.");
    assert!(s.builtin_seed_done(&seed_key(one().id)).unwrap(), "recorded, so it is not retried at every start");
}

#[test]
fn the_shipped_skills_have_what_a_skill_needs_and_name_nothing_private() {
    let mut ids = std::collections::HashSet::new();
    for b in BUILTINS {
        assert!(ids.insert(b.id), "ids are unique: {}", b.id);
        assert!(!b.name.is_empty() && !b.description.is_empty() && !b.content.trim().is_empty());
        assert!(b.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "{}", b.name);
        assert!(!b.content.contains("{{"), "the template expander would take it: {}", b.name);
        // The text ships to every install: no host names, accounts or private systems.
        let text = format!("{} {} {}", b.name, b.description, b.content).to_lowercase();
        for private in ["narko", "charlie", "starpower", "area54", "a5af", "agentmuxai", "shared-infrastructure", "asafe", "@gmail", "arn:aws"] {
            assert!(!text.contains(private), "{private} in {}", b.name);
        }
    }
}

#[test]
fn the_disk_cleanup_skill_says_to_ask_first_and_what_never_to_touch() {
    let b = BUILTINS.iter().find(|b| b.name == "disk-cleanup").expect("shipped");
    let c = b.content.to_lowercase();
    assert!(c.contains("ask before deleting anything"));
    for protected in ["shared/", "memory", "transcript", "agents/<name>/"] {
        assert!(c.contains(protected), "{protected}");
    }
    assert!(c.contains("measure"));
}
