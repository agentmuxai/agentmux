// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's memory, delivered through Claude Code's `SessionStart` hook
//! (docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2).
//!
//! The hook's `additionalContext` reaches the model without showing as a
//! conversation turn — but, measured against the CLI AgentMux bundles
//! (2.1.280), only up to about 10,000 characters **per hook command**: past
//! that the CLI swaps it for a saved file plus a 2 KB preview, which the model
//! can't reliably read. Several hook commands' contexts are all delivered,
//! each under its own cap, in the order the hooks *finish* (they run in
//! parallel). So the memory is split into [`MAX_PART_CHARS`] parts, each
//! labelled "part N of M", and [`HOOK_PARTS`] hook commands each ask for one.
//!
//! This module is pure: composition, splitting and the source rules. The
//! per-session cache, the acknowledgements and the notice live in the server
//! handlers.

/// The most characters one part carries. The measured per-hook cap is about
/// 10,000; the margin covers the part label and any counting difference.
pub const MAX_PART_CHARS: usize = 9_000;

/// How many hook commands are installed, so the most a delivery can carry is
/// `HOOK_PARTS * MAX_PART_CHARS` characters. A memory larger than that is cut
/// at the end, and the last part says so.
pub const HOOK_PARTS: usize = 8;

/// Why memory is being delivered — the `SessionStart` hook's `source`, for the
/// sources that get a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// A new session (`startup`).
    Startup,
    /// `/clear` started a new, empty conversation.
    Clear,
    /// The conversation was just compacted.
    Compact,
}

impl Reason {
    /// The reason for a hook `source`, or `None` when that source gets no
    /// delivery: `resume` and `fork` continue a conversation that is intact,
    /// and the startup file still carries the memory (owner decision D10).
    pub fn from_hook_source(source: &str) -> Option<Reason> {
        match source {
            "startup" => Some(Reason::Startup),
            "clear" => Some(Reason::Clear),
            "compact" => Some(Reason::Compact),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Startup => "startup",
            Reason::Clear => "clear",
            Reason::Compact => "compact",
        }
    }

    fn clause(self) -> &'static str {
        match self {
            Reason::Startup => "This is the start of a new session.",
            Reason::Clear => "The conversation was just cleared.",
            Reason::Compact => COMPACTION_CLAUSE,
        }
    }
}

/// Whether an entry is Global Memory (Operator Config included) or the agent's
/// own Personal Memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Global,
    Personal,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Global => "global",
            Tier::Personal => "personal",
        }
    }
}

/// One memory entry: a Global Memory section as the startup file carries it,
/// or one Personal Memory file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What the notice calls it (`[AgentMux System] MuxAPI`, `notes.md`, …).
    pub label: String,
    pub tier: Tier,
    /// The full text sent to the model.
    pub text: String,
    /// The entry's own name: the Global Memory entry's name, or the file name.
    pub name: String,
    /// Global Memory only: AgentMux's own system tier, not the workspace's.
    pub system: bool,
    /// Global Memory only: the entry's id.
    pub bundle_id: Option<String>,
    /// Personal Memory only: the file it was read from.
    pub path: Option<String>,
}

/// The text delivered to the model: the Global Memory block (its sections
/// joined exactly as the startup file joins them), the Personal Memory files,
/// and — after a compaction — AgentMux's running summary, which the provider's
/// own compaction summary can't drop.
///
/// `None` when there is nothing to deliver.
#[cfg(test)]
pub fn compose(entries: &[Entry], reason: Reason, running_summary: Option<&str>) -> Option<String> {
    compose_items(entries, reason, running_summary).map(|c| c.text)
}

/// Where one item sits in a composed delivery, in characters
/// (SPEC_CONTEXT_DELIVERY_2026_09_30 §3.4 step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemSpan {
    /// The item's index in `entries`, or `None` for the running summary.
    pub entry: Option<usize>,
    pub start: usize,
    pub end: usize,
}

/// A composed delivery and where each item sits in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed {
    pub text: String,
    /// In delivery order: Global Memory, Personal Memory, then the summary.
    pub spans: Vec<ItemSpan>,
}

/// A string being built, with its length in characters kept alongside.
struct Tracked {
    out: String,
    chars: usize,
}

