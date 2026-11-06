// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each agent has used through the gateway today, counted from the usage in each response times
//! the route's price table. In memory: it starts again with srv, and the day is the UTC day.

use super::catalog::Model;
use crate::agents::types::TokenCounts;
use std::collections::HashMap;
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

/// Adds one response to the agent's day and returns that response's cost.
pub fn record(agent: &str, model: &Model, tokens: &TokenCounts, now_ms: i64) -> f64 {
    let cost = cost(model, tokens);
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
        let one = record("spend-agent", model, &used, 1_000);
        assert!((one - (0.15 + 0.30 + 0.006)).abs() < 1e-9, "{one}");
        record("spend-agent", model, &used, 2_000);
        let sum = today("spend-agent", 3_000);
        assert_eq!((sum.requests, sum.tokens.output), (2, 1_000_000));
        assert!((sum.cost_usd - 2.0 * one).abs() < 1e-9);
        assert_eq!(today("spend-agent", 86_400_000 + 5), Spend::default(), "the next UTC day starts at zero");
        assert_eq!(today("spend-other", 3_000), Spend::default());
    }
}
