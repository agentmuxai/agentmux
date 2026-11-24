// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's records: what a capture writes from the Kanban's own tables (and nothing twice), what it ends,
// and the merge rule per record between machines.

use agentmux_common::jekt_sign::generate_lan_keypair;
use rusqlite::Connection;
use serde_json::Value;

use super::hlc::Clock;
use super::kanban::{self, collection_id, Captured, KanbanAdapter};
use super::log::{self, Outcome, Pins, Signer, Vector};
use crate::backend::storage::work_board::{CardUpdate, NewCard, WorkEvent};
use crate::backend::storage::work_queue::{ClaimFilter, WorkItem};
use crate::backend::storage::store::Store;

const NOW: i64 = 1_800_000_000_000;

struct Local {
    store: Store,
    _dir: tempfile::TempDir,
    board: String,
    private: [u8; 32],
    clock: Clock,
}

fn local() -> Local {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    kanban::init(&store.conn().lock().unwrap()).unwrap();
    let board = store.work_board_default().unwrap().id;
    let (_, private) = generate_lan_keypair([7; 32]);
    Local { store, _dir: dir, board, private, clock: Clock::new("m1") }
}

fn item(id: &str) -> WorkItem {
    WorkItem {
        id: id.into(),
        title: format!("task {id}"),
        payload: "do it".into(),
        kind: String::new(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: String::new(),
        claimed_by: String::new(),
        target_agent_uid: String::new(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: "tester".into(),
        created_by_uid: String::new(),
        created_at: NOW,
        updated_at: NOW,
        not_before: None,
        result: String::new(),
    }
}

impl Local {
    fn file(&self, id: &str) {
        self.store.work_queue_enqueue(&item(id)).unwrap();
        self.store.work_card_create(id, &NewCard { filed_by: "Loap-3".into(), ..Default::default() }, NOW).unwrap();
    }

    fn capture(&mut self, at: i64) -> Captured {
        let conn = self.store.conn().lock().unwrap();
        let mut signer = Signer { machine: "m1", private: &self.private, clock: &mut self.clock };
        kanban::capture(&conn, &mut signer, &self.board, at).unwrap()
    }

    /// A record's current value in this board's log, as JSON (None for a tombstone or no record).
    fn value(&self, board: &str, record: &str) -> Option<Value> {
        let conn = self.store.conn().lock().unwrap();
        let rec = log::current(&conn, &collection_id(board), record).unwrap()?;
        (!rec.header.tombstone).then(|| serde_json::from_slice(&rec.body).unwrap())
    }

    fn sql(&self, sql: &str) {
        self.store.conn().lock().unwrap().execute(sql, []).unwrap();
    }
}

#[test]
fn a_task_is_written_once_and_an_edit_writes_only_what_it_changed() {
    let mut k = local();
    k.file("a");
    let first = k.capture(NOW + 1);
    assert!(first.written > 15, "every field of a new task: {first:?}");
    assert_eq!(first.tombstoned, 0);
    assert_eq!(k.capture(NOW + 2), Captured::default(), "nothing changed, nothing written");
    let b = k.board.clone();
    assert_eq!(k.value(&b, "a.title"), Some(Value::String("task a".into())));
    assert_eq!(k.value(&b, "a.filed_by"), Some(Value::String("Loap-3".into())));

    k.store.work_card_update("a", &CardUpdate { note: Some("hello".into()), ..Default::default() }, NOW + 3).unwrap();
    assert_eq!(k.capture(NOW + 4), Captured { written: 1, tombstoned: 0 }, "one field changed");
    assert_eq!(k.value(&b, "a.note"), Some(Value::String("hello".into())));
}

#[test]
fn the_lifecycle_is_one_record_and_a_lease_is_not_recorded() {
    let mut k = local();
    k.file("a");
    k.capture(NOW + 1);
    let b = k.board.clone();
    assert_eq!(k.value(&b, "a.life").unwrap()["state"], "open");
    let f = ClaimFilter { kind: None, agent_id: "naki-2".into(), agent_uid: String::new(), groups: vec![] };
    k.store.work_queue_claim(&f, NOW + 5, 60_000).unwrap().unwrap();
    assert_eq!(k.capture(NOW + 6), Captured { written: 1, tombstoned: 0 }, "state, holder and attempts move together");
    let life = k.value(&b, "a.life").unwrap();
    assert_eq!((life["state"].clone(), life["claimed_by"].clone(), life["attempts"].clone()), ("claimed".into(), "naki-2".into(), 1.into()));
    assert!(life.get("claim_expires").is_none());
    // A heartbeat only moves the lease.
    k.sql("UPDATE db_work_queue SET claim_expires = claim_expires + 60000, updated_at = updated_at + 1");
    assert_eq!(k.capture(NOW + 7), Captured::default());
}

#[test]
fn a_dependency_taken_off_and_a_task_that_is_gone_end_their_records() {
    let mut k = local();
    k.file("a");
    k.file("b");
    assert_eq!(k.store.work_deps_set("b", &["a".to_string()]).unwrap(), None);
    k.capture(NOW + 1);
    let b = k.board.clone();
    assert_eq!(k.value(&b, "b.after.a"), Some(Value::Bool(true)));
    assert_eq!(k.store.work_deps_set("b", &[]).unwrap(), None);
    assert_eq!(k.capture(NOW + 2), Captured { written: 0, tombstoned: 1 });
    assert_eq!(k.value(&b, "b.after.a"), None);
    k.sql("DELETE FROM db_work_queue WHERE id = 'a'");
    k.sql("DELETE FROM db_work_cards WHERE work_id = 'a'");
    let gone = k.capture(NOW + 3);
    assert!(gone.tombstoned > 15 && gone.written == 0, "{gone:?}");
    assert_eq!(k.value(&b, "a.title"), None);
    assert_eq!(k.value(&b, "b.title"), Some(Value::String("task b".into())), "the other task is untouched");
    assert_eq!(k.capture(NOW + 4), Captured::default());
}

#[test]
fn a_task_moved_to_another_board_is_ended_on_the_old_one_and_written_on_the_new() {
    let mut k = local();
    k.file("a");
    k.capture(NOW + 1);
    let old = k.board.clone();
    k.sql("UPDATE db_work_cards SET board_id = 'other' WHERE work_id = 'a'");
    let moved = k.capture(NOW + 2);
    assert!(moved.written > 15 && moved.tombstoned > 15, "{moved:?}");
    assert_eq!(k.value(&old, "a.title"), None);
    assert_eq!(k.value("other", "a.title"), Some(Value::String("task a".into())));
}

#[test]
fn history_lines_are_written_once_each_under_the_machine_that_made_them() {
    let mut k = local();
    k.file("a");
    let ev = |kind: &str| WorkEvent {
        id: 0,
        work_id: "a".into(),
        at: NOW,
        actor: "Loap-3".into(),
        actor_kind: "agent".into(),
        kind: kind.into(),
        from_state: String::new(),
        to_state: "open".into(),
        body: String::new(),
    };
    k.store.work_event_add(&ev("created")).unwrap();
    let first = k.capture(NOW + 1);
    let b = k.board.clone();
    let id: i64 = k.store.conn().lock().unwrap().query_row("SELECT MIN(id) FROM db_work_events", [], |r| r.get(0)).unwrap();
    assert_eq!(k.value(&b, &format!("a.event.m1.{id}")).unwrap()["kind"], "created");
    assert!(first.written > 15);
    k.store.work_event_add(&ev("claimed")).unwrap();
    assert_eq!(k.capture(NOW + 2), Captured { written: 1, tombstoned: 0 }, "only the new line");
    assert_eq!(k.capture(NOW + 3), Captured::default());
}

struct Peer {
    id: String,
    conn: Connection,
    public: [u8; 32],
    private: [u8; 32],
    clock: Clock,
}

fn peer(name: &str, seed: u8) -> Peer {
    let (public, private) = generate_lan_keypair([seed; 32]);
    let conn = Connection::open_in_memory().unwrap();
    log::init(&conn).unwrap();
    log::ensure_collection(&conn, "kanban.main", "kanban", NOW).unwrap();
    Peer { id: name.into(), conn, public, private, clock: Clock::new(name) }
}

impl Peer {
    fn write(&mut self, record: &str, body: &str, at: i64) {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        log::append_local(&self.conn, &mut signer, "kanban.main", record, body.as_bytes().to_vec(), false, at).unwrap();
    }

    fn delete(&mut self, record: &str, at: i64) {
        let mut signer = Signer { machine: &self.id, private: &self.private, clock: &mut self.clock };
        log::append_local(&self.conn, &mut signer, "kanban.main", record, Vec::new(), true, at).unwrap();
    }

    fn state(&self, record: &str) -> Option<String> {
        let r = log::current(&self.conn, "kanban.main", record).unwrap()?;
        (!r.header.tombstone).then(|| String::from_utf8(r.body).unwrap())
    }
}

/// What `to` takes from `from`.
fn take(from: &Peer, to: &mut Peer, all: &Pins, at: i64) {
    let mine = log::vector(&to.conn, "kanban.main").unwrap();
    for r in log::since(&from.conn, "kanban.main", &mine, 1000).unwrap() {
        let outcome = log::apply_remote(&to.conn, &r, all, &mut to.clock, at, &KanbanAdapter).unwrap();
        assert_ne!(outcome, Outcome::Conflict);
    }
}

fn exchange(a: &mut Peer, b: &mut Peer, at: i64) {
    let all: Pins = [(a.id.clone(), a.public), (b.id.clone(), b.public)].into();
    take(a, b, &all, at);
    take(b, a, &all, at);
}

#[test]
fn two_machines_editing_different_fields_of_a_card_both_keep_their_edit() {
    let mut a = peer("a", 1);
    let mut b = peer("b", 2);
    a.write("t.title", "\"one\"", NOW);
    a.write("t.note", "\"\"", NOW);
    exchange(&mut a, &mut b, NOW + 1);
    a.write("t.note", "\"from a\"", NOW + 10);
    b.write("t.title", "\"from b\"", NOW + 20);
    exchange(&mut a, &mut b, NOW + 30);
    for p in [&a, &b] {
        assert_eq!(p.state("t.note").as_deref(), Some("\"from a\""));
        assert_eq!(p.state("t.title").as_deref(), Some("\"from b\""));
    }
    // The same field edited on both: the newer wins, on both.
    a.write("t.note", "\"a again\"", NOW + 40);
    b.write("t.note", "\"b later\"", NOW + 50);
    exchange(&mut a, &mut b, NOW + 60);
    assert!(a.state("t.note") == b.state("t.note") && a.state("t.note").as_deref() == Some("\"b later\""));
}

#[test]
fn a_deletion_wins_over_an_older_edit_and_loses_to_a_newer_one() {
    let mut a = peer("a", 1);
    let mut b = peer("b", 2);
    a.write("t.after.x", "true", NOW);
    exchange(&mut a, &mut b, NOW + 1);
    a.delete("t.after.x", NOW + 20);
    b.write("t.after.x", "true", NOW + 10);
    exchange(&mut a, &mut b, NOW + 30);
    assert_eq!((a.state("t.after.x"), b.state("t.after.x")), (None, None), "the later deletion wins");
    b.write("t.after.x", "true", NOW + 40);
    exchange(&mut a, &mut b, NOW + 50);
    assert_eq!((a.state("t.after.x").as_deref(), b.state("t.after.x").as_deref()), (Some("true"), Some("true")), "the later edit restores it");
}

#[test]
fn a_history_line_is_written_once_and_the_first_one_held_stays() {
    let mut a = peer("a", 1);
    let mut b = peer("b", 2);
    a.write("t.event.a.1", "{\"kind\":\"created\"}", NOW);
    b.write("t.event.a.1", "{\"kind\":\"other\"}", NOW + 5);
    // B holds its own copy first; A's arrives later and does not replace it.
    let all: Pins = [("a".to_string(), a.public), ("b".to_string(), b.public)].into();
    for r in log::since(&a.conn, "kanban.main", &Vector::new(), 10).unwrap() {
        assert_eq!(log::apply_remote(&b.conn, &r, &all, &mut b.clock, NOW + 6, &KanbanAdapter).unwrap(), Outcome::KeptLocal);
    }
    assert_eq!(b.state("t.event.a.1").as_deref(), Some("{\"kind\":\"other\"}"));
}
