// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Writing an agent's transcript: the line appends to the block's `output` and
//! the agent's global zone, the order lock that keeps one block's events in line
//! order, and the announcements of a replaced or deleted transcript.
//!
//! These lived in `blockcontroller/shell/file_ops.rs` next to the terminal output
//! path, which made the store-lock warnings (`site=`) and a reader's search for
//! "where does a transcript get written" point at the terminal code. Terminal
//! output (`handle_append_block_file` for `term`) stays in `shell/`.

use std::sync::Arc;

use base64::Engine as _;

use super::origin_sig;
use crate::backend::message_origin::RecordOrigin;
use crate::backend::mps;
use crate::backend::storage::filestore::{AppendPos, FileStore};

use crate::backend::blockcontroller::shell::handle_append_block_file;

/// `agent:transcriptsinglecopy`: write a defined agent's transcript to the
/// agent's zone in the global store only, not also to the block's own store
/// (docs/specs/SPEC_AGENT_TRANSCRIPT_SINGLE_COPY_2026_10_19.md, step B). Off
/// unless set to `true`: the two writes are what every build did until now.
pub const SETTING_SINGLE_COPY: &str = "agent:transcriptsinglecopy";

static CONFIG: std::sync::OnceLock<Arc<crate::backend::wconfig::ConfigState>> = std::sync::OnceLock::new();

/// Attach the live settings (bootstrap), so [`SETTING_SINGLE_COPY`] can be read.
pub fn install_config(config: Arc<crate::backend::wconfig::ConfigState>) {
    let _ = CONFIG.set(config);
}

fn single_copy_enabled() -> bool {
    // Read per transcript line: one key under the read lock, no clone of the settings.
    CONFIG.get().and_then(|c| c.extra_bool(SETTING_SINGLE_COPY)).unwrap_or(false)
}

/// Whether `zone` is a defined agent's own zone (`agent:<id>:current`), whose
/// transcript is one conversation, and not a quick-launch provider's, which
/// every pane of that provider shares.
fn is_defined_agent_zone(zone: &str) -> bool {
    zone.strip_prefix("agent:")
        .and_then(|z| z.strip_suffix(":current"))
        .is_some_and(|id| crate::backend::providers::get_provider(id).is_none())
}

/// Persist a user message the controller wrote to the agent's stdin, so
/// `parseHistoryLines` can reconstruct its `user_message` node on reopen.
///
/// A transcript record like any other (Phase 5a-3c): written to the block's
/// file and the global zone with positions, and published — so a pane reading
/// the stream by line never sees a gap where this record sits — but marked
/// `echo: "stdin"`, because the pane already shows the message (its
/// `agent-message-accepted` node) and must not add a second one. Before 5a-3c
/// this write published nothing at all.
///
/// A message a paired device sent is the exception: no pane typed it, so it
/// is stored with who sent it (`viewer::origin`) and published as a plain
/// record, which the open pane shows.
pub fn persist_user_line(
    broker: Option<&mps::Broker>,
    block_id: &str,
    data: &[u8],
    filestore: Option<&Arc<FileStore>>,
    global_output_zone: Option<&str>,
    record: Option<&RecordOrigin>,
) {
    persist_user_line_as(broker, block_id, data, filestore, global_output_zone, true, record);
}

/// [`persist_user_line`] for a message no pane typed (a jekt, a broadcast, a
/// schedule, the app API): the record is published as a plain one, which an
/// open pane shows, because no `agent-message-accepted` node stands for it.
pub fn persist_injected_user_line(
    broker: Option<&mps::Broker>,
    block_id: &str,
    data: &[u8],
    filestore: Option<&Arc<FileStore>>,
    global_output_zone: Option<&str>,
    record: Option<&RecordOrigin>,
) {
    persist_user_line_as(broker, block_id, data, filestore, global_output_zone, false, record);
}

