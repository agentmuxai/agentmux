// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The srv backstop of a route agent's compaction budget (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 4.2).
//! The launch environment moves the CLI's own auto-compact down to the model's budget; if an agent's context is still
//! more than 1.5 times that budget (the variable was ignored, or the CLI's compaction is off or failing), srv sends the
//! agent `/compact` the way `Agents compact` does: once per try, between turns, never mid-turn. After two tries that
//! left it over, the agent's pane gets a warning line and its leader a note, once.
//!
//! It runs off the spend watcher's per-response event, so it costs a map lookup per response of a route agent.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::{json, Value};

use crate::backend::context_reading::{self, ContextReading};
use crate::backend::model_gateway::token::Grant;
use crate::backend::model_gateway::{catalog, launch};
use crate::backend::{obj, worker_notes};
use crate::server::spend_watch;
use crate::server::AppState;

/// Compactions sent for one stretch over the limit before the agent's pane and leader are told.
const TRIES: u8 = 2;
/// After the last try, how long the context may stay over the limit before that try counts as failed.
const SETTLE_MS: u64 = context_reading::MIN_MS_BETWEEN;

/// What srv has done about one pane's context since it passed the limit.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct Episode {
    tries: u8,
    last_try_ms: u64,
    told: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Nothing,
    Compact,
    Tell,
    Recovered,
}

