// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How much an agent may add or change in an hour (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md
//! R6). Adding and editing ask nobody, so this is the bound on what one misled agent can do before the person looks: a pace,
//! never a permission. A call over the budget is refused at once with the time the budget next has room, and changes nothing.
//! The budget resets by itself as the hour slides.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The window the budget is counted over.
pub const WINDOW_MS: i64 = 60 * 60 * 1000;
/// Writes of any kind in the window.
pub const WRITES: usize = 30;
/// Replacements of an existing entry's value in the window.
pub const REPLACES: usize = 6;
/// Replacements of one entry in the window.
pub const REPLACES_PER_ENTRY: usize = 5;
/// New entries in the window.
pub const NEW_ENTRIES: usize = 20;
/// "Refused: pace" history lines one agent may have written for it in the window: a refusal is a record, and an agent that
/// keeps asking over its pace must not be able to fill an entry's history with its own words.
pub const REFUSAL_LINES: usize = 3;

/// One thing an agent writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// A new entry.
    New,
    /// A new value for the entry with this id.
    Replace(String),
    /// A change to an entry's details (label, fields, tags, where its machine copy is).
    Edit,
}

#[derive(Default)]
pub struct Pace {
    log: Mutex<HashMap<String, Vec<(i64, Act)>>>,
    refusals: Mutex<HashMap<String, Vec<i64>>>,
}

/// The time `limit` leaves room again: when the `count - limit + needed`-th oldest of `times` leaves the window.
fn room_at(mut times: Vec<i64>, limit: usize, needed: usize) -> i64 {
    times.sort_unstable();
    let leaves = (times.len() + needed).saturating_sub(limit).max(1);
    times.get(leaves - 1).map_or(0, |t| t + WINDOW_MS)
}

fn when(now: i64, at: i64) -> String {
    let minutes = ((at - now).max(0) + 59_999) / 60_000;
    if minutes <= 1 {
        "in about a minute".to_string()
    } else {
        format!("in about {minutes} minutes")
    }
}

/// The refusal when one call asks for more than the pace ever allows in an hour: waiting would not help.
fn too_large(what: &str, asked: usize, most: usize) -> String {
    format!("this call changes {asked} {what} and the Vault's pace allows at most {most} in an hour, so it can never be taken as one: split it into smaller calls")
}

impl Pace {
    /// Counts `acts` against `agent`'s budget at `now` (ms) and records them, or refuses all of them, saying when there is
    /// room. A batch is `acts` with one per row.
    pub fn admit(&self, agent: &str, acts: &[Act], now: i64) -> Result<(), String> {
        let (new, replaces) = (acts.iter().filter(|a| **a == Act::New).count(), acts.iter().filter(|a| matches!(a, Act::Replace(_))).count());
        for (what, asked, most) in [("things", acts.len(), WRITES), ("new entries", new, NEW_ENTRIES), ("values", replaces, REPLACES)] {
            if asked > most {
                return Err(too_large(what, asked, most));
            }
        }
        let mut log = self.log.lock().unwrap();
        let mine = log.entry(agent.to_string()).or_default();
        mine.retain(|(at, _)| now - at < WINDOW_MS);
        let times = |keep: &dyn Fn(&Act) -> bool| mine.iter().filter(|(_, a)| keep(a)).map(|(t, _)| *t).collect::<Vec<_>>();
        let refuse = |what: String, at: i64| Err(format!("this agent has {what}; the Vault's pace gives it room again {}", when(now, at)));
        if mine.len() + acts.len() > WRITES {
            return refuse(format!("made {} changes to the Vault in the last hour (at most {WRITES})", mine.len()), room_at(times(&|_| true), WRITES, acts.len()));
        }
        if new > 0 && times(&|a| *a == Act::New).len() + new > NEW_ENTRIES {
            return refuse(format!("added {} entries in the last hour (at most {NEW_ENTRIES})", times(&|a| *a == Act::New).len()), room_at(times(&|a| *a == Act::New), NEW_ENTRIES, new));
        }
        if replaces > 0 && times(&|a| matches!(a, Act::Replace(_))).len() + replaces > REPLACES {
            return refuse(
                format!("replaced the value of {} entries in the last hour (at most {REPLACES})", times(&|a| matches!(a, Act::Replace(_))).len()),
                room_at(times(&|a| matches!(a, Act::Replace(_))), REPLACES, replaces),
            );
        }
        for id in acts.iter().filter_map(|a| if let Act::Replace(id) = a { Some(id) } else { None }) {
            let of_it = |a: &Act| matches!(a, Act::Replace(i) if i == id);
            let asked = acts.iter().filter(|a| of_it(a)).count();
            if asked > REPLACES_PER_ENTRY {
                return Err(too_large("new values of one entry", asked, REPLACES_PER_ENTRY));
            }
            if times(&of_it).len() + asked > REPLACES_PER_ENTRY {
                return refuse(format!("replaced the value of one entry {} times in the last hour (at most {REPLACES_PER_ENTRY})", times(&of_it).len()), room_at(times(&of_it), REPLACES_PER_ENTRY, asked));
            }
        }
        mine.extend(acts.iter().map(|a| (now, a.clone())));
        Ok(())
    }

    /// Gives back `acts` an agent was admitted for and did not make (the call failed before anything was stored): the latest
    /// matching entry of each is dropped.
    pub fn refund(&self, agent: &str, acts: &[Act]) {
        let mut log = self.log.lock().unwrap();
        let Some(mine) = log.get_mut(agent) else { return };
        for act in acts {
            if let Some(at) = mine.iter().rposition(|(_, a)| a == act) {
                mine.remove(at);
            }
        }
    }