fn persist_user_line_as(
    broker: Option<&mps::Broker>,
    block_id: &str,
    data: &[u8],
    filestore: Option<&Arc<FileStore>>,
    global_output_zone: Option<&str>,
    shown_by_pane: bool,
    record: Option<&RecordOrigin>,
) {
    let global = global_output_zone.and_then(|zone| {
        crate::backend::agent_session::global_transcript_store().map(|gfs| (gfs, zone))
    });
    // A message a paired device sent says so (`viewer::origin`); every one
    // says where it came from and which notes went with it.
    let from_device = crate::backend::viewer::origin::stamp(block_id, data);
    let echo = if from_device.is_none() && shown_by_pane { Some("stdin") } else { None };
    let data = from_device.as_deref().unwrap_or(data);
    let stamped;
    let data = match record {
        Some(record) => {
            stamped = crate::backend::message_origin::stamp_bytes(data, record);
            stamped.as_slice()
        }
        None => data,
    };
    // Signed last, over everything in the `amx`, for the agent (or block) the record belongs to.
    let owner = origin_sig::owner_of_zone(global_output_zone.unwrap_or(block_id));
    let signed = origin_sig::sign_line(owner, data, agentmux_common::time::now_ms());
    let data = signed.as_slice();
    append_transcript(broker, block_id, data, filestore, global, global_output_zone.is_some(), echo);
}

/// `line` stored as the record of a prompt srv delivered, when it is the provider echoing one: with
/// the origin the ledger holds, signed. The echo's own `amx`, if it has one, is dropped first: only
/// srv says where a message came from.
fn origin_of_echo(block_id: &str, line: &str, global_output_zone: Option<&str>) -> Option<String> {
    let record = origin_sig::echoed_prompt(line).and_then(|text| super::origin_ledger::take(block_id, &text))?;
    let mut value: serde_json::Value = serde_json::from_str(line).ok()?;
    value.as_object_mut()?.remove("amx");
    let stamped = crate::backend::message_origin::stamp_bytes(format!("{value}\n").as_bytes(), &record);
    let owner = origin_sig::owner_of_zone(global_output_zone.unwrap_or(block_id));
    String::from_utf8(origin_sig::sign_line(owner, &stamped, agentmux_common::time::now_ms())).ok()
}

/// Appends one NDJSON stdout line, plus the newline the frontend's line
/// splitter needs, to the block's `output` file and `global_output_zone`.
///
/// A line that is the provider echoing a prompt srv delivered (`origin_ledger`) is stored with
/// that message's origin, signed like a record srv writes itself; any other line is stored as it is.
pub fn append_output_line(
    broker: &mps::Broker,
    block_id: &str,
    line: &str,
    filestore: Option<&Arc<FileStore>>,
    global_output_zone: Option<&str>,
) {
    let line_with_newline = origin_of_echo(block_id, line, global_output_zone).unwrap_or_else(|| format!("{line}\n"));
    handle_append_block_file(
        broker,
        block_id,
        crate::backend::agent_session::OUTPUT_FILE,
        line_with_newline.as_bytes(),
        filestore,
        global_output_zone,
    );
}

/// Append `data` to the global transcript zone's `output` file, creating it
/// lazily on first write, and return where it landed. Mirrors the
/// per-channel write-through in [`crate::backend::blockcontroller::shell::handle_append_block_file`]; all errors are
/// logged and swallowed (`None`) so the hot stdout path is never blocked by
/// the global store.
pub(crate) fn mirror_append_to_global(gfs: &Arc<FileStore>, zone: &str, data: &[u8]) -> Option<AppendPos> {
    use crate::backend::agent_session::OUTPUT_FILE;
    // Global zones are agent transcripts by construction — always stamp
    // (§4.4), keyed at the offset this append actually landed at (codex P2 on
    // PR #2508), in the same transaction as the append.
    append_transcript_lines(gfs, zone, OUTPUT_FILE, data, true, "global transcripts", true)
}

