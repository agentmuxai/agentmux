// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `s1_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::check_s1_resolved;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::store::Store;

// `resolve_agent_id`'s registry tier reads the real `~/.agentmux` unless
// AGENTMUX_HOME_OVERRIDE is set, which would make the deny assertions
// depend on which agents happen to be running. Same crate-wide guard the
// env var's other consumers take.
use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

fn with_store<T>(f: impl FnOnce(&Store) -> T) -> T {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path().to_str().unwrap());
    let store = Store::open_in_memory().unwrap();
    let out = f(&store);
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    out
}

fn agent(store: &Store, id: &str, name: &str, slug: &str) {
    let mut def = test_agent_def(id, name, "claude", "agent", 1, "");
    def.slug = slug.to_string();
    store.agent_def_insert(&mut def).unwrap();
    assert_eq!(def.slug, slug, "fixture precondition: slug not suffix-resolved");
}

#[test]
fn allows_a_caller_acting_on_itself_by_the_same_string() {
    with_store(|store| {
        // The fast path: no store lookup, behaviour identical to the
        // pre-migration byte comparison.
        assert!(check_s1_resolved(store, "agenty", "agenty").is_ok());
    });
}

#[test]
fn denies_an_unauthenticated_connection() {
    with_store(|store| {
        let err = check_s1_resolved(store, "", "agenty").unwrap_err();
        assert!(err.contains("unauthenticated"), "got: {err}");
    });
}

// The assertion that matters. §7 warns that testing only the allow path
// would let a half-migrated comparison through: an implementation that
// resolved one side and not the other would deny everyone, and one that
// resolved nothing would admit everyone. This pins the deny direction.
#[test]
fn denies_a_caller_naming_a_different_agent() {
    with_store(|store| {
        agent(store, "def-a", "Agent A", "agent-a");
        agent(store, "def-b", "Agent B", "agent-b");

        let err = check_s1_resolved(store, "agent-a", "agent-b").unwrap_err();
        assert!(err.contains("mismatch"), "got: {err}");
        // …and by the other agent's canonical id, which is the form this
        // change newly teaches the comparison to understand.
        assert!(check_s1_resolved(store, "agent-a", "def-b").is_err());
    });
}

// The capability this phase adds: the caller authenticates with its slug
// (what `bus:register` stamps) but names itself by canonical id. Byte
// comparison rejected that; both sides now resolve to the same id.
#[test]
fn allows_a_caller_that_names_itself_by_canonical_id() {
    with_store(|store| {
        agent(store, "def-a", "Agent A", "agent-a");
        assert!(check_s1_resolved(store, "agent-a", "def-a").is_ok());
    });
}

// The spec's named hazard, pinned in the direction that actually bites.
// §6 Phase 4 proposed stamping the RESOLVED id into `RpcContext` at
// `bus:register`; had that landed while request bodies kept sending slugs,
// every caller would compare `"def-a" != "agent-a"` and be denied — a
// total authz outage that no allow-path test using matching forms would
// notice. Both sides resolving makes the comparison indifferent to which
// form either side carries, so that deploy ordering cannot happen.
#[test]
fn allows_a_caller_stamped_with_a_canonical_id_that_names_itself_by_slug() {
    with_store(|store| {
        agent(store, "def-a", "Agent A", "agent-a");
        assert!(check_s1_resolved(store, "def-a", "agent-a").is_ok());
    });
}

// A slug shared by two agents identifies neither, so it must not authorize
// anything — resolution fails closed on both sides and the caller is
// denied rather than silently granted one of the two.
#[test]
fn denies_a_caller_whose_slug_is_ambiguous() {
    with_store(|store| {
        let mut tmpl = test_agent_def("tmpl-1", "Shared Template", "claude", "agent", 1, "");
        tmpl.slug = "shared-template".to_string();
        tmpl.is_seeded = 1;
        store.agent_def_insert(&mut tmpl).unwrap();

        let mut a = crate::backend::storage::agents::AgentInstance {
            id: "launch-a".to_string(),
            definition_id: "tmpl-1".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "init".to_string(),
            github_context: String::new(),
            started_at: 1,
            ended_at: 0,
            created_at: 1,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: "Launch A".to_string(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&a).unwrap();
        a.id = "launch-b".to_string();
        a.instance_name = "Launch B".to_string();
        store.instance_create(&a).unwrap();

        // Force the duplicate the §4 safety net now prevents, one row at a
        // time: the first must AUTHORIZE, so the denial below is caused by
        // the second row rather than by the slug matching nothing.
        store.test_force_slug("launch-a", "collide-me").unwrap();
        assert!(
            check_s1_resolved(store, "collide-me", "launch-a").is_ok(),
            "one row with the slug must authorize — otherwise the denial below is vacuous"
        );

        store.test_force_slug("launch-b", "collide-me").unwrap();
        let err = check_s1_resolved(store, "collide-me", "launch-a").unwrap_err();
        assert!(err.contains("mismatch"), "got: {err}");
    });
}

// An unresolvable agent must not be distinguishable from a mismatch:
// whether some other agent exists is not something this boundary should
// let an unauthorized caller probe.
#[test]
fn reports_an_unknown_agent_as_a_mismatch_not_as_not_found() {
    with_store(|store| {
        agent(store, "def-a", "Agent A", "agent-a");
        let err = check_s1_resolved(store, "agent-a", "no-such-agent").unwrap_err();
        assert!(err.contains("mismatch"), "got: {err}");
        assert!(!err.to_lowercase().contains("unknown"), "leaks existence: {err}");
    });
}
