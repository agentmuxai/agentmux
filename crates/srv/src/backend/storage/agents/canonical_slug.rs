// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One answer to "what is this agent's slug"
//! (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` P0.1).
//!
//! An agent's slug is stored twice: in this channel's `db_agents.slug` and in
//! its shared definition (`shared/agents/definitions/<uid>.json`). Every
//! reader that turns a uid into a slug or a slug into a uid goes through
//! [`canonical_slug`], so the two copies can't give two answers.
//!
//! **The rule: the shared record's slug when it has one, else the local
//! one.** The shared record is the copy every channel and version reads, and
//! the one a rename writes first; a local row is whatever this `objects.db`
//! held when it last saw the agent, so it is the copy that can be stale. And
//! the shared slug can't be moved back by a stale local one, because an update
//! never changes it (`DefinitionStore::upsert`, P0.2). The local slug answers
//! only where there is no shared slug: no shared store, a template or a
//! template launch (never mirrored), an empty shared slug, or a failed read.
//! A disagreement is logged once per agent and pair of values.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use super::*;

/// `local` unless `shared` is a non-empty slug, which wins. Logs (once) when
/// both are set and differ.
pub(in crate::backend::storage) fn canonical_slug(uid: &str, local: &str, shared: Option<&str>) -> String {
    let Some(shared) = shared.filter(|s| !s.trim().is_empty()) else {
        return local.to_string();
    };
    if !local.trim().is_empty() && local != shared {
        warn_disagreement_once(uid, local, shared);
    }
    shared.to_string()
}

fn warn_disagreement_once(uid: &str, local: &str, shared: &str) {
    static SEEN: OnceLock<Mutex<HashSet<(String, String, String)>>> = OnceLock::new();
    let first = SEEN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert((uid.to_string(), local.to_string(), shared.to_string()));
    if first {
        tracing::warn!(
            uid,
            local_slug = local,
            shared_slug = shared,
            "agent slug: this channel's db_agents row and the shared definition disagree; using the shared one"
        );
    }
}

/// The slug and former slugs of one shared definition record.
pub(in crate::backend::storage) use crate::registry::SlugEntry as SharedNames;

/// Every active shared definition's names, by uid. Empty with no shared
/// store, or when it can't be listed (logged): every slug then reads as the
/// local one, which is what this channel answered before. Shared, not
/// copied: the store's slug index hands out the same map until a file changes.
#[derive(Debug, Default)]
pub(in crate::backend::storage) struct SharedSlugs(Arc<HashMap<String, SharedNames>>);

impl SharedSlugs {
    pub fn slug(&self, uid: &str) -> Option<&str> {
        self.0.get(uid).map(|n| n.slug.as_str())
    }

    pub fn names(&self, uid: &str) -> Option<&SharedNames> {
        self.0.get(uid)
    }

    pub fn others<'a>(&'a self, uid: &'a str) -> impl Iterator<Item = &'a SharedNames> + 'a {
        self.0.iter().filter(move |(id, _)| id.as_str() != uid).map(|(_, n)| n)
    }

    /// The uids whose shared slug is `folded` (trimmed and lowercased, as the
    /// key tables fold).
    pub fn ids_with_slug<'a>(&'a self, folded: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.0.iter().filter(move |(_, n)| n.slug.trim().to_lowercase() == folded).map(|(id, _)| id.as_str())
    }

    /// Whether any shared definition lists `folded` among its former slugs.
    pub fn is_former(&self, folded: &str) -> bool {
        self.0.values().any(|n| n.former.iter().any(|f| f.trim().to_lowercase() == folded))
    }
}

