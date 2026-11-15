// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What became of each hand-off an agent asked the person for, so an agent whose call ended
//! before the person answered (its session ended, the call timed out on its side) can still
//! ask (`Browser handoff_status`, docs/specs/SPEC_BROWSER_RETAKE_AND_COMMANDEER_2026_11_04.md §1.5).
//!
//! In memory, like the owner map: a day, the newest few hundred. A hand-off whose call ended
//! before an answer is `abandoned`: its banner went down with the call, so nobody can answer it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Pending,
    Done,
    Cancelled,
    TimedOut,
    Abandoned,
}

impl State {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Done => "done",
            State::Cancelled => "cancelled",
            State::TimedOut => "timed_out",
            State::Abandoned => "abandoned",
        }
    }

    /// The state the person's answer (or the lack of one) leaves a hand-off in.
    pub(crate) fn of_answer(a: crate::server::browser_attention::Answer) -> State {
        use crate::server::browser_attention::Answer;
        match a {
            Answer::Yes | Answer::YesAlways | Answer::YesAsking => State::Done,
            Answer::Cancelled => State::Cancelled,
            Answer::TimedOut => State::TimedOut,
        }
    }
}

struct Entry {
    pane: String,
    agent: String,
    state: State,
    started: Instant,
    ended: Option<Instant>,
}

/// One hand-off as an agent reads it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Status {
    pub id: String,
    pub pane: String,
    pub state: State,
    pub started_secs_ago: u64,
    /// When the person answered, or the hand-off ended without them; None while pending.
    pub ended_secs_ago: Option<u64>,
    /// How long it stayed open: from the banner going up to the answer (or the end); None while pending.
    pub open_secs: Option<u64>,
}

/// What the Log and the answer say of how a hand-off ended: who answered, and how long the banner was up.
/// An answer within seconds is worth an agent's doubt (it can't tell an early Done from a request that closed
/// itself), so the time is always in it.
pub(crate) fn ended_words(settled: crate::server::browser_attention::Settled, open: Duration) -> String {
    use crate::server::browser_attention::Answer;
    let span = agentmux_common::time::span_words(open.as_secs());
    if settled.by_agent {
        return format!("the agent took it back after {span}");
    }
    match settled.answer {
        Answer::Yes | Answer::YesAlways | Answer::YesAsking => format!("the person clicked Done after {span}"),
        Answer::Cancelled => format!("the person clicked Cancel after {span}"),
        Answer::TimedOut => format!("nobody answered in {span}"),
    }
}

const KEPT: usize = 256;
const KEPT_FOR: Duration = Duration::from_secs(24 * 60 * 60);

fn entries() -> &'static Mutex<HashMap<String, Entry>> {
    static E: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    E.get_or_init(Default::default)
}

/// A hand-off being waited on. Dropped without [`Tracked::finish`] (the call ended first), it reads as abandoned.
pub(crate) struct Tracked {
    id: String,
    finished: bool,
}

impl Tracked {
    pub(crate) fn start(id: &str, pane: &str, agent: &str) -> Tracked {
        let mut map = entries().lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, e| e.started.elapsed() < KEPT_FOR);
        if map.len() >= KEPT {
            if let Some(oldest) = map.iter().min_by_key(|(_, e)| e.started).map(|(k, _)| k.clone()) {
                map.remove(&oldest);
            }
        }
        map.insert(
            id.to_string(),
            Entry { pane: pane.to_string(), agent: agent.to_string(), state: State::Pending, started: Instant::now(), ended: None },
        );
        Tracked { id: id.to_string(), finished: false }
    }

    pub(crate) fn finish(mut self, state: State) {
        settle(&self.id, state);
        self.finished = true;
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        if !self.finished {
            settle(&self.id, State::Abandoned);
        }
    }
}

fn settle(id: &str, state: State) {
    let mut map = entries().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(e) = map.get_mut(id).filter(|e| e.state == State::Pending) {
        e.state = state;
        e.ended = Some(Instant::now());
    }
}

