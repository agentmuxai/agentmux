// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! A quiet agent whose lead is on another machine: the note that machine's AgentMux sends over the LAN, and what the
//! lead's machine accepts of it (`docs/specs/SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md` section 19).
//!
//! The sending side builds a [`Note`] of closed fields and queues it ([`queue`]); `srv` sends it as a jekt signed by the
//! machine's own sender key. The receiving side checks who sent it (in `srv`), parses it strictly ([`parse`]), limits
//! how many it shows ([`Limiter`]) and writes the lead's copy itself ([`render`]): nothing the sender wrote reaches the
//! lead except the closed fields, cleaned again here.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::backend::reactive::sanitize::{neutralize_markers, quote_in_system_note};

/// What a quiet note's text starts with. The text after it is one JSON object of exactly the fields of [`Note`].
pub const PREFIX: &str = "QUIET-NOTE ";
/// The most task numbers, characters of the last line and characters of a name that are kept.
const MAX_TASKS: usize = 8;
const MAX_LAST: usize = 120;
const MAX_NAME: usize = agentmux_common::slug::AGENT_ID_MAX_LEN;
const MAX_MINUTES: u64 = 100_000;
/// A lead is shown this many quiet notes an hour from one machine; the rest are folded into one line.
pub const SHOWN_PER_HOUR: usize = 4;
const HOUR_MS: u64 = 60 * 60 * 1000;
/// Notes waiting to be sent, at most (the oldest is dropped: a note is about a moment).
const OUTBOX_MAX: usize = 32;

/// The whole of a quiet note. A field that is not here is not sent, and a text with any other field is not a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// The quiet agent's name.
    pub agent: String,
    /// `idle` (its turn ended) or `running` (its turn runs and has written nothing).
    pub state: String,
    pub minutes: u64,
    /// The numbers of the tasks it owes.
    pub tasks: Vec<i64>,
    /// The end of its last line, one line, at most 120 characters, with marker text quoted. May be empty.
    pub last: String,
}

/// The agent's name as the LAN lists it (its route name, `Loap #3` is `Loap-3`), kept to the characters a name may hold.
/// The receiving machine compares it with the agents the sending machine lists, so it must be that name and not the
/// definition's display text.
fn clean_name(raw: &str) -> String {
    let kept: String = agentmux_common::slug::agent_route_name(raw.trim()).chars().filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ')).collect();
    kept.trim().chars().take(MAX_NAME).collect()
}

fn clean_last(raw: &str) -> String {
    quote_in_system_note(raw).chars().take(MAX_LAST).collect::<String>().trim().to_string()
}

impl Note {
    /// The note for `agent`, every field cleaned to what it may hold.
    pub fn new(agent: &str, running: bool, minutes: u64, tasks: &[i64], last: &str) -> Note {
        let mut nums: Vec<i64> = tasks.iter().copied().filter(|n| *n > 0).collect();
        nums.sort_unstable();
        nums.dedup();
        nums.truncate(MAX_TASKS);
        Note { agent: clean_name(agent), state: if running { "running" } else { "idle" }.to_string(), minutes: minutes.min(MAX_MINUTES), tasks: nums, last: clean_last(last) }
    }

    /// The text that is sent: the prefix and the note as JSON.
    pub fn wire(&self) -> String {
        format!("{PREFIX}{}", serde_json::to_string(self).unwrap_or_default())
    }
}

/// Whether `text` claims to be a quiet note.
pub fn is_quiet_note(text: &str) -> bool {
    text.starts_with(PREFIX)
}

/// The note in `text`, or `None` when it is not exactly one: another field, a state that is not `idle` or `running`, no
/// agent name. The fields are cleaned again, whatever the sender did.
pub fn parse(text: &str) -> Option<Note> {
    let note: Note = serde_json::from_str(text.strip_prefix(PREFIX)?).ok()?;
    if (note.state != "idle" && note.state != "running") || !note.agent.chars().any(char::is_alphanumeric) {
        return None;
    }
    let clean = Note::new(&note.agent, note.state == "running", note.minutes, &note.tasks, &note.last);
    (!clean.agent.is_empty()).then_some(clean)
}

