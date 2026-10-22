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
    /// How many `find`s (and snapshots scoped by name) this pane has had: the
    /// `k` of their `n{k}e1` references. Kept when the table is replaced, so a
    /// reference from an earlier find never comes to name another element.
    pub finds: u32,
    /// How many times this pane's page has gone away. A snapshot or find notes it when it starts
    /// and its references are kept only if it is still the same when they are ready, so a request
    /// that was in flight as the page closed can't put the dead page's node ids back.
    pub epoch: u64,
}

/// Per-host store of each pane's latest reference table, by block id.
/// Bounded: past `MAX_TABLES` panes, the least recently snapshotted is
/// dropped (its agent just takes a new snapshot).
#[derive(Default)]
pub struct RefTables(std::sync::Mutex<Tables>);

type Tables = (u64, HashMap<String, (u64, RefTable)>);

const MAX_TABLES: usize = 64;

impl RefTables {
    fn lock(&self) -> std::sync::MutexGuard<'_, Tables> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[cfg(test)]
    pub(super) fn put(&self, block_id: &str, table: RefTable) {
        Self::put_locked(&mut self.lock(), block_id, table);
    }

    fn put_locked(g: &mut Tables, block_id: &str, mut table: RefTable) {
        if let Some((_, old)) = g.1.get(block_id) {
            table.finds = table.finds.max(old.finds);
            table.epoch = table.epoch.max(old.epoch);
        }
        g.0 += 1;
        let stamp = g.0;
        g.1.insert(block_id.to_string(), (stamp, table));
        while g.1.len() > MAX_TABLES {
            let oldest = g.1.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    g.1.remove(&k);
                }
                None => break,
            }
        }
    }

    pub(super) fn get(&self, block_id: &str) -> Option<RefTable> {
        self.lock().1.get(block_id).map(|(_, t)| t.clone())
    }

    /// Forget the references of a pane whose page is gone (closed, or put to sleep). A
    /// reference is a DevTools node id, which a new page in a new renderer process numbers from
    /// the start again: kept, an old `e5` would name whatever element has that number on the
    /// reloaded page, and a fill or a click would land in the wrong field. The find counter stays,
    /// so an earlier find's `n1e3` still never names another element.
    pub(crate) fn forget(&self, block_id: &str) {
        let mut g = self.lock();
        let (finds, epoch) = g.1.remove(block_id).map(|(_, old)| (old.finds, old.epoch)).unwrap_or((0, 0));
        // Even for a pane with no table yet: a first snapshot may be in flight.
        Self::put_locked(&mut g, block_id, RefTable { finds, epoch: epoch + 1, ..RefTable::default() });
    }

    /// The pane's epoch now (see [`RefTable::epoch`]); noted when a request starts.
    pub(super) fn epoch(&self, block_id: &str) -> u64 {
        self.lock().1.get(block_id).map(|(_, t)| t.epoch).unwrap_or(0)
    }

    /// [`put`](Self::put), unless the pane's page has gone away since `epoch`.
    pub(super) fn put_at(&self, block_id: &str, epoch: u64, table: RefTable) -> bool {
        let mut g = self.lock();
        if g.1.get(block_id).map(|(_, t)| t.epoch).unwrap_or(0) != epoch {
            return false;
        }
        Self::put_locked(&mut g, block_id, table);
        true
    }

    /// The prefix for the next find's references on this pane, `n{k}`.
    /// Counted under one lock, so two finds at once can't share one.
    pub(super) fn next_find_prefix(&self, block_id: &str) -> String {
        let mut g = self.lock();
        let mut table = g.1.get(block_id).map(|(_, t)| t.clone()).unwrap_or_default();
        table.finds += 1;
        let k = table.finds;
        Self::put_locked(&mut g, block_id, table);
        format!("n{k}")
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
        let mut g = self.lock();
        if epoch.is_some_and(|e| g.1.get(block_id).map(|(_, t)| t.epoch).unwrap_or(0) != e) {
            return false;
        }
        let mut table = g.1.get(block_id).map(|(_, t)| t.clone()).unwrap_or_default();
        if table.url != url {
            table = RefTable { finds: table.finds, ..RefTable::default() };
        }
        table.url = url.to_string();
        table.refs.extend(refs.iter().cloned());
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
        assert!(tables.get("pane").unwrap().refs.is_empty());
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
        let table = RefTable { url: "https://example.com/".into(), refs: HashMap::from([("e5".to_string(), 42)]), finds: 2, ..RefTable::default() };
        tables.put("pane", table);
        assert_eq!(tables.get("pane").unwrap().refs.get("e5"), Some(&42));
        tables.forget("pane");
        let after = tables.get("pane").unwrap();
        assert!(after.refs.is_empty() && after.frames.is_empty() && after.url.is_empty());
        assert_eq!(tables.next_find_prefix("pane"), "n3", "an earlier find's references are never reused");
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
}
