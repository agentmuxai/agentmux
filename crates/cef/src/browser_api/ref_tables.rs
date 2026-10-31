// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The element references each browser pane's latest snapshot handed out, by pane: what `click`,
//! `fill`, a Vault fill and `wait` resolve a reference like `e5` with. They are DevTools node ids
//! of one page, so they end when it does (`RefTables::forget`).

use std::collections::HashMap;

use super::frames::FrameRef;

/// The references one snapshot handed out for one pane, and the page they
/// belong to. A new snapshot or a navigation makes them stale.
#[derive(Debug, Clone, Default)]
pub struct RefTable {
    pub url: String,
    pub refs: HashMap<String, i64>,
    /// For each reference inside an iframe (`f1e3`), how to find its frame.
    pub frames: HashMap<String, FrameRef>,
}

/// What a pane keeps across its tables: how many `find`s (and snapshots scoped by name) it has
/// had, the `k` of their `n{k}e1` references, so a reference from an earlier find never comes to
/// name another element; and how many times its page has gone away. A snapshot or find notes the
/// second when it starts and its references are kept only if it is still the same when they are
/// ready, so a request that was in flight as the page closed can't put the dead page's node ids
/// back. Kept apart from the tables, which are bounded, so closed panes can't push a live pane's
/// table out. It costs a few bytes for each pane that has ever been closed.
#[derive(Default)]
struct PaneMark {
    finds: u32,
    epoch: u64,
}

#[derive(Default)]
struct Tables {
    clock: u64,
    tables: HashMap<String, (u64, RefTable)>,
    marks: HashMap<String, PaneMark>,
}

/// Per-host store of each pane's latest reference table, by block id.
/// Bounded: past `MAX_TABLES` panes, the least recently snapshotted is
/// dropped (its agent just takes a new snapshot).
#[derive(Default)]
pub struct RefTables(std::sync::Mutex<Tables>);

const MAX_TABLES: usize = 64;

impl RefTables {
    fn lock(&self) -> std::sync::MutexGuard<'_, Tables> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[cfg(test)]
    pub(super) fn put(&self, block_id: &str, table: RefTable) {
        Self::put_locked(&mut self.lock(), block_id, table);
    }

