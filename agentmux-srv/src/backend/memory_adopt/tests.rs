// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const UID: &str = "agent-adopt";
const NAME: &str = "-work-adopt";

struct Machine {
    shared: tempfile::TempDir,
    fs: FileStore,
    store: Store,
}

fn machine() -> Machine {
    let store = Store::open_in_memory().unwrap();
    let mut def = crate::backend::storage::agents::test_agent_def(UID, UID, "claude", "agent", 1, "");
    def.working_directory = "/work/adopt".into();
    store.agent_def_insert(&mut def).unwrap();
    Machine { shared: tempfile::tempdir().unwrap(), fs: FileStore::open_in_memory().unwrap(), store }
}

impl Machine {
    fn dir(&self, account: &str) -> PathBuf {
        self.shared.path().join("identities").join(account).join("claude").join("projects").join(NAME).join("memory")
    }
    fn put(&self, account: &str, name: &str, body: &str, age_secs: u64) {
        let dir = self.dir(account);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let when = std::time::SystemTime::now() - Duration::from_secs(age_secs);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
    }
    fn list(&self) -> AdoptionList {
        list_in(&self.fs, &self.store, UID, &self.dir("now"), self.shared.path()).unwrap()
    }
    fn record(&self, name: &str, body: &str) {
        let head = record::heads(&self.fs, UID).unwrap().files.get(name).map(|h| h.version.clone());
        record::append_version(
            &self.fs,
            UID,
            NewVersion {
                file: name,
                body: Some(body.as_bytes()),
                expected_parent: head.as_deref(),
                merged_parent: None,
                conflicts_with: None,
                source: "agent",
                source_detail: "",
                project_to: None,
            },
        )
        .unwrap();
    }
    fn head(&self, name: &str) -> Option<String> {
        let sha = record::heads(&self.fs, UID).unwrap().files.get(name)?.sha256.clone()?;
        Some(String::from_utf8(record::body(&self.fs, UID, &sha).unwrap().unwrap()).unwrap())
    }
    fn bodies(&self, name: &str) -> Vec<String> {
        record::history(&self.fs, UID, name)
            .unwrap()
            .into_iter()
            .filter_map(|v| v.sha256)
            .map(|sha| String::from_utf8(record::body(&self.fs, UID, &sha).unwrap().unwrap()).unwrap())
            .collect()
    }
}

fn choice(c: &Candidate) -> (usize, String) {
    (c.index, c.dir_hash.clone())
}

