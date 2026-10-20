// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's informational `[AgentMux]` notes wait for the agent's next turn
//! instead of starting one. A note sent as input is a turn of its own: when
//! MuxBus and the model API went down together, every agent spent minutes
//! retrying a turn for a note that asked nothing of it, and the user's own
//! messages queued behind those turns.
//!
//! A held note goes with the next message the agent's pane is given by
//! anything else (the user's input, a jekt, a broadcast, a scheduled prompt),
//! after that message's own text ([`attach`], taken by
//! `blockcontroller::deliver_agent_message_from` and
//! `agent_handlers::run_agent_turn_joining`). A pane holds one note per key,
//! a newer one replacing the older, and at most [`MAX_PER_PANE`]. A terminal
//! agent typed into by keystrokes is never given held notes.
//!
//! A note the agent must get now (a worker's report to its leader, the answer
//! to its own request) goes through `Handler::deliver_system_note` instead.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

use super::handler::{Handler, ReactiveHandler, TargetResolution};

/// Notes one pane holds at most; the oldest goes first.
pub const MAX_PER_PANE: usize = 8;
/// Panes with held notes at most; the one held for longest goes first (its
/// pane is most likely closed).
pub const MAX_PANES: usize = 512;

/// Between a message and the notes that go with it.
const SEPARATOR: &str = "\n\n";

/// Held notes, per pane (block id): `(key, text)`, oldest first.
#[derive(Debug, Default)]
pub struct HeldNotes {
    panes: HashMap<String, Pane>,
    clock: u64,
}

#[derive(Debug, Default)]
struct Pane {
    notes: Vec<(String, String)>,
    touched: u64,
}

impl HeldNotes {
    fn pane(&mut self, block_id: &str) -> &mut Pane {
        self.clock += 1;
        if !self.panes.contains_key(block_id) && self.panes.len() >= MAX_PANES {
            if let Some(oldest) = self.panes.iter().min_by_key(|(_, p)| p.touched).map(|(b, _)| b.clone()) {
                self.panes.remove(&oldest);
            }
        }
        let pane = self.panes.entry(block_id.to_string()).or_default();
        pane.touched = self.clock;
        pane
    }

    /// Hold `text` for `block_id` under `key`, replacing a note held under it.
    pub fn hold(&mut self, block_id: &str, key: &str, text: &str) {
        let pane = self.pane(block_id);
        pane.notes.retain(|(k, _)| k != key);
        pane.notes.push((key.to_string(), text.to_string()));
        if pane.notes.len() > MAX_PER_PANE {
            pane.notes.remove(0);
        }
    }

    /// Hold `text` under `key` in place of the note held under `old_key`.
    /// `false`, holding nothing, when no note is held under `old_key`.
    pub fn replace(&mut self, block_id: &str, old_key: &str, key: &str, text: &str) -> bool {
        let Some(pane) = self.panes.get_mut(block_id) else { return false };
        let before = pane.notes.len();
        pane.notes.retain(|(k, _)| k != old_key);
        if pane.notes.len() == before {
            return false;
        }
        self.hold(block_id, key, text);
        true
    }

    /// Whether a note is held for `block_id` under `key`.
    pub fn holds(&self, block_id: &str, key: &str) -> bool {
        self.panes.get(block_id).is_some_and(|p| p.notes.iter().any(|(k, _)| k == key))
    }

    /// Every note held for `block_id`, oldest first; none are held after.
    pub fn take(&mut self, block_id: &str) -> Vec<(String, String)> {
        self.panes.remove(block_id).map(|p| p.notes).unwrap_or_default()
    }

    /// `notes`, taken for a message that didn't go, are held again ahead of
    /// any held since, except where a newer note took their key.
    pub fn put_back(&mut self, block_id: &str, notes: Vec<(String, String)>) {
        if notes.is_empty() {
            return;
        }
        let pane = self.pane(block_id);
        let mut all: Vec<(String, String)> =
            notes.into_iter().filter(|(k, _)| !pane.notes.iter().any(|(newer, _)| newer == k)).collect();
        all.append(&mut pane.notes);
        let excess = all.len().saturating_sub(MAX_PER_PANE);
        pane.notes = all.split_off(excess);
    }
}

