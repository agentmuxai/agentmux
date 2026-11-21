// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What every entry a filter matches cost, not only a page's. The audit file and the activity day files are only
//! ever appended to, so a sum is kept per file and filter and carried on from where the last scan of the file ended:
//! the first asks reads the file, the next ones only what was written since.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use super::activity::{day_end_ms, entry_of};
use super::{from_audit, jekt_file, Ctx, Filter, Sources, KIND_ACTION, KIND_JEKT, KIND_REGISTRY, KIND_THOUGHT, KIND_TOOL, KIND_TURN};
use crate::backend::audit_activity::{day_files, ActivityRow};
use crate::backend::rpc_types::AuditTotals;

/// Sums kept: a file and a filter each, and a month of day files is thirty files for each filter a view asks totals of.
const KEPT: usize = 128;

struct Kept {
    /// The file's bytes read, and the sum of the entries in them.
    read: u64,
    sum: AuditTotals,
    used: u64,
}

static KEPT_SUMS: Mutex<Option<HashMap<String, Kept>>> = Mutex::new(None);

/// Links aren't part of a sum: this keeps looking them up out of a scan of the whole file.
fn no_links() -> Ctx<'static> {
    Ctx { slug_of_uid: &|_| None, uid_of_slug: &|_| None, pane_of_agent: &|_| None, block_exists: &|_| false }
}

fn add(sum: &mut AuditTotals, e: &crate::backend::rpc_types::AuditEntry) {
    // The sender's half of a message is the same row as the receiver's in the list: it is not counted twice.
    if e.provenance.as_ref().is_some_and(|p| p.sent_only == Some(true)) {
        return;
    }
    sum.entries += 1;
    if let Some(cost) = &e.cost {
        sum.input += cost.input;
        sum.cache_read += cost.cache_read;
        sum.output += cost.output;
        if let Some(usd) = cost.usd {
            *sum.usd.get_or_insert(0.0) += usd;
        }
        sum.estimated |= cost.basis != "measured";
    }
}

/// The sum over the audit file's and the ledger's entries `filter` matches.
pub(super) fn of(src: &Sources, filter: &Filter) -> AuditTotals {
    let mut sum = AuditTotals::default();
    if let Some(store) = src.segments {
        if filter.kinds.contains(super::KIND_AGENT_START) || filter.kinds.contains(super::KIND_AGENT_END) {
            for e in super::segments::entries(store, filter.agent.as_deref(), &src.ctx).iter().filter(|e| filter.keep(e)) {
                add(&mut sum, e);
            }
        }
    }
    let wants_file = [KIND_JEKT, KIND_ACTION, KIND_REGISTRY].iter().any(|k| filter.kinds.contains(*k));
    // The two files hold different lines (a line goes to one), so their sums add.
    for path in [src.jekt_file, src.own_file].into_iter().flatten().filter(|_| wants_file) {
        add_file(&mut sum, path, filter, |from, s| scan_into(path, from, filter, s));
    }
    add_activity(&mut sum, src, filter);
    sum
}

/// Adds the tool, turn and thought rows of the activity day files `filter` matches. A turn's row carries what the
/// turn cost, so leaving these out left the totals short of every turn.
fn add_activity(sum: &mut AuditTotals, src: &Sources, filter: &Filter) {
    if ![KIND_TOOL, KIND_TURN, KIND_THOUGHT].iter().any(|k| filter.kinds.contains(*k)) {
        return;
    }
    // A row names its agent by UID: the filter names it by slug. Nothing else of the links is read.
    let ctx = Ctx { slug_of_uid: src.ctx.slug_of_uid, uid_of_slug: &|_| None, pane_of_agent: &|_| None, block_exists: &|_| false };
    for dir in [src.activity_dir, src.own_activity_dir].into_iter().flatten() {
        // A day that ended before the window began holds none of it.
        for file in day_files(dir).into_iter().filter(|f| day_end_ms(&f.day) >= filter.since) {
            add_file(sum, &file.path, filter, |from, s| scan_activity(&file.path, &file.day, from, filter, &ctx, s));
        }
    }
}