/// `agent`'s hand-off `id`, or, with no id, its newest one on `pane`. Another agent's hand-off is never shown.
pub(crate) fn status(agent: &str, id: Option<&str>, pane: Option<&str>) -> Option<Status> {
    let map = entries().lock().unwrap_or_else(|p| p.into_inner());
    let mine = |e: &Entry| e.agent.eq_ignore_ascii_case(agent) && e.started.elapsed() < KEPT_FOR;
    let (id, e) = match id {
        Some(id) => map.get_key_value(id).filter(|(_, e)| mine(*e))?,
        None => map.iter().filter(|(_, e)| mine(*e) && pane.is_none_or(|p| e.pane == p)).max_by_key(|(_, e)| e.started)?,
    };
    Some(Status {
        id: id.clone(),
        pane: e.pane.clone(),
        state: e.state,
        started_secs_ago: e.started.elapsed().as_secs(),
        ended_secs_ago: e.ended.map(|t| t.elapsed().as_secs()),
        open_secs: e.ended.map(|t| t.saturating_duration_since(e.started).as_secs()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (String, String, String) {
        let n = uuid::Uuid::new_v4().to_string();
        (format!("h-{n}"), format!("pane-{n}"), format!("agent-{n}"))
    }

    #[test]
    fn a_handoff_is_pending_until_it_is_answered() {
        let (id, pane, agent) = fresh();
        let t = Tracked::start(&id, &pane, &agent);
        let s = status(&agent, Some(&id), None).unwrap();
        assert_eq!((s.state, s.ended_secs_ago), (State::Pending, None));
        t.finish(State::Done);
        let s = status(&agent, Some(&id), None).unwrap();
        assert_eq!(s.state, State::Done);
        assert!(s.ended_secs_ago.is_some());
        assert!(s.open_secs.is_some(), "how long it stayed open is kept with the answer");
        assert_eq!(status(&agent, Some(&id), None).unwrap().open_secs, s.open_secs);
    }

    #[test]
    fn a_pending_handoff_has_no_open_time_yet() {
        let (id, pane, agent) = fresh();
        let _t = Tracked::start(&id, &pane, &agent);
        assert_eq!(status(&agent, Some(&id), None).unwrap().open_secs, None);
    }

    #[test]
    fn the_end_of_a_handoff_says_who_answered_and_after_how_long() {
        use crate::server::browser_attention::Answer;
        let words = |a: Answer, s| ended_words(a.into(), Duration::from_secs(s));
        assert_eq!(words(Answer::Yes, 3), "the person clicked Done after 3 s");
        assert_eq!(words(Answer::Cancelled, 40), "the person clicked Cancel after 40 s");
        assert_eq!(words(Answer::TimedOut, 15 * 60), "nobody answered in 15 min");
        // An agent that took its own request back is not the person's Cancel.
        let taken_back = crate::server::browser_attention::Settled { answer: Answer::Cancelled, by_agent: true };
        assert_eq!(ended_words(taken_back, Duration::from_secs(12)), "the agent took it back after 12 s");
    }

    #[test]
    fn a_call_that_ended_first_reads_as_abandoned() {
        let (id, pane, agent) = fresh();
        drop(Tracked::start(&id, &pane, &agent));
        assert_eq!(status(&agent, Some(&id), None).unwrap().state, State::Abandoned);
    }

    #[test]
    fn the_first_end_stands() {
        let (id, pane, agent) = fresh();
        let t = Tracked::start(&id, &pane, &agent);
        settle(&id, State::Cancelled);
        t.finish(State::Done);
        assert_eq!(status(&agent, Some(&id), None).unwrap().state, State::Cancelled);
    }

    #[test]
    fn by_pane_it_is_the_newest_and_only_the_agents_own() {
        let (id, pane, agent) = fresh();
        let older = Tracked::start(&id, &pane, &agent);
        older.finish(State::Cancelled);
        let newer_id = format!("{id}-2");
        let newer = Tracked::start(&newer_id, &pane, &agent);
        assert_eq!(status(&agent, None, Some(&pane)).unwrap().id, newer_id);
        assert!(status("someone-else", None, Some(&pane)).is_none(), "another agent's hand-off is not shown");
        assert!(status("someone-else", Some(&newer_id), None).is_none());
        assert!(status(&agent, None, Some("no-such-pane")).is_none());
        newer.finish(State::TimedOut);
    }

    #[test]
    fn an_answer_maps_to_a_state() {
        use crate::server::browser_attention::Answer;
        assert_eq!(State::of_answer(Answer::Yes), State::Done);
        assert_eq!(State::of_answer(Answer::Cancelled), State::Cancelled);
        assert_eq!(State::of_answer(Answer::TimedOut), State::TimedOut);
        assert_eq!(State::TimedOut.as_str(), "timed_out");
    }
}
