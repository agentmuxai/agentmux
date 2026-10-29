// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The audit file's entries in time order. Several srvs append to the file, each stamping its entries when it
//! queues them, so the file is in time order only to within a short skew. Read from its end, a line is given only
//! once every line not yet read must be older: a window of the newest lines is held and sorted.

use std::collections::BinaryHeap;

use crate::backend::reactive::types::AuditLogEntry;
use crate::backend::rpc_types::AuditEntry;

/// How far out of time order two lines of the file may be.
const SKEW_MS: u64 = 60_000;

struct Held {
    entry: AuditEntry,
    /// Where the next line after this one starts.
    end: u64,
}

impl Held {
    fn key(&self) -> (u64, &str) {
        (self.entry.ts_ms, self.entry.id.as_str())
    }
}

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
impl Eq for Held {}
impl PartialOrd for Held {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Held {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

/// The lines of the file, newest first, mapped to entries and kept if `keep` says so.
pub(super) struct Ordered<'a> {
    lines: Box<dyn Iterator<Item = (u64, u64, AuditLogEntry)> + 'a>,
    map: Box<dyn Fn(u64, &AuditLogEntry) -> AuditEntry + 'a>,
    keep: Box<dyn Fn(&AuditEntry) -> bool + 'a>,
    /// Lines older than this need not be read.
    since: u64,
    /// Entries at or past this were given already (by the page before).
    after: Option<(u64, String)>,
    held: BinaryHeap<Held>,
    /// The oldest time read so far, and where the oldest line read starts.
    oldest_ms: u64,
    oldest_start: Option<u64>,
    done: bool,
}

impl<'a> Ordered<'a> {
    pub fn new(
        lines: impl Iterator<Item = (u64, u64, AuditLogEntry)> + 'a,
        map: impl Fn(u64, &AuditLogEntry) -> AuditEntry + 'a,
        keep: impl Fn(&AuditEntry) -> bool + 'a,
        since: u64,
        after: Option<(u64, String)>,
    ) -> Self {
        Self { lines: Box::new(lines), map: Box::new(map), keep: Box::new(keep), since, after, held: BinaryHeap::new(), oldest_ms: u64::MAX, oldest_start: None, done: false }
    }

    /// Reads until the newest held entry can be given: no line left unread can be newer.
    fn fill(&mut self) {
        loop {
            if self.held.peek().is_some_and(|top| self.done || top.entry.ts_ms >= self.oldest_ms.saturating_add(SKEW_MS)) {
                return;
            }
            if self.done {
                return;
            }
            let Some((start, end, raw)) = self.lines.next() else {
                self.done = true;
                continue;
            };
            if raw.timestamp.saturating_add(SKEW_MS) < self.since {
                self.done = true;
                continue;
            }
            self.oldest_ms = self.oldest_ms.min(raw.timestamp);
            self.oldest_start = Some(start);
            let entry = (self.map)(start, &raw);
            let fresh = self.after.as_ref().is_none_or(|(ts, id)| (entry.ts_ms, entry.id.as_str()) < (*ts, id.as_str()));
            if fresh && (self.keep)(&entry) {
                self.held.push(Held { entry, end });
            }
        }
    }

    pub fn peek(&mut self) -> Option<&AuditEntry> {
        self.fill();
        self.held.peek().map(|h| &h.entry)
    }

    pub fn pop(&mut self) -> Option<AuditEntry> {
        self.fill();
        self.held.pop().map(|h| h.entry)
    }

    /// Where reading starts again to meet every entry not given yet: after the last line held, or where the
    /// oldest line read starts when none is held; `from` when nothing was read.
    pub fn resume(&self, from: Option<u64>) -> Option<u64> {
        self.held.iter().map(|h| h.end).max().or(self.oldest_start).or(from)
    }
}
