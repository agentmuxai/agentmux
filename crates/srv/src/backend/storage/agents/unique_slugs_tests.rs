// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A slug another agent holds anywhere on this host is never given to a
//! second one: not by a creating path (the shared definitions and every
//! `objects.db` and live registration are checked, under the slug allocation
//! lock), and not by the mirror publishing a channel's row.

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::backend::slug_migration::Home;
use crate::backend::storage::agents::test_agent_def;
use crate::registry::DefinitionStore;

/// An AgentMux root with channels, each version an `objects.db` on disk.
struct Host {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Host {
    fn new() -> Host {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        Host { _tmp: tmp, root }
    }

    fn data(&self, channel: &str) -> PathBuf {
        let data = self.root.join("channels").join(channel).join("versions").join("1.0.0").join("data");
        std::fs::create_dir_all(data.join("db")).unwrap();
        data
    }

    fn db(&self, channel: &str) -> PathBuf {
        self.data(channel).join("db").join("objects.db")
    }

    fn defs(&self) -> Arc<DefinitionStore> {
        Arc::new(DefinitionStore::open(Home::at(&self.root).definitions()).unwrap())
    }

    /// `channel`'s store as srv opens it: the shared definitions attached and
    /// every other store on the root checked.
    fn channel(&self, channel: &str) -> Arc<Store> {
        let store = Arc::new(Store::open(&self.db(channel)).unwrap());
        store.set_def_registry(self.defs());
        store.set_slug_home(Home::at(&self.root));
        store
    }

    /// A row only `channel`'s `objects.db` knows, as an agent made before the
    /// shared definitions (or by a build that checked only its own channel)
    /// is.
    fn unpublished(&self, channel: &str, uid: &str, name: &str, slug: &str) {
        let store = Store::open(&self.db(channel)).unwrap();
        let mut def = named(uid, name, slug);
        store.agent_def_insert(&mut def).unwrap();
        assert_eq!(def.slug, slug, "the fixture's slug is kept");
    }

