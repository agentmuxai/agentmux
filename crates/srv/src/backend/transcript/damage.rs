// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A zone of the shared transcript store that SQLite reports as corrupt
//! (`SQLITE_CORRUPT`, "database disk image is malformed").
//!
//! Until now that was a WARN line per failed append: on one machine a single
//! agent's zone logged 218,133 of them over thirteen days while nothing told the
//! user and every line kept being sent to the damaged pages
//! (docs/analysis/RCA_UPGRADE_0_59_30_TO_0_59_34_ISSUES_2026_10_19.md §6). Now
//! the first corrupt error on a zone, from an append, a tail read or an index
//! rebuild:
//!
//! - marks the zone damaged for the rest of the process: appends to it skip the
//!   shared store (the block's own copy keeps being written) and readers use the
//!   block's own copy instead;
//! - records one incident for the user and tells the agent once.
//!
//! Only the shared store's zones are marked: a block's own store is a
//! per-channel file and is not touched by this.

use std::path::Path;
use std::sync::Mutex;

use crate::backend::storage::error::StoreError;

/// The broker event that shows a recorded incident in every open window now; the window shows
/// what is in `incidents.jsonl` again when it starts.
pub const EVENT_INCIDENT_RECORDED: &str = "incident:recorded";

static BROKER: std::sync::OnceLock<std::sync::Arc<crate::backend::mps::Broker>> = std::sync::OnceLock::new();

/// Attach the broker (bootstrap), so an incident reaches the open windows when it is recorded.
pub fn install_broker(broker: std::sync::Arc<crate::backend::mps::Broker>) {
    let _ = BROKER.set(broker);
}

/// The `kind` of the incident (`agentmux_common::incidents::Incident::kind`).
pub const INCIDENT_KIND: &str = "transcript_damaged";

/// Whether `e` is SQLite saying the database file is corrupt.
pub fn is_corrupt(e: &StoreError) -> bool {
    matches!(e, StoreError::Sqlite(rusqlite::Error::SqliteFailure(f, _)) if f.code == rusqlite::ErrorCode::DatabaseCorrupt)
}

/// The zones found damaged, in the order they were found.
pub struct Damaged {
    zones: Mutex<Vec<String>>,
}

impl Damaged {
    pub const fn new() -> Self {
        Damaged { zones: Mutex::new(Vec::new()) }
    }

    /// Marks `zone`; true the first time, false when it was already marked.
    pub fn mark(&self, zone: &str) -> bool {
        let mut zones = self.zones.lock().unwrap_or_else(|e| e.into_inner());
        if zones.iter().any(|z| z == zone) {
            return false;
        }
        zones.push(zone.to_string());
        true
    }

    pub fn contains(&self, zone: &str) -> bool {
        self.zones.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|z| z == zone)
    }

    /// How many zones are marked: a reader compares it before and after a read to
    /// see that the read found one.
    pub fn count(&self) -> usize {
        self.zones.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

static DAMAGED: Damaged = Damaged::new();

/// Whether `zone` of the shared store was found corrupt in this process.
pub fn is_damaged(zone: &str) -> bool {
    DAMAGED.contains(zone)
}

/// How many zones have been found damaged in this process.
pub fn damaged_count() -> usize {
    DAMAGED.count()
}

/// If `e` is a corrupt-database error from `op` on `zone` of the shared store,
/// marks the zone damaged and, the first time, records the incident and tells
/// the agent. Returns whether `e` was one, so a caller can fall back to the
/// block's own copy instead of treating it as a transient failure.
pub fn note_corrupt(zone: &str, op: &str, e: &StoreError) -> bool {
    if !is_corrupt(e) {
        return false;
    }
    if DAMAGED.mark(zone) {
        tracing::error!(
            zone = %zone,
            op = %op,
            error = %e,
            "shared transcript store: SQLITE_CORRUPT; no more writes to this zone and its readers use the block's own copy for the rest of this process"
        );
        announce(zone, op);
    }
    true
}

/// [`note_corrupt`] for a reader that has already turned the error into text
/// (the segment log, the archive and snapshot code return `String` errors): SQLite's
/// own wording for a corrupt file is what is looked for.
pub fn note_corrupt_text(zone: &str, op: &str, text: &str) -> bool {
    if !(text.contains("database disk image is malformed") || text.contains("malformed database schema")) {
        return false;
    }
    if DAMAGED.mark(zone) {
        tracing::error!(
            zone = %zone,
            op = %op,
            error = %text,
            "shared transcript store: SQLITE_CORRUPT; no more writes to this zone and its readers use the block's own copy for the rest of this process"
        );
        announce(zone, op);
    }
    true
}

/// The shared transcript store, unless `zone` of it was found corrupt in this process: a
/// reader with a copy of its own to fall back to asks this instead of
/// `agent_session::global_transcript_store()`.
pub fn global_for(zone: &str) -> Option<&'static std::sync::Arc<crate::backend::storage::filestore::FileStore>> {
    crate::backend::agent_session::global_transcript_store().filter(|_| !is_damaged(zone))
}