impl Tracked {
    /// Appends `s` and returns the character range it now occupies.
    fn push(&mut self, s: &str) -> (usize, usize) {
        let start = self.chars;
        self.out.push_str(s);
        self.chars += s.chars().count();
        (start, self.chars)
    }
}

/// [`compose`], also recording where each item landed so the notice can say
/// which items a cut delivery carried whole, in part, or not at all.
#[cfg(test)]
pub fn compose_items(entries: &[Entry], reason: Reason, running_summary: Option<&str>) -> Option<Composed> {
    compose_items_with(entries, reason, running_summary, false)
}

/// [`compose_items`], saying so when the Global Memory was left out because
/// the agent's startup file already carries it (a new Claude session that
/// loaded `AGENTMUX_MEMORY.md` or an AgentMux-owned `CLAUDE.md`;
/// SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md §4.4).
pub fn compose_items_with(
    entries: &[Entry],
    reason: Reason,
    running_summary: Option<&str>,
    global_in_startup_file: bool,
) -> Option<Composed> {
    let global: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].tier == Tier::Global).collect();
    let personal: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].tier == Tier::Personal).collect();
    let summary = running_summary.filter(|s| !s.trim().is_empty() && reason == Reason::Compact);
    if global.is_empty() && personal.is_empty() && summary.is_none() {
        return None;
    }

    let mut b = Tracked { out: String::new(), chars: 0 };
    if global_in_startup_file {
        b.push(&format!(
            "AgentMux memory for this agent. {} Your Global Memory is already in your startup \
             instructions, so it isn't repeated here. Read all of this now — it is your complete \
             Personal Memory, not just the index.\n\n",
            reason.clause()
        ));
    } else {
        b.push(&format!(
            "AgentMux memory for this agent. {} Read all of it now — it is your complete Global \
             Memory and Personal Memory, not just the index.\n\n",
            reason.clause()
        ));
    }
    let mut spans = Vec::new();
    let wrote_section = push_sections(&mut b, entries, &global, &personal, &mut spans);
    if let Some(summary) = summary {
        if wrote_section {
            b.push("\n");
        }
        let (start, end) = b.push(summary);
        spans.push(ItemSpan { entry: None, start, end });
    }
    Some(Composed { text: b.out, spans })
}

/// `# Global Memory (n entries)` and `# Personal Memory (…)`, each item's
/// text whole, sections separated by a blank line; empty sections left out.
/// The frontend's `composeReinjectionMessage` writes the same, so a
/// re-delivery carries the startup block's sections byte for byte. Returns
/// whether anything was written.
fn push_sections(b: &mut Tracked, entries: &[Entry], global: &[usize], personal: &[usize], spans: &mut Vec<ItemSpan>) -> bool {
    let mut wrote_section = false;
    let sections = [
        ("Global Memory", global, crate::backend::storage::bundles::GLOBAL_SECTION_SEPARATOR),
        ("Personal Memory", personal, "\n---\n"),
    ];
    for (heading, list, separator) in sections {
        if list.is_empty() {
            continue;
        }
        if wrote_section {
            b.push("\n");
        }
        let noun = if list.len() == 1 { "entry" } else { "entries" };
        b.push(&format!("# {heading} ({} {noun})\n", list.len()));
        for (k, &i) in list.iter().enumerate() {
            if k > 0 {
                b.push(separator);
            }
            let (start, end) = b.push(&entries[i].text);
            spans.push(ItemSpan { entry: Some(i), start, end });
        }
        b.push("\n");
        wrote_section = true;
    }
    wrote_section
}

/// The sentence after the signature when a compaction caused the delivery:
/// one definition for the hook's text, the fallback's, and the check that
/// adds the running summary (`continuity_state`).
pub const COMPACTION_CLAUSE: &str = "Your recent conversation was just compacted into a summary.";

/// The sentence every hidden memory re-delivery opens with, whatever the
/// reason: what `is_hidden_reinjection_text` (srv) and
/// `isMemoryReinjectionMessage` (frontend) recognise it by.
pub const REINJECTION_SIGNATURE: &str = "Your memory was reinjected because your working context was just reset.";