/// Rows `filter` selects (a `WHERE …` clause, or empty), oldest first, each
/// with its canonical slug. `NULL` slugs and instance names read as empty.
pub(in crate::backend::storage) fn rows_with_canonical_slugs(
    conn: &rusqlite::Connection,
    filter: &str,
    shared: &SharedSlugs,
) -> rusqlite::Result<Vec<AgentNameMatch>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, name, COALESCE(slug, ''), COALESCE(instance_name, '') FROM db_agents {filter}
         ORDER BY created_at ASC, id ASC"
    ))?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let local: String = row.get(2)?;
        Ok(AgentNameMatch {
            slug: canonical_slug(&id, &local, shared.slug(&id)),
            id,
            name: row.get(1)?,
            instance_name: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Whether `row` answers to `name`: its slug exactly, or its display or
/// instance name ignoring ASCII case (the `COLLATE NOCASE` these lookups
/// used in SQL).
pub(super) fn answers_to(row: &AgentNameMatch, name: &str) -> bool {
    row.slug == name || row.name.eq_ignore_ascii_case(name) || row.instance_name.eq_ignore_ascii_case(name)
}

impl Store {
    /// The shared definition's names for `uid`, if it has an active record:
    /// one file's stat, and a parse only if it changed. A read failure is
    /// logged and reads as no record.
    pub(in crate::backend::storage) fn shared_names_of(&self, uid: &str) -> Option<SharedNames> {
        let reg = self.shared_def_registry()?;
        match reg.slug_of(uid) {
            Ok(names) => names,
            Err(e) => {
                tracing::warn!(uid, error = %e, "agent slug: shared definition unreadable; using this channel's slug");
                None
            }
        }
    }

    /// [`Self::shared_names_of`]'s slug, for a uid-to-slug read.
    pub(in crate::backend::storage) fn shared_slug_of(&self, uid: &str) -> Option<String> {
        self.shared_names_of(uid).map(|n| n.slug)
    }

    /// [`Self::shared_slugs`] for deleting `uid`, which must not guess: the
    /// slugs its signing keys sit under are its shared record's current and
    /// former slugs, and the record is retired with the delete, so a purge
    /// that missed them would leave keys for the next agent of that name.
    /// `uid`'s own record is read strictly (`slug_of_strict`), and a record
    /// that exists but can't be read or parsed (or a shared directory that
    /// can't be listed) is an error, before anything is deleted. A record that doesn't exist,
    /// or no shared store, is not: there are no shared names to cover.
    pub(in crate::backend::storage) fn shared_slugs_for_delete(&self, uid: &str) -> Result<SharedSlugs, StoreError> {
        let Some(reg) = self.shared_def_registry() else {
            return Ok(SharedSlugs::default());
        };
        let refuse = |what: &str, e: crate::registry::DefStoreError| {
            StoreError::Other(format!(
                "not deleting agent {uid}: {what} ({e}), so the names its signing keys are filed under \
                 are unknown. Nothing was deleted; fix or restore the file and try again."
            ))
        };
        let own = reg.slug_of_strict(uid).map_err(|e| refuse("its shared definition can't be read", e))?;
        let index = reg.slug_index().map_err(|e| refuse("the shared definitions can't be listed", e))?;
        let mut all = (*index).clone();
        match own {
            Some(names) => {
                all.insert(uid.to_string(), names);
            }
            None => {
                all.remove(uid);
            }
        }
        Ok(SharedSlugs(Arc::new(all)))
    }

    /// Every active shared definition's names, for a slug-to-uid read, from
    /// the store's slug index (`DefinitionStore::slug_index`): a stat per
    /// file, and a parse only of the files that changed. Read before taking
    /// the connection lock: it is file I/O.
    pub(in crate::backend::storage) fn shared_slugs(&self) -> SharedSlugs {
        let Some(reg) = self.shared_def_registry() else {
            return SharedSlugs::default();
        };
        match reg.slug_index() {
            Ok(index) => SharedSlugs(index),
            Err(e) => {
                tracing::warn!(error = %e, "agent slug: shared definitions unreadable; using this channel's slugs");
                SharedSlugs::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;
    use crate::registry::{DefinitionStore, FormerSlug};
    use std::sync::Arc;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    /// A local row holding `local` and a shared record that has moved on to
    /// `shared` (with `local` among its former slugs), as a rename leaves a
    /// channel it hasn't reached yet.
    fn diverged(uid: &str, local: &str, shared: &str) -> (tempfile::TempDir, Arc<Store>, Arc<DefinitionStore>) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let tmp = tempfile::tempdir().unwrap();
        let defs = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        store.set_def_registry(defs.clone());
        let mut def = test_agent_def(uid, "Loap #3", "claude", "host", 1, "");
        def.slug = local.to_string();
        store.agent_def_insert(&mut def).unwrap();
        let mut rec = defs.get(uid).unwrap().unwrap();
        // Written as Phase 2's rename will: straight to the file, since an
        // update keeps the slug.
        rec.data.slug = shared.to_string();
        rec.data.former_slugs = vec![FormerSlug { slug: local.to_string(), retired_at: 1 }];
        std::fs::write(defs.root().join(format!("{uid}.json")), serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
        (tmp, store, defs)
    }

    #[test]
    fn the_shared_slug_wins_and_an_empty_one_does_not() {
        assert_eq!(canonical_slug("u-rule", "loap-loap-2", Some("loap-2")), "loap-2");
        assert_eq!(canonical_slug("u-rule", "loap-loap-2", Some("  ")), "loap-loap-2");
        assert_eq!(canonical_slug("u-rule", "loap-loap-2", None), "loap-loap-2");
        assert_eq!(canonical_slug("u-rule", "", Some("loap-2")), "loap-2");
    }

    /// P0.1: every reader that changed gives the shared slug, by uid and by
    /// name, and the stale local slug resolves to nobody.
    #[test]
    fn every_reader_gives_one_answer() {
        let uid = "uid-p01-readers";
        let (_tmp, store, _defs) = diverged(uid, "loap-loap-2-loap-3", "loap-3");

        assert_eq!(store.agent_def_get(uid).unwrap().unwrap().slug, "loap-3");
        assert_eq!(store.agent_def_get_strict(uid).unwrap().unwrap().slug, "loap-3");
        let listed = store.agent_def_list().unwrap();
        assert_eq!(listed.iter().find(|d| d.id == uid).unwrap().slug, "loap-3");
        assert_eq!(store.agent_names_by_id(uid).unwrap().unwrap().slug, "loap-3");

        assert_eq!(store.instance_get_by_slug("loap-3").unwrap().map(|i| i.id).as_deref(), Some(uid));
        assert!(store.instance_get_by_slug("loap-loap-2-loap-3").unwrap().is_none());
        let by_name = store.agents_matching_name("loap-3").unwrap();
        assert_eq!(by_name.iter().map(|m| (m.id.as_str(), m.slug.as_str())).collect::<Vec<_>>(), vec![(uid, "loap-3")]);
        assert!(store.agents_matching_name("loap-loap-2-loap-3").unwrap().is_empty());
        assert_eq!(store.rows_answering_to("loap-3").unwrap().len(), 1);
        assert_eq!(store.agent_ids_for_slug_folded("LOAP-3").unwrap(), vec![uid.to_string()]);
        assert!(store.agent_ids_for_slug_folded("loap-loap-2-loap-3").unwrap().is_empty());
        assert_eq!(store.agent_slugs_folded("loap-3").unwrap(), vec!["loap-3".to_string()]);
    }

    /// The disagreement is logged, once per agent and pair of values.
    #[test]
    fn a_disagreement_is_logged_once() {
        let out = Capture::default();
        let subscriber = tracing_subscriber::fmt().with_writer(out.clone()).with_max_level(tracing::Level::WARN).finish();
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..3 {
                canonical_slug("uid-p01-log", "naki-naki-2", Some("naki-2"));
            }
        });
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        assert_eq!(text.matches("disagree").count(), 1, "{text}");
        assert!(text.contains("uid-p01-log") && text.contains("naki-naki-2") && text.contains("naki-2"), "{text}");
    }

    /// With no shared store, nothing changes: the local slug answers.
    #[test]
    fn without_a_shared_store_the_local_slug_answers() {
        let store = Store::open_in_memory().unwrap();
        let mut def = test_agent_def("uid-p01-local", "Solo", "claude", "host", 1, "");
        def.slug = "solo".to_string();
        store.agent_def_insert(&mut def).unwrap();
        assert_eq!(store.agent_def_get("uid-p01-local").unwrap().unwrap().slug, "solo");
        assert!(store.instance_get_by_slug("solo").unwrap().is_some());
        assert_eq!(store.agent_names_by_id("uid-p01-local").unwrap().unwrap().slug, "solo");
    }

    /// P0.4: deleting a renamed agent purges and tombstones the keys filed
    /// under its current slug and every former slug, in `objects.db` and
    /// `wan.db`, through both deletion paths.
    #[test]
    fn delete_covers_the_current_and_every_former_slug() {
        for via_instance_delete in [false, true] {
            let uid = if via_instance_delete { "uid-p04-instance" } else { "uid-p04-def" };
            let (_tmp, store, defs) = diverged(uid, "loap-loap-2", "loap-2");
            let mut rec = defs.get(uid).unwrap().unwrap();
            rec.data.former_slugs.push(FormerSlug { slug: "Loap-Older".to_string(), retired_at: 2 });
            std::fs::write(defs.root().join(format!("{uid}.json")), serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
            let (_wan_dir, wan) = crate::backend::storage::wan_identity::attach_temp_wan_identity(&store);
            let names = ["loap-2", "loap-loap-2", "loap-older"];
            for name in names {
                store.agent_jekt_key_ensure(name).unwrap();
                store.agent_lan_key_ensure(name).unwrap();
                wan.agent_key_ensure(name, None).unwrap();
            }

            let deleted = if via_instance_delete {
                store.instance_delete(uid).unwrap()
            } else {
                store.agent_def_delete(uid).unwrap()
            };
            assert!(deleted);
            for name in names {
                assert!(store.agent_jekt_key_load(name).unwrap().is_none(), "{name}");
                assert!(store.agent_lan_key_load(name).unwrap().is_none(), "{name}");
                assert!(wan.agent_key_load(name).unwrap().is_none(), "{name}");
            }
            let tombstones = {
                let conn = store.conn.lock().unwrap();
                crate::backend::storage::agents::key_tombstones_for_tests(&conn)
            };
            for name in names {
                assert!(tombstones.contains(&(name.to_string(), uid.to_string())), "{name}: {tombstones:?}");
            }
        }
    }

    /// Two channels (two `objects.db`, two handles on one shared directory)
    /// creating agents with the same base slug at once end with distinct
    /// slugs: the allocation lock spans the pick and the publication, so the
    /// second sees the first's record.
    #[test]
    fn concurrent_inserts_in_two_channels_get_distinct_slugs() {
        let tmp = tempfile::tempdir().unwrap();
        let channels: Vec<Arc<Store>> = (0..2)
            .map(|_| {
                let store = Arc::new(Store::open_in_memory().unwrap());
                store.set_def_registry(Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap()));
                store
            })
            .collect();
        for round in 0..8 {
            let start = Arc::new(std::sync::Barrier::new(2));
            let picked: Vec<String> = channels
                .iter()
                .enumerate()
                .map(|(i, store)| {
                    let (store, start) = (store.clone(), start.clone());
                    std::thread::spawn(move || {
                        let mut def = test_agent_def(&format!("uid-race-{round}-{i}"), "Racer", "claude", "host", 1, "");
                        def.slug = format!("racer-{round}");
                        start.wait();
                        store.agent_def_insert(&mut def).unwrap();
                        def.slug
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect();
            assert_ne!(picked[0], picked[1], "round {round}: {picked:?}");
        }
    }

    /// A new agent whose shared record can't be written is not inserted, by
    /// either insert path: its slug was never published, so the row would let
    /// another channel take the same slug.
    #[test]
    fn an_insert_whose_publication_fails_is_undone() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let defs = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        store.set_def_registry(defs.clone());
        // A directory where the record goes: the write fails, as on a full disk.
        std::fs::create_dir_all(defs.root().join("uid-unpublished.json")).unwrap();
        let mut def = test_agent_def("uid-unpublished", "Solo", "claude", "host", 1, "");
        def.slug = "solo".to_string();
        let err = store.agent_def_insert(&mut def).expect_err("the publish failed");
        assert!(err.to_string().contains("could not publish"), "{err}");
        assert!(store.agent_row_get("uid-unpublished").unwrap().is_none(), "the local row is undone");

        let mut defined = test_agent_def("uid-unpublished", "Solo", "claude", "host", 1, "");
        defined.slug = String::new();
        assert!(store.agent_def_find_or_insert(&mut defined).is_err());
        assert!(store.agent_row_get("uid-unpublished").unwrap().is_none());
    }

    /// `agent.define` finds an existing agent by its canonical slug, not by a
    /// stale local one, so it doesn't mint a second agent beside it.
    #[test]
    fn define_finds_an_agent_by_its_canonical_slug() {
        let uid = "uid-p01-define";
        let (_tmp, store, _defs) = diverged(uid, "loap-loap-2-loap-3", "loap-3");
        let mut asked = test_agent_def("uid-p01-define-new", "Loap 3", "claude", "host", 2, "");
        asked.slug = String::new();
        let found = store.agent_def_find_or_insert(&mut asked).unwrap().expect("the existing agent");
        assert_eq!((found.id.as_str(), found.slug.as_str()), (uid, "loap-3"));
        assert!(store.agent_row_get("uid-p01-define-new").unwrap().is_none(), "nothing inserted");
    }

    /// With no local row (an agent only the shared registry knows here, or a
    /// retry after an earlier attempt deleted the row and failed before
    /// retiring the record), a delete still purges and tombstones every slug
    /// the shared record holds. Both states look the same from here (an
    /// active record, no row); each runs through one of the delete paths.
    #[test]
    fn delete_purges_shared_names_without_a_local_row() {
        for (uid, retry) in [("uid-p04-shared-only", false), ("uid-p04-retry", true)] {
            let (_tmp, store, defs) = diverged(uid, "naki-naki-2", "naki-2");
            store.conn.lock().unwrap().execute("DELETE FROM db_agents WHERE id = ?1", params![uid]).unwrap();
            let (_wan_dir, wan) = crate::backend::storage::wan_identity::attach_temp_wan_identity(&store);
            for name in ["naki-2", "naki-naki-2"] {
                store.agent_jekt_key_ensure(name).unwrap();
                store.agent_lan_key_ensure(name).unwrap();
                wan.agent_key_ensure(name, None).unwrap();
            }
            if retry { store.instance_delete(uid).unwrap() } else { store.agent_def_delete(uid).unwrap() };
            let tombstones = {
                let conn = store.conn.lock().unwrap();
                crate::backend::storage::agents::key_tombstones_for_tests(&conn)
            };
            for name in ["naki-2", "naki-naki-2"] {
                assert!(store.agent_jekt_key_load(name).unwrap().is_none(), "{uid}: {name}");
                assert!(store.agent_lan_key_load(name).unwrap().is_none(), "{uid}: {name}");
                assert!(wan.agent_key_load(name).unwrap().is_none(), "{uid}: {name}");
                assert!(tombstones.contains(&(name.to_string(), uid.to_string())), "{uid}: {name}: {tombstones:?}");
            }
            assert!(!defs.exists(uid), "the record is retired");
        }
    }

    /// A delete that can't read the agent's own shared record (unparseable,
    /// or failing validation) refuses, through both paths, and leaves the
    /// row, its keys and the record as they were.
    #[test]
    fn delete_refuses_when_its_shared_record_is_unreadable() {
        for via_instance_delete in [false, true] {
            let uid = if via_instance_delete { "uid-p04-bad-instance" } else { "uid-p04-bad-def" };
            let (_tmp, store, defs) = diverged(uid, "loap-loap-2", "loap-2");
            for name in ["loap-2", "loap-loap-2"] {
                store.agent_jekt_key_ensure(name).unwrap();
            }
            let path = defs.root().join(format!("{uid}.json"));
            let mismatched = r#"{"schema_version":1,"data":{"id":"someone-else","name":"X","provider":"claude"}}"#;
            for bad in ["not json", mismatched] {
                std::fs::write(&path, bad).unwrap();
                let result = if via_instance_delete { store.instance_delete(uid) } else { store.agent_def_delete(uid) };
                let err = result.expect_err("an unreadable shared record refuses the delete");
                assert!(err.to_string().contains("Nothing was deleted"), "{err}");
                assert!(store.agent_row_get(uid).unwrap().is_some(), "the row stays");
                for name in ["loap-2", "loap-loap-2"] {
                    assert!(store.agent_jekt_key_load(name).unwrap().is_some(), "{name}'s key stays");
                }
                assert_eq!(std::fs::read_to_string(&path).unwrap(), bad, "the record stays");
            }
        }
    }

    /// A definition with no shared record (never mirrored, or no shared
    /// store at all) is still deleted, keys included.
    #[test]
    fn delete_proceeds_without_a_shared_record() {
        let plain = Store::open_in_memory().unwrap();
        let mut def = test_agent_def("uid-p04-plain", "Plain", "claude", "host", 1, "");
        def.slug = "plain".to_string();
        plain.agent_def_insert(&mut def).unwrap();
        plain.agent_jekt_key_ensure("plain").unwrap();
        assert!(plain.agent_def_delete("uid-p04-plain").unwrap());
        assert!(plain.agent_jekt_key_load("plain").unwrap().is_none());

        let (_tmp, store, defs) = diverged("uid-p04-absent", "absent", "absent");
        std::fs::remove_file(defs.root().join("uid-p04-absent.json")).unwrap();
        store.agent_jekt_key_ensure("absent").unwrap();
        assert!(store.instance_delete("uid-p04-absent").unwrap());
        assert!(store.agent_jekt_key_load("absent").unwrap().is_none());
    }

    /// The guard still applies: a former slug another agent holds as its
    /// current one is that agent's, and is left alone.
    #[test]
    fn delete_spares_a_name_another_agent_holds() {
        let uid = "uid-p04-spare";
        let (_tmp, store, _defs) = diverged(uid, "naki-naki-2", "naki-2");
        let mut other = test_agent_def("uid-p04-other", "Other", "claude", "host", 1, "");
        other.slug = "naki-naki-2-x".to_string();
        store.agent_def_insert(&mut other).unwrap();
        store.test_force_slug("uid-p04-other", "naki-naki-2").unwrap();
        store.agent_jekt_key_ensure("naki-naki-2").unwrap();
        store.agent_jekt_key_ensure("naki-2").unwrap();
        assert!(store.agent_def_delete(uid).unwrap());
        assert!(store.agent_jekt_key_load("naki-2").unwrap().is_none());
        assert!(store.agent_jekt_key_load("naki-naki-2").unwrap().is_some(), "the other agent's slug keeps its key");
    }

    /// A new agent can't take a slug the shared definitions hold: another
    /// agent's current slug that this channel's row hasn't caught up with,
    /// or a former slug. A cross-channel backfill keeps its own slug.
    #[test]
    fn a_new_slug_avoids_shared_current_and_former_slugs() {
        let uid = "uid-p01-reserve";
        let (_tmp, store, defs) = diverged(uid, "loap-loap-2", "loap-2");
        let mut rec = defs.get(uid).unwrap().unwrap();
        rec.data.former_slugs.push(FormerSlug { slug: "loap-ancient".to_string(), retired_at: 2 });
        std::fs::write(defs.root().join(format!("{uid}.json")), serde_json::to_vec_pretty(&rec).unwrap()).unwrap();

        for (id, base, want) in [("uid-new-a", "loap-2", "loap-2-2"), ("uid-new-b", "Loap-Ancient", "Loap-Ancient-2")] {
            let mut def = test_agent_def(id, "New", "claude", "host", 1, "");
            def.slug = base.to_string();
            store.agent_def_insert(&mut def).unwrap();
            assert_eq!(def.slug, want);
        }
        assert_eq!(store.instance_get_by_slug("loap-2").unwrap().map(|i| i.id).as_deref(), Some(uid), "still one owner");

        let remote = crate::registry::DefinitionRecord {
            schema_version: crate::registry::DEF_MAX_SUPPORTED_SCHEMA,
            data: crate::registry::DefinitionRecordV1 {
                id: "uid-remote".to_string(),
                slug: "remote-slug".to_string(),
                name: "Remote".to_string(),
                provider: "claude".to_string(),
                ..Default::default()
            },
        };
        defs.upsert(&remote).unwrap();
        assert!(store.agent_row_ensure_local("uid-remote").unwrap());
        assert_eq!(store.agent_row_get("uid-remote").unwrap().unwrap().slug, "remote-slug");
    }

    /// P0.2 at the store: an edit from the channel holding the old slug
    /// mirrors, and leaves the shared slug where the rename put it.
    #[test]
    fn an_edit_from_a_stale_channel_mirrors_without_moving_the_slug() {
        let uid = "uid-p02-edit";
        let (_tmp, store, defs) = diverged(uid, "naki-naki-2", "naki-2");
        let mut def = store.agent_def_get(uid).unwrap().unwrap();
        def.description = "edited".to_string();
        def.slug = "naki-naki-2".to_string();
        assert!(store.agent_def_update(&mut def).unwrap());
        let rec = defs.get(uid).unwrap().unwrap();
        assert_eq!(rec.data.description, "edited");
        assert_eq!(rec.data.slug, "naki-2");
        assert_eq!(rec.data.former_slugs.len(), 1);
    }
}