    fn exec(&self, channel: &str, sql: &str) {
        rusqlite::Connection::open(self.db(channel)).unwrap().execute_batch(sql).unwrap();
    }
}

fn named(uid: &str, name: &str, slug: &str) -> AgentDefinition {
    let mut def = test_agent_def(uid, name, "claude", "host", 1, "");
    def.slug = slug.to_string();
    def
}

fn template(uid: &str, slug: &str) -> AgentDefinition {
    let mut def = named(uid, "Coder", slug);
    def.is_seeded = 1;
    def
}

fn launch(id: &str, template_id: &str) -> crate::backend::storage::store::AgentInstance {
    crate::backend::storage::store::AgentInstance {
        id: id.to_string(),
        definition_id: template_id.to_string(),
        parent_instance_id: String::new(),
        block_id: String::new(),
        session_id: String::new(),
        status: "running".to_string(),
        github_context: String::new(),
        started_at: 1,
        ended_at: 0,
        created_at: 1,
        identity_id: String::new(),
        memory_id: String::new(),
        instance_name: String::new(),
        working_directory: String::new(),
        display_hidden: false,
    }
}

/// Every creating path: `agent_def_insert` (createagent, create from a
/// template, fork, `Agents spawn`, import, each with its derived or explicit
/// slug suffixed as before), `agent_def_find_or_insert` (`agent.define`) and a
/// fresh template launch (`instance_create`). Each skips a slug only another
/// channel's `objects.db` holds, as a slug, a former slug or a former display
/// name, and one a live pane is registered under.
#[test]
fn every_creating_path_skips_a_slug_another_channel_holds() {
    let host = Host::new();
    host.unpublished("old", "uid-fork-old", "ForkLead", "forklead");
    host.unpublished("old", "uid-coder-launch", "Coder", "coder-2");
    host.exec(
        "old",
        "INSERT INTO db_agent_former_slugs (agent_uid, slug, retired_at) VALUES ('uid-fork-old', 'was-fork', 1);
         INSERT INTO db_agent_former_names (agent_id, name, recorded_at) VALUES ('uid-fork-old', 'Was Lead', 1);",
    );
    let registrations = host.data("old").join("agents");
    std::fs::create_dir_all(&registrations).unwrap();
    let entry = serde_json::json!({
        "agent_id": "live-one", "local_url": "http://127.0.0.1:1", "block_id": "b-live",
        "pid": std::process::id(), "updated_at": 1,
    });
    std::fs::write(registrations.join("live-one.json"), serde_json::to_vec(&entry).unwrap()).unwrap();

    let b = host.channel("b");
    let mut created = named("uid-created", "ForkLead", "");
    b.agent_def_insert(&mut created).unwrap();
    assert_eq!(created.slug, "forklead-2", "derived from the name");
    for (uid, asked, want) in [
        ("uid-explicit", "forklead", "forklead-3"),
        ("uid-former-slug", "was-fork", "was-fork-2"),
        ("uid-former-name", "was-lead", "was-lead-2"),
        ("uid-live", "live-one", "live-one-2"),
        ("uid-free", "free-one", "free-one"),
    ] {
        let mut def = named(uid, "Other", asked);
        b.agent_def_insert(&mut def).unwrap();
        assert_eq!(def.slug, want, "{asked}");
    }

    let c = host.channel("c");
    let mut defined = named("uid-defined", "ForkLead", "");
    assert!(c.agent_def_find_or_insert(&mut defined).unwrap().is_none(), "nothing of that name in this channel");
    assert_eq!(defined.slug, "forklead-4");

    let mut tpl = template("tpl-coder", "coder");
    b.agent_def_insert(&mut tpl).unwrap();
    assert_eq!(tpl.slug, "coder", "a template keeps its slug: every channel seeds the same one");
    b.instance_create(&launch("launch-1", "tpl-coder")).unwrap();
    assert_eq!(b.agent_row_get("launch-1").unwrap().unwrap().slug, "coder-3");

    let defs = host.defs();
    for uid in ["uid-created", "uid-explicit", "uid-defined"] {
        assert!(defs.exists(uid), "{uid} is published");
    }
}

/// A store that can't be read leaves the slug unchecked: nothing is created.
#[test]
fn an_unreadable_objects_db_stops_a_creation() {
    let host = Host::new();
    let b = host.channel("b");
    std::fs::write(host.db("broken"), b"this is not a database at all").unwrap();
    let mut def = named("uid-blocked", "Blocked", "");
    let err = b.agent_def_insert(&mut def).expect_err("the broken store stops it");
    assert!(err.to_string().contains("Nothing was created"), "{err}");
    assert!(b.agent_row_get("uid-blocked").unwrap().is_none());
    assert!(!host.defs().exists("uid-blocked"));
}

/// Two channels on one root creating at once end with distinct slugs: for a
/// published agent, and for a template launch, whose row is never published
/// and is seen only through the other channel's `objects.db`.
#[test]
fn concurrent_creations_in_two_channels_get_distinct_slugs() {
    let host = Host::new();
    let channels = [host.channel("one"), host.channel("two")];
    for store in &channels {
        store.agent_def_insert(&mut template("tpl-racer", "tpl")).unwrap();
    }
    for round in 0..4 {
        let start = Arc::new(std::sync::Barrier::new(2));
        let picked: Vec<(String, String)> = channels
            .iter()
            .enumerate()
            .map(|(i, store)| {
                let (store, start) = (store.clone(), start.clone());
                std::thread::spawn(move || {
                    let mut def = named(&format!("uid-race-{round}-{i}"), "Racer", &format!("racer-{round}"));
                    let launch_id = format!("launch-{round}-{i}");
                    start.wait();
                    store.agent_def_insert(&mut def).unwrap();
                    store.instance_create(&launch(&launch_id, "tpl-racer")).unwrap();
                    (def.slug, store.agent_row_get(&launch_id).unwrap().unwrap().slug)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect();
        assert_ne!(picked[0].0, picked[1].0, "round {round}: {picked:?}");
        assert_ne!(picked[0].1, picked[1].1, "round {round}: {picked:?}");
    }
}

/// The mirror publishes a channel's row (an agent with no shared record
/// yet, on its next edit or content write) only when no other agent holds
/// its slug, in the shared definitions or in any `objects.db`. Otherwise it
/// writes no record, and the agent keeps its slug in its own channel; a
/// record with no slug keeps none. A free slug is published as before.
#[test]
fn the_mirror_never_publishes_a_slug_another_agent_holds() {
    let host = Host::new();
    let defs = host.defs();
    host.channel("a").agent_def_insert(&mut named("uid-claude-a", "Claude", "claude-2")).unwrap();
    host.unpublished("b", "uid-claude-b", "Claude", "claude-2");
    host.unpublished("c", "uid-solo-c", "Solo", "solo");
    host.unpublished("b", "uid-solo-b", "Solo", "solo");
    host.unpublished("b", "uid-free", "Free", "free");
    let b = host.channel("b");

    for uid in ["uid-claude-b", "uid-solo-b"] {
        let mut def = b.agent_def_get(uid).unwrap().unwrap();
        def.description = "edited".to_string();
        assert!(b.agent_def_update(&mut def).unwrap(), "{uid}: the edit lands");
        assert!(!defs.exists(uid), "{uid}: its taken slug isn't published");
        let local = b.agent_def_get(uid).unwrap().unwrap();
        assert_eq!((local.description.as_str(), local.slug.as_str()), ("edited", def.slug.as_str()), "{uid}");
    }
    assert_eq!(defs.get("uid-claude-a").unwrap().unwrap().data.slug, "claude-2", "the holder's record is untouched");

    let mut free = b.agent_def_get("uid-free").unwrap().unwrap();
    free.description = "edited".to_string();
    b.agent_def_update(&mut free).unwrap();
    assert_eq!(defs.get("uid-free").unwrap().unwrap().data.slug, "free");

    // A record without a slug (an older build's) isn't given the taken one,
    // by an edit or by a content write's refresh.
    let mut blank = crate::backend::storage::def_registry_mirror::agent_definition_to_record(
        &b.agent_def_get("uid-claude-b").unwrap().unwrap(),
        &[],
        &[],
    );
    blank.data.slug = String::new();
    defs.upsert(&blank).unwrap();
    let mut def = b.agent_def_get("uid-claude-b").unwrap().unwrap();
    def.description = "edited again".to_string();
    b.agent_def_update(&mut def).unwrap();
    b.agent_content_set(&crate::backend::storage::store::AgentContent {
        agent_id: "uid-claude-b".to_string(),
        content_type: "ui:zoom".to_string(),
        content: "1.2".to_string(),
        updated_at: 2,
    })
    .unwrap();
    let rec = defs.get("uid-claude-b").unwrap().unwrap();
    assert_eq!((rec.data.slug.as_str(), rec.data.description.as_str()), ("", "edited again"));
    assert!(rec.data.content.iter().any(|c| c.content_type == "ui:zoom"), "the refresh still mirrors");
}
