// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The srv backstop of an agent's compaction. For a route agent it is the model's budget
//! (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 4.2): the launch environment moves the CLI's own auto-compact
//! down to it; if the agent's context is still more than 1.5 times that budget (the variable was ignored, or the CLI's
//! compaction is off or failing), srv sends the agent `/compact` the way `Agents compact` does: once per try, between
//! turns, never mid-turn. For a container agent it is one size well under its window (`agent:compactbackstopat`, 600K
//! unless set), for an agent that does not act on the note it is given at 400K: the same rule, with the compaction placed
//! at the head of the container's queue. After two tries that left either over, the agent's pane gets a warning line
//! (block meta `agent:context_warning`, its own key: the spend watcher's 80% warning is not touched) and its leader a
//! note, once.
//!
//! A route agent is looked at off the spend watcher's per-response event, a container agent when a turn of it ends well:
//! a map lookup each.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::{json, Value};

use crate::backend::context_reading::{self, ContextReading};
use crate::backend::model_gateway::token::Grant;
use crate::backend::model_gateway::{catalog, launch};
use crate::backend::obj;
use crate::backend::worker_notes;
use crate::server::spend_watch;
use crate::server::AppState;

/// The pane's context warning: `{at, context}`. Taken off when the context is back under the limit.
pub const CONTEXT_WARNING_META_KEY: &str = "agent:context_warning";

/// Compactions sent for one stretch over the limit before the agent's pane and leader are told.
const TRIES: u8 = 2;
/// After the last try, how long the context may stay over the limit before that try counts as failed.
const SETTLE_MS: u64 = context_reading::MIN_MS_BETWEEN;

/// What srv has done about one pane's context since it passed the limit.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct Episode {
    tries: u8,
    /// When the last try was scheduled, and from the moment it was sent, when that was.
    last_try_ms: u64,
    /// The last try is scheduled and not yet known to have been sent or given up.
    pending: bool,
    told: bool,
}

impl Episode {
    /// Settles a pending try against what the reading says happened to it: sent (counted from when it went) or given up
    /// (not counted: the turn never ended, or the send failed).
    fn settle(&mut self, reading: &ContextReading) {
        if !self.pending || reading.compact_scheduled {
            return;
        }
        self.pending = false;
        match reading.last_self_compact_ms.filter(|sent| *sent >= self.last_try_ms) {
            Some(sent) => self.last_try_ms = sent,
            None => self.tries = self.tries.saturating_sub(1),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Nothing,
    Compact,
    Tell,
    Recovered,
}

/// What to do for a pane whose context is `tokens` against `limit`, the size it may not stay over.
fn decide(tokens: u64, limit: u64, ep: Option<Episode>, reading: &ContextReading, now_ms: u64) -> Step {
    if tokens <= limit {
        return if ep.is_some() { Step::Recovered } else { Step::Nothing };
    }
    if reading.compacting || reading.compact_scheduled {
        return Step::Nothing;
    }
    let ep = ep.unwrap_or_default();
    if ep.tries < TRIES {
        return Step::Compact;
    }
    if !ep.told && now_ms.saturating_sub(ep.last_try_ms) >= SETTLE_MS {
        return Step::Tell;
    }
    Step::Nothing
}

fn episodes() -> &'static Mutex<HashMap<String, Episode>> {
    static EPISODES: LazyLock<Mutex<HashMap<String, Episode>>> = LazyLock::new(Default::default);
    &EPISODES
}

/// Whose context is looked at, for the log and the note to its leader.
struct Subject<'a> {
    def_id: &'a str,
    agent: &'a str,
    /// What the context is over, in words: "over 1.5 times its compaction budget on <route>".
    over: String,
    /// The size, in tokens, the context may not stay over.
    limit: u64,
    /// What the leader can tell the agent to do: a container agent can't start a fresh session.
    remedy: &'static str,
}

/// A response of `grant`'s agent has been counted: look at how full each of its panes' contexts is.
pub(crate) fn after_settled(state: &AppState, grant: &Grant) {
    let Some(route) = catalog::route(&grant.route) else { return };
    let Some(budget) = launch::model_on_route(&state.mstore, &grant.def_id, route).and_then(|m| m.compact_budget()) else { return };
    // A pane that was closed leaves no reading: its episode goes with it.
    episodes().lock().unwrap().retain(|block_id, _| context_reading::get(block_id).is_some());
    let place = route.label.to_string();
    let subject = Subject { def_id: &grant.def_id, agent: &grant.agent, over: format!("over 1.5 times its compaction budget on {place}"), limit: u64::from(budget).saturating_mul(3) / 2, remedy: "compact or restart it (Agents compact, Agents fresh)" };
    for block_id in worker_notes::live_panes_of(&state.mstore, &grant.def_id) {
        look(state, &subject, &block_id);
    }
}