/// Adds to `sum` what `scan` finds in the file at `path` for `filter`: `scan(from, sum)` reads from byte `from` and
/// returns where it ended.
fn add_file(sum: &mut AuditTotals, path: &Path, filter: &Filter, scan: impl Fn(u64, &mut AuditTotals) -> std::io::Result<u64>) {
    let key = format!("{}|{}", path.display(), filter.key());
    let (mut read, mut from_file) = (0u64, AuditTotals::default());
    let mut guard = KEPT_SUMS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(kept) = guard.as_mut().and_then(|m| m.get(&key)) {
        // A file shorter than what was read was replaced: start again.
        if std::fs::metadata(path).is_ok_and(|m| m.len() >= kept.read) {
            (read, from_file) = (kept.read, kept.sum.clone());
        }
    }
    drop(guard);
    // The scan's sum is kept with where it ended only if it ended cleanly: one that failed partway would
    // otherwise be counted again from the old position next time.
    let mut scanned = from_file.clone();
    match scan(read, &mut scanned) {
        Ok(end) => {
            let (read, from_file_now) = (end, scanned);
            let mut guard = KEPT_SUMS.lock().unwrap_or_else(|e| e.into_inner());
            let map = guard.get_or_insert_with(HashMap::new);
            if map.len() >= KEPT && !map.contains_key(&key) {
                if let Some(oldest) = map.iter().min_by_key(|(_, k)| k.used).map(|(k, _)| k.clone()) {
                    map.remove(&oldest);
                }
            }
            let used = map.values().map(|k| k.used).max().unwrap_or(0) + 1;
            map.insert(key, Kept { read, sum: from_file_now.clone(), used });
            from_file = from_file_now;
        }
        // A day file that went away between listing and opening (rotated or pruned) has nothing to add: not a warning.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(path = %path.display(), "audit.query totals: the audit file is gone; answering from what was read before")
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "audit.query totals: the audit file couldn't be read to its end; answering from what was read before")
        }
    }
    sum.entries += from_file.entries;
    sum.input += from_file.input;
    sum.cache_read += from_file.cache_read;
    sum.output += from_file.output;
    sum.usd = match (sum.usd, from_file.usd) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    };
    sum.estimated |= from_file.estimated;
}

fn scan_into(path: &Path, from: u64, filter: &Filter, sum: &mut AuditTotals) -> std::io::Result<u64> {
    let ctx = no_links();
    jekt_file::scan(path, from, |raw: crate::backend::reactive::types::AuditLogEntry| {
        let e = from_audit(0, &raw, &ctx);
        if filter.keep(&e) {
            add(sum, &e);
        }
    })
}

fn scan_activity(path: &Path, day: &str, from: u64, filter: &Filter, ctx: &Ctx, sum: &mut AuditTotals) -> std::io::Result<u64> {
    jekt_file::scan(path, from, |row: ActivityRow| {
        let e = entry_of('t', day, 0, &row, ctx);
        if filter.keep(&e) {
            add(sum, &e);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn jekt_filter() -> Filter {
        Filter { agent: None, kinds: HashSet::from([KIND_JEKT.to_string()]), since: 0, until: u64::MAX, text: String::new(), channel: None }
    }

    /// A day file pruned between the listing and the open adds nothing and is no failure: the scan says why it
    /// couldn't read (so the warning can), and the sum is left as it was.
    #[test]
    fn a_file_that_is_gone_adds_nothing_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("2026-11-17.jsonl");
        let filter = jekt_filter();

        let mut scanned = AuditTotals::default();
        let err = scan_into(&missing, 0, &filter, &mut scanned).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);

        let mut sum = AuditTotals::default();
        add_file(&mut sum, &missing, &filter, |from, s| scan_into(&missing, from, &filter, s));
        assert_eq!(sum, AuditTotals::default());
    }

    /// A line the writer was cut in the middle of is not read, and the scan still succeeds.
    #[test]
    fn a_cut_last_line_is_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        std::fs::write(&path, b"{\"half\":").unwrap();
        let mut sum = AuditTotals::default();
        assert_eq!(scan_into(&path, 0, &jekt_filter(), &mut sum).unwrap(), 0);
        assert_eq!(sum, AuditTotals::default());
    }
}
