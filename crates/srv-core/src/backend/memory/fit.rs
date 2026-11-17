// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What of an agent's Personal Memory fits in a delivery, and what the agent is told about the rest
//! (the re-send after a compaction; `memory_delivery` composes and splits what this keeps).
//!
//! A delivery carries at most `HOOK_PARTS * MAX_PART_CHARS` characters. Before this, the files went in by
//! name and the cut fell wherever the budget ended: one stale 50 KB snapshot at the front used it all, and
//! every later file, including the one the index marks "read first", showed as not sent.
//!
//! - Order: newest first (the file's `modified` frontmatter, else its mtime), done where the files are read.
//! - Index: Personal Memory goes in as an index (`index.rs`), one line per file, always complete. Only files marked
//!   `deliver: always` (up to [`ALWAYS_MAX_FILES`] files and [`ALWAYS_MAX_CHARS`] characters) go in full.
//! - Fit: Global Memory entries that still don't fit are left out whole and listed by name with the way to read
//!   them, never cut at the end of the text.
//! - Hygiene: on a compaction, one note when Personal Memory is over [`PERSONAL_BUDGET_CHARS`], a file is
//!   over the cap, or a project file hasn't been touched for [`STALE_DAYS`]. AgentMux never deletes memory:
//!   it can't tell a stale snapshot from a durable rule, so the agent trims its own.

use super::delivery::{Entry, Tier};
use super::index::{self, IndexPlan, ALWAYS_MAX_CHARS, ALWAYS_MAX_FILES};

/// The most characters one Personal Memory file contributes.
pub const FILE_CAP_CHARS: usize = 8_000;
/// Personal Memory above this (in characters, counted before the cap's cut) is worth pruning.
pub const PERSONAL_BUDGET_CHARS: usize = 40_000;
/// A `project` file untouched for this many days is called stale.
pub const STALE_DAYS: i64 = 7;
/// A capacity for a delivery that has no part cap (the fallback's one message).
pub const NO_CAP: usize = usize::MAX / 4;
/// The characters kept back for the header, the section headings and the part labels.
const OVERHEAD_CHARS: usize = 1_500;
/// The most names the hygiene note spells out.
const MAX_LISTED: usize = 25;
const DAY_MS: i64 = 86_400_000;
/// The most Global Memory entries the left-out note names; a name is about 60 characters, so the note stays small.
const MAX_LEFT_OUT_LISTED: usize = 100;

/// What the model gets to read: everything that fit, and a note about the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    /// Sent in full: the Global Memory that fit, then the Personal files marked always.
    pub kept: Vec<Entry>,
    /// Personal files listed in the index, not sent in full.
    pub indexed: Vec<Entry>,
    /// Names of files marked always that went over the limit and are in the index with the reason.
    pub over_limit: Vec<String>,
    /// How many Personal files went in full because they are marked always.
    pub always: usize,
    /// Global Memory entries left out whole because the delivery is full, named in the tail.
    pub skipped: Vec<Entry>,
    /// The kept files that went in cut at the cap, with what they were before it.
    /// The not-sent list and the hygiene note, joined; `None` when there is nothing to say.
    pub tail: Option<String>,
}

