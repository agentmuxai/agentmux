// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The activity rows (`tool`, `turn`, `thought`) of one audit folder's day files, newest first.
//!
//! A day's file holds only that day's rows (`audit_activity` names a row's file by its own time), so the files do not
//! overlap in time: the stream reads the newest day to its start, then the day before, and so on. A page's cursor
//! says which day and byte to carry on from.

use std::path::Path;

use super::ordered::Ordered;
use super::{jekt_file, Ctx, Filter};
use crate::backend::audit_activity::{day_files, ActivityRow, DayFile};
use crate::backend::rpc_types::{AuditEntry, AuditHistoryLink, AuditLinks, AuditPaneLink, AuditProvenance};

/// The start of the day after `day`, in Unix ms: a file whose day ended before the window started holds nothing of it.
pub(super) fn day_end_ms(day: &str) -> u64 {
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.succ_opt())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map_or(u64::MAX, |t| t.and_utc().timestamp_millis().max(0) as u64)
}

/// The Audit's entry for `row`, which is the line starting at byte `at` of the file for `day` in the folder tagged `tag`.
pub(super) fn entry_of(tag: char, day: &str, at: u64, row: &ActivityRow, ctx: &Ctx) -> AuditEntry {
    let slug = (ctx.slug_of_uid)(&row.agent).unwrap_or_else(|| row.agent.clone());
    AuditEntry {
        id: format!("a{tag}{day}:{at:016x}"),
        ts_ms: row.ts,
        kind: row.kind.clone(),
        agent_id: Some(slug.clone()),
        from: None,
        to: None,
        summary: row.summary.clone(),
        outcome: row.outcome.clone(),
        reason: None,
        cost: row.cost.clone(),
        links: AuditLinks {
            history: Some(AuditHistoryLink {
                agent_id: slug,
                definition_id: Some(row.agent.clone()),
                zone: Some(row.zone.clone()).filter(|_| row.byte.is_some()),
                at_ms: row.ts,
                at_byte: row.byte,
            }),
            pane: Some(row.block.as_str()).filter(|b| !b.is_empty() && (ctx.block_exists)(b)).map(|b| AuditPaneLink { block_id: b.to_string() }),
        },
        // Where the row was written, which a channel scope and the Audit's sections read.
        provenance: row.channel.as_ref().map(|c| AuditProvenance { channel: Some(c.clone()), ..Default::default() }),
        prompt: None,
        answer: None,
        level: None,
        standing: None,
    }
}

/// One folder's activity entries that pass the filter, newest first.
pub(super) struct DayStream<'a> {
    tag: char,
    ctx: &'a Ctx<'a>,
    filter: &'a Filter,
    after: Option<(u64, String)>,
    /// The days still to open, newest first.
    pending: std::vec::IntoIter<DayFile>,
    /// The day being read, and where its lines may end when it was opened.
    current: Option<(String, Option<u64>, Ordered<'a, ActivityRow>)>,
    /// The day and byte a page before stopped at: older days than it are not read again.
    resume_from: Option<(String, u64)>,
}

impl<'a> DayStream<'a> {
    /// `dir`: the folder (None: no activity here). `resume` is `YYYY-MM-DD:<byte>` from the cursor of the page before.
    pub fn new(dir: Option<&Path>, tag: char, filter: &'a Filter, ctx: &'a Ctx<'a>, after: Option<(u64, String)>, resume: Option<&str>) -> Self {
        let resume_from = resume.and_then(|r| r.split_once(':')).and_then(|(day, byte)| Some((day.to_string(), byte.parse().ok()?)));
        let days = dir
            .map(day_files)
            .unwrap_or_default()
            .into_iter()
            // A day newer than where the page before stopped was read already; one that ended before the window began holds none of it.
            .filter(|f| resume_from.as_ref().is_none_or(|(day, _)| f.day <= *day))
            .filter(|f| day_end_ms(&f.day) >= filter.since)
            .collect::<Vec<_>>();
        Self { tag, ctx, filter, after, pending: days.into_iter(), current: None, resume_from }
    }

    /// Opens days until the current one has an entry to give, or none are left.
    fn fill(&mut self) {
        loop {
            if let Some((_, _, stream)) = self.current.as_mut() {
                if stream.peek().is_some() {
                    return;
                }
            }
            let Some(file) = self.pending.next() else {
                return;
            };
            let end = self.resume_from.as_ref().filter(|(day, _)| *day == file.day).map(|(_, byte)| *byte);
            let (tag, ctx, filter, day) = (self.tag, self.ctx, self.filter, file.day.clone());
            let lines = jekt_file::entries::<ActivityRow>(&file.path, end);
            let stream = Ordered::new(lines, |row: &ActivityRow| row.ts, move |at, row| entry_of(tag, &day, at, row, ctx), move |e| filter.keep(e), filter.since, self.after.clone());
            self.current = Some((file.day, end, stream));
        }
    }

    pub fn peek(&mut self) -> Option<&AuditEntry> {
        self.fill();
        self.current.as_mut().and_then(|(_, _, s)| s.peek())
    }

    pub fn pop(&mut self) -> Option<AuditEntry> {
        self.fill();
        self.current.as_mut().and_then(|(_, _, s)| s.pop())
    }

    /// Where the next page carries on from, as `YYYY-MM-DD:<byte>`: the day being read and where in it. The days
    /// before it are opened in turn. None when no day was opened.
    pub fn resume(&self, previous: Option<&str>) -> Option<String> {
        match self.current.as_ref() {
            Some((day, opened_at, stream)) => Some(format!("{day}:{}", stream.resume(*opened_at)?)),
            None => previous.map(str::to_string),
        }
    }
}
