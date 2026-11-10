// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each agent has used through the gateway today, counted from the usage in each response times
//! the route's price table. The live table is in memory; once [`attach`] has been called every change is also
//! written to the history store (`storage::spend_day`) and the last two days are read back at start, so an srv
//! restart does not start the day again. The day is the UTC day.

use super::catalog::Model;
use crate::agents::types::TokenCounts;
use crate::backend::storage::spend_day::SpendDayRow;
use crate::backend::storage::store::Store;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spend {
    pub requests: u64,
    pub tokens: TokenCounts,
    pub cost_usd: f64,
}

/// One agent's day on one route, with the highest alert level already sent for the agent's day.
#[derive(Debug, Clone, Default)]
struct Day {
    spend: Spend,
    alerted: u8,
}

/// Agent, route and UTC day.
type DayKey = (String, String, i64);

fn table() -> &'static Mutex<HashMap<DayKey, Day>> {
    static TABLE: OnceLock<Mutex<HashMap<DayKey, Day>>> = OnceLock::new();
    TABLE.get_or_init(Default::default)
}

enum Write {
    Save(SpendDayRow),
    Clear(String, i64),
}

/// The writer thread's inbox, once [`attach`] has started it. A change is queued, not written, so a response
/// settling (on the stream's task) never waits on SQLite.
fn writer() -> &'static OnceLock<mpsc::Sender<Write>> {
    static WRITER: OnceLock<mpsc::Sender<Write>> = OnceLock::new();
    &WRITER
}

fn row_of(key: &DayKey, day: &Day) -> SpendDayRow {
    let t = &day.spend.tokens;
    SpendDayRow {
        agent_id: key.0.clone(),
        route: key.1.clone(),
        day: key.2,
        requests: day.spend.requests,
        input: t.input,
        output: t.output,
        cache_creation: t.cache_creation,
        cache_read: t.cache_read,
        cost_usd: day.spend.cost_usd,
        alerted: day.alerted,
    }
}

fn queue(write: Write) {
    if let Some(tx) = writer().get() {
        let _ = tx.send(write);
    }
}

/// Reads the last two days back from `store` into the live table, keeping what the table already holds.
/// Returns the number of rows read.
pub fn restore(store: &Store, now_ms: i64) -> usize {
    let rows = match store.spend_day_load(day(now_ms) - 1) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "model gateway: the spend of earlier days could not be read; today starts at zero");
            return 0;
        }
    };
    let mut table = table().lock().unwrap();
    let n = rows.len();
    for r in rows {
        let spend = Spend {
            requests: r.requests,
            tokens: TokenCounts { input: r.input, output: r.output, cache_creation: r.cache_creation, cache_read: r.cache_read, ..Default::default() },
            cost_usd: r.cost_usd,
        };
        table.entry((r.agent_id, r.route, r.day)).or_insert(Day { spend, alerted: r.alerted });
    }
    n
}