/// What to do for a pane whose context is `tokens` against `budget`.
fn decide(tokens: u64, budget: u64, ep: Option<Episode>, reading: &ContextReading, now_ms: u64) -> Step {
    if tokens.saturating_mul(2) <= budget.saturating_mul(3) {
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

/// A response of `grant`'s agent has been counted: look at how full each of its panes' contexts is.
pub(crate) fn after_settled(state: &AppState, grant: &Grant) {
    let Some(route) = catalog::route(&grant.route) else { return };
    let Some(budget) = launch::model_on_route(&state.mstore, &grant.def_id, route).and_then(|m| m.compact_budget()) else { return };
    for block_id in worker_notes::live_panes_of(&state.mstore, &grant.def_id) {
        let Some(reading) = context_reading::get(&block_id) else { continue };
        let Some(tokens) = reading.tokens else { continue };
        let now = agentmux_common::time::now_ms_u64();
        let ep = episodes().lock().unwrap().get(&block_id).copied();
        match decide(tokens, u64::from(budget), ep, &reading, now) {
            Step::Nothing => {}
            Step::Recovered => recovered(state, grant, &block_id, ep.is_some_and(|e| e.told)),
            Step::Compact => compact(state, &block_id, tokens, budget, now),
            Step::Tell => tell(state, grant, &block_id, tokens),
        }
    }
}

fn compact(state: &AppState, block_id: &str, tokens: u64, budget: u32, now: u64) {
    match crate::server::app_api::agent_context::compact_for_backstop(state, block_id) {
        Ok(()) => {
            let mut map = episodes().lock().unwrap();
            let ep = map.entry(block_id.to_string()).or_default();
            ep.tries += 1;
            ep.last_try_ms = now;
            tracing::info!(block_id, tokens, budget, try_no = ep.tries, "compaction backstop: the context is over 1.5 times the budget, /compact sent when the turn ends");
        }
        // Too soon after the last compaction, or one already scheduled: the next response looks again.
        Err(why) => tracing::debug!(block_id, tokens, why, "compaction backstop: not sent"),
    }
}

fn tell(state: &AppState, grant: &Grant, block_id: &str, tokens: u64) {
    if let Some(ep) = episodes().lock().unwrap().get_mut(block_id) {
        ep.told = true;
    }
    let (_, cap, spent) = crate::backend::model_gateway::cap::agent_level(&state.mstore, grant, agentmux_common::time::now_ms());
    let place = catalog::route(&grant.route).map_or_else(|| grant.route.clone(), |r| r.label.to_string());
    let card = json!({ "at": agentmux_common::time::now_ms(), "scope": "agent", "cap": cap.unwrap_or(0.0), "spent": spent, "route": grant.route, "place": place, "context": tokens });
    spend_watch::set_meta(state, block_id, spend_watch::WARNING_META_KEY, card);
    let name = state.mstore.agent_def_get(&grant.def_id).ok().flatten().map_or_else(|| grant.def_id.clone(), |d| d.name);
    let note = format!(
        "{name}'s context is {}K tokens, over 1.5 times its compaction budget on {place}, and two /compact attempts did not bring it down. It re-reads that context on every turn: compact or restart it (Agents compact, Agents fresh).",
        tokens / 1000
    );
    tracing::warn!(agent = %grant.agent, tokens, "compaction backstop: compaction did not run, the agent's leader is told");
    worker_notes::tell_leader(&state.mstore, &grant.def_id, &note);
}

/// The context is back under the limit: forget the episode, and take our warning off the pane.
fn recovered(state: &AppState, grant: &Grant, block_id: &str, told: bool) {
    episodes().lock().unwrap().remove(block_id);
    if !told {
        return;
    }
    let ours = state.mstore.get::<obj::Block>(block_id).ok().flatten().is_some_and(|b| is_context_card(b.meta.get(spend_watch::WARNING_META_KEY)));
    if ours {
        spend_watch::set_meta(state, block_id, spend_watch::WARNING_META_KEY, Value::Null);
    }
    tracing::info!(agent = %grant.agent, "compaction backstop: the context is back under the limit");
}

/// Whether a pane's warning card is the context one, not the 80% spend one.
fn is_context_card(card: Option<&Value>) -> bool {
    card.and_then(|c| c.get("context")).is_some_and(Value::is_number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading() -> ContextReading {
        ContextReading::default()
    }

    #[test]
    fn a_context_within_one_and_a_half_times_the_budget_is_left_alone() {
        assert_eq!(decide(450_000, 300_000, None, &reading(), 0), Step::Nothing);
        assert_eq!(decide(450_001, 300_000, None, &reading(), 0), Step::Compact);
    }

    #[test]
    fn it_tries_twice_then_tells_once_after_the_last_try_had_time() {
        let at = |tries, last_try_ms, told| Some(Episode { tries, last_try_ms, told });
        assert_eq!(decide(480_000, 300_000, at(1, 0, false), &reading(), 1), Step::Compact);
        assert_eq!(decide(480_000, 300_000, at(2, 1_000, false), &reading(), 1_000 + SETTLE_MS - 1), Step::Nothing, "the second try still has time");
        assert_eq!(decide(480_000, 300_000, at(2, 1_000, false), &reading(), 1_000 + SETTLE_MS), Step::Tell);
        assert_eq!(decide(480_000, 300_000, at(2, 1_000, true), &reading(), 1_000 + 10 * SETTLE_MS), Step::Nothing, "told once");
    }

    #[test]
    fn nothing_is_sent_while_a_compaction_runs_or_waits_for_the_turn() {
        let (mut compacting, mut scheduled) = (reading(), reading());
        compacting.compacting = true;
        scheduled.compact_scheduled = true;
        for r in [compacting, scheduled] {
            assert_eq!(decide(900_000, 300_000, None, &r, 0), Step::Nothing);
        }
    }

    #[test]
    fn a_context_that_came_back_under_ends_the_episode() {
        let ep = Some(Episode { tries: 2, last_try_ms: 1, told: true });
        assert_eq!(decide(200_000, 300_000, ep, &reading(), 5), Step::Recovered);
        assert_eq!(decide(200_000, 300_000, None, &reading(), 5), Step::Nothing);
    }

    #[test]
    fn only_a_card_with_a_context_is_ours_to_take_off() {
        assert!(is_context_card(Some(&json!({ "cap": 20, "spent": 17, "context": 480_000 }))));
        assert!(!is_context_card(Some(&json!({ "cap": 20, "spent": 17 }))));
        assert!(!is_context_card(None));
    }
}
