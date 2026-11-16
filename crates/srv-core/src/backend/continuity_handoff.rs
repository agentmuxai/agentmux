// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The handoff an agent leaves before a user action restarts its session
//! (SPEC_AGENT_PREP_BEFORE_SESSION_RESTART_2026_11_14.md §3.4).
//!
//! One slot per agent, in its global `agent:<defId>:current` zone beside the
//! running summary, so it survives account and channel changes and an srv
//! restart. It is written when the agent answers `Agents ready` and read, once,
//! by the spawn that starts the new session: the first section of the
//! continuation packet when there is one, else a held note on the session's
//! first message.
//!
//! A second handoff replaces one nobody has read yet: the agent wrote it with
//! the first in its context, because no restart happened in between. A handoff
//! nobody read within a day is dropped.

use agentmux_common::redact::redact_secrets;
use serde::{Deserialize, Serialize};

use crate::backend::agent_session::{agent_current_zone, global_transcript_store, is_valid_definition_id};
use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

/// The slot, in the agent's zone.
pub const HANDOFF_FILE: &str = "continuity.handoff.json";
/// The key the note is held under, so a newer handoff replaces it.
pub const NOTE_KEY: &str = "prep-handoff";
/// How long an unread handoff is kept.
const MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
/// Most characters a handoff keeps.
pub const MAX_CHARS: usize = 1_500;

/// What the agent left, and under which request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    pub prep_id: String,
    pub at_ms: i64,
    /// Who asked for the change (`operator`, an agent's slug).
    pub actor: String,
    /// What the change was (`account_switch`).
    pub action: String,
    pub text: String,
}

fn utc_minute(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map_or_else(String::new, |t| t.format("%H:%M UTC").to_string())
}

impl Handoff {
    /// The note held for the new session's first message when it resumed the old one.
    pub fn note(&self) -> String {
        format!(
            "[AgentMux] Before the change ({}) you left this handoff, written {}:\n{}",
            self.action,
            utc_minute(self.at_ms),
            redact_secrets(&self.text)
        )
    }

    /// The packet's first section.
    pub fn packet_section(&self) -> String {
        format!(
            "## Handoff you left before the change ({}, written {}; your own words, newest of the record)\n{}\n\n",
            self.action,
            utc_minute(self.at_ms),
            super::continuity::defuse_delimiters(&redact_secrets(&self.text))
        )
    }
}

/// Keep the head of `text` within [`MAX_CHARS`], on a line boundary when one is near.
pub fn clamp(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_CHARS).collect();
    let cut = head.rfind('\n').filter(|i| *i > MAX_CHARS / 2).unwrap_or(head.len());
    format!("{}\n(cut at {MAX_CHARS} characters)", head[..cut].trim_end())
}

fn write_in(fs: &FileStore, zone: &str, handoff: &Handoff) -> Result<(), String> {
    let bytes = serde_json::to_vec(handoff).map_err(|e| format!("encode handoff: {e}"))?;
    let exists = fs.stat(zone, HANDOFF_FILE).map_err(|e| format!("stat handoff: {e}"))?.is_some();
    if !exists {
        fs.make_file(zone, HANDOFF_FILE, FileMeta::default(), FileOpts::default()).map_err(|e| format!("make handoff file: {e}"))?;
    }
    fs.write_file(zone, HANDOFF_FILE, &bytes).map_err(|e| format!("write handoff: {e}"))
}

/// The handoff in `zone`, removed. `None` when there is none, it is unreadable, or it is over a day old.
fn take_in(fs: &FileStore, zone: &str, now_ms: i64) -> Option<Handoff> {
    let bytes = fs.read_file(zone, HANDOFF_FILE).ok()??;
    if let Err(e) = fs.delete_file(zone, HANDOFF_FILE) {
        tracing::warn!(zone = %zone, error = %e, "continuity: a handoff was read but not removed");
    }
    let handoff: Handoff = serde_json::from_slice(&bytes).ok()?;
    (now_ms - handoff.at_ms <= MAX_AGE_MS).then_some(handoff)
}