/// `append_lines` on a transcript file, creating it on first write (or if
/// another instance deleted it since). Whole non-blank lines only: a torn
/// last line is closed, not continued (`filestore/counter.rs`). With `stamp`,
/// the `output.tsidx` receive-time record is written in the same transaction
/// (one commit, not two). Errors are logged and swallowed.
///
/// `shared` is a write to the shared store's zone: one found corrupt (here or
/// by a reader) is not written to again in this process
/// ([`damage`](crate::backend::transcript::damage)), and the first corrupt error
/// is an incident, not only a WARN line.
fn append_transcript_lines(fs: &Arc<FileStore>, zone: &str, name: &str, data: &[u8], stamp: bool, what: &str, shared: bool) -> Option<AppendPos> {
    use crate::backend::agent_session::TSIDX_FILE;
    use crate::backend::storage::error::StoreError;
    use crate::backend::transcript::damage;
    if shared && damage::is_damaged(zone) {
        return None;
    }
    let append = || {
        if stamp {
            fs.append_lines_stamped(zone, name, data, TSIDX_FILE)
        } else {
            fs.append_lines(zone, name, data)
        }
    };
    let failed = |e: &StoreError| {
        if shared && damage::note_corrupt(zone, "append", e) {
            return;
        }
        tracing::warn!(zone = %zone, error = %e, "{what}: append failed");
    };
    match append() {
        Ok(pos) => return Some(pos),
        Err(StoreError::NotFound) => {}
        Err(e) => {
            failed(&e);
            return None;
        }
    }
    if let Err(e) = fs.make_file(
        zone,
        name,
        std::collections::HashMap::new(),
        crate::backend::storage::filestore::FileOpts::default(),
    ) {
        if !matches!(e, StoreError::AlreadyExists) {
            if !(shared && damage::note_corrupt(zone, "append", &e)) {
                tracing::warn!(zone = %zone, error = %e, "{what}: make_file failed; skipping write");
            }
            return None;
        }
    }
    append().map_err(|e| failed(&e)).ok()
}

/// The agent-transcript half of [`crate::backend::blockcontroller::shell::handle_append_block_file`] (Phase 5a-3,
/// SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7 item 5).
///
/// The block's `output` and the agent's global zone are written FIRST, and
/// the event goes out after, carrying where the records landed in each stream
/// (`pos`) and the exact byte offset in the block's file. The block's order
/// lock is held from the first write to the publish, so one block's events
/// leave in line order. Only transcripts come here: publishing after a
/// database write would add that write to every terminal keystroke echo.
///
/// `stamp_tsidx` is whether this is an agent stream (it has a global zone,
/// even if no global store is installed) — as before, only those get the
/// receive-time sidecar.
pub(crate) fn append_transcript(
    broker: Option<&mps::Broker>,
    block_id: &str,
    data: &[u8],
    filestore: Option<&Arc<FileStore>>,
    global: Option<(&Arc<FileStore>, &str)>,
    stamp_tsidx: bool,
    echo: Option<&str>,
) {
    append_transcript_with(broker, block_id, data, filestore, global, stamp_tsidx, echo, single_copy_enabled());
}

/// [`append_transcript`] with the single-copy setting given.
///
/// With it on, a defined agent's line goes to the global zone only. The block's
/// own copy is still written when there is no global write to rely on: a
/// fenced writer (it passes no global zone), a quick-launch pane (a provider's
/// shared zone), or a global write that failed (the line must not be lost).
#[allow(clippy::too_many_arguments)]
pub(crate) fn append_transcript_with(
    broker: Option<&mps::Broker>,
    block_id: &str,
    data: &[u8],
    filestore: Option<&Arc<FileStore>>,
    global: Option<(&Arc<FileStore>, &str)>,
    stamp_tsidx: bool,
    echo: Option<&str>,
    single_copy: bool,
) {
    use crate::backend::agent_session::OUTPUT_FILE;
    // The records exactly as they are written (`append_lines` normalizes to
    // complete, non-blank lines): the event carries these bytes, and its
    // `offset` is where they landed, so a consumer can match the event to a
    // byte range read back from the file (review of #3635). Nothing to write,
    // nothing to announce.
    let records = crate::backend::storage::filestore::normalized_records(data);
    if records.is_empty() {
        return;
    }
    let _order = transcript_order_lock(block_id);

    let mut offset = None;
    let mut pos = Vec::new();
    // Single copy: the agent's zone first; the block's copy only if that failed.
    let global_only = single_copy && global.is_some_and(|(_, zone)| is_defined_agent_zone(zone));
    let mut global_pos = None;
    if global_only {
        if let Some((gfs, zone)) = global {
            global_pos = Some(mirror_append_to_global(gfs, zone, &records));
        }
    }
    let write_block_copy = !global_only || global_pos.as_ref().is_some_and(Option::is_none);
    if let Some(fs) = filestore.filter(|_| write_block_copy) {
        if let Some(p) = append_transcript_lines(fs, block_id, OUTPUT_FILE, &records, stamp_tsidx, "filestore", false) {
            // Past the `\n` that closed a torn last line, if one was written.
            offset = Some(p.records_offset.max(0) as u64);
            if let Some(c) = p.counted {
                pos.push(mps::StreamPos { stream: format!("b:{block_id}"), gen: c.gen, line: c.first_line, lines: c.lines });
            }
        }
    }
    if let Some((gfs, zone)) = global {
        let landed = match global_pos {
            Some(done) => done,
            None => mirror_append_to_global(gfs, zone, &records),
        };
        if let Some(c) = landed.and_then(|p| p.counted) {
            pos.push(mps::StreamPos { stream: format!("g:{zone}"), gen: c.gen, line: c.first_line, lines: c.lines });
        }
    }

    let event_data = mps::WSFileEventData {
        zoneid: block_id.to_string(),
        filename: OUTPUT_FILE.to_string(),
        fileop: mps::FILE_OP_APPEND.to_string(),
        data64: base64::engine::general_purpose::STANDARD.encode(&records),
        offset,
        pos,
        echo: echo.map(str::to_string),
    };
    // No broker (a controller built without one): written, not announced.
    if let Some(broker) = broker {
        broker.publish_event::<crate::backend::mps_events::BlockFile>(&event_data, vec![format!("block:{block_id}")], 0);
    }
}

