// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What every entry a filter matches cost, not only a page's. The audit file is only ever appended to, so a sum
//! is kept per file and filter and carried on from where the last scan of the file ended: the first asks reads
//! the file, the next ones only what was written since.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use super::{from_audit, jekt_file, Ctx, Filter, Sources, KIND_ACTION, KIND_JEKT, KIND_REGISTRY};
use crate::backend::rpc_types::AuditTotals;

/// Sums kept: the filters a view asks totals of are few.
const KEPT: usize = 16;

struct Kept {
    /// The file's bytes read, and the sum of the entries in them.
    read: u64,
    sum: AuditTotals,
    used: u64,
}

static KEPT_SUMS: Mutex<Option<HashMap<String, Kept>>> = Mutex::new(None);

/// Links aren't part of a sum: this keeps looking them up out of a scan of the whole file.
fn no_links() -> Ctx<'static> {
    Ctx { slug_of_uid: &|_| None, pane_of_agent: &|_| None, block_exists: &|_| false }
}

fn add(sum: &mut AuditTotals, e: &crate::backend::rpc_types::AuditEntry) {
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
    let Some(path) = src.jekt_file.filter(|_| wants_file) else { return sum };
    let key = format!("{}|{}", path.display(), filter.key());
    let (mut read, mut from_file) = (0, AuditTotals::default());
    let mut guard = KEPT_SUMS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(kept) = guard.as_mut().and_then(|m| m.get(&key)) {
        // A file shorter than what was read was replaced: start again.
        if std::fs::metadata(path).is_ok_and(|m| m.len() >= kept.read) {
            (read, from_file) = (kept.read, kept.sum.clone());
        }
    }
    drop(guard);
    scan_into(path, read, filter, &mut from_file).map_or((), |end| read = end);
    let mut guard = KEPT_SUMS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() >= KEPT && !map.contains_key(&key) {
        if let Some(oldest) = map.iter().min_by_key(|(_, k)| k.used).map(|(k, _)| k.clone()) {
            map.remove(&oldest);
        }
    }
    let used = map.values().map(|k| k.used).max().unwrap_or(0) + 1;
    map.insert(key, Kept { read, sum: from_file.clone(), used });
    drop(guard);
    sum.entries += from_file.entries;
    sum.input += from_file.input;
    sum.cache_read += from_file.cache_read;
    sum.output += from_file.output;
    sum.usd = match (sum.usd, from_file.usd) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    };
    sum.estimated |= from_file.estimated;
    sum
}

fn scan_into(path: &Path, from: u64, filter: &Filter, sum: &mut AuditTotals) -> Option<u64> {
    let ctx = no_links();
    jekt_file::scan(path, from, |raw| {
        let e = from_audit(0, &raw, &ctx);
        if filter.keep(&e) {
            add(sum, &e);
        }
    })
    .ok()
}
