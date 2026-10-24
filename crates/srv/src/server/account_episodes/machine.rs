// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The account episodes, as a pure state machine: a scan's blocked panes in,
//! what to publish, raise and tell out
//! (`docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §6.1, §6.2). No clock,
//! no store: `mod.rs` scans and applies.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::backend::agent_state::AgentState;

/// The provider's name for a session (five-hour) limit window. A session limit
/// raises no toast (spec §13, decision 1).
pub(crate) const SESSION_WINDOW: &str = "five_hour";

/// A pane that is `limited` or `signed_out` on an account, as one scan found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockedPane {
    pub block_id: String,
    pub agent_name: String,
    pub account_id: String,
    /// Canonical provider.
    pub provider: String,
    pub state: AgentState,
    pub resets_at_ms: Option<u64>,
    pub limit_window: Option<String>,
    /// The stored failure's class, reset and title: the same signature is the
    /// same failure.
    pub signature: String,
}

impl BlockedPane {
    /// A sign-in problem, or a usage limit that isn't a session limit (weekly,
    /// monthly, or no window known).
    fn wants_toast(&self) -> bool {
        self.state == AgentState::SignedOut || self.limit_window.as_deref() != Some(SESSION_WINDOW)
    }
}

/// Every pane blocked on one account, from the first until none is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Episode {
    pub account_id: String,
    pub provider: String,
    pub opened_at_ms: u64,
    /// Sorted by agent name, then block.
    pub panes: Vec<BlockedPane>,
    /// Its toast was raised (or is owed no more).
    pub toasted: bool,
    /// Panes whose leaders were told, or that needed no telling.
    pub announced: HashSet<String>,
}

impl Episode {
    /// `signed_out` when any pane on the account is: the operator has to act.
    pub fn state(&self) -> AgentState {
        if self.panes.iter().any(|p| p.state == AgentState::SignedOut) {
            AgentState::SignedOut
        } else {
            AgentState::Limited
        }
    }

    /// When the last of its limited panes' limits lifts; `None` while signed
    /// out, or when no reset is known.
    pub fn resets_at_ms(&self) -> Option<u64> {
        if self.state() == AgentState::SignedOut {
            return None;
        }
        self.panes.iter().filter_map(|p| p.resets_at_ms).max()
    }

    /// The window of the pane whose limit lifts last (or any pane's).
    pub fn limit_window(&self) -> Option<String> {
        let latest = self.resets_at_ms();
        self.panes
            .iter()
            .find(|p| latest.is_some() && p.resets_at_ms == latest)
            .or_else(|| self.panes.iter().find(|p| p.limit_window.is_some()))
            .and_then(|p| p.limit_window.clone())
    }
}

/// What a scan changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Opened, or its panes, state or reset changed: publish it.
    Changed(Episode),
    /// Raise its one toast.
    Toast(Episode),
    /// Tell the leaders of these newly blocked panes.
    Announce { episode: Episode, panes: Vec<BlockedPane> },
    /// No pane is blocked on the account any more: publish `ok`, take the toast down.
    Closed { account_id: String, provider: String },
}

/// The open episodes, by account.
#[derive(Debug, Default)]
pub(crate) struct Episodes {
    open: BTreeMap<String, Episode>,
    /// Failures stored before srv started (block → signature). Finding one is
    /// the episode rebuilt, not news: no toast and no notes for it.
    baseline: HashMap<String, String>,
}

impl Episodes {
    pub fn new(baseline: HashMap<String, String>) -> Self {
        Episodes { open: BTreeMap::new(), baseline }
    }

    pub fn list(&self) -> impl Iterator<Item = &Episode> {
        self.open.values()
    }

    pub fn get(&self, account_id: &str) -> Option<&Episode> {
        self.open.get(account_id)
    }