static HELD: LazyLock<Mutex<HeldNotes>> = LazyLock::new(Default::default);

fn held() -> MutexGuard<'static, HeldNotes> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Hold `text` for the pane `block_id` under `key`, replacing a note held
/// under it. For a caller that already knows the pane; an agent's name or
/// UID goes through [`ReactiveHandler::hold_system_note`].
pub fn hold(block_id: &str, key: &str, text: &str) {
    held().hold(block_id, key, text);
}

/// `message`, then the notes held for its pane, one per line.
pub fn with_notes(message: &str, notes: &[(String, String)]) -> String {
    if notes.is_empty() {
        return message.to_string();
    }
    let notes = notes.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join("\n");
    if message.trim().is_empty() { notes } else { format!("{message}{SEPARATOR}{notes}") }
}

/// What a person wrote in a message that may carry held notes: the text
/// before notes srv attached to it ([`AttachedLog`]). Text that only looks
/// like attached notes, a pasted note say, is kept.
pub fn strip_attached(text: &str) -> &str {
    let mut log = ATTACHED.lock().unwrap_or_else(|e| e.into_inner());
    strip_logged(text, &mut log)
}

fn strip_logged<'a>(text: &'a str, log: &mut AttachedLog) -> &'a str {
    let start = format!("{SEPARATOR}{} ", crate::muxbus::delivery_status::notes::NOTE_MARKER);
    text.match_indices(&start)
        .map(|(at, _)| at)
        .find(|&at| log.contains(&text[at..]))
        .map_or(text, |at| &text[..at])
}

/// The exact text srv appended to messages that went (the separator and the
/// notes), as hashes, so input history takes off exactly that and never
/// what a person wrote ([`strip_attached`]). Kept in the channel folder, one
/// hash a line: history is read from past conversations, across restarts.
#[derive(Debug, Default)]
struct AttachedLog {
    path: Option<PathBuf>,
    /// Read from `path` on first use.
    hashes: Option<(VecDeque<String>, HashSet<String>)>,
}

/// The file [`AttachedLog`] is kept in.
const ATTACHED_FILE: &str = "held-notes-attached.txt";
/// Hashes [`AttachedLog`] keeps; the file is cut back to this many once it
/// holds twice as many.
const ATTACHED_KEPT: usize = 2000;

fn suffix_hash(suffix: &str) -> String {
    use sha2::Digest;
    hex::encode(&sha2::Sha256::digest(suffix.as_bytes())[..16])
}

impl AttachedLog {
    fn loaded(&mut self) -> &mut (VecDeque<String>, HashSet<String>) {
        let path = self.path.clone();
        self.hashes.get_or_insert_with(|| {
            let lines: Vec<String> = path
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|t| t.lines().map(str::to_string).collect())
                .unwrap_or_default();
            let order: VecDeque<String> = lines.into_iter().rev().take(ATTACHED_KEPT).rev().collect();
            let set = order.iter().cloned().collect();
            (order, set)
        })
    }

    fn contains(&mut self, suffix: &str) -> bool {
        self.loaded().1.contains(&suffix_hash(suffix))
    }

    fn record(&mut self, suffix: &str) {
        let hash = suffix_hash(suffix);
        let (order, set) = self.loaded();
        if !set.insert(hash.clone()) {
            return;
        }
        order.push_back(hash.clone());
        let mut rewrite = false;
        if order.len() > ATTACHED_KEPT {
            if let Some(old) = order.pop_front() {
                set.remove(&old);
            }
            rewrite = true;
        }
        let Some(path) = &self.path else { return };
        let written = if rewrite && std::fs::metadata(path).is_ok_and(|m| m.len() > (ATTACHED_KEPT as u64) * 2 * 33) {
            let all: String = self.hashes.iter().flat_map(|(o, _)| o.iter()).map(|h| format!("{h}\n")).collect();
            std::fs::write(path, all)
        } else {
            std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| writeln!(f, "{hash}"))
        };
        if let Err(e) = written {
            tracing::warn!(path = %path.display(), error = %e, "held notes: couldn't record attached notes");
        }
    }
}

