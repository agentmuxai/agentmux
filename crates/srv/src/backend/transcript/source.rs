// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where an agent block's transcript is read from.
//!
//! An agent's output is stored in the agent's own zone in the global transcript
//! store (`agent:<definitionId>:current`) and, until it stops being written
//! (SPEC_AGENT_TRANSCRIPT_SINGLE_COPY_2026_10_19), also in the block's own store.
//! A reader that wants "the block's recent output" asks here which copy to use:
//! the agent's zone when it has content (the copy the pane reads, see
//! `global_output_source`), the block's own otherwise.
//!
//! The reads go through the WAL reader pool (`read_snapshot`), so they never
//! queue behind the store's single writer.

use crate::backend::agent_session::{agent_zone_for_block_meta, global_transcript_store, OUTPUT_FILE};
use crate::backend::obj::Block;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

/// The store and zone a block's transcript is read from.
pub struct TranscriptSource<'a> {
    pub fs: &'a FileStore,
    pub zone: String,
    /// The agent's zone in the global store, rather than the block's own.
    pub global: bool,
}

/// What a tail read found.
pub struct Tail {
    pub bytes: Vec<u8>,
    /// The file's size and last-modified time at that snapshot.
    pub size: i64,
    pub modts: i64,
}

impl<'a> TranscriptSource<'a> {
    /// The agent's zone when the block is an agent block that is not archived
    /// and the zone has content; the block's own store otherwise (no
    /// `agentId`, no global store, an archived session, or an empty zone).
    pub fn resolve(block_fs: &'a FileStore, mstore: &Store, block_id: &str) -> Self {
        Self::resolve_with(block_fs, global_transcript_store().map(|g| g.as_ref()), mstore, block_id)
    }

    /// The block's own copy, whatever the global store holds.
    pub fn own(block_fs: &'a FileStore, block_id: &str) -> Self {
        TranscriptSource { fs: block_fs, zone: block_id.to_string(), global: false }
    }

    /// [`Self::resolve`] with the global store given (tests install none).
    pub fn resolve_with(block_fs: &'a FileStore, global: Option<&'a FileStore>, mstore: &Store, block_id: &str) -> Self {
        let own = || TranscriptSource { fs: block_fs, zone: block_id.to_string(), global: false };
        let Some(gfs) = global else { return own() };
        let Some(block) = mstore.get::<Block>(block_id).ok().flatten() else { return own() };
        let archived = block
            .meta
            .get(crate::backend::session_archive::META_SESSION_ARCHIVED_AT)
            .and_then(|v| v.as_i64())
            .map(|v| v > 0)
            .unwrap_or(false);
        if archived {
            return own();
        }
        let Some(zone) = agent_zone_for_block_meta(&block.meta) else { return own() };
        let has_content = gfs
            .read_snapshot(|r| Ok(r.file(&zone, OUTPUT_FILE)?.map(|f| f.size)))
            .ok()
            .flatten()
            .unwrap_or(0)
            > 0;
        if has_content {
            TranscriptSource { fs: gfs, zone, global: true }
        } else {
            own()
        }
    }

    /// The file's size and last-modified time, or `None` if it does not exist.
    pub fn stat(&self) -> Option<(i64, i64)> {
        self.fs.read_snapshot(|r| Ok(r.file(&self.zone, OUTPUT_FILE)?.map(|f| (f.size, f.modts)))).ok().flatten()
    }

    /// The last `max_bytes` of the file (all of it if smaller) from one snapshot.
    /// `None` for no file, an empty one, or bytes that are not all stored yet.
    pub fn tail(&self, max_bytes: i64) -> Option<Tail> {
        self.fs
            .read_snapshot(|r| {
                let Some(file) = r.file(&self.zone, OUTPUT_FILE)? else { return Ok(None) };
                if file.size <= 0 {
                    return Ok(None);
                }
                let start = (file.size - max_bytes).max(0);
                Ok(r.bytes(&self.zone, OUTPUT_FILE, start, file.size - start)?.map(|bytes| Tail {
                    bytes,
                    size: file.size,
                    modts: file.modts,
                }))
            })
            .ok()
            .flatten()
    }

    /// The whole file.
    pub fn read_all(&self) -> Option<Vec<u8>> {
        self.fs
            .read_snapshot(|r| {
                let Some(file) = r.file(&self.zone, OUTPUT_FILE)? else { return Ok(None) };
                r.bytes(&self.zone, OUTPUT_FILE, 0, file.size)
            })
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::Block;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};

    const BLOCK: &str = "blk-source";
    const DEF: &str = "def-source-1";