/// Why the frontend's fallback re-delivers memory
/// (`memory-reinjection.ts`'s `ReinjectionReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// The conversation was compacted.
    Compaction,
    /// AgentMux couldn't resume the prior session, so a fresh one started.
    FreshSession,
}

impl FallbackReason {
    pub fn from_wire(reason: &str) -> Option<Self> {
        match reason {
            "compaction" => Some(Self::Compaction),
            "fresh_session" => Some(Self::FreshSession),
            _ => None,
        }
    }

    /// The second sentence — the frontend's `REASON_CLAUSE`, verbatim.
    pub fn clause(self) -> &'static str {
        match self {
            Self::Compaction => COMPACTION_CLAUSE,
            Self::FreshSession => {
                "AgentMux could not resume this agent's prior session, so a fresh one was started. Any record \
                 of the prior conversation AgentMux had came with your first message, in an \
                 <agentmux-continuation> block."
            }
        }
    }
}

/// The fallback's hidden message, composed on srv
/// (SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.4 step 2, CD2b). Its wire format
/// is the frontend's `composeReinjectionMessage`, unchanged: the
/// `<system-reminder>` envelope and [`REINJECTION_SIGNATURE`] are how srv's
/// digest, the continuation packet and replay recognise a hidden turn. No
/// part cap: the fallback is one message. `None` when there is no memory.
/// A compaction's running summary is not here: srv's input handler appends
/// it (`continuity_state::with_state_after_compaction`).
pub fn compose_fallback(entries: &[Entry], reason: FallbackReason) -> Option<Composed> {
    let global: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].tier == Tier::Global).collect();
    let personal: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].tier == Tier::Personal).collect();
    if global.is_empty() && personal.is_empty() {
        return None;
    }
    let mut b = Tracked { out: String::new(), chars: 0 };
    b.push(&format!(
        "<system-reminder>\n{REINJECTION_SIGNATURE} {} Below is your\n\
         complete Global Memory and Personal Memory content — read all of it now,\n\
         not just the index.\n\n",
        reason.clause()
    ));
    let mut spans = Vec::new();
    push_sections(&mut b, entries, &global, &personal, &mut spans);
    b.push("</system-reminder>\n");
    Some(Composed { text: b.out, spans })
}

/// What opens the catch-up block of a resume phase's message, and what
/// `is_hidden_reinjection_text` (srv) and the stream parser (frontend) know
/// the message by when it carries no memory. The block is AgentMux's, not the
/// user's: the pane shows a card for it, never the text, but the agent's reply
/// is shown (SPEC_RESUME_PHASE_CATCH_UP_TURN_2026_10_13.md §3.3).
pub const CATCH_UP_SIGNATURE: &str = "AgentMux catch-up:";

/// Which resume phase a catch-up turn follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchUpKind {
    /// A fresh session started and carries AgentMux's record of the
    /// conversation (an account switch, a refused or rejected resume).
    FreshSession,
    /// The conversation was compacted.
    Compaction,
    /// The user resumed the conversation; the provider kept its context.
    Resumed,
}

impl CatchUpKind {
    pub fn from_wire(reason: &str) -> Option<Self> {
        match reason {
            "fresh_session" => Some(Self::FreshSession),
            "compaction" => Some(Self::Compaction),
            "resumed" => Some(Self::Resumed),
            _ => None,
        }
    }

    /// The memory event this phase shares with the `SessionStart` hook, when
    /// the phase delivers memory at all.
    pub fn memory_reason(self) -> Option<FallbackReason> {
        match self {
            Self::FreshSession => Some(FallbackReason::FreshSession),
            Self::Compaction => Some(FallbackReason::Compaction),
            Self::Resumed => None,
        }
    }

    fn situation(self) -> &'static str {
        match self {
            Self::FreshSession => {
                "You are picking up this conversation in a new session, for example after an account switch."
            }
            Self::Compaction => "You are picking up this conversation after a compaction.",
            Self::Resumed => "You are picking up this conversation after it was resumed.",
        }
    }
}