static ATTACHED: LazyLock<Mutex<AttachedLog>> = LazyLock::new(|| {
    let path = if cfg!(test) {
        None
    } else {
        agentmux_common::DataPaths::from_env().map(|p| p.instance_dir.join(ATTACHED_FILE))
    };
    Mutex::new(AttachedLog { path, hashes: None })
});

/// The notes taken for one message: held again when this is dropped, unless
/// the message went ([`Attached::delivered`]).
#[must_use = "call `delivered` once the message has gone, or the notes are held again"]
pub struct Attached {
    block_id: String,
    notes: Vec<(String, String)>,
    /// What was appended to the message's own text, if it had any.
    suffix: Option<String>,
}

impl Attached {
    /// The message carrying the notes went to the agent.
    pub fn delivered(mut self) {
        self.notes.clear();
        if let Some(suffix) = self.suffix.take() {
            ATTACHED.lock().unwrap_or_else(|e| e.into_inner()).record(&suffix);
        }
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        if !self.notes.is_empty() {
            held().put_back(&self.block_id, std::mem::take(&mut self.notes));
        }
    }
}

/// `message` with the notes held for `block_id` after it, and the guard that
/// holds them again if the message doesn't go.
pub fn attach(block_id: &str, message: &str) -> (String, Attached) {
    let notes = held().take(block_id);
    let full = with_notes(message, &notes);
    let suffix = (!notes.is_empty() && !message.trim().is_empty()).then(|| full[message.len()..].to_string());
    (full, Attached { block_id: block_id.to_string(), notes, suffix })
}

impl Handler {
    /// The block a system note for `target_agent` goes to.
    pub(super) fn note_target(&mut self, target_agent: &str) -> Result<String, String> {
        match self.resolve_target(target_agent) {
            TargetResolution::Uid(b) | TargetResolution::Name(b) => Ok(b),
            TargetResolution::Ambiguous(candidates) => {
                Err(format!("ambiguous agent name: '{target_agent}' is held by {} live agents", candidates.len()))
            }
            TargetResolution::NotFound => Err(format!("agent not found: {target_agent}")),
        }
    }

    fn audit_note(&mut self, target_agent: &str, block_id: &str, text: &str, outcome: &str) {
        let request_id = uuid::Uuid::new_v4().to_string();
        self.log_audit(Some("agentmux"), target_agent, block_id, text, true, None, &request_id, Some(outcome), None);
    }
}

impl ReactiveHandler {
    /// Hold srv's own note `text` for `target_agent`'s next turn, under `key`
    /// (a note held under it is replaced). Never input of its own: see the
    /// module doc. Audited as `system_note_held` from `agentmux`.
    pub fn hold_system_note(&self, target_agent: &str, key: &str, text: &str) -> Result<(), String> {
        let mut handler = self.inner.lock().unwrap();
        let block_id = handler.note_target(target_agent)?;
        held().hold(&block_id, key, text);
        handler.audit_note(target_agent, &block_id, text, "system_note_held");
        Ok(())
    }

    /// [`Self::hold_system_note`] in place of the note held under `old_key`;
    /// `Ok(false)`, holding nothing, when none is held under it any more.
    pub fn replace_held_system_note(&self, target_agent: &str, old_key: &str, key: &str, text: &str) -> Result<bool, String> {
        let mut handler = self.inner.lock().unwrap();
        let block_id = handler.note_target(target_agent)?;
        let replaced = held().replace(&block_id, old_key, key, text);
        if replaced {
            handler.audit_note(target_agent, &block_id, text, "system_note_held");
        }
        Ok(replaced)
    }

    /// Audit a note `block_id` was given without this handler
    /// (`blockcontroller::system_note_into_turn_or_hold`): as `system_note`
    /// when it joined the running turn, else as `system_note_held`.
    pub fn audit_system_note(&self, target_agent: &str, block_id: &str, text: &str, joined: bool) {
        let outcome = if joined { "system_note" } else { "system_note_held" };
        self.inner.lock().unwrap().audit_note(target_agent, block_id, text, outcome);
    }

