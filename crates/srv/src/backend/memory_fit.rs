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
//! - Cap: a file over [`FILE_CAP_CHARS`] goes in cut, with a line saying how much is left and where.
//! - Fit: files that still don't fit are left out whole and listed by name and description, so the agent
//!   knows to read them.
//! - Hygiene: on a compaction, one note when Personal Memory is over [`PERSONAL_BUDGET_CHARS`], a file is
//!   over the cap, or a project file hasn't been touched for [`STALE_DAYS`]. AgentMux never deletes memory:
//!   it can't tell a stale snapshot from a durable rule, so the agent trims its own.

use super::memory_delivery::{Entry, Tier, HOOK_PARTS, MAX_PART_CHARS};

/// The most characters one Personal Memory file contributes.
pub const FILE_CAP_CHARS: usize = 8_000;
/// Personal Memory above this (in characters, counted before the cap's cut) is worth pruning.
pub const PERSONAL_BUDGET_CHARS: usize = 40_000;
/// A `project` file untouched for this many days is called stale.
pub const STALE_DAYS: i64 = 7;
/// The characters kept back for the header, the section headings and the part labels.
const OVERHEAD_CHARS: usize = 1_500;
/// The most names the not-sent list and the hygiene note spell out.
const MAX_LISTED: usize = 25;
/// The most characters of a description in either list.
const MAX_DESCRIPTION_CHARS: usize = 110;

/// What the model gets to read: everything that fit, and a note about the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    pub kept: Vec<Entry>,
    pub skipped: Vec<Entry>,
    /// The kept files that went in cut at the cap, with what they were before it.
    pub capped: Vec<Capped>,
    /// The not-sent list and the hygiene note, joined; `None` when there is nothing to say.
    pub tail: Option<String>,
}

/// A kept file the cap cut: its size before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capped {
    pub name: String,
    pub source_chars: usize,
    pub source_bytes: usize,
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

/// `entry` cut to [`FILE_CAP_CHARS`], at a line end when one falls in the last fifth, with the line saying how
/// much was left out. A Global Memory entry is never cut: it is the startup file's text, byte for byte.
pub fn cap_entry(mut entry: Entry) -> Entry {
    if entry.tier != Tier::Personal {
        return entry;
    }
    let total = entry.text.chars().count();
    if total <= FILE_CAP_CHARS {
        return entry;
    }
    let cut_at = entry.text.char_indices().nth(FILE_CAP_CHARS).map_or(entry.text.len(), |(i, _)| i);
    let head = &entry.text[..cut_at];
    let end = head.rfind('\n').filter(|&i| head[..i].chars().count() >= FILE_CAP_CHARS * 4 / 5).unwrap_or(head.len());
    let left = total - entry.text[..end].chars().count();
    let at = entry.path.as_deref().map(|p| format!(": {p}")).unwrap_or_default();
    let note = format!("\n[cut: {} KB more, read the file{at}]", left.div_ceil(1024));
    entry.text.truncate(end);
    entry.text.push_str(&note);
    entry
}

/// One name per line with its description, at most [`MAX_LISTED`] of them.
fn name_list<'a>(entries: impl Iterator<Item = &'a Entry>) -> String {
    let all: Vec<&Entry> = entries.collect();
    let mut out = String::new();
    for e in all.iter().take(MAX_LISTED) {
        out.push_str(&format!("- {}", e.name));
        if let Some(d) = frontmatter_value(&e.text, "description") {
            let d: String = d.chars().take(MAX_DESCRIPTION_CHARS).collect();
            out.push_str(&format!(" — {d}"));
        }
        out.push('\n');
    }
    if all.len() > MAX_LISTED {
        out.push_str(&format!("- … and {} more\n", all.len() - MAX_LISTED));
    }
    out
}

/// The note naming what was left out of the delivery.
pub fn skipped_note(skipped: &[Entry]) -> Option<String> {
    if skipped.is_empty() {
        return None;
    }
    let noun = if skipped.len() == 1 { "file was" } else { "files were" };
    Some(format!(
        "[AgentMux] {} Personal Memory {noun} not sent here (the delivery is full). Read what is relevant from your memory folder:\n{}",
        skipped.len(),
        name_list(skipped.iter())
    ))
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
        "[AgentMux] Your Personal Memory is {} KB ({} files; a delivery carries about {} KB of it).",
        total.div_ceil(1024),
        files.len(),
        PERSONAL_BUDGET_CHARS / 1024
    )];
    let list = |v: &[&Seen], how: &dyn Fn(&Seen) -> String| v.iter().take(MAX_LISTED).map(|f| format!("{} ({})", f.name, how(f))).collect::<Vec<_>>().join(", ");
    if !big.is_empty() {
        lines.push(format!("Over the {} KB per-file cap, so only the start is sent: {}.", FILE_CAP_CHARS / 1000, list(&big, &|f| format!("{} KB", f.chars.div_ceil(1024)))));
    }
    if !stale.is_empty() {
        lines.push(format!("Project files not touched for {STALE_DAYS}+ days: {}.", list(&stale, &|f| format!("{} days", f.age_days.unwrap_or(0)))));
    }
    lines.push("Trim or archive what is done or out of date. AgentMux never deletes memory, so what you leave stays.".to_string());
    Some(lines.join(" "))
}

/// Caps the Personal files and keeps, in the order given (the caller put the newest first), as many as fit in
/// one delivery after `reserve_chars` (the running summary and anything else that rides along). Global
/// Memory always stays whole. The tail is the not-sent list, and the hygiene note when `hygiene` is set.
pub fn fit(entries: Vec<Entry>, reserve_chars: usize, hygiene: bool, age_days: &dyn Fn(&Entry) -> Option<i64>) -> Fitted {
    let seen: Vec<Seen> = entries
        .iter()
        .filter(|e| e.tier == Tier::Personal)
        .map(|e| Seen { name: e.name.clone(), chars: e.text.chars().count(), kind: frontmatter_value(&e.text, "type"), age_days: age_days(e) })
        .collect();
    // Each file is judged at its capped size, but a file left out is handed back as it was, so the card shows its real size.
    let capped: Vec<(Entry, Entry)> = entries.into_iter().map(|e| (cap_entry(e.clone()), e)).collect();
    // Leave room for the two notes before counting: each skipped name costs a line.
    let notes_room = 1_200 + MAX_LISTED * (MAX_DESCRIPTION_CHARS + 40);
    let budget = (HOOK_PARTS * MAX_PART_CHARS).saturating_sub(OVERHEAD_CHARS + reserve_chars + notes_room);
    let mut used = 0usize;
    let (mut kept, mut skipped, mut cut) = (Vec::new(), Vec::new(), Vec::new());
    for (e, original) in capped {
        let n = e.text.chars().count() + 5;
        if e.tier == Tier::Global || used + n <= budget {
            used += n;
            if e.text != original.text {
                cut.push(Capped { name: e.name.clone(), source_chars: original.text.chars().count(), source_bytes: original.text.len() });
            }
            kept.push(e);
        } else {
            skipped.push(original);
        }
    }
    let mut tail: Vec<String> = Vec::new();
    tail.extend(skipped_note(&skipped));
    if hygiene {
        tail.extend(hygiene_note(&seen));
    }
    Fitted { kept, skipped, capped: cut, tail: (!tail.is_empty()).then(|| tail.join("\n")) }
}

#[cfg(test)]
#[path = "memory_fit_tests.rs"]
mod tests;