/// Keeps the day's table in `store` from now on: reads the last two days, then writes each change from a thread of its own.
pub fn attach(store: Arc<Store>) {
    let restored = restore(&store, agentmux_common::time::now_ms());
    let (tx, rx) = mpsc::channel::<Write>();
    if writer().set(tx).is_err() {
        return;
    }
    tracing::info!(rows = restored, "model gateway: the day's spend is kept in the history store");
    let spawned = std::thread::Builder::new().name("gateway-spend".into()).spawn(move || {
        for write in rx {
            let done = match write {
                Write::Save(row) => store.spend_day_save(&row),
                Write::Clear(agent, d) => store.spend_day_clear(&agent, d).map(|_| ()),
            };
            if let Err(e) = done {
                tracing::warn!(error = %e, "model gateway: a day of spend was not written");
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "model gateway: the spend writer did not start; the day is kept in memory only");
    }
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

/// Adds one response to the agent's day on `route` and returns that response's cost: the
/// provider's own charge when the response reported one (`actual`), else the
/// price table's estimate.
pub fn record(agent: &str, route: &str, model: &Model, tokens: &TokenCounts, actual: Option<f64>, now_ms: i64) -> f64 {
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
    // This agent's old days go; another agent's are dropped when it records, and `today` never reads them.
    table.retain(|(a, _, d), _| a != agent || *d >= day_now - 1);
    let key = (agent.to_string(), route.to_string(), day_now);
    let entry = table.entry(key.clone()).or_default();
    entry.spend.requests += 1;
    entry.spend.tokens.input += tokens.input;
    entry.spend.tokens.output += tokens.output;
    entry.spend.tokens.cache_creation += tokens.cache_creation;
    entry.spend.tokens.cache_read += tokens.cache_read;
    entry.spend.cost_usd += cost;
    queue(Write::Save(row_of(&key, entry)));
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

/// The agent's use so far today, over every route it used.
pub fn today(agent: &str, now_ms: i64) -> Spend {
    let d = day(now_ms);
    let mut sum = Spend::default();
    for (_, one) in table().lock().unwrap().iter().filter(|((a, _, day), _)| a == agent && *day == d) {
        sum.requests += one.spend.requests;
        sum.cost_usd += one.spend.cost_usd;
        sum.tokens.input += one.spend.tokens.input;
        sum.tokens.output += one.spend.tokens.output;
        sum.tokens.cache_creation += one.spend.tokens.cache_creation;
        sum.tokens.cache_read += one.spend.tokens.cache_read;
    }
    sum
}

/// What every agent has spent on `route` so far today.
pub fn route_today_usd(route: &str, now_ms: i64) -> f64 {
    let d = day(now_ms);
    table().lock().unwrap().iter().filter(|((_, r, day), _)| r == route && *day == d).map(|(_, one)| one.spend.cost_usd).sum()
}

/// The highest alert level (0, 50, 80 or 100) already sent for the agent's day.
pub fn alerted(agent: &str, now_ms: i64) -> u8 {
    let d = day(now_ms);
    table().lock().unwrap().iter().filter(|((a, _, day), _)| a == agent && *day == d).map(|(_, one)| one.alerted).max().unwrap_or(0)
}

/// Records the alert level sent for the agent's day, on each of its rows so it survives a restart. A raised cap sets it
/// back down to where the day now stands.
pub fn set_alerted(agent: &str, level: u8, now_ms: i64) {
    let d = day(now_ms);
    let mut table = table().lock().unwrap();
    let mut any = false;
    for (key, one) in table.iter_mut().filter(|((a, _, day), _)| a == agent && *day == d) {
        one.alerted = level;
        queue(Write::Save(row_of(key, one)));
        any = true;
    }
    // An agent refused by its route's cap has spent nothing of its own today: its level lives on a row with no route.
    if !any && level > 0 {
        let key = (agent.to_string(), String::new(), d);
        let one = Day { alerted: level, ..Default::default() };
        queue(Write::Save(row_of(&key, &one)));
        table.insert(key, one);
    }
}

/// Zeroes the agent's day (a spend that was wrong): its rows are removed, here and in the store. Returns the
/// dollars that were counted.
pub fn reset_today(agent: &str, now_ms: i64) -> f64 {
    let d = day(now_ms);
    let mut removed = 0.0;
    table().lock().unwrap().retain(|(a, _, day), one| {
        let gone = a == agent && *day == d;
        if gone {
            removed += one.spend.cost_usd;
        }
        !gone
    });
    queue(Write::Clear(agent.to_string(), d));
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::model_gateway::catalog;

    #[test]
    fn a_response_is_priced_by_the_models_table_and_summed_per_agent_and_day() {
        let model = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let used = TokenCounts { input: 1_000_000, output: 500_000, cache_read: 2_000_000, ..Default::default() };
        let one = record("spend-agent", "openrouter", model, &used, None, 1_000);
        assert!((one - (0.15 + 0.30 + 0.006)).abs() < 1e-9, "{one}");
        record("spend-agent", "openrouter", model, &used, None, 2_000);
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
        let one = record("spend-route", "openrouter", model, &used, Some(0.004), 1_000);
        assert!((one - 0.004).abs() < 1e-12, "{one}");
        assert_eq!(session_usd("spend-route"), Some(0.004));
        record("spend-route", "openrouter", model, &used, Some(0.006), 2_000);
        assert!((session_usd("spend-route").unwrap() - 0.010).abs() < 1e-12);
        assert!((today("spend-route", 3_000).cost_usd - 0.010).abs() < 1e-12);
        // A route response that reported no charge (an interrupted stream) is
        // priced by the route's table and still counts, rather than vanishing
        // from the pane's session figure.
        let estimate = cost(model, &used);
        record("spend-route", "openrouter", model, &used, None, 3_000);
        let total = session_usd("spend-route").unwrap();
        assert!((total - (0.010 + estimate)).abs() < 1e-12, "{total}");
        reset_session("spend-route");
        assert_eq!(session_usd("spend-route"), None, "a new process starts its own total");
        record("spend-route", "openrouter", model, &used, None, 4_000);
        assert_eq!(session_usd("spend-route"), None, "and a costless response alone does not make one");
    }

    #[test]
    fn a_route_agents_result_totals_are_rewritten_to_the_providers_charge() {
        let model = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let used = TokenCounts { input: 1_000_000, ..Default::default() };
        record("spend-rewrite", "openrouter", model, &used, Some(0.0042), 1_000);
        let line = r#"{"type":"result","total_cost_usd":97.41,"cost_usd":97.41,"num_turns":1}"#;
        let out = rewrite_result_total(line, "spend-rewrite").expect("rewritten");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["total_cost_usd"].as_f64(), Some(0.0042));
        assert_eq!(v["cost_usd"].as_f64(), Some(0.0042), "the first frame's turn cost is the whole total");
        // A second turn: the total is cumulative, the turn figure is the rise.
        record("spend-rewrite", "openrouter", model, &used, Some(0.0058), 2_000);
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

    #[test]
    fn a_day_is_summed_over_routes_and_a_routes_over_agents_and_it_comes_back_from_the_store() {
        let model = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let used = TokenCounts { input: 1_000_000, ..Default::default() };
        let now = agentmux_common::time::now_ms();
        record("spend-sum-a", "route-x", model, &used, Some(1.5), now);
        record("spend-sum-a", "route-y", model, &used, Some(0.5), now);
        record("spend-sum-b", "route-x", model, &used, Some(2.0), now);
        assert!((today("spend-sum-a", now).cost_usd - 2.0).abs() < 1e-12, "both routes count for the agent");
        assert!((route_today_usd("route-x", now) - 3.5).abs() < 1e-12, "both agents count for the route");
        assert_eq!(alerted("spend-sum-a", now), 0);
        set_alerted("spend-sum-a", 50, now);
        assert_eq!(alerted("spend-sum-a", now), 50);
        set_alerted("spend-sum-none", 100, now);
        assert_eq!(alerted("spend-sum-none", now), 100, "kept for an agent with no spend of its own");
        assert_eq!(today("spend-sum-none", now), Spend::default());

        // What was saved is what a restart reads back: the rows the table would write, loaded into a store.
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        for (key, one) in table().lock().unwrap().iter().filter(|((a, _, _), _)| a == "spend-sum-a") {
            store.spend_day_save(&row_of(key, one)).unwrap();
        }
        table().lock().unwrap().retain(|(a, _, _), _| a != "spend-sum-a");
        assert_eq!(today("spend-sum-a", now), Spend::default());
        assert_eq!(restore(&store, now), 2);
        assert!((today("spend-sum-a", now).cost_usd - 2.0).abs() < 1e-12);
        assert_eq!(alerted("spend-sum-a", now), 50, "the alert already sent is not sent again after a restart");

        let removed = reset_today("spend-sum-a", now);
        assert!((removed - 2.0).abs() < 1e-12);
        assert_eq!(today("spend-sum-a", now), Spend::default());
        assert!((today("spend-sum-b", now).cost_usd - 2.0).abs() < 1e-12, "another agent's day is untouched");
    }
}