/// [`note_corrupt`] when `fs` is the shared store itself, which a reader of
/// either store (the index code takes a store and a zone) cannot tell otherwise;
/// false, and nothing marked, for any other store.
pub fn note_if_shared(fs: &crate::backend::storage::filestore::FileStore, zone: &str, op: &str, e: &StoreError) -> bool {
    is_shared(fs) && note_corrupt(zone, op, e)
}

fn is_shared(fs: &crate::backend::storage::filestore::FileStore) -> bool {
    #[cfg(test)]
    {
        if test_support::registered(fs) {
            return true;
        }
    }
    crate::backend::agent_session::global_transcript_store().is_some_and(|g| std::ptr::eq(g.as_ref(), fs))
}

/// The agent's id in an `agent:<id>:current` (or `:archive:`) zone.
///
/// A transcript zone is keyed by the agent's id (`agentId` in the block's meta, the
/// agent's own `definition_id`) and a segment log by its `AGENTMUX_AGENT_UID`
/// (`db_agents.id`, see `Controller::stable_agent_uid`). The code names both as the
/// agent's own id; if they ever differ for an agent, it gets a notice for each.
fn agent_of(zone: &str) -> Option<&str> {
    zone.strip_prefix("agent:")
        .or_else(|| zone.strip_prefix("agent-uid:"))?
        .split(':')
        .next()
        .filter(|id| !id.is_empty())
}

fn short(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(i, _)| &id[..i])
}

/// The incident for a damaged zone, as the user is told about it.
pub fn incident_for(zone: &str, op: &str) -> agentmux_common::incidents::Incident {
    let label = agent_of(zone).map_or_else(|| format!("zone {zone}"), |id| format!("agent {}", short(id)));
    let mut i = agentmux_common::incidents::Incident::new(INCIDENT_KIND, format!("Saved history for {label} is damaged"), "srv");
    i.cause = Some(format!("The shared transcript database reported corruption ({op})."));
    i.did = vec![
        "AgentMux stopped writing this history to the shared database and keeps saving it in this version's own copy, so nothing new is lost.".to_string(),
        "Its history views read that copy instead.".to_string(),
        "Before you repair the shared database, quit every AgentMux window and keep the shared/agents/transcripts folder as it is: repairing it needs the files in it.".to_string(),
    ];
    i.lost = vec!["Lines the shared database failed to take, and what was on its damaged pages.".to_string()];
    // The full agent id, which `what` shortens: a restart finds the same damage by it.
    i.evidence = i.evidence.map(|e| format!("{e}, {}", zone_tag(zone)));
    i
}

/// What a restart finds the same damage by: the agent, as one notice covers all of its zones
/// (its transcript, segment log and archives), else the zone.
fn zone_tag(zone: &str) -> String {
    agent_of(zone).map_or_else(|| format!("zone {zone}"), |id| format!("agent {id}"))
}

static TOLD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// True the first time `tag` is asked: the agent is told once for all of its zones.
fn first_to_tell(tag: &str) -> bool {
    let mut told = TOLD.lock().unwrap_or_else(|e| e.into_inner());
    if told.iter().any(|t| t == tag) {
        return false;
    }
    told.push(tag.to_string());
    true
}

/// What the agent is told, once.
pub fn agent_note() -> String {
    "[AgentMux] The saved copy of your conversation history in the shared transcript database is damaged (the database reported corruption). \
     AgentMux has stopped writing it there and keeps saving it in its own copy, so nothing new is lost; your own session context is not affected. \
     You need not do anything. If the user asks, tell them to quit every AgentMux window before repairing the shared transcripts folder, and not to delete it."
        .to_string()
}