/// What the agent is told to do once it has read what it was given. One
/// constant so every resume phase says the same thing.
const CATCH_UP_INSTRUCTION: &str = "Review the conversation and anything AgentMux just gave you. Before replying you may re-check real state \
read-only (git status, open pull requests, files) to confirm what is still true; do not change anything yet. \
Then reply to the user in a few lines: where you are picking up (the last thing they asked and whether it was \
done), anything in flight or waiting on them, and two or three suggested next steps they can choose from. Then \
stop and wait for them.";

/// The message of a catch-up turn: the memory re-delivery when `memory` is the
/// text of one (`compose_fallback`), then the catch-up block, in one
/// `<system-reminder>`. Without memory the block stands alone: the hook
/// already delivered it, or the phase delivers none.
pub fn compose_catch_up(kind: CatchUpKind, memory: Option<&str>) -> String {
    let mut out = match memory {
        Some(text) => text.trim_end().trim_end_matches("</system-reminder>").trim_end().to_string() + "\n\n",
        None => "<system-reminder>\n".to_string(),
    };
    out.push_str(&format!("{CATCH_UP_SIGNATURE} {} {CATCH_UP_INSTRUCTION}\n</system-reminder>\n", kind.situation()));
    out
}

/// The last words of the instruction, which end every catch-up block. The
/// frontend's `isCatchUpMessage` matches the block by them too.
pub const CATCH_UP_TAIL: &str = "Then stop and wait for them.";

/// Whether `text` is a catch-up message, which has no reinjection signature
/// when it carries no memory. Its input is hidden, but the agent's reply is
/// shown and read: the ambient digest and the continuation record skip the
/// input without hiding what follows, and the hidden-reinjection gate
/// (`set_hidden_reinjection_active`) is not set for it.
///
/// Structural, not a substring: the block is the single last line of the
/// message, so a memory body that happens to contain the marker is not taken
/// for one.
pub fn is_catch_up_text(text: &str) -> bool {
    if !text.starts_with("<system-reminder>") {
        return false;
    }
    let Some(body) = text.strip_suffix("\n</system-reminder>\n") else { return false };
    let Some(at) = body.rfind(&format!("\n{CATCH_UP_SIGNATURE} ")) else { return false };
    let block = &body[at + 1..];
    !block.contains('\n') && block.ends_with(CATCH_UP_TAIL)
}

/// How much of one item a delivery carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivered {
    Full,
    /// Cut by the part cap: this many of its characters went out.
    Partial(usize),
    Omitted,
}

impl Delivered {
    /// Where `span` falls against the first `delivered_chars` characters of
    /// the composed text, which is all a cut delivery carries.
    pub fn of(span: &ItemSpan, delivered_chars: usize) -> Delivered {
        if span.end <= delivered_chars {
            Delivered::Full
        } else if span.start >= delivered_chars {
            Delivered::Omitted
        } else {
            Delivered::Partial(delivered_chars - span.start)
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Delivered::Full => "full",
            Delivered::Partial(_) => "partial",
            Delivered::Omitted => "omitted",
        }
    }
}

/// `text` split into parts of at most `max_chars` characters each, label
/// included, every one saying which part of how many it is (the hooks
/// deliver in the order they finish, not in order). Cuts at a line end where
/// one falls in the second half of a part, else mid-line. More than
/// `max_parts` parts: the text is cut, and the last part ends with a note
/// giving how much was left out.
#[cfg(test)]
pub fn split_into_parts(text: &str, max_chars: usize, max_parts: usize) -> Vec<String> {
    split_into_parts_counted(text, max_chars, max_parts).0
}