/// Run `f` holding `block_id`'s transcript order lock — the lock every append
/// holds from its first write to its publish (see [`append_transcript`]).
/// A replace or delete of the block's transcript, and its announcement
/// ([`publish_transcript_changed`]), run inside it, so no append's write and
/// event can interleave with them and one block's events stay in order
/// (review of #3636). `f` must not append to the same block: the lock isn't
/// reentrant.
pub fn with_transcript_order<T>(block_id: &str, f: impl FnOnce() -> T) -> T {
    let _order = transcript_order_lock(block_id);
    f()
}

/// Tell a block's panes that its transcript `output` was replaced or deleted
/// as a whole (Phase 5a-3), so an open pane resyncs at once instead of on its
/// next read. `fileop` is [`mps::FILE_OP_REPLACE`] — with the new generation
/// and line count read from `filestore` — or [`mps::FILE_OP_DELETE`]. Call it,
/// with the replace or delete itself, inside [`with_transcript_order`].
pub fn publish_transcript_changed(
    broker: &mps::Broker,
    block_id: &str,
    fileop: &str,
    filestore: &FileStore,
    mstore: &crate::backend::storage::store::Store,
) {
    let source = crate::backend::transcript::source::TranscriptSource::resolve(filestore, mstore, block_id);
    publish_transcript_changed_from(broker, block_id, fileop, filestore, &source);
}