/// The value of `key` in the frontmatter block at the top of `text` (`---` … `---`), at any indent so a
/// nested `metadata:` key is found too.
pub fn frontmatter_value(text: &str, key: &str) -> Option<String> {
    let rest = text.strip_prefix("---")?.strip_prefix(['\r', '\n'])?;
    let mut prefix = String::from(key);
    prefix.push(':');
    for line in rest.lines() {
        if line.trim_end() == "---" {
            return None;
        }
        if let Some(value) = line.trim_start().strip_prefix(prefix.as_str()) {
            let value = value.trim().trim_matches('"').trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

/// The note naming the Global Memory entries left out of the delivery, with how to read each.
pub fn skipped_note(skipped: &[Entry]) -> Option<String> {
    if skipped.is_empty() {
        return None;
    }
    let noun = if skipped.len() == 1 { "entry was" } else { "entries were" };
    let mut out = format!(
        "[AgentMux] {} Global Memory {noun} left out here (the delivery is full). Read each with the Memory tool (action read, scope global, id <id>):\n",
        skipped.len()
    );
    for e in skipped.iter().take(MAX_LEFT_OUT_LISTED) {
        out.push_str(&format!("- {} ({})\n", e.name, e.bundle_id.as_deref().unwrap_or("no id")));
    }
    if skipped.len() > MAX_LEFT_OUT_LISTED {
        out.push_str(&format!("- … and {} more: list them with the Memory tool (action list, scope global)\n", skipped.len() - MAX_LEFT_OUT_LISTED));
    }
    Some(out)
}

/// What a Personal Memory file looks like to the hygiene check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub name: String,
    /// The file's size in characters before the cap's cut.
    pub chars: usize,
    pub kind: Option<String>,
    /// Days since it was last modified, if known.
    pub age_days: Option<i64>,
}

/// The note asking the agent to trim its Personal Memory, when it needs it: over the budget, a file over the
/// cap, or a project file stale for [`STALE_DAYS`].
pub fn hygiene_note(files: &[Seen]) -> Option<String> {
    let total: usize = files.iter().map(|f| f.chars).sum();
    let big: Vec<&Seen> = files.iter().filter(|f| f.chars > FILE_CAP_CHARS).collect();
    let stale: Vec<&Seen> = files.iter().filter(|f| f.kind.as_deref() == Some("project") && f.age_days.is_some_and(|d| d >= STALE_DAYS)).collect();
    if total <= PERSONAL_BUDGET_CHARS && big.is_empty() && stale.is_empty() {
        return None;
    }
    let mut lines = vec![format!(
        "[AgentMux] Your Personal Memory is {} KB ({} files; over about {} KB it is worth trimming).",
        total.div_ceil(1024),
        files.len(),
        PERSONAL_BUDGET_CHARS / 1024
    )];
    let list = |v: &[&Seen], how: &dyn Fn(&Seen) -> String| v.iter().take(MAX_LISTED).map(|f| format!("{} ({})", f.name, how(f))).collect::<Vec<_>>().join(", ");
    if !big.is_empty() {
        lines.push(format!("Over {} KB each, so their index line is all you see until you read them; worth splitting: {}.", FILE_CAP_CHARS / 1000, list(&big, &|f| format!("{} KB", f.chars.div_ceil(1024)))));
    }
    if !stale.is_empty() {
        lines.push(format!("Project files not touched for {STALE_DAYS}+ days: {}.", list(&stale, &|f| format!("{} days", f.age_days.unwrap_or(0)))));
    }
    lines.push("Trim or archive what is done or out of date. AgentMux never deletes memory, so what you leave stays.".to_string());
    Some(lines.join(" "))
}

/// Caps the Personal files and keeps, in the order given (the caller put the newest first), as many as fit in
/// one delivery after `reserve_chars` (the running summary and anything else that rides along). Global Memory
/// goes in whole entries only; Personal Memory goes in the index, and the few files marked `deliver: always`
/// in full within their limit. The tail is the left-out note, and the hygiene note when `hygiene` is set.
pub fn fit(entries: Vec<Entry>, capacity_chars: usize, reserve_chars: usize, hygiene: bool, modified_ms: &dyn Fn(&Entry) -> Option<i64>, now_ms: i64) -> Fitted {
    let seen: Vec<Seen> = entries
        .iter()
        .filter(|e| e.tier == Tier::Personal)
        .map(|e| Seen {
            name: e.name.clone(),
            chars: e.text.chars().count(),
            kind: frontmatter_value(&e.text, "type"),
            age_days: modified_ms(e).map(|m| (now_ms - m).max(0) / DAY_MS),
        })
        .collect();
    let (global, personal): (Vec<Entry>, Vec<Entry>) = entries.into_iter().partition(|e| e.tier == Tier::Global);

    // Marked files, oldest marking first (by the modification time, not by whole days: files changed today keep their
    // order), until a limit is reached; the rest of them go in the index with the reason. A file with no known time
    // counts as the newest.
    let mut marked: Vec<&Entry> = personal.iter().filter(|e| index::is_always(&e.text)).collect();
    marked.sort_by_key(|e| modified_ms(e).unwrap_or(i64::MAX));
    let (mut always, mut over_limit, mut always_chars) = (Vec::<Entry>::new(), Vec::<String>::new(), 0usize);
    for e in marked {
        let n = e.text.chars().count() + 5;
        if always.len() < ALWAYS_MAX_FILES && always_chars + n <= ALWAYS_MAX_CHARS {
            always_chars += n;
            always.push(e.clone());
        } else {
            over_limit.push(e.name.clone());
        }
    }
    let indexed: Vec<Entry> = personal.into_iter().filter(|e| !always.iter().any(|o| o.name == e.name)).collect();
    let index_chars = index::render(&IndexPlan { files: &indexed, over_limit: &over_limit, always: always.len(), tool_only: false })
        .map_or(0, |t| t.chars().count());

    // Leave room for the notes before counting.
    let notes_room = 1_200 + MAX_LEFT_OUT_LISTED * 80;
    let budget = capacity_chars.saturating_sub(OVERHEAD_CHARS + reserve_chars + notes_room + index_chars + always_chars);
    // System entries (Operator Config) are always kept: they are small, and the agent cannot read one back with the
    // `Memory` tool, so a left-out note could not tell it how to recover it. They count against the budget first.
    let weight = |e: &Entry| e.text.chars().count() + 5;
    let mut used: usize = global.iter().filter(|e| e.system).map(weight).sum();
    let (mut kept, mut skipped) = (Vec::new(), Vec::new());
    for e in global {
        let n = weight(&e);
        if e.system {
            kept.push(e);
        } else if used + n <= budget {
            used += n;
            kept.push(e);
        } else {
            skipped.push(e);
        }
    }
    let always_count = always.len();
    kept.extend(always);

    let mut tail: Vec<String> = Vec::new();
    tail.extend(skipped_note(&skipped));
    if hygiene {
        tail.extend(hygiene_note(&seen));
    }
    Fitted { kept, indexed, over_limit, always: always_count, skipped, tail: (!tail.is_empty()).then(|| tail.join("\n")) }
}

#[cfg(test)]
#[path = "fit_tests.rs"]
mod tests;