/// [`split_into_parts`], plus how many of `text`'s characters the parts carry:
/// all of them unless the text was cut.
pub fn split_into_parts_counted(text: &str, max_chars: usize, max_parts: usize) -> (Vec<String>, usize) {
    // The label is at most "[AgentMux memory — part 99 of 99]\n".
    const LABEL_ROOM: usize = 40;
    const CUT_NOTE_ROOM: usize = 120;
    let budget = max_chars.saturating_sub(LABEL_ROOM).max(1);

    let chars: Vec<char> = text.chars().collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let end = (at + budget).min(chars.len());
        let cut = if end == chars.len() {
            end
        } else {
            // The last newline in the second half of this chunk, if any.
            (at + budget / 2..end).rev().find(|&i| chars[i] == '\n').map_or(end, |i| i + 1)
        };
        chunks.push(chars[at..cut].iter().collect());
        at = cut;
    }

    let mut delivered = chars.len();
    if chunks.len() > max_parts {
        let kept: usize = chunks[..max_parts].iter().map(|c| c.chars().count()).sum();
        let omitted = chars.len() - kept;
        chunks.truncate(max_parts);
        let last = chunks.last_mut().expect("max_parts >= 1");
        // Make room for the note inside the last part's budget.
        let keep = last.chars().count().min(budget.saturating_sub(CUT_NOTE_ROOM));
        let dropped = last.chars().count() - keep;
        *last = last.chars().take(keep).collect();
        last.push_str(&format!(
            "\n\n[AgentMux: memory cut here — {} more characters did not fit in one delivery.]",
            omitted + dropped
        ));
        delivered = kept - dropped;
    }

    let of = chunks.len();
    let parts = chunks
        .into_iter()
        .enumerate()
        .map(|(i, c)| format!("[AgentMux memory — part {} of {of}]\n{c}", i + 1))
        .collect();
    (parts, delivered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, tier: Tier, text: &str) -> Entry {
        Entry { label: label.into(), tier, text: text.into(), name: label.into(), system: false, bundle_id: None, path: None }
    }

    #[test]
    fn spans_point_at_each_item_in_the_composed_text() {
        let entries = [
            entry("A", Tier::Global, "alpha"),
            entry("n.md", Tier::Personal, "notes"),
            entry("B", Tier::Global, "beta"),
        ];
        let c = compose_items(&entries, Reason::Compact, Some("summary")).unwrap();
        assert_eq!(c.text, compose(&entries, Reason::Compact, Some("summary")).unwrap(), "same text as compose");
        let chars: Vec<char> = c.text.chars().collect();
        let at = |s: &ItemSpan| chars[s.start..s.end].iter().collect::<String>();
        // Global first (in entry order), then Personal, then the summary.
        let order: Vec<Option<usize>> = c.spans.iter().map(|s| s.entry).collect();
        assert_eq!(order, vec![Some(0), Some(2), Some(1), None]);
        assert_eq!(c.spans.iter().map(at).collect::<Vec<_>>(), vec!["alpha", "beta", "notes", "summary"]);
    }

    #[test]
    fn spans_are_counted_in_characters_not_bytes() {
        let entries = [entry("é", Tier::Global, "ééé"), entry("x", Tier::Personal, "xyz")];
        let c = compose_items(&entries, Reason::Startup, None).unwrap();
        let chars: Vec<char> = c.text.chars().collect();
        assert_eq!(chars[c.spans[1].start..c.spans[1].end].iter().collect::<String>(), "xyz");
    }

    #[test]
    fn delivered_status_follows_the_cut() {
        let span = ItemSpan { entry: Some(0), start: 100, end: 200 };
        assert_eq!(Delivered::of(&span, 200), Delivered::Full);
        assert_eq!(Delivered::of(&span, 5_000), Delivered::Full);
        assert_eq!(Delivered::of(&span, 150), Delivered::Partial(50));
        assert_eq!(Delivered::of(&span, 100), Delivered::Omitted);
        assert_eq!(Delivered::of(&span, 10), Delivered::Omitted);
    }

    #[test]
    fn counted_split_reports_everything_when_uncut() {
        let text = "z".repeat(20_000);
        let (parts, delivered) = split_into_parts_counted(&text, MAX_PART_CHARS, HOOK_PARTS);
        assert_eq!(delivered, text.chars().count());
        assert_eq!(parts, split_into_parts(&text, MAX_PART_CHARS, HOOK_PARTS));
    }

    #[test]
    fn counted_split_reports_exactly_the_delivered_prefix_when_cut() {
        let text = (0..200_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect::<String>();
        let (parts, delivered) = split_into_parts_counted(&text, MAX_PART_CHARS, HOOK_PARTS);
        assert!(delivered < text.chars().count());
        let carried: String = parts
            .iter()
            .map(|p| p.split_once('\n').unwrap().1.split("\n\n[AgentMux: memory cut here").next().unwrap())
            .collect();
        assert_eq!(carried.chars().count(), delivered);
        assert!(text.starts_with(&carried), "the parts carry a prefix of the text");
    }

    #[test]
    fn resume_and_fork_get_no_delivery() {
        assert_eq!(Reason::from_hook_source("startup"), Some(Reason::Startup));
        assert_eq!(Reason::from_hook_source("clear"), Some(Reason::Clear));
        assert_eq!(Reason::from_hook_source("compact"), Some(Reason::Compact));
        assert_eq!(Reason::from_hook_source("resume"), None, "D10");
        assert_eq!(Reason::from_hook_source("fork"), None, "a fork continues an intact conversation");
        assert_eq!(Reason::from_hook_source("anything"), None);
    }

    #[test]
    fn composes_global_block_then_personal_then_summary_on_compact() {
        let entries = [
            entry("[AgentMux System] App API", Tier::Global, "IMPORTANT: …\n\n# [AgentMux System] App API\n\napi"),
            entry("[Workspace] Rules", Tier::Global, "# [Workspace] Rules\n\nrules"),
            entry("notes.md", Tier::Personal, "my notes"),
        ];
        let text = compose(&entries, Reason::Compact, Some("# Running summary …\nsummary")).unwrap();
        let global_block = format!(
            "IMPORTANT: …\n\n# [AgentMux System] App API\n\napi{}# [Workspace] Rules\n\nrules",
            crate::backend::storage::bundles::GLOBAL_SECTION_SEPARATOR
        );
        assert!(text.contains(&format!("# Global Memory (2 entries)\n{global_block}\n")), "{text}");
        assert!(text.contains("# Personal Memory (1 entry)\nmy notes\n"));
        let (g, p, s) = (text.find("# Global").unwrap(), text.find("# Personal").unwrap(), text.find("# Running summary").unwrap());
        assert!(g < p && p < s);
        assert!(text.contains("just compacted"));
    }

    #[test]
    fn the_summary_rides_only_on_a_compaction() {
        let entries = [entry("notes.md", Tier::Personal, "n")];
        let startup = compose(&entries, Reason::Startup, Some("# Running summary …")).unwrap();
        assert!(!startup.contains("Running summary"));
        assert!(startup.contains("start of a new session"));
    }

    #[test]
    fn nothing_to_deliver_is_none() {
        assert_eq!(compose(&[], Reason::Startup, None), None);
        assert_eq!(compose(&[], Reason::Startup, Some("summary")), None, "no summary outside a compaction");
        assert!(compose(&[], Reason::Compact, Some("summary")).is_some(), "a summary alone is worth delivering");
    }

    #[test]
    fn small_text_is_one_labelled_part() {
        let parts = split_into_parts("hello", MAX_PART_CHARS, HOOK_PARTS);
        assert_eq!(parts, vec!["[AgentMux memory — part 1 of 1]\nhello".to_string()]);
    }

    #[test]
    fn every_part_fits_the_cap_and_the_parts_rebuild_the_text() {
        let text = (0..3_000).map(|i| format!("line {i} of the memory\n")).collect::<String>();
        let parts = split_into_parts(&text, MAX_PART_CHARS, HOOK_PARTS);
        assert!(parts.len() > 1);
        for (i, p) in parts.iter().enumerate() {
            assert!(p.chars().count() <= MAX_PART_CHARS, "part {i} is {} chars", p.chars().count());
            assert!(p.starts_with(&format!("[AgentMux memory — part {} of {}]\n", i + 1, parts.len())));
        }
        let rebuilt: String = parts.iter().map(|p| p.split_once('\n').unwrap().1).collect();
        assert_eq!(rebuilt, text);
        assert!(parts[..parts.len() - 1].iter().all(|p| p.ends_with('\n')), "cut at line ends");
    }

    #[test]
    fn a_long_line_is_cut_mid_line() {
        let text = "x".repeat(20_000);
        let parts = split_into_parts(&text, MAX_PART_CHARS, HOOK_PARTS);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_PART_CHARS));
        let rebuilt: String = parts.iter().map(|p| p.split_once('\n').unwrap().1).collect();
        assert_eq!(rebuilt, text);
    }

    #[test]
    fn more_than_the_hooks_can_carry_is_cut_with_a_note() {
        let text = "y".repeat(MAX_PART_CHARS * (HOOK_PARTS + 2));
        let parts = split_into_parts(&text, MAX_PART_CHARS, HOOK_PARTS);
        assert_eq!(parts.len(), HOOK_PARTS);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_PART_CHARS));
        let last = parts.last().unwrap();
        assert!(last.contains("memory cut here"), "{}", &last[last.len() - 200..]);
        let kept: usize = parts
            .iter()
            .map(|p| p.split_once('\n').unwrap().1.split("\n\n[AgentMux: memory cut here").next().unwrap().len())
            .sum();
        let omitted: usize = last.rsplit_once("— ").unwrap().1.split(' ').next().unwrap().parse().unwrap();
        assert_eq!(kept + omitted, text.len(), "the note's count is exact");
    }

    #[test]
    fn multibyte_text_is_counted_in_characters() {
        let text = "é".repeat(12_000);
        let parts = split_into_parts(&text, MAX_PART_CHARS, HOOK_PARTS);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_PART_CHARS));
        let rebuilt: String = parts.iter().map(|p| p.split_once('\n').unwrap().1).collect();
        assert_eq!(rebuilt, text);
    }

    /// The fallback's wire format is the frontend's `composeReinjectionMessage`
    /// byte for byte (CD2b). `memory-reinjection.test.ts` pins this same
    /// literal for the same entries, so the two can't drift.
    #[test]
    fn compose_fallback_writes_the_frontends_exact_message() {
        let entries = vec![
            Entry {
                label: "[AgentMux System] App API".into(),
                tier: Tier::Global,
                text: "# [AgentMux System] App API\n\nG1".into(),
                name: "App API".into(),
                system: true,
                bundle_id: Some("b1".into()),
                path: None,
            },
            Entry {
                label: "[Workspace] Rules".into(),
                tier: Tier::Global,
                text: "# [Workspace] Rules\n\nG2".into(),
                name: "Rules".into(),
                system: false,
                bundle_id: Some("b2".into()),
                path: None,
            },
            Entry {
                label: "notes.md".into(),
                tier: Tier::Personal,
                text: "P1".into(),
                name: "notes.md".into(),
                system: false,
                bundle_id: None,
                path: Some("/m/notes.md".into()),
            },
        ];
        let c = compose_fallback(&entries, FallbackReason::Compaction).unwrap();
        assert_eq!(
            c.text,
            "<system-reminder>\n\
             Your memory was reinjected because your working context was just reset. Your recent conversation was just compacted into a summary. Below is your\n\
             complete Global Memory and Personal Memory content — read all of it now,\n\
             not just the index.\n\n\
             # Global Memory (2 entries)\n\
             # [AgentMux System] App API\n\nG1\n\n---\n\n# [Workspace] Rules\n\nG2\n\
             \n\
             # Personal Memory (1 entry)\n\
             P1\n\
             </system-reminder>\n"
        );
        assert_eq!(c.spans.len(), 3);
        assert!(crate::ambient::digest::is_hidden_reinjection_text(&c.text));

        let fresh = compose_fallback(&entries[2..], FallbackReason::FreshSession).unwrap();
        assert!(fresh.text.contains(
            "AgentMux could not resume this agent's prior session, so a fresh one was started. Any record of the prior \
             conversation AgentMux had came with your first message, in an <agentmux-continuation> block. Below is your\n"
        ));
        assert!(!fresh.text.contains("# Global Memory"), "an empty section is left out");
        assert_eq!(compose_fallback(&[], FallbackReason::Compaction), None);
    }

    #[test]
    fn fallback_reasons_are_the_frontends() {
        assert_eq!(FallbackReason::from_wire("compaction"), Some(FallbackReason::Compaction));
        assert_eq!(FallbackReason::from_wire("fresh_session"), Some(FallbackReason::FreshSession));
        assert_eq!(FallbackReason::from_wire("startup"), None);
    }

    #[test]
    fn a_delivery_without_the_global_memory_says_where_it_is() {
        let personal = vec![Entry {
            label: "notes.md".into(),
            tier: Tier::Personal,
            text: "P1".into(),
            name: "notes.md".into(),
            system: false,
            bundle_id: None,
            path: None,
        }];
        let c = compose_items_with(&personal, Reason::Startup, None, true).unwrap();
        assert!(c.text.contains("Your Global Memory is already in your startup instructions"), "{}", c.text);
        assert!(!c.text.contains("# Global Memory"));
        assert!(c.text.contains("# Personal Memory (1 entry)\nP1\n"));
        assert_eq!(compose_items_with(&[], Reason::Startup, None, true), None);
    }
    #[test]
    fn catch_up_kinds_are_the_frontends() {
        assert_eq!(CatchUpKind::from_wire("fresh_session"), Some(CatchUpKind::FreshSession));
        assert_eq!(CatchUpKind::from_wire("compaction"), Some(CatchUpKind::Compaction));
        assert_eq!(CatchUpKind::from_wire("resumed"), Some(CatchUpKind::Resumed));
        assert_eq!(CatchUpKind::from_wire("startup"), None);
        assert_eq!(CatchUpKind::Resumed.memory_reason(), None);
        assert_eq!(CatchUpKind::FreshSession.memory_reason(), Some(FallbackReason::FreshSession));
    }

    #[test]
    fn a_catch_up_without_memory_is_the_block_alone() {
        let text = compose_catch_up(CatchUpKind::Resumed, None);
        assert!(text.starts_with("<system-reminder>\nAgentMux catch-up: "), "{text}");
        assert!(text.ends_with("</system-reminder>\n"));
        assert!(!text.contains(REINJECTION_SIGNATURE));
        assert!(is_catch_up_text(&text));
        assert!(text.contains("suggested next steps"));
    }

    #[test]
    fn a_catch_up_with_memory_keeps_the_reinjection_message_whole_and_adds_one_block() {
        let entries = vec![Entry {
            label: "notes.md".into(),
            tier: Tier::Personal,
            text: "P1".into(),
            name: "notes.md".into(),
            system: false,
            bundle_id: None,
            path: None,
        }];
        let memory = compose_fallback(&entries, FallbackReason::FreshSession).unwrap();
        let text = compose_catch_up(CatchUpKind::FreshSession, Some(&memory.text));
        // Everything before the closing tag is the re-delivery, unchanged, so
        // the card's item spans still index into it.
        let body = memory.text.trim_end().trim_end_matches("</system-reminder>");
        assert!(text.starts_with(body), "{text}");
        assert_eq!(text.matches("<system-reminder>").count(), 1);
        assert_eq!(text.matches("</system-reminder>").count(), 1);
        assert!(text.contains(REINJECTION_SIGNATURE) && is_catch_up_text(&text));
        assert!(text.ends_with("</system-reminder>\n"));
    }

    #[test]
    fn ordinary_text_is_not_a_catch_up() {
        assert!(!is_catch_up_text("AgentMux catch-up: please"));
        assert!(!is_catch_up_text("<system-reminder>unrelated</system-reminder>"));
    }

    #[test]
    fn the_instruction_ends_with_the_tail_the_matchers_use() {
        assert!(CATCH_UP_INSTRUCTION.ends_with(CATCH_UP_TAIL));
    }

    /// A memory body is copied into a reinjection verbatim: one that happens to
    /// contain the marker, even as its last line, is still a reinjection.
    #[test]
    fn a_memory_body_that_contains_the_marker_is_not_a_catch_up() {
        for body in [
            "notes\nAgentMux catch-up: how this works\nmore notes".to_string(),
            "notes\nAgentMux catch-up: how this works".to_string(),
            format!("notes\nAgentMux catch-up: how this works. {CATCH_UP_TAIL}\nmore notes"),
        ] {
            let entries = vec![Entry {
                label: "notes.md".into(),
                tier: Tier::Personal,
                text: body,
                name: "notes.md".into(),
                system: false,
                bundle_id: None,
                path: None,
            }];
            let memory = compose_fallback(&entries, FallbackReason::Compaction).unwrap();
            assert!(!is_catch_up_text(&memory.text), "{}", memory.text);
            // And appending the real block still makes it one.
            assert!(is_catch_up_text(&compose_catch_up(CatchUpKind::Compaction, Some(&memory.text))));
        }
    }
}