    fn put_locked(g: &mut Tables, block_id: &str, table: RefTable) {
        g.clock += 1;
        let stamp = g.clock;
        g.tables.insert(block_id.to_string(), (stamp, table));
        while g.tables.len() > MAX_TABLES {
            let oldest = g.tables.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    g.tables.remove(&k);
                }
                None => break,
            }
        }
    }

    pub(super) fn get(&self, block_id: &str) -> Option<RefTable> {
        self.lock().tables.get(block_id).map(|(_, t)| t.clone())
    }

    /// The pane's table with the epoch it belongs to, read together. A route that acts on a
    /// reference checks [`still_at`](Self::still_at) once it has opened the pane, so a table it
    /// took before the page closed and came back can't point at the new page's elements.
    pub(super) fn get_at(&self, block_id: &str) -> Option<(RefTable, u64)> {
        let g = self.lock();
        let (_, table) = g.tables.get(block_id)?;
        Some((table.clone(), g.marks.get(block_id).map(|m| m.epoch).unwrap_or(0)))
    }

    /// Whether the pane's page is still the one `epoch` was noted for.
    pub(super) fn still_at(&self, block_id: &str, epoch: u64) -> bool {
        Self::epoch_is(&self.lock(), block_id, epoch)
    }

    /// Forget the references of a pane whose page is gone (closed, or put to sleep). A
    /// reference is a DevTools node id, which a new page in a new renderer process numbers from
    /// the start again: kept, an old `e5` would name whatever element has that number on the
    /// reloaded page, and a fill or a click would land in the wrong field. The find counter stays,
    /// so an earlier find's `n1e3` still never names another element.
    pub(crate) fn forget(&self, block_id: &str) {
        let mut g = self.lock();
        g.tables.remove(block_id);
        // Even for a pane with no table yet: a first snapshot may be in flight.
        g.marks.entry(block_id.to_string()).or_default().epoch += 1;
    }

    /// The pane's epoch now (see [`PaneMark`]). A request notes it before it opens the pane, so
    /// a close at any point after that is seen.
    pub(super) fn epoch(&self, block_id: &str) -> u64 {
        self.lock().marks.get(block_id).map(|m| m.epoch).unwrap_or(0)
    }

    fn epoch_is(g: &Tables, block_id: &str, epoch: u64) -> bool {
        g.marks.get(block_id).map(|m| m.epoch).unwrap_or(0) == epoch
    }

    /// [`put`](Self::put), unless the pane's page has gone away since `epoch`.
    pub(super) fn put_at(&self, block_id: &str, epoch: u64, table: RefTable) -> bool {
        let mut g = self.lock();
        if !Self::epoch_is(&g, block_id, epoch) {
            return false;
        }
        Self::put_locked(&mut g, block_id, table);
        true
    }

    /// The prefix for the next find's references on this pane, `n{k}`.
    /// Counted under one lock, so two finds at once can't share one.
    pub(super) fn next_find_prefix(&self, block_id: &str) -> String {
        let mut g = self.lock();
        let mark = g.marks.entry(block_id.to_string()).or_default();
        mark.finds += 1;
        format!("n{}", mark.finds)
    }

    /// Add references to the pane's table for `url`, the way a scoped
    /// snapshot's are: beside the ones already there, unless the page
    /// changed, which starts a new table.
    #[cfg(test)]
    pub(super) fn merge(&self, block_id: &str, url: &str, refs: &[(String, i64)]) {
        self.merge_at(block_id, None, url, refs);
    }

    /// [`merge`](Self::merge), unless the pane's page has gone away since `epoch`.
    pub(super) fn merge_at(&self, block_id: &str, epoch: Option<u64>, url: &str, refs: &[(String, i64)]) -> bool {
        self.merge_framed_at(block_id, epoch, url, refs, &[])
    }

    /// [`merge_at`](Self::merge_at) for references inside iframes, with the frame each one is in.
    pub(super) fn merge_framed_at(
        &self,
        block_id: &str,
        epoch: Option<u64>,
        url: &str,
        refs: &[(String, i64)],
        frames: &[(String, FrameRef)],
    ) -> bool {
        let mut g = self.lock();
        if epoch.is_some_and(|e| !Self::epoch_is(&g, block_id, e)) {
            return false;
        }
        let mut table = g.tables.get(block_id).map(|(_, t)| t.clone()).unwrap_or_default();
        if table.url != url {
            table = RefTable::default();
        }
        table.url = url.to_string();
        table.refs.extend(refs.iter().cloned());
        table.frames.extend(frames.iter().cloned());
        Self::put_locked(&mut g, block_id, table);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_read_before_the_page_closed_are_not_kept_after_it() {
        let tables = RefTables::default();
        let started = tables.epoch("pane");
        tables.forget("pane");
        // A snapshot and a find that began before the close finish after it: neither is kept.
        let table = RefTable { url: "https://example.com/".into(), refs: HashMap::from([("e5".to_string(), 42)]), ..RefTable::default() };
        assert!(!tables.put_at("pane", started, table.clone()));
        assert!(!tables.merge_at("pane", Some(started), "https://example.com/", &[("n1e1".to_string(), 7)]));
        assert!(tables.get("pane").is_none());
        // One that began after it is.
        let after = tables.epoch("pane");
        assert!(tables.put_at("pane", after, table));
        assert_eq!(tables.get("pane").unwrap().refs.get("e5"), Some(&42));
        assert!(tables.merge_at("pane", Some(after), "https://example.com/", &[("n1e1".to_string(), 7)]));
        // A close while one is in flight, for a pane with no table yet.
        let first = tables.epoch("fresh");
        tables.forget("fresh");
        assert!(!tables.put_at("fresh", first, RefTable::default()));
    }

    #[test]
    fn a_pane_whose_page_is_gone_keeps_no_references_but_keeps_counting_finds() {
        let tables = RefTables::default();
        let table = RefTable { url: "https://example.com/".into(), refs: HashMap::from([("e5".to_string(), 42)]), ..RefTable::default() };
        tables.put("pane", table);
        assert_eq!(tables.next_find_prefix("pane"), "n1");
        assert_eq!(tables.next_find_prefix("pane"), "n2");
        assert_eq!(tables.get("pane").unwrap().refs.get("e5"), Some(&42));
        tables.forget("pane");
        assert!(tables.get("pane").is_none());
        assert_eq!(tables.next_find_prefix("pane"), "n3", "an earlier find's references are never reused");
    }

    #[test]
    fn a_scoped_snapshot_in_a_frame_keeps_each_reference_with_its_frame() {
        let tables = RefTables::default();
        let frame = FrameRef { frame_id: "A".into(), session: None, origin: "http://a".into(), url: "http://a/in".into(), chain: vec![] };
        tables.put("p", RefTable { url: "http://a/".into(), refs: HashMap::from([("f1e12".to_string(), 5)]), frames: HashMap::from([("f1e12".to_string(), frame.clone())]) });
        let refs = [("f1e12.e1".to_string(), 8)];
        assert!(tables.merge_framed_at("p", None, "http://a/", &refs, &[("f1e12.e1".to_string(), frame.clone())]));
        let t = tables.get("p").unwrap();
        assert_eq!(t.refs.get("f1e12.e1"), Some(&8));
        assert_eq!(t.frames.get("f1e12.e1"), Some(&frame));
        assert_eq!(t.refs.get("f1e12"), Some(&5), "the full snapshot's references stay");
    }

    #[test]
    fn find_references_merge_into_the_table_and_are_never_numbered_twice() {
        let tables = RefTables::default();
        // A find on a fresh pane starts the table.
        assert_eq!(tables.next_find_prefix("b"), "n1");
        tables.merge("b", "https://a.example/", &[("n1e1".into(), 7)]);
        // A full snapshot replaces the table, but not the find count.
        tables.put("b", RefTable { url: "https://a.example/".into(), refs: HashMap::from([("e1".into(), 3)]), ..RefTable::default() });
        assert_eq!(tables.next_find_prefix("b"), "n2");
        tables.merge("b", "https://a.example/", &[("n2e1".into(), 9)]);
        let t = tables.get("b").unwrap();
        assert_eq!((t.refs.get("e1"), t.refs.get("n2e1"), t.refs.get("n1e1")), (Some(&3), Some(&9), None));
        // Another page starts a new table; the count goes on.
        tables.merge("b", "https://b.example/", &[("e4.e1".into(), 1)]);
        let t = tables.get("b").unwrap();
        assert_eq!((t.url.as_str(), t.refs.len()), ("https://b.example/", 1));
        assert_eq!(tables.next_find_prefix("b"), "n3");
        assert_eq!(tables.next_find_prefix("other"), "n1");
    }

    #[test]
    fn closed_panes_do_not_push_a_live_panes_table_out() {
        let tables = RefTables::default();
        tables.put("live", RefTable { url: "https://example.com/".into(), refs: HashMap::from([("e1".to_string(), 3)]), ..RefTable::default() });
        for i in 0..(MAX_TABLES * 2) {
            tables.forget(&format!("closed{i}"));
        }
        assert_eq!(tables.get("live").unwrap().refs.get("e1"), Some(&3));
        // A close is still seen by a request that began before it.
        let started = tables.epoch("later");
        tables.forget("later");
        assert!(!tables.put_at("later", started, RefTable::default()));
    }

    #[test]
    fn a_table_taken_before_the_page_closed_is_seen_to_be_stale() {
        let tables = RefTables::default();
        tables.put("pane", RefTable { url: "https://example.com/".into(), refs: HashMap::from([("e5".to_string(), 42)]), ..RefTable::default() });
        let (table, epoch) = tables.get_at("pane").unwrap();
        assert_eq!(table.refs.get("e5"), Some(&42));
        assert!(tables.still_at("pane", epoch));
        tables.forget("pane");
        assert!(!tables.still_at("pane", epoch));
        assert!(tables.get_at("pane").is_none());
    }
}
