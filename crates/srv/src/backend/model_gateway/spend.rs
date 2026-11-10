// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each agent has used through the gateway today, counted from the usage in each response times
//! the route's price table. In memory: it starts again with srv, and the day is the UTC day.

use super::catalog::Model;
use crate::agents::types::TokenCounts;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spend {
    pub requests: u64,
    pub tokens: TokenCounts,
    pub cost_usd: f64,
}

fn table() -> &'static Mutex<HashMap<(String, i64), Spend>> {
    static TABLE: OnceLock<Mutex<HashMap<(String, i64), Spend>>> = OnceLock::new();
    TABLE.get_or_init(Default::default)
}

/// Provider-reported spend per agent since its current process started (see
/// [`reset_session`]). An agent whose responses never carried a charge (not on
/// a route) has no entry, and keeps the price-table estimate.
fn sessions() -> &'static Mutex<HashMap<String, f64>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();
    SESSIONS.get_or_init(Default::default)
}

/// The agents whose responses have carried a provider charge: they are on a
/// route, so a response of theirs that reports none is priced by the route's
/// own table and still belongs in their session total.
fn routed() -> &'static Mutex<HashSet<String>> {
    static ROUTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ROUTED.get_or_init(Default::default)
}

/// The cumulative provider total last put on a `result` frame, per agent: what
/// a frame's turn-scoped `cost_usd` is measured from.
fn last_totals() -> &'static Mutex<HashMap<String, f64>> {
    static LAST: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();
    LAST.get_or_init(Default::default)
}

/// Clears `agent`'s provider-reported session total: a new process (spawn,
/// restart, resume) starts its own, as the harness's own total does.
pub fn reset_session(agent: &str) {
    sessions().lock().unwrap().remove(agent);
    last_totals().lock().unwrap().remove(agent);
    routed().lock().unwrap().remove(agent);
}

/// The provider-reported spend for `agent` since [`reset_session`], or `None`
/// when none of its responses reported a charge (the agent is not on a route).
pub fn session_usd(agent: &str) -> Option<f64> {
    sessions().lock().unwrap().get(agent).copied()
}

fn day(now_ms: i64) -> i64 {
    now_ms.div_euclid(86_400_000)
}

/// The cost of `tokens` on `model`: a cached read at the cache price, the rest of the input at the input price.
pub fn cost(model: &Model, tokens: &TokenCounts) -> f64 {
    let per_m = |n: u64, price: f64| n as f64 * price / 1_000_000.0;
    per_m(tokens.input, model.price_in)
        + per_m(tokens.cache_creation, model.price_in)
        + per_m(tokens.cache_read, model.price_cache_read)
        + per_m(tokens.output, model.price_out)
}

/// Adds one response to the agent's day and returns that response's cost: the
/// provider's own charge when the response reported one (`actual`), else the
/// price table's estimate.
pub fn record(agent: &str, model: &Model, tokens: &TokenCounts, actual: Option<f64>, now_ms: i64) -> f64 {
    let actual = actual.filter(|c| c.is_finite() && *c >= 0.0);
    let cost = actual.unwrap_or_else(|| cost(model, tokens));
    // A route agent's session total holds every response of its own, priced by
    // the provider where it said and by the route's table where it did not (an
    // interrupted stream, say); only a response that never saw a charge at all
    // leaves an agent off a route with no total.
    let on_route = match actual {
        Some(_) => {
            routed().lock().unwrap().insert(agent.to_string());
            true
        }
        None => routed().lock().unwrap().contains(agent),
    };
    if on_route {
        *sessions().lock().unwrap().entry(agent.to_string()).or_insert(0.0) += actual.unwrap_or(cost);
    }
    let mut table = table().lock().unwrap();
    let day_now = day(now_ms);
    table.retain(|(_, d), _| *d >= day_now - 1);
    let entry = table.entry((agent.to_string(), day_now)).or_default();
    entry.requests += 1;
    entry.tokens.input += tokens.input;
    entry.tokens.output += tokens.output;
    entry.tokens.cache_creation += tokens.cache_creation;
    entry.tokens.cache_read += tokens.cache_read;
    entry.cost_usd += cost;
    cost
}