    fn stores() -> (FileStore, FileStore, Store) {
        (FileStore::open_in_memory().unwrap(), FileStore::open_in_memory().unwrap(), Store::open_in_memory().unwrap())
    }

    fn block(mstore: &Store, agent_id: Option<&str>, archived: bool) {
        let mut meta = crate::backend::obj::MetaMapType::default();
        if let Some(a) = agent_id {
            meta.insert("agentId".into(), serde_json::json!(a));
        }
        if archived {
            meta.insert(crate::backend::session_archive::META_SESSION_ARCHIVED_AT.into(), serde_json::json!(1_700_000_000_000i64));
        }
        let mut b = Block { oid: BLOCK.into(), parentoref: String::new(), version: 1, runtimeopts: None, stickers: None, meta, subblockids: None };
        mstore.insert(&mut b).unwrap();
    }

    fn put(fs: &FileStore, zone: &str, body: &str) {
        fs.make_file(zone, "output", FileMeta::default(), FileOpts::default()).unwrap();
        fs.append_lines(zone, "output", body.as_bytes()).unwrap();
    }

    fn zone() -> String {
        crate::backend::agent_session::agent_current_zone(DEF)
    }

    #[test]
    fn an_agent_block_reads_the_agents_zone_when_it_has_content() {
        let (own, global, mstore) = stores();
        block(&mstore, Some(DEF), false);
        put(&own, BLOCK, "{\"copy\":\"block\"}\n");
        put(&global, &zone(), "{\"copy\":\"global\"}\n{\"n\":2}\n");
        let s = TranscriptSource::resolve_with(&own, Some(&global), &mstore, BLOCK);
        assert!(s.global);
        assert_eq!(s.zone, zone());
        assert_eq!(s.read_all().unwrap(), b"{\"copy\":\"global\"}\n{\"n\":2}\n");
        let tail = s.tail(1024).unwrap();
        assert_eq!(tail.bytes, b"{\"copy\":\"global\"}\n{\"n\":2}\n");
        assert_eq!(tail.size, tail.bytes.len() as i64);
        assert_eq!(s.stat().unwrap().0, tail.size);
    }

    #[test]
    fn a_block_with_no_global_content_no_agent_id_or_an_archive_reads_its_own_copy() {
        let (own, global, mstore) = stores();
        put(&own, BLOCK, "{\"copy\":\"block\"}\n");
        // No block row, no global store.
        assert!(!TranscriptSource::resolve_with(&own, None, &mstore, BLOCK).global);
        // A block row without an agentId (a terminal, a /btw throwaway).
        block(&mstore, None, false);
        put(&global, &zone(), "{\"copy\":\"global\"}\n");
        assert!(!TranscriptSource::resolve_with(&own, Some(&global), &mstore, BLOCK).global);
    }

    #[test]
    fn an_agent_block_with_an_empty_global_zone_falls_back_to_its_own_copy() {
        let (own, global, mstore) = stores();
        block(&mstore, Some(DEF), false);
        put(&own, BLOCK, "{\"copy\":\"block\"}\n");
        global.make_file(&zone(), "output", FileMeta::default(), FileOpts::default()).unwrap();
        let s = TranscriptSource::resolve_with(&own, Some(&global), &mstore, BLOCK);
        assert!(!s.global);
        assert_eq!(s.read_all().unwrap(), b"{\"copy\":\"block\"}\n");
    }

    #[test]
    fn an_archived_session_reads_its_own_copy() {
        let (own, global, mstore) = stores();
        block(&mstore, Some(DEF), true);
        put(&own, BLOCK, "{\"copy\":\"block\"}\n");
        put(&global, &zone(), "{\"copy\":\"global\"}\n");
        assert!(!TranscriptSource::resolve_with(&own, Some(&global), &mstore, BLOCK).global);
    }

    #[test]
    fn a_missing_file_has_no_tail_and_no_size() {
        let (own, _global, mstore) = stores();
        let s = TranscriptSource::resolve_with(&own, None, &mstore, BLOCK);
        assert!(s.tail(1024).is_none() && s.stat().is_none() && s.read_all().is_none());
    }

    #[test]
    fn a_tail_is_the_end_of_the_file() {
        let (own, _g, mstore) = stores();
        put(&own, BLOCK, &format!("{}\n{}\n", "a".repeat(5000), "b".repeat(5000)));
        let s = TranscriptSource::resolve_with(&own, None, &mstore, BLOCK);
        let tail = s.tail(100).unwrap();
        assert_eq!(tail.bytes.len(), 100);
        assert!(tail.bytes.iter().rev().skip(1).all(|b| *b == b'b'));
        assert_eq!(tail.size, 10002);
    }
}