    /// Take one scan's blocked panes (every account's) at `now_ms`.
    pub fn update(&mut self, panes: Vec<BlockedPane>, now_ms: u64) -> Vec<Effect> {
        let mut by_account: BTreeMap<String, Vec<BlockedPane>> = BTreeMap::new();
        for p in panes {
            by_account.entry(p.account_id.clone()).or_default().push(p);
        }
        let mut out = Vec::new();
        let closed: Vec<String> = self.open.keys().filter(|a| !by_account.contains_key(*a)).cloned().collect();
        for account_id in closed {
            if let Some(ep) = self.open.remove(&account_id) {
                out.push(Effect::Closed { account_id, provider: ep.provider });
            }
        }
        for (account_id, mut panes) in by_account {
            panes.sort_by(|a, b| a.agent_name.to_lowercase().cmp(&b.agent_name.to_lowercase()).then(a.block_id.cmp(&b.block_id)));
            panes.dedup_by(|a, b| a.block_id == b.block_id);
            let ep = self.open.entry(account_id.clone()).or_insert_with(|| Episode {
                account_id: account_id.clone(),
                provider: panes[0].provider.clone(),
                opened_at_ms: now_ms,
                panes: Vec::new(),
                toasted: false,
                announced: HashSet::new(),
            });
            // A pane that left and came back is news again.
            ep.announced.retain(|b| panes.iter().any(|p| &p.block_id == b));
            let changed = ep.panes != panes;
            if changed {
                ep.panes = panes;
            }
            let fresh: Vec<BlockedPane> = ep.panes.iter().filter(|p| !ep.announced.contains(&p.block_id)).cloned().collect();
            let mut news = Vec::new();
            for p in fresh {
                ep.announced.insert(p.block_id.clone());
                let rebuilt = self.baseline.get(&p.block_id) == Some(&p.signature);
                self.baseline.remove(&p.block_id);
                if !rebuilt {
                    news.push(p);
                }
            }
            if changed {
                out.push(Effect::Changed(ep.clone()));
            }
            if !ep.toasted && news.iter().any(BlockedPane::wants_toast) {
                ep.toasted = true;
                out.push(Effect::Toast(ep.clone()));
            }
            if !news.is_empty() {
                out.push(Effect::Announce { episode: ep.clone(), panes: news });
            }
        }
        out
    }
}

/// "until Sun 7:00 PM", in this machine's time zone.
pub(crate) fn until_label(resets_at_ms: u64) -> Option<String> {
    let t = chrono::DateTime::from_timestamp_millis(resets_at_ms as i64)?;
    Some(format!("until {}", t.with_timezone(&chrono::Local).format("%a %-I:%M %p")))
}

/// The toast's body: how many agents, and why. Never the account (spec §6.2).
pub(crate) fn toast_body(ep: &Episode) -> String {
    let n = ep.panes.len();
    let agents = if n == 1 { "1 agent".to_string() } else { format!("{n} agents") };
    match ep.state() {
        AgentState::SignedOut => format!("{agents} {} you to sign in again", if n == 1 { "needs" } else { "need" }),
        _ => {
            let verb = if n == 1 { "is" } else { "are" };
            match ep.resets_at_ms().and_then(until_label) {
                Some(until) => format!("{agents} {verb} out of quota {until}"),
                None => format!("{agents} {verb} out of quota"),
            }
        }
    }
}

/// "#163", "#163 and #140", "#1, #2 and #3".
fn task_list(numbers: &[i64]) -> String {
    let tags: Vec<String> = numbers.iter().map(|n| format!("#{n}")).collect();
    match tags.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The body of the note a task's requester gets about `pane` (spec §6.2):
/// "Naki-2 is out of quota until Sun 7:00 PM and holds #163. …". `mod.rs`
/// quotes it into an `[AgentMux]` note.
pub(crate) fn leader_note(pane: &BlockedPane, numbers: &[i64]) -> String {
    let why = match pane.state {
        AgentState::SignedOut => "needs the user to sign in again".to_string(),
        _ => match pane.resets_at_ms.and_then(until_label) {
            Some(until) => format!("is out of quota {until}"),
            None => "is out of quota".to_string(),
        },
    };
    // Until step 2c holds messages to an unavailable agent (#1849), one sent
    // to it fails; when that lands, say they are held instead.
    format!(
        "{} {why} and holds {}. Until then, a message to it starts a turn that fails; move it to another account or wait for the reset.",
        pane.agent_name,
        task_list(numbers)
    )
}

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;