#[test]
fn the_list_offers_other_accounts_folders_not_the_agents_own_or_anothers() {
    let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let m = machine();
    m.put("now", "MEMORY.md", "current", 0);
    m.put("old-a", "MEMORY.md", "a", 10);
    m.put("old-b", "notes.md", "b", 10);
    std::fs::create_dir_all(m.dir("empty")).unwrap();
    m.put("theirs", "MEMORY.md", "another agent's", 10);
    crate::backend::memory_dir_claims::check_and_claim(
        &m.fs,
        &m.store,
        "agent-other",
        &m.dir("theirs"),
        "/work/other",
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    let list = m.list();
    let accounts: Vec<&str> = list.candidates.iter().map(|c| c.account.as_str()).collect();
    assert_eq!(accounts, vec!["old-a", "old-b"]);
    assert_eq!(list.candidates[1].files[0].name, "notes.md");
    assert_eq!(list.candidates[1].files[0].sha256, record::sha256_hex(b"b"));
}

/// The agent's current memory keeps its heads; what it didn't have is
/// adopted; the indexes are unioned by link target.
#[test]
fn adopting_adds_missing_files_keeps_current_ones_and_unions_the_index() {
    let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let m = machine();
    m.record("MEMORY.md", "# Memory\n- [Notes](notes.md) — current\n");
    m.record("notes.md", "current notes");
    m.put("old", "MEMORY.md", "# Memory\n- [Notes](notes.md) — old wording\n- [Topic](topic.md) — only here\n", 100);
    m.put("old", "notes.md", "old notes", 100);
    m.put("old", "topic.md", "a topic the new account lost", 100);
    let list = m.list();
    let r = adopt(&m.fs, &m.store, UID, &list.list_id, &[choice(&list.candidates[0])]).unwrap();
    assert_eq!(r, AdoptReport { files_added: 1, versions_kept: 2, index_lines_added: 1 });
    assert_eq!(m.head("notes.md").as_deref(), Some("current notes"), "current memory keeps its head");
    assert!(m.bodies("notes.md").contains(&"old notes".to_string()), "the old body is history");
    assert_eq!(m.head("topic.md").as_deref(), Some("a topic the new account lost"));
    assert_eq!(
        m.head("MEMORY.md").as_deref(),
        Some("# Memory\n- [Notes](notes.md) — current\n- [Topic](topic.md) — only here\n")
    );
    // The list is spent.
    assert_eq!(adopt(&m.fs, &m.store, UID, &list.list_id, &[choice(&list.candidates[0])]), Err(AdoptError::UnknownList));
}

/// A file in several chosen folders: every body kept, the newest the head.
#[test]
fn the_newest_body_of_a_new_file_becomes_its_head() {
    let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let m = machine();
    m.put("older", "topic.md", "older", 200);
    m.put("newer", "topic.md", "newer", 100);
    let list = m.list();
    let all: Vec<_> = list.candidates.iter().map(choice).collect();
    adopt(&m.fs, &m.store, UID, &list.list_id, &all).unwrap();
    assert_eq!(m.head("topic.md").as_deref(), Some("newer"));
    assert_eq!(m.bodies("topic.md"), vec!["older".to_string(), "newer".to_string()]);
}

#[test]
fn a_folder_changed_since_the_list_is_refused_and_nothing_is_recorded() {
    let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let m = machine();
    m.put("old", "topic.md", "as listed", 100);
    let list = m.list();
    m.put("old", "topic.md", "changed after", 0);
    assert_eq!(
        adopt(&m.fs, &m.store, UID, &list.list_id, &[choice(&list.candidates[0])]),
        Err(AdoptError::Changed { index: 0 })
    );
    assert!(record::heads(&m.fs, UID).unwrap().files.is_empty());
}

/// Choices are checked against the issued list: its agent, index and hash.
#[test]
fn only_choices_from_the_agents_own_list_are_accepted() {
    let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let m = machine();
    m.put("old", "topic.md", "t", 100);
    let list = m.list();
    let c = choice(&list.candidates[0]);
    assert_eq!(adopt(&m.fs, &m.store, "agent-else", &list.list_id, &[c.clone()]), Err(AdoptError::UnknownList));
    assert_eq!(adopt(&m.fs, &m.store, UID, "no-such-list", &[c.clone()]), Err(AdoptError::UnknownList));
    assert_eq!(adopt(&m.fs, &m.store, UID, &list.list_id, &[(7, c.1.clone())]), Err(AdoptError::BadChoice));
    assert_eq!(adopt(&m.fs, &m.store, UID, &list.list_id, &[(0, "wrong".into())]), Err(AdoptError::BadChoice));
    assert_eq!(adopt(&m.fs, &m.store, UID, &list.list_id, &[]), Err(AdoptError::BadChoice));
    assert!(record::heads(&m.fs, UID).unwrap().files.is_empty());
}

#[test]
fn a_folder_another_agent_claimed_since_the_list_is_refused() {
    let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let m = machine();
    m.put("old", "topic.md", "t", 100);
    let list = m.list();
    crate::backend::memory_dir_claims::check_and_claim(
        &m.fs,
        &m.store,
        "agent-other",
        &m.dir("old"),
        "/work/other",
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(
        adopt(&m.fs, &m.store, UID, &list.list_id, &[choice(&list.candidates[0])]),
        Err(AdoptError::Changed { index: 0 })
    );
}

#[test]
fn nothing_is_adopted_while_a_reconcile_holds_the_lease() {
    let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let m = machine();
    m.put("old", "topic.md", "t", 100);
    let list = m.list();
    let lease = crate::backend::memory_reconcile::take_pass_lease(&m.fs, UID, Instant::now() + Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(adopt(&m.fs, &m.store, UID, &list.list_id, &[choice(&list.candidates[0])]), Err(AdoptError::Busy));
    crate::backend::memory_reconcile::release_pass_lease(&m.fs, UID, &lease);
    assert!(adopt(&m.fs, &m.store, UID, &list.list_id, &[choice(&list.candidates[0])]).is_ok());
}

#[test]
fn the_index_union_dedups_by_link_target_and_keeps_other_lines_once() {
    let base = "# Memory\n- [A](a.md) — mine\n";
    let (out, added) = union_index(base, &["# Memory\n- [A](a.md) — theirs\n- [B](b.md)\nloose note\n", "- [B](b.md) again\nloose note\n"]);
    assert_eq!(out, "# Memory\n- [A](a.md) — mine\n- [B](b.md)\nloose note\n");
    assert_eq!(added, 2);
}

/// Files held in the agent's own folder at first sighting (another agent's
/// record had the same content) are on the list too; adopting them records
/// them and ends the hold.
#[test]
fn held_files_in_the_agents_own_folder_are_offered_and_adopting_ends_the_hold() {
    let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let m = machine();
    m.put("now", "copied.md", "shared fact", 0);
    m.put("now", "rewritten.md", "was held, since changed", 0);
    let own = crate::backend::memory_dir_claims::dir_id(&m.dir("now"));
    record::hold_for_adoption(
        &m.fs,
        UID,
        &own,
        &[
            ("copied.md".into(), record::sha256_hex(b"shared fact")),
            ("rewritten.md".into(), record::sha256_hex(b"the held content")),
        ],
    )
    .unwrap();
    let list = m.list();
    assert_eq!(list.candidates.len(), 1);
    let held = &list.candidates[0];
    assert_eq!(held.account, "held");
    assert_eq!(held.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["copied.md"], "only while it still holds what was held");
    let r = adopt(&m.fs, &m.store, UID, &list.list_id, &[choice(held)]).unwrap();
    assert_eq!(r.files_added, 1);
    assert_eq!(m.head("copied.md").as_deref(), Some("shared fact"));
    assert!(!record::heads(&m.fs, UID).unwrap().held.get(&own).is_some_and(|h| h.contains_key("copied.md")));
    assert!(m.list().candidates.is_empty(), "no longer offered");
}