    /// Whether a "refused: pace" line may be written for `agent` now, and counts it when so.
    pub fn may_note_refusal(&self, agent: &str, now: i64) -> bool {
        let mut notes = self.refusals.lock().unwrap();
        let mine = notes.entry(agent.to_string()).or_default();
        mine.retain(|at| now - at < WINDOW_MS);
        if mine.len() >= REFUSAL_LINES {
            return false;
        }
        mine.push(now);
        true
    }
}

/// The one budget every handler counts against.
pub fn pace() -> &'static Pace {
    static PACE: OnceLock<Pace> = OnceLock::new();
    PACE.get_or_init(Pace::default)
}

/// [`Pace::refund`].
pub fn refund(agent: &str, acts: &[Act]) {
    pace().refund(agent, acts);
}

/// [`Pace::may_note_refusal`] now.
pub fn may_note_refusal(agent: &str) -> bool {
    pace().may_note_refusal(agent, agentmux_common::time::now_ms())
}

/// [`Pace::admit`] now.
pub fn admit(agent: &str, acts: &[Act]) -> Result<(), String> {
    pace().admit(agent, acts, agentmux_common::time::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 1_000_000_000;

    fn replace(id: &str) -> Act {
        Act::Replace(id.to_string())
    }

    #[test]
    fn the_thirty_first_write_in_an_hour_is_refused_and_says_when_there_is_room() {
        let p = Pace::default();
        for i in 0..WRITES as i64 {
            p.admit("a", &[Act::Edit], T + i * 1000).unwrap();
        }
        let why = p.admit("a", &[Act::Edit], T + 60_000).unwrap_err();
        assert!(why.contains("30 changes") && why.contains("in about 59 minutes"), "{why}");
        // Another agent has its own budget.
        p.admit("b", &[Act::Edit], T + 60_000).unwrap();
        // The window slides: the first write leaves it an hour on.
        p.admit("a", &[Act::Edit], T + WINDOW_MS + 1).unwrap();
    }

    #[test]
    fn the_twenty_first_new_entry_and_the_seventh_replacement_are_refused() {
        let p = Pace::default();
        for _ in 0..NEW_ENTRIES {
            p.admit("a", &[Act::New], T).unwrap();
        }
        assert!(p.admit("a", &[Act::New], T + 1).unwrap_err().contains("added 20 entries"));
        let q = Pace::default();
        for i in 0..REPLACES {
            q.admit("a", &[replace(&format!("e{i}"))], T).unwrap();
        }
        assert!(q.admit("a", &[replace("e9")], T + 1).unwrap_err().contains("replaced the value of 6 entries"));
        // A new entry and an edit are not replacements.
        q.admit("a", &[Act::New, Act::Edit], T + 2).unwrap();
    }

    #[test]
    fn the_sixth_replacement_of_one_entry_is_refused_though_the_hour_has_room() {
        let p = Pace::default();
        for _ in 0..REPLACES_PER_ENTRY {
            p.admit("a", &[replace("same")], T).unwrap();
        }
        assert!(p.admit("a", &[replace("same")], T + 1).unwrap_err().contains("one entry 5 times"));
        p.admit("a", &[replace("other")], T + 1).unwrap();
    }

    #[test]
    fn a_call_larger_than_the_pace_ever_allows_is_told_to_split_not_to_wait() {
        let p = Pace::default();
        let why = p.admit("a", &vec![Act::New; NEW_ENTRIES + 1], T).unwrap_err();
        assert!(why.contains("split it") && !why.contains("room again"), "{why}");
        assert!(p.admit("a", &vec![Act::Edit; WRITES + 1], T).unwrap_err().contains("split it"));
        p.admit("a", &vec![Act::New; NEW_ENTRIES], T).unwrap();
    }

    #[test]
    fn a_refund_gives_back_what_a_failed_call_took_and_refusal_lines_are_few() {
        let p = Pace::default();
        for _ in 0..WRITES {
            p.admit("a", &[Act::Edit], T).unwrap();
        }
        assert!(p.admit("a", &[Act::Edit], T + 1).is_err());
        p.refund("a", &[Act::Edit]);
        p.admit("a", &[Act::Edit], T + 2).unwrap();
        p.refund("a", &[Act::New]);
        assert!(p.admit("a", &[Act::Edit], T + 3).is_err(), "nothing of that kind was held, so nothing came back");
        for _ in 0..REFUSAL_LINES {
            assert!(p.may_note_refusal("a", T));
        }
        assert!(!p.may_note_refusal("a", T + 1) && p.may_note_refusal("b", T + 1));
        assert!(p.may_note_refusal("a", T + WINDOW_MS + 1));
    }

    #[test]
    fn a_batch_counts_a_write_a_row_and_is_taken_whole_or_not_at_all() {
        let p = Pace::default();
        p.admit("a", &vec![Act::Edit; 25], T).unwrap();
        let rows: Vec<Act> = (0..6).map(|_| Act::New).collect();
        assert!(p.admit("a", &rows, T + 1).is_err());
        // Nothing of the refused batch was counted: five more still fit.
        p.admit("a", &rows[..5], T + 2).unwrap();
        assert!(p.admit("a", &[Act::Edit], T + 3).is_err());
    }
}