    /// Whether a note is held for `target_agent` under `key`.
    pub fn holds_system_note(&self, target_agent: &str, key: &str) -> bool {
        let block_id = self.inner.lock().unwrap().note_target(target_agent);
        block_id.is_ok_and(|b| held().holds(&b, key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::reactive::types::SenderDelivery;
    use std::sync::Arc;

    fn texts(notes: &[(String, String)]) -> Vec<&str> {
        notes.iter().map(|(_, t)| t.as_str()).collect()
    }

    #[test]
    fn a_note_is_held_once_per_key_the_newer_replacing_the_older() {
        let mut h = HeldNotes::default();
        h.hold("b", "paused", "[AgentMux] paused since 03:12");
        h.hold("b", "other", "[AgentMux] other");
        h.hold("b", "paused", "[AgentMux] paused since 03:12");
        assert_eq!(texts(&h.take("b")), ["[AgentMux] other", "[AgentMux] paused since 03:12"], "no duplicate");
        assert!(h.take("b").is_empty(), "taken once");
    }

    #[test]
    fn a_replacement_needs_the_note_it_replaces_still_held() {
        let mut h = HeldNotes::default();
        assert!(!h.replace("b", "paused", "outage", "x"));
        assert!(h.take("b").is_empty(), "nothing held in its place");
        h.hold("b", "paused", "[AgentMux] paused");
        assert!(h.holds("b", "paused"));
        assert!(h.replace("b", "paused", "outage", "[AgentMux] were paused"));
        assert!(!h.holds("b", "paused"));
        assert_eq!(texts(&h.take("b")), ["[AgentMux] were paused"]);
    }

    #[test]
    fn the_queue_is_bounded_per_pane_and_in_panes() {
        let mut h = HeldNotes::default();
        for i in 0..MAX_PER_PANE + 3 {
            h.hold("b", &format!("k{i}"), &format!("n{i}"));
        }
        let notes = h.take("b");
        assert_eq!(notes.len(), MAX_PER_PANE);
        assert_eq!(notes[0].1, "n3", "the oldest went first");
        for i in 0..MAX_PANES + 1 {
            h.hold(&format!("pane{i}"), "k", "n");
        }
        assert_eq!(h.panes.len(), MAX_PANES);
        assert!(!h.holds("pane0", "k"), "the pane held for longest went");
        assert!(h.holds(&format!("pane{MAX_PANES}"), "k"));
    }

    #[test]
    fn notes_put_back_go_ahead_of_newer_ones_unless_a_newer_one_took_their_key() {
        let mut h = HeldNotes::default();
        h.hold("b", "a", "old a");
        h.hold("b", "c", "c");
        let taken = h.take("b");
        h.hold("b", "a", "new a");
        h.put_back("b", taken);
        assert_eq!(texts(&h.take("b")), ["c", "new a"]);
    }

    #[test]
    fn held_notes_follow_the_message_and_never_lead_it() {
        let notes = vec![("k".to_string(), "[AgentMux] one".to_string()), ("j".to_string(), "[AgentMux] two".to_string())];
        let message = with_notes("[JEKT:FROM=a TIER=coord]\nreview PR 12", &notes);
        assert_eq!(message, "[JEKT:FROM=a TIER=coord]\nreview PR 12\n\n[AgentMux] one\n[AgentMux] two");
        assert_eq!(with_notes("fix the build", &[]), "fix the build");
        assert_eq!(with_notes("", &notes[..1]), "[AgentMux] one");
    }

    /// Input history takes off exactly the notes srv attached to a message
    /// that went, never `[AgentMux]` text a person typed or pasted.
    #[test]
    fn only_notes_srv_attached_are_taken_off_what_a_person_typed() {
        let block = "held-notes-test-strip";
        let note = "[AgentMux] Cloud messages are paused since 03:12 (held-notes-test-strip).";
        let pasted = format!("explain this:\n\n{note}");
        assert_eq!(strip_attached(&pasted), pasted, "nothing attached yet: all of it is the person's");
        held().hold(block, "k", note);
        let (not_sent, notes) = attach(block, "explain this:");
        drop(notes);
        assert_eq!(not_sent, pasted);
        assert_eq!(strip_attached(&pasted), pasted, "that message didn't go");
        let (sent, notes) = attach(block, "fix the build");
        notes.delivered();
        assert_eq!(strip_attached(&sent), "fix the build");
        let both = format!("{pasted}\n\n{note}");
        assert_eq!(strip_attached(&both), pasted, "the pasted copy stays, the attached one goes");
        assert_eq!(strip_attached("fix the build"), "fix the build");
    }

    #[test]
    fn what_was_attached_is_remembered_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ATTACHED_FILE);
        let mut log = AttachedLog { path: Some(path.clone()), hashes: None };
        log.record("\n\n[AgentMux] one");
        log.record("\n\n[AgentMux] one");
        let mut again = AttachedLog { path: Some(path.clone()), hashes: None };
        assert!(again.contains("\n\n[AgentMux] one"));
        assert!(!again.contains("\n\n[AgentMux] two"));
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1, "once");
        assert_eq!(strip_logged("hi\n\n[AgentMux] one", &mut again), "hi");
        for i in 0..ATTACHED_KEPT * 2 + 1 {
            again.record(&format!("\n\n[AgentMux] n{i}"));
        }
        let lines = std::fs::read_to_string(&path).unwrap().lines().count();
        assert!(lines <= ATTACHED_KEPT * 2 + 1, "the file is cut back: {lines}");
        let mut reread = AttachedLog { path: Some(path), hashes: None };
        assert!(reread.contains(&format!("\n\n[AgentMux] n{}", ATTACHED_KEPT * 2)));
        assert!(!reread.contains("\n\n[AgentMux] one"), "the oldest went");
    }

