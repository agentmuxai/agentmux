// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One note to a leader about every agent of its that needs a look
//! (docs/specs/SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §7).
//!
//! A note about a watched agent (quiet, failed, exited, waiting on a question) is queued under its leader's pane and
//! the agent. The first note into an empty window opens it; whatever else arrives for that leader within
//! `WINDOW_MS` joins it (a later note about the same agent replaces its line), and the window leaves as one note.
//! A note that must be heard now (no wait key: a failure, an exit, a question) goes out at once when the leader's
//! window is empty, so a lone failure is never delayed. A window of one line is the note it always was.
//!
//! The decision (`Outbox`) is pure and takes the time as an argument, so it is tested with no clock and no
//! runtime; `queue` and `flush_due` hand what is due to `worker_notes::deliver`, and `worker_notes::install`'s tick
//! calls `flush_due`.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::backend::storage::store::Store;
use crate::backend::wconfig::ConfigState;

/// How long notes to one leader are gathered after the first.
pub(crate) const WINDOW_MS: u64 = 20_000;

/// One agent's line in a note to a leader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    /// Who it is about (an agent definition id): a second line about it replaces the first.
    pub agent: String,
    /// What the leader reads, already quoted for a system note, with no `[AgentMux]` prefix.
    pub text: String,
    /// `Some` when the line asks nothing and may wait for a cold leader's next turn (the key it waits under);
    /// `None` when the leader must hear at once.
    pub held_key: Option<String>,
}

/// A note ready to be delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Out {
    pub leader: String,
    pub text: String,
    pub wait_key: Option<String>,
}

#[derive(Debug)]
struct Window {
    opened_ms: u64,
    lines: Vec<Line>,
}

/// The windows now open, by the leader's pane.
#[derive(Debug, Default)]
pub(crate) struct Outbox {
    windows: HashMap<String, Window>,
}

impl Outbox {
    /// `line` for `leader` at `now_ms`: the note to send at once, or `None` when it waits in the window (opened
    /// here when there was none and the line can wait).
    pub fn push(&mut self, leader: &str, line: Line, now_ms: u64) -> Option<Out> {
        match self.windows.get_mut(leader) {
            Some(w) => {
                match w.lines.iter_mut().find(|l| l.agent == line.agent) {
                    Some(have) => *have = line,
                    None => w.lines.push(line),
                }
                None
            }
            None if line.held_key.is_none() => Some(compose(leader, now_ms, &[line])),
            None => {
                self.windows.insert(leader.to_string(), Window { opened_ms: now_ms, lines: vec![line] });
                None
            }
        }
    }

    /// The notes whose window has run its time at `now_ms`; their windows close.
    pub fn take_due(&mut self, now_ms: u64) -> Vec<Out> {
        let due: Vec<String> = self.windows.iter().filter(|(_, w)| now_ms.saturating_sub(w.opened_ms) >= WINDOW_MS).map(|(k, _)| k.clone()).collect();
        let mut out: Vec<Out> = due.into_iter().filter_map(|leader| self.windows.remove(&leader).map(|w| compose(&leader, w.opened_ms, &w.lines))).collect();
        out.sort_by(|a, b| a.leader.cmp(&b.leader));
        out
    }

    /// Whether any window is open.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

/// The note for `lines` (at least one) to `leader`, whose window opened at `opened_ms`. One line is today's note
/// unchanged; several are a heading and a line each. It waits for a cold leader's next turn only when every line
/// can, under a key of its own, so a later note never replaces the lines of an earlier one.
pub(crate) fn compose(leader: &str, opened_ms: u64, lines: &[Line]) -> Out {
    if let [only] = lines {
        return Out { leader: leader.to_string(), text: format!("[AgentMux] {}", only.text), wait_key: only.held_key.clone() };
    }
    let mut text = format!("[AgentMux] {} of your agents need a look:", lines.len());
    for l in lines {
        text.push_str("\n- ");
        text.push_str(&l.text);
    }
    let wait_key = lines.iter().all(|l| l.held_key.is_some()).then(|| format!("group:{leader}:{opened_ms}"));
    Out { leader: leader.to_string(), text, wait_key }
}

static OUTBOX: LazyLock<parking_lot::Mutex<Outbox>> = LazyLock::new(Default::default);

/// Queue `line` for each pane of `leaders` and deliver what is due at once.
pub(crate) fn queue(mstore: &Store, config: &ConfigState, leaders: &[String], line: &Line, now_ms: u64) {
    let now_out: Vec<Out> = {
        let mut outbox = OUTBOX.lock();
        leaders.iter().filter_map(|leader| outbox.push(leader, line.clone(), now_ms)).collect()
    };
    send(mstore, config, now_out);
}

/// Deliver the notes whose window is over. Called by a tick.
pub(crate) fn flush_due(mstore: &Store, config: &ConfigState, now_ms: u64) {
    let due = {
        let mut outbox = OUTBOX.lock();
        if outbox.is_empty() {
            return;
        }
        outbox.take_due(now_ms)
    };
    send(mstore, config, due);
}

fn send(mstore: &Store, config: &ConfigState, notes: Vec<Out>) {
    for n in notes {
        super::worker_notes::deliver(mstore, config, &n.leader, &n.text, n.wait_key.as_deref());
    }
}

#[cfg(test)]
#[path = "note_outbox_tests.rs"]
mod tests;