fn tasks_words(tasks: &[i64]) -> String {
    match tasks {
        [] => String::new(),
        _ => format!(" on {}", tasks.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ")),
    }
}

/// What the lead reads for `note`, from `sender` (the machine's sender name, as proven here) at `ts_secs`: a block with
/// its own marker, the trust line this machine computed and the words this machine wrote. It is not an `[AgentMux]`
/// note: those are this machine's own.
pub fn render(sender: &str, machine: &str, ts_secs: i64, note: &Note) -> String {
    let machine = clean_name(machine);
    let state = if note.state == "running" { "Its turn is still running but has written nothing." } else { "It is idle." };
    let last = if note.last.is_empty() { String::new() } else { format!(" Its last words: \"{}\"", note.last) };
    let body = format!("Your agent {} (on {machine}) has been quiet for {} min{}. {state}{last}", note.agent, note.minutes, tasks_words(&note.tasks));
    block(sender, ts_secs, &body)
}

/// The one line that stands for the notes past the limit.
pub fn render_folded(sender: &str, machine: &str, ts_secs: i64, more: usize) -> String {
    let what = if more == 1 { "1 more quiet note".to_string() } else { format!("{more} more quiet notes") };
    block(sender, ts_secs, &format!("{what} from {} in the last hour, not shown here.", clean_name(machine)))
}

fn block(sender: &str, ts_secs: i64, body: &str) -> String {
    let from: String = sender.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    format!("[BULLETIN:FROM={from} TRUST=lan-verified INSTANCE_STATUS=approved TS={ts_secs}]\n{}\n[/BULLETIN]", neutralize_markers(body))
}

/// What the limiter says of one more note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Show it.
    Show,
    /// Past the limit: show `more` notes' worth as one line instead (all the notes beyond the shown ones in the hour).
    Fold { more: usize },
}

/// How many quiet notes each lead has had from each sending machine in the last hour.
#[derive(Default)]
pub struct Limiter {
    seen: HashMap<(String, String), Vec<u64>>,
}

impl Limiter {
    pub fn admit(&mut self, lead: &str, machine: &str, now_ms: u64) -> Admit {
        let times = self.seen.entry((lead.to_lowercase(), machine.to_lowercase())).or_default();
        times.retain(|t| now_ms.saturating_sub(*t) < HOUR_MS);
        times.push(now_ms);
        let n = times.len();
        if self.seen.len() > 256 {
            self.seen.retain(|_, v| v.last().is_some_and(|t| now_ms.saturating_sub(*t) < HOUR_MS));
        }
        if n <= SHOWN_PER_HOUR {
            Admit::Show
        } else {
            Admit::Fold { more: n - SHOWN_PER_HOUR }
        }
    }
}

/// A note waiting to be sent to the lead `lead` (`Name@machine`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub lead: String,
    pub note: Note,
}

static OUTBOX: LazyLock<Mutex<Vec<Outgoing>>> = LazyLock::new(Default::default);

/// Queue `note` for the lead `lead`. A later note about the same agent for the same lead replaces the one waiting.
pub fn queue(lead: &str, note: Note) {
    let mut outbox = OUTBOX.lock().unwrap_or_else(|e| e.into_inner());
    outbox.retain(|o| !(o.lead.eq_ignore_ascii_case(lead) && o.note.agent == note.agent));
    if outbox.len() >= OUTBOX_MAX {
        outbox.remove(0);
    }
    outbox.push(Outgoing { lead: lead.to_string(), note });
}

/// Everything waiting, oldest first.
pub fn drain() -> Vec<Outgoing> {
    std::mem::take(&mut *OUTBOX.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `Name@machine` as `(Name, machine)`.
pub fn lead_parts(lead: &str) -> Option<(&str, &str)> {
    let (name, machine) = lead.split_once('@')?;
    (!name.is_empty() && !machine.is_empty()).then_some((name, machine))
}

static DROPPED: LazyLock<Mutex<std::collections::HashSet<String>>> = LazyLock::new(Default::default);

/// True the first time it is asked for `key`: a drop is logged once, not at every quiet.
pub fn first_drop(key: &str) -> bool {
    DROPPED.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_lowercase())
}

#[cfg(test)]
#[path = "quiet_remote_tests.rs"]
mod tests;