/// `line`, a JSON frame from the harness, with a route agent's `result` totals
/// replaced by the provider's own charge: the harness prices its tokens with
/// Claude's rates, which on a route are wrong by orders of magnitude (#30143).
/// `total_cost_usd` becomes the session total, which the turn ledger takes the
/// rise of; `cost_usd` becomes this turn's rise, because the pane adds each
/// frame's value to its running total. `None` when the line is not a `result`
/// frame, reports no total, or the agent is not on a route (none of its
/// responses carried a charge).
pub fn rewrite_result_total(line: &str, agent: &str) -> Option<String> {
    let real = session_usd(agent)?;
    let mut frame: serde_json::Value = serde_json::from_str(line).ok()?;
    if frame.get("type").and_then(serde_json::Value::as_str) != Some("result") {
        return None;
    }
    frame.get("total_cost_usd")?.as_f64()?;
    frame["total_cost_usd"] = serde_json::json!(real);
    // `total_cost_usd` is the session total (the turn ledger takes its rise);
    // `cost_usd` is turn-scoped — the pane ADDS each frame's value, so it must
    // be the rise since the previous frame, never the cumulative total.
    let previous = last_totals().lock().unwrap().insert(agent.to_string(), real).unwrap_or(0.0);
    if frame.get("cost_usd").is_some() {
        frame["cost_usd"] = serde_json::json!((real - previous).max(0.0));
    }
    Some(frame.to_string())
}

/// The agent's use so far today.
pub fn today(agent: &str, now_ms: i64) -> Spend {
    table().lock().unwrap().get(&(agent.to_string(), day(now_ms))).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::model_gateway::catalog;

    #[test]
    fn a_response_is_priced_by_the_models_table_and_summed_per_agent_and_day() {
        let model = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let used = TokenCounts { input: 1_000_000, output: 500_000, cache_read: 2_000_000, ..Default::default() };
        let one = record("spend-agent", model, &used, None, 1_000);
        assert!((one - (0.15 + 0.30 + 0.006)).abs() < 1e-9, "{one}");
        record("spend-agent", model, &used, None, 2_000);
        let sum = today("spend-agent", 3_000);
        assert_eq!((sum.requests, sum.tokens.output), (2, 1_000_000));
        assert!((sum.cost_usd - 2.0 * one).abs() < 1e-9);
        assert_eq!(session_usd("spend-agent"), None, "no provider charge reported: not a route agent");
        assert_eq!(today("spend-agent", 86_400_000 + 5), Spend::default(), "the next UTC day starts at zero");
        assert_eq!(today("spend-other", 3_000), Spend::default());
    }

    /// The provider's own charge, when the response reports one, is the cost
    /// (not the table's Claude-priced estimate) and is what a route agent's
    /// session total sums (issue #30143).
    #[test]
    fn a_providers_own_charge_is_used_and_summed_per_session() {
        let model = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let used = TokenCounts { input: 1_000_000, ..Default::default() };
        let one = record("spend-route", model, &used, Some(0.004), 1_000);
        assert!((one - 0.004).abs() < 1e-12, "{one}");
        assert_eq!(session_usd("spend-route"), Some(0.004));
        record("spend-route", model, &used, Some(0.006), 2_000);
        assert!((session_usd("spend-route").unwrap() - 0.010).abs() < 1e-12);
        assert!((today("spend-route", 3_000).cost_usd - 0.010).abs() < 1e-12);
        // A route response that reported no charge (an interrupted stream) is
        // priced by the route's table and still counts, rather than vanishing
        // from the pane's session figure.
        let estimate = cost(model, &used);
        record("spend-route", model, &used, None, 3_000);
        let total = session_usd("spend-route").unwrap();
        assert!((total - (0.010 + estimate)).abs() < 1e-12, "{total}");
        reset_session("spend-route");
        assert_eq!(session_usd("spend-route"), None, "a new process starts its own total");
        record("spend-route", model, &used, None, 4_000);
        assert_eq!(session_usd("spend-route"), None, "and a costless response alone does not make one");
    }

    #[test]
    fn a_route_agents_result_totals_are_rewritten_to_the_providers_charge() {
        let model = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let used = TokenCounts { input: 1_000_000, ..Default::default() };
        record("spend-rewrite", model, &used, Some(0.0042), 1_000);
        let line = r#"{"type":"result","total_cost_usd":97.41,"cost_usd":97.41,"num_turns":1}"#;
        let out = rewrite_result_total(line, "spend-rewrite").expect("rewritten");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["total_cost_usd"].as_f64(), Some(0.0042));
        assert_eq!(v["cost_usd"].as_f64(), Some(0.0042), "the first frame's turn cost is the whole total");
        // A second turn: the total is cumulative, the turn figure is the rise.
        record("spend-rewrite", model, &used, Some(0.0058), 2_000);
        let out = rewrite_result_total(line, "spend-rewrite").expect("rewritten");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let total = v["total_cost_usd"].as_f64().unwrap();
        assert!((total - 0.01).abs() < 1e-12, "{total}");
        let turn = v["cost_usd"].as_f64().unwrap();
        assert!((turn - 0.0058).abs() < 1e-12, "never the cumulative total: {turn}");
        assert_eq!(v["num_turns"].as_u64(), Some(1), "the rest of the frame is untouched");
        assert!(rewrite_result_total(r#"{"type":"assistant"}"#, "spend-rewrite").is_none(), "not a result frame");
        assert!(rewrite_result_total(line, "spend-estimate-only").is_none(), "no provider charge: unchanged");
    }
}