    #[test]
    fn notes_attached_to_a_message_that_did_not_go_are_held_again() {
        let block = "held-notes-test-attach";
        held().hold(block, "k", "[AgentMux] note");
        let (message, notes) = attach(block, "hi");
        assert_eq!(message, "hi\n\n[AgentMux] note");
        assert!(!held().holds(block, "k"), "taken while the message is on its way");
        drop(notes);
        assert!(held().holds(block, "k"), "held again: the message didn't go");
        let (_, notes) = attach(block, "hi again");
        notes.delivered();
        assert!(held().take(block).is_empty(), "gone with the message");
        let (message, notes) = attach(block, "nothing held");
        assert_eq!(message, "nothing held");
        notes.delivered();
    }

    /// A held note is never input: no structured message and no keystrokes
    /// reach the agent, so it starts no turn. It goes with the next message.
    #[test]
    fn holding_a_note_sends_the_agent_nothing() {
        let sent = Arc::new(Mutex::new(0usize));
        let handler = ReactiveHandler::new();
        let (s1, s2) = (sent.clone(), sent.clone());
        handler.set_message_sender(Arc::new(move |_: &str, _: &str, _: &[serde_json::Value]| {
            *s1.lock().unwrap() += 1;
            Ok(SenderDelivery::Delivered)
        }));
        handler.set_input_sender(Arc::new(move |_: &str, _: &[u8]| {
            *s2.lock().unwrap() += 1;
            Ok(())
        }));
        let block = "held-notes-test-handler";
        handler.register_agent("noteagent", block, None).unwrap();
        let note = "[AgentMux] Cloud messages are paused since 14:22 (MuxBus needs a sign-in).";
        handler.hold_system_note("NoteAgent", "muxbus:paused", note).unwrap();
        assert_eq!(*sent.lock().unwrap(), 0, "nothing was sent");
        assert!(handler.holds_system_note("noteagent", "muxbus:paused"));
        assert_eq!(handler.get_audit_log(1)[0].outcome.as_deref(), Some("system_note_held"));
        assert!(handler.hold_system_note("nobody", "k", note).unwrap_err().contains("agent not found"));
        assert_eq!(handler.replace_held_system_note("noteagent", "muxbus:paused", "muxbus:outage", "[AgentMux] were"), Ok(true));
        assert_eq!(handler.replace_held_system_note("noteagent", "muxbus:paused", "muxbus:outage", "[AgentMux] x"), Ok(false));
        let (message, notes) = attach(block, "what's next?");
        notes.delivered();
        assert_eq!(message, "what's next?\n\n[AgentMux] were");
    }
}