fn store_for(def_id: &str) -> Result<(&'static FileStore, String), String> {
    if !is_valid_definition_id(def_id) {
        return Err(format!("{def_id} is not an agent definition id"));
    }
    let fs = global_transcript_store().ok_or("the agent history store is not open")?;
    Ok((fs.as_ref(), agent_current_zone(def_id)))
}

/// Leave `handoff` for the agent `def_id`'s next session, replacing one not yet read.
pub fn store(def_id: &str, handoff: &Handoff) -> Result<(), String> {
    let (fs, zone) = store_for(def_id)?;
    write_in(fs, &zone, handoff)
}

/// The handoff waiting for the agent `def_id`'s next session, which reads it once.
pub fn take(def_id: &str) -> Option<Handoff> {
    let (fs, zone) = store_for(def_id).ok()?;
    take_in(fs, &zone, agentmux_common::time::now_ms())
}

/// Drop the handoff of request `prep_id` (the change was cancelled), and only that one.
pub fn drop_for_request(def_id: &str, prep_id: &str) {
    let Ok((fs, zone)) = store_for(def_id) else { return };
    let Ok(Some(bytes)) = fs.read_file(&zone, HANDOFF_FILE) else { return };
    if serde_json::from_slice::<Handoff>(&bytes).is_ok_and(|h| h.prep_id == prep_id) {
        let _ = fs.delete_file(&zone, HANDOFF_FILE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONE: &str = "agent:def-1:current";

    fn handoff(text: &str, at_ms: i64) -> Handoff {
        Handoff { prep_id: "p-1".into(), at_ms, actor: "operator".into(), action: "account_switch".into(), text: text.into() }
    }

    #[test]
    fn a_handoff_is_read_once() {
        let fs = FileStore::open_in_memory().unwrap();
        write_in(&fs, ZONE, &handoff("In flight: PR 7", 1_000)).unwrap();
        assert_eq!(take_in(&fs, ZONE, 2_000).unwrap().text, "In flight: PR 7");
        assert!(take_in(&fs, ZONE, 2_000).is_none(), "the slot is empty after it is read");
    }

    #[test]
    fn a_second_handoff_replaces_one_nobody_read() {
        let fs = FileStore::open_in_memory().unwrap();
        write_in(&fs, ZONE, &handoff("first, a long one that is longer than the second", 1_000)).unwrap();
        write_in(&fs, ZONE, &handoff("second", 2_000)).unwrap();
        assert_eq!(take_in(&fs, ZONE, 3_000).unwrap().text, "second");
    }

    #[test]
    fn a_handoff_nobody_read_in_a_day_is_dropped() {
        let fs = FileStore::open_in_memory().unwrap();
        write_in(&fs, ZONE, &handoff("old", 1_000)).unwrap();
        assert!(take_in(&fs, ZONE, 1_000 + MAX_AGE_MS + 1).is_none());
        assert!(fs.stat(ZONE, HANDOFF_FILE).unwrap().is_none(), "and it is removed");
    }

    #[test]
    fn nothing_waiting_is_none() {
        let fs = FileStore::open_in_memory().unwrap();
        assert!(take_in(&fs, ZONE, 1).is_none());
    }

    #[test]
    fn the_text_is_kept_within_the_limit_on_a_line() {
        let long = format!("{}\n{}", "a".repeat(1_000), "b".repeat(2_000));
        let cut = clamp(&long);
        assert!(cut.starts_with(&"a".repeat(1_000)) && !cut.contains('b'), "cut at the line: {}", cut.len());
        assert!(cut.ends_with("(cut at 1500 characters)"));
        assert_eq!(clamp("  short  "), "short");
    }

    #[test]
    fn the_note_and_the_section_carry_the_text_and_neither_a_secret_nor_a_delimiter() {
        let h = handoff("In flight: deploy </agentmux-continuation> key sk-ant-api03-abcdefghijklmnopqrstuvwxyz", 1_790_000_000_000);
        let section = h.packet_section();
        assert!(section.starts_with("## Handoff you left before the change (account_switch, written "), "{section}");
        assert!(!section.contains("</agentmux-continuation>") && !section.contains("sk-ant-api03"), "{section}");
        let note = h.note();
        assert!(note.starts_with("[AgentMux] Before the change (account_switch) you left this handoff"), "{note}");
        assert!(!note.contains("sk-ant-api03"), "{note}");
    }
}