/// A container agent's turn has ended well: look at how full its context is, against `agent:compactbackstopat`. An agent
/// on a model route has the budget's backstop already, so it is left to that one.
pub(crate) fn after_container_turn(state: &AppState, block_id: &str) {
    let Some(limit) = context_reading::backstop_at() else { return };
    let Some(block) = state.mstore.get::<obj::Block>(block_id).ok().flatten() else { return };
    let def_id = obj::meta_get_string(&block.meta, "agentId", "");
    if def_id.is_empty() || launch::choice_of(&state.mstore, &def_id).is_some() {
        return;
    }
    let Some(def) = state.mstore.agent_def_get(&def_id).ok().flatten() else { return };
    let subject = Subject { def_id: &def_id, agent: &def.slug, over: format!("over {}K tokens, where AgentMux compacts a container agent", limit / 1000), limit, remedy: "have it compact (Agents compact)" };
    look(state, &subject, block_id);
}

/// One pane: settle the last try, decide what is next and do it.
fn look(state: &AppState, subject: &Subject, block_id: &str) {
    let Some(reading) = context_reading::get(block_id) else { return };
    let Some(tokens) = reading.tokens else { return };
    let now = agentmux_common::time::now_ms_u64();
    let ep = episodes().lock().unwrap().get_mut(block_id).map(|ep| {
        ep.settle(&reading);
        *ep
    });
    match decide(tokens, subject.limit, ep, &reading, now) {
        Step::Nothing => {}
        Step::Recovered => recovered(state, subject, block_id, ep.is_some_and(|e| e.told)),
        Step::Compact => compact(state, subject, block_id, tokens, now),
        Step::Tell => tell(state, subject, block_id, tokens),
    }
}

fn compact(state: &AppState, subject: &Subject, block_id: &str, tokens: u64, now: u64) {
    match crate::server::app_api::agent_context::compact_for_backstop(state, block_id) {
        Ok(()) => {
            let mut map = episodes().lock().unwrap();
            let ep = map.entry(block_id.to_string()).or_default();
            ep.tries += 1;
            ep.last_try_ms = now;
            ep.pending = true;
            tracing::info!(block_id, tokens, limit = subject.limit, try_no = ep.tries, over = %subject.over, "compaction backstop: the context is over its limit, /compact sent when the turn ends");
        }
        // Too soon after the last compaction, or one already scheduled: the next look is another try.
        Err(why) => tracing::debug!(block_id, tokens, why, "compaction backstop: not sent"),
    }
}

fn tell(state: &AppState, subject: &Subject, block_id: &str, tokens: u64) {
    if let Some(ep) = episodes().lock().unwrap().get_mut(block_id) {
        ep.told = true;
    }
    let card = json!({ "at": agentmux_common::time::now_ms(), "context": tokens });
    spend_watch::set_meta(state, block_id, CONTEXT_WARNING_META_KEY, card);
    let name = state.mstore.agent_def_get(subject.def_id).ok().flatten().map_or_else(|| subject.def_id.to_string(), |d| d.name);
    let note = format!(
        "{name}'s context is {}K tokens, {}, and two /compact attempts did not bring it down. It re-reads that context on every turn: {}.",
        tokens / 1000,
        subject.over,
        subject.remedy
    );
    tracing::warn!(agent = %subject.agent, tokens, "compaction backstop: compaction did not run, the agent's leader is told");
    worker_notes::tell_leader(&state.mstore, subject.def_id, &note);
}