/// Writes the incident to `incidents.jsonl` in `dir` unless an unacknowledged one
/// for the same agent is already there (a restart finds the same damage again).
pub fn write_incident(dir: &Path, zone: &str, op: &str) -> Option<agentmux_common::incidents::Incident> {
    let path = dir.join(agentmux_common::incidents::FILE_NAME);
    let incident = incident_for(zone, op);
    let seen = agentmux_common::incidents::read(&path, 200)
        .iter()
        .any(|i| i.kind == INCIDENT_KIND && i.acknowledged_at_ms.is_none() && i.evidence.as_deref().is_some_and(|e| e.ends_with(&zone_tag(zone))));
    if seen {
        return None;
    }
    if let Err(e) = agentmux_common::incidents::append(&path, &incident) {
        tracing::warn!(error = %e, zone = %zone, "shared transcript store: couldn't record the damage as an incident");
        return None;
    }
    Some(incident)
}

/// Shows `incident` in every open window now. One not published here (already open from an
/// earlier start, or no window yet) is shown from the file when a window starts.
fn publish(incident: &agentmux_common::incidents::Incident) {
    if let Some(broker) = BROKER.get() {
        publish_to(broker, incident);
    }
}

fn publish_to(broker: &crate::backend::mps::Broker, incident: &agentmux_common::incidents::Incident) {
    broker.publish_event::<crate::backend::mps_events::IncidentRecorded>(&incident.into(), vec![], 0);
}