/// [`publish_transcript_changed`] with the transcript's source resolved. The
/// position names the block's own stream when it has a counted copy, the
/// agent's zone otherwise (a defined agent written to the zone only, which is
/// the stream its pane follows).
pub(crate) fn publish_transcript_changed_from(
    broker: &mps::Broker,
    block_id: &str,
    fileop: &str,
    filestore: &FileStore,
    source: &crate::backend::transcript::source::TranscriptSource<'_>,
) {
    use crate::backend::agent_session::OUTPUT_FILE;
    let pos_in = |fs: &FileStore, zone: &str, stream: String| match fs.line_state(zone, OUTPUT_FILE) {
        Ok(Some(state)) => state
            .counted
            .map(|c| vec![mps::StreamPos { stream, gen: c.gen, line: 0, lines: c.lines }])
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut pos = pos_in(filestore, block_id, format!("b:{block_id}"));
    if pos.is_empty() && source.global {
        pos = pos_in(source.fs, &source.zone, format!("g:{}", source.zone));
    }
    let event_data = mps::WSFileEventData {
        zoneid: block_id.to_string(),
        filename: OUTPUT_FILE.to_string(),
        fileop: fileop.to_string(),
        data64: String::new(),
        offset: None,
        pos,
        echo: None,
    };
    broker.publish_event::<crate::backend::mps_events::BlockFile>(&event_data, vec![format!("block:{block_id}")], 0);
}

/// Serializes one block's transcript appends with their publish (see
/// [`append_transcript`]). Striped rather than per block: bounded memory and
/// nothing to clean up; two blocks that share a stripe only wait for each
/// other's append.
fn transcript_order_lock(block_id: &str) -> std::sync::MutexGuard<'static, ()> {
    use std::hash::{Hash, Hasher};
    const STRIPES: usize = 64;
    static LOCKS: [std::sync::Mutex<()>; STRIPES] = [const { std::sync::Mutex::new(()) }; STRIPES];
    let mut h = std::collections::hash_map::DefaultHasher::new();
    block_id.hash(&mut h);
    LOCKS[(h.finish() % STRIPES as u64) as usize]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Resolve a block's GLOBAL transcript zone (`agent:<defId>:current`) from its
/// `agentId` meta, looking the block up in `mstore`. Returns `None` for
/// non-agent blocks, when there's no store, or when the block can't be loaded —
/// the caller then passes `None` and no global mirror happens. Shared by the
/// subprocess / persistent / acp agent controllers.
pub(crate) fn resolve_global_output_zone(
    mstore: &Option<Arc<crate::backend::storage::store::Store>>,
    block_id: &str,
) -> Option<String> {
    let store = mstore.as_ref()?;
    let block = store
        .must_get::<crate::backend::obj::Block>(block_id)
        .ok()?;
    crate::backend::agent_session::agent_zone_for_block_meta(&block.meta)
}

#[cfg(test)]
mod damage_tests {
    use super::*;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};
    use crate::backend::transcript::damage::{self, test_support};

    const LINE: &[u8] = b"{\"type\":\"user\"}
";

    fn own_copy(own: &FileStore, block: &str) -> Vec<u8> {
        own.read_file(block, "output").unwrap().unwrap_or_default()
    }

    #[test]
    fn a_corrupt_error_on_the_shared_zone_marks_it_and_the_blocks_copy_keeps_the_line() {
        for (n, single_copy) in [(1, false), (2, true)] {
            let (own, gfs) = (Arc::new(FileStore::open_in_memory().unwrap()), Arc::new(FileStore::open_in_memory().unwrap()));
            test_support::corrupt(&gfs);
            let (block, zone) = (format!("blk-damage-append-{n}"), format!("agent:damage-append-{n}:current"));
            assert!(!damage::is_damaged(&zone));
            append_transcript_with(None, &block, LINE, Some(&own), Some((&gfs, zone.as_str())), true, None, single_copy);
            assert!(damage::is_damaged(&zone), "single_copy={single_copy}: the first corrupt error marks the zone");
            assert_eq!(own_copy(&own, &block), LINE, "single_copy={single_copy}: the block's copy has the line, so nothing is lost");
        }
    }

    #[test]
    fn a_damaged_zone_is_not_written_to_again_but_the_blocks_copy_is() {
        let (own, gfs) = (Arc::new(FileStore::open_in_memory().unwrap()), Arc::new(FileStore::open_in_memory().unwrap()));
        let (block, zone) = ("blk-damage-append-3", "agent:damage-append-3:current");
        // The shared store is healthy again (or was never what failed): the zone stays out of it.
        damage::note_corrupt(zone, "append", &StoreError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
            None,
        )));
        use crate::backend::storage::error::StoreError;
        append_transcript_with(None, block, LINE, Some(&own), Some((&gfs, zone)), true, None, true);
        append_transcript_with(None, block, LINE, Some(&own), Some((&gfs, zone)), true, None, false);
        assert!(gfs.stat(zone, "output").unwrap().is_none(), "nothing was sent to the shared store");
        assert_eq!(own_copy(&own, block), [LINE, LINE].concat());
        assert!(mirror_append_to_global(&gfs, zone, LINE).is_none());
    }

    #[test]
    fn a_zone_that_is_not_damaged_is_written_as_before() {
        let (own, gfs) = (Arc::new(FileStore::open_in_memory().unwrap()), Arc::new(FileStore::open_in_memory().unwrap()));
        let (block, zone) = ("blk-damage-append-4", "agent:damage-append-4:current");
        gfs.make_file(zone, "output", FileMeta::default(), FileOpts::default()).unwrap();
        append_transcript_with(None, block, LINE, Some(&own), Some((&gfs, zone)), true, None, false);
        assert_eq!(gfs.read_file(zone, "output").unwrap().unwrap(), LINE);
        assert!(!damage::is_damaged(zone));
    }
}