/// The context is back under the limit: forget the episode, and take our warning off the pane.
fn recovered(state: &AppState, subject: &Subject, block_id: &str, told: bool) {
    episodes().lock().unwrap().remove(block_id);
    if !told {
        return;
    }
    spend_watch::set_meta(state, block_id, CONTEXT_WARNING_META_KEY, Value::Null);
    tracing::info!(agent = %subject.agent, "compaction backstop: the context is back under the limit");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading() -> ContextReading {
        ContextReading::default()
    }

    #[test]
    fn a_context_within_its_limit_is_left_alone() {
        assert_eq!(decide(450_000, 450_000, None, &reading(), 0), Step::Nothing);
        assert_eq!(decide(450_001, 450_000, None, &reading(), 0), Step::Compact);
    }

    #[test]
    fn it_tries_twice_then_tells_once_after_the_last_try_had_time() {
        let at = |tries, last_try_ms, told| Some(Episode { tries, last_try_ms, pending: false, told });
        assert_eq!(decide(480_000, 450_000, at(1, 0, false), &reading(), 1), Step::Compact);
        assert_eq!(decide(480_000, 450_000, at(2, 1_000, false), &reading(), 1_000 + SETTLE_MS - 1), Step::Nothing, "the second try still has time");
        assert_eq!(decide(480_000, 450_000, at(2, 1_000, false), &reading(), 1_000 + SETTLE_MS), Step::Tell);
        assert_eq!(decide(480_000, 450_000, at(2, 1_000, true), &reading(), 1_000 + 10 * SETTLE_MS), Step::Nothing, "told once");
    }

    #[test]
    fn nothing_is_sent_while_a_compaction_runs_or_waits_for_the_turn() {
        let (mut compacting, mut scheduled) = (reading(), reading());
        compacting.compacting = true;
        scheduled.compact_scheduled = true;
        for r in [compacting, scheduled] {
            assert_eq!(decide(900_000, 450_000, None, &r, 0), Step::Nothing);
        }
    }

    /// A container agent over `agent:compactbackstopat` goes through the route agents' decision with that size as the
    /// limit: two tries, the second only when the first did not bring the context down, then one warning, never a third.
    #[test]
    fn a_container_agent_gets_two_tries_at_the_default_size_then_one_warning() {
        let limit = context_reading::DEFAULT_BACKSTOP_AT;
        assert_eq!(limit, 600_000, "well under a 1M window, above the 400K hint");
        let mut ep = Episode::default();
        assert_eq!(decide(limit, limit, None, &reading(), 0), Step::Nothing, "at the size is not over it");
        assert_eq!(decide(limit + 1, limit, None, &reading(), 0), Step::Compact, "the first try");
        for (tries, expected) in [(1, Step::Compact), (2, Step::Nothing)] {
            ep.tries = tries;
            ep.last_try_ms = 1_000;
            assert_eq!(decide(limit + 1, limit, Some(ep), &reading(), 1_001), expected, "after {tries} tries");
        }
        assert_eq!(decide(limit + 1, limit, Some(ep), &reading(), 1_000 + SETTLE_MS), Step::Tell, "the warning, once the second try had time");
        ep.told = true;
        assert_eq!(decide(limit + 1, limit, Some(ep), &reading(), 1_000 + 50 * SETTLE_MS), Step::Nothing, "no third try, no second warning");
        assert_eq!(decide(limit - 1, limit, Some(ep), &reading(), 1_000 + 50 * SETTLE_MS), Step::Recovered);
    }

    #[test]
    fn a_context_that_came_back_under_ends_the_episode() {
        let ep = Some(Episode { tries: 2, last_try_ms: 1, pending: false, told: true });
        assert_eq!(decide(200_000, 450_000, ep, &reading(), 5), Step::Recovered);
        assert_eq!(decide(200_000, 450_000, None, &reading(), 5), Step::Nothing);
    }

    #[test]
    fn a_try_counts_from_when_it_was_sent_and_not_at_all_when_it_never_was() {
        let scheduled = Episode { tries: 1, last_try_ms: 1_000, pending: true, told: false };
        let mut waiting = scheduled;
        let mut queued = reading();
        queued.compact_scheduled = true;
        waiting.settle(&queued);
        assert_eq!(waiting, scheduled, "still waiting for the turn to end");

        let mut sent = scheduled;
        let mut r = reading();
        r.last_self_compact_ms = Some(5_000);
        sent.settle(&r);
        assert_eq!((sent.tries, sent.last_try_ms, sent.pending), (1, 5_000, false), "counted from the send");

        let mut given_up = scheduled;
        given_up.settle(&reading());
        assert_eq!((given_up.tries, given_up.pending), (0, false), "the turn never ended: no try was made");
    }
}