fn announce(zone: &str, op: &str) {
    match std::env::var_os("AGENTMUX_DATA_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => {
            if let Some(incident) = write_incident(Path::new(&dir), zone, op) {
                publish(&incident);
            }
        }
        None => tracing::warn!(zone = %zone, "shared transcript store: no data folder to record the damage in"),
    }
    let Some(id) = agent_of(zone).map(str::to_string) else { return };
    if !first_to_tell(&zone_tag(zone)) {
        return;
    }
    let text = agent_note();
    let tell = move || {
        let handler = crate::backend::reactive::get_global_handler();
        // It asks nothing of the agent, so it waits for its next turn.
        if let Err(e) = handler.hold_system_note(&id, "transcript-damage", &text) {
            // Not running, or not registered yet: the incident has told the user.
            tracing::info!(agent = %id, error = %e, "shared transcript store: the agent wasn't told about the damage");
        }
    };
    // The caller may hold a lock of the transcript path: tell from a thread of its own.
    if let Err(e) = std::thread::Builder::new().name("transcript-damage-note".into()).spawn(tell) {
        tracing::warn!(error = %e, "shared transcript store: couldn't start the note to the agent");
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::backend::storage::filestore::FileStore;
    use std::sync::Mutex;

    /// Makes every access to the store fail with SQLite's own `SQLITE_CORRUPT`, as
    /// in a process that starts on a damaged file: the schema's root pages are
    /// pointed past the end of the file, which is what a damaged page looks like
    /// to a query that walks it.
    pub(crate) fn corrupt(fs: &FileStore) {
        fs.drop_cache_for_test();
        break_tables(fs);
    }

    /// [`corrupt`] for a process that already holds the zone's file in its cache
    /// (`stat` answers from it): the damage shows on the first read of the database.
    pub(crate) fn corrupt_behind_the_cache(fs: &FileStore) {
        break_tables(fs);
    }

    fn break_tables(fs: &FileStore) {
        let conn = fs.conn().lock().unwrap();
        let version: i64 = conn.query_row("PRAGMA schema_version", [], |r| r.get(0)).unwrap();
        // A new schema version makes the connection read the schema again, with the
        // changed root pages; without it the parsed one stays in use.
        conn.execute_batch(&format!(
            "PRAGMA writable_schema=ON;
             UPDATE sqlite_master SET rootpage=99999 WHERE type='table' AND name IN ('db_wave_file','db_file_data');
             PRAGMA schema_version={};
             PRAGMA writable_schema=OFF;",
            version + 1
        ))
        .unwrap();
    }

    /// SQLite's `SQLITE_CORRUPT`, as a store error.
    pub(crate) fn corrupt_error() -> crate::backend::storage::error::StoreError {
        crate::backend::storage::error::StoreError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
            Some("database disk image is malformed".into()),
        ))
    }

    static SHARED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

    /// Stands in for the process-global shared store while it lives: stores the
    /// index code is given are recognised as the shared one by address.
    pub(crate) struct Shared(usize);

    pub(crate) fn register(fs: &FileStore) -> Shared {
        let addr = fs as *const FileStore as usize;
        SHARED.lock().unwrap().push(addr);
        Shared(addr)
    }

    pub(crate) fn registered(fs: &FileStore) -> bool {
        SHARED.lock().unwrap().contains(&(fs as *const FileStore as usize))
    }

    impl Drop for Shared {
        fn drop(&mut self) {
            SHARED.lock().unwrap().retain(|a| *a != self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::filestore::FileStore;

    fn corrupt_error() -> StoreError {
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
            Some("database disk image is malformed".into()),
        ))
    }

    #[test]
    fn only_a_corrupt_database_is_corrupt() {
        assert!(is_corrupt(&corrupt_error()));
        assert!(!is_corrupt(&StoreError::NotFound));
        assert!(!is_corrupt(&StoreError::Other("database disk image is malformed".into())));
        let busy = StoreError::Sqlite(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY), None));
        assert!(!is_corrupt(&busy));
    }

    #[test]
    fn a_zone_is_marked_once() {
        let d = Damaged::new();
        assert!(!d.contains("agent:a:current"));
        assert!(d.mark("agent:a:current"));
        assert!(!d.mark("agent:a:current"));
        assert!(d.contains("agent:a:current") && !d.contains("agent:b:current"));
        assert_eq!(d.count(), 1);
        assert!(d.mark("agent:b:current"));
        assert_eq!(d.count(), 2);
    }

    #[test]
    fn a_corrupt_error_marks_the_zone_and_other_errors_do_not() {
        let zone = "agent:note-corrupt-1:current";
        assert!(!note_corrupt(zone, "append", &StoreError::NotFound));
        assert!(!is_damaged(zone));
        assert!(note_corrupt(zone, "append", &corrupt_error()));
        assert!(is_damaged(zone));
        let n = damaged_count();
        assert!(note_corrupt(zone, "tail read", &corrupt_error()), "still reported as corrupt");
        assert_eq!(damaged_count(), n, "a zone already marked is not marked again");
    }

    #[test]
    fn the_incident_names_the_agent_and_what_was_done() {
        let i = incident_for("agent:4775c42b-aaaa-bbbb:current", "append");
        assert_eq!(i.kind, INCIDENT_KIND);
        assert_eq!(i.what, "Saved history for agent 4775c42b is damaged");
        assert!(i.cause.as_deref().unwrap().contains("append"));
        let did = i.did.join(" ");
        assert!(did.contains("stopped writing") && did.contains("own copy") && did.contains("quit every AgentMux window"), "{did}");
        assert!(!i.lost.is_empty());
    }

    #[test]
    fn the_incident_is_written_once_until_it_is_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let zone = "agent:incident-once-1:current";
        write_incident(dir.path(), zone, "append");
        write_incident(dir.path(), zone, "tail read");
        let path = dir.path().join(agentmux_common::incidents::FILE_NAME);
        let all = agentmux_common::incidents::read(&path, 50);
        assert_eq!(all.len(), 1, "a second start finds the same damage: no second notice");
        write_incident(dir.path(), "agent:incident-once-2:current", "append");
        assert_eq!(agentmux_common::incidents::read(&path, 50).len(), 2, "another agent's damage is its own incident");
        agentmux_common::incidents::append_ack(&path, &all[0].id, 1).unwrap();
        write_incident(dir.path(), zone, "append");
        assert_eq!(agentmux_common::incidents::read(&path, 50).len(), 3, "after the user closed it, new damage is told again");
    }

    #[test]
    fn the_agent_note_is_a_system_note_that_says_what_to_do() {
        let t = agent_note();
        assert!(t.starts_with("[AgentMux] "));
        assert!(t.contains("own copy") && t.contains("quit every AgentMux window") && t.contains("not to delete"));
    }

    #[test]
    fn sqlite_reports_the_injected_damage_as_corrupt() {
        // The helpers the path tests rely on: a real SQLITE_CORRUPT from a real query.
        let fs = FileStore::open_in_memory().unwrap();
        fs.make_file("z", "output", Default::default(), Default::default()).unwrap();
        fs.append_lines("z", "output", b"{\"a\":1}
").unwrap();
        test_support::corrupt_behind_the_cache(&fs);
        assert!(fs.stat("z", "output").unwrap().is_some(), "answered from the cache");
        let e = fs.read_bytes_db("z", "output", 0, 8).unwrap_err();
        assert!(is_corrupt(&e), "{e:?}");

        let fs = FileStore::open_in_memory().unwrap();
        fs.make_file("z", "output", Default::default(), Default::default()).unwrap();
        test_support::corrupt(&fs);
        let e = fs.stat("z", "output").unwrap_err();
        assert!(is_corrupt(&e), "{e:?}");
    }

    #[test]
    fn only_the_shared_store_marks_a_zone_from_a_reader_that_takes_any_store() {
        let own = FileStore::open_in_memory().unwrap();
        let shared = FileStore::open_in_memory().unwrap();
        let _guard = test_support::register(&shared);
        assert!(!note_if_shared(&own, "agent:shared-only-1:current", "index read", &corrupt_error()));
        assert!(!is_damaged("agent:shared-only-1:current"), "a block's own store is not the shared one");
        assert!(note_if_shared(&shared, "agent:shared-only-1:current", "index read", &corrupt_error()));
        assert!(is_damaged("agent:shared-only-1:current"));
    }

    #[test]
    fn an_index_rebuild_that_hits_a_corrupt_part_marks_the_zone() {
        use crate::backend::storage::filestore::{FileMeta, FileOpts};
        let zone = "agent:index-corrupt-1:current";
        let gfs = FileStore::open_in_memory().unwrap();
        gfs.make_file(zone, "output", FileMeta::default(), FileOpts::default()).unwrap();
        gfs.append_lines(zone, "output", b"{\"a\":1}
{\"b\":2}
").unwrap();
        let _guard = test_support::register(&gfs);
        test_support::corrupt(&gfs);
        assert_eq!(crate::backend::blockcontroller::shell::extend_output_idx(&gfs, zone), None);
        assert!(is_damaged(zone), "the rebuild's read error is the corruption");
    }

    #[test]
    fn a_rebuild_on_a_store_that_is_not_the_shared_one_marks_nothing() {
        use crate::backend::storage::filestore::{FileMeta, FileOpts};
        let zone = "agent:index-corrupt-own-1:current";
        let own = FileStore::open_in_memory().unwrap();
        own.make_file(zone, "output", FileMeta::default(), FileOpts::default()).unwrap();
        own.append_lines(zone, "output", b"{\"a\":1}
").unwrap();
        test_support::corrupt(&own);
        assert_eq!(crate::backend::blockcontroller::shell::extend_output_idx(&own, zone), None);
        assert!(!is_damaged(zone));
    }

    #[test]
    fn a_recorded_incident_goes_to_the_open_windows_as_a_typed_event() {
        use crate::backend::mps::{Broker, MuxEvent};
        let broker = Broker::new();
        let seen = std::sync::Arc::new(Mutex::new(Vec::<MuxEvent>::new()));
        let s2 = seen.clone();
        broker.add_observer(std::sync::Arc::new(move |e: &MuxEvent| s2.lock().unwrap().push(e.clone())));
        let incident = incident_for("agent:live-toast-1:current", "append");
        publish_to(&broker, &incident);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].event, EVENT_INCIDENT_RECORDED);
        assert!(seen[0].scopes.is_empty(), "to every window, not one pane's");
        let data = seen[0].data.clone().unwrap();
        assert_eq!(data["id"], incident.id);
        assert_eq!(data["kind"], INCIDENT_KIND);
        assert_eq!(data["what"], incident.what);
        assert_eq!(data["did"].as_array().unwrap().len(), incident.did.len());
        assert!(data.get("acknowledged_at_ms").is_none(), "{data}");
    }

    #[test]
    fn an_agent_is_told_once_for_all_of_its_zones() {
        assert!(first_to_tell(&zone_tag("agent:told-once-1:current")));
        assert!(!first_to_tell(&zone_tag("agent:told-once-1:archive:17")), "its archive zone: same agent");
        assert!(!first_to_tell(&zone_tag("agent-uid:told-once-1:segments")), "its segment log: same agent id");
        assert!(first_to_tell(&zone_tag("agent:told-once-2:current")), "another agent is told");
    }

    #[test]
    fn only_an_incident_that_was_written_is_published() {
        let dir = tempfile::tempdir().unwrap();
        let zone = "agent:live-toast-2:current";
        assert!(write_incident(dir.path(), zone, "append").is_some());
        assert!(write_incident(dir.path(), zone, "tail read").is_none(), "already open: shown from the file, not again");
    }
}
