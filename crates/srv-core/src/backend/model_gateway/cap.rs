// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The spend caps (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 3.2): how much an agent may use
//! through the gateway in a UTC day, and how much a route may. The gateway reads them at each request, so a
//! cap raised mid-turn lets the next request through.
//!
//! - **Per agent:** the agent definition's content `agent:spend_cap_usd`. Absent is [`DEFAULT_AGENT_CAP_USD`];
//!   `none` or `0` is no cap.
//! - **Per route:** the app setting `model:spend_cap_usd:<route id>`. Absent is [`DEFAULT_ROUTE_CAP_USD`];
//!   `none` or `0` is no cap.

use super::spend;
use super::token::Grant;
use crate::backend::storage::content::AgentContent;
use crate::backend::storage::store::Store;
use std::collections::HashMap;

/// The agent-definition content type the agent's cap is kept under.
pub const AGENT_CAP_TYPE: &str = "agent:spend_cap_usd";
/// The prefix of the app setting a route's cap is kept under; the route id follows.
pub const ROUTE_CAP_SETTING: &str = "model:spend_cap_usd:";
pub const DEFAULT_AGENT_CAP_USD: f64 = 20.0;
pub const DEFAULT_ROUTE_CAP_USD: f64 = 100.0;
/// The least a cap may be set to: a smaller one is not a cap anyone meant.
pub const MIN_CAP_USD: f64 = 0.01;

/// What a cap was reached for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Agent,
    Route,
}

/// A cap that has been reached.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub scope: Scope,
    pub cap: f64,
    pub spent: f64,
}

/// A cap as written: a positive number of dollars, or `none` (also `0`) for no cap. `None` for anything else.
fn parse(value: &serde_json::Value) -> Option<Option<f64>> {
    let number = match value {
        serde_json::Value::String(s) if matches!(s.trim().to_ascii_lowercase().as_str(), "none" | "unlimited" | "off") => return Some(None),
        serde_json::Value::String(s) => s.trim().trim_start_matches('$').parse::<f64>().ok()?,
        other => other.as_f64()?,
    };
    match number {
        n if !n.is_finite() || n < 0.0 => None,
        n if n == 0.0 => Some(None),
        n => Some(Some(n)),
    }
}

/// The agent's cap in dollars a day, or `None` when it has none.
pub fn agent_cap(mstore: &Store, def_id: &str) -> Option<f64> {
    let stored = mstore.agent_content_get(def_id, AGENT_CAP_TYPE).ok().flatten();
    match stored.and_then(|c| parse(&serde_json::Value::String(c.content))) {
        Some(cap) => cap,
        None => Some(DEFAULT_AGENT_CAP_USD),
    }
}

/// Sets the agent's cap: `Some(dollars)`, or `None` for no cap.
pub fn set_agent_cap(mstore: &Store, def_id: &str, cap: Option<f64>) -> Result<(), String> {
    let content = cap.map_or_else(|| "none".to_string(), |c| c.to_string());
    mstore
        .agent_content_set(&AgentContent {
            agent_id: def_id.to_string(),
            content_type: AGENT_CAP_TYPE.to_string(),
            content,
            updated_at: agentmux_common::time::now_ms(),
        })
        .map_err(|e| e.to_string())
}

/// What `Agents update spend_cap_usd` takes: a positive number of dollars, `0` or `"none"` for no cap.
pub fn parse_request(value: &serde_json::Value) -> Result<Option<f64>, String> {
    let cap = parse(value).ok_or_else(|| "spend_cap_usd is a positive number of dollars a day, or 0 / \"none\" for no cap".to_string())?;
    match cap {
        Some(c) if c < MIN_CAP_USD => Err(format!("spend_cap_usd is at least ${MIN_CAP_USD:.2}; 0 or \"none\" removes the cap")),
        other => Ok(other),
    }
}

/// The route's cap in dollars a day, or `None` when it has none.
pub fn route_cap(settings: &HashMap<String, serde_json::Value>, route: &str) -> Option<f64> {
    match settings.get(&format!("{ROUTE_CAP_SETTING}{route}")).and_then(parse) {
        Some(cap) => cap,
        None => Some(DEFAULT_ROUTE_CAP_USD),
    }
}

/// The cap the agent's use today has reached, if any. The agent's own is checked first.
pub fn reached(mstore: &Store, settings: &HashMap<String, serde_json::Value>, grant: &Grant, now_ms: i64) -> Option<Hit> {
    if let Some(cap) = agent_cap(mstore, &grant.def_id) {
        let spent = spend::today(&grant.agent, now_ms).cost_usd;
        if spent >= cap {
            return Some(Hit { scope: Scope::Agent, cap, spent });
        }
    }
    let cap = route_cap(settings, &grant.route)?;
    let spent = spend::route_today_usd(&grant.route, now_ms);
    (spent >= cap).then_some(Hit { scope: Scope::Route, cap, spent })
}

/// How far `spent` is into `cap`, as the alert level it has passed: 0, 50, 80 or 100.
pub fn level(spent: f64, cap: f64) -> u8 {
    match spent / cap {
        r if r >= 1.0 => 100,
        r if r >= 0.8 => 80,
        r if r >= 0.5 => 50,
        _ => 0,
    }
}

/// The alert level the agent's day is at under its own cap; 0 when it has none.
pub fn agent_level(mstore: &Store, grant: &Grant, now_ms: i64) -> (u8, Option<f64>, f64) {
    let spent = spend::today(&grant.agent, now_ms).cost_usd;
    match agent_cap(mstore, &grant.def_id) {
        Some(cap) => (level(spent, cap), Some(cap), spent),
        None => (0, None, spent),
    }
}

/// The refusal the harness shows: what was used, what the cap is and who can lift it.
pub fn refusal_text(hit: &Hit) -> String {
    match hit.scope {
        Scope::Agent => format!(
            "spend cap: this agent has used ${:.2} of its ${:.2} for today (UTC). A person or its leader can raise the cap.",
            hit.spent, hit.cap
        ),
        Scope::Route => format!(
            "spend cap: this model route's cap of ${:.2} for today (UTC) is used (${:.2}). A person can raise it in Settings.",
            hit.cap, hit.spent
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::types::TokenCounts;
    use crate::backend::model_gateway::catalog;
    use serde_json::json;

    fn store_with(def_id: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        let mut def = crate::backend::storage::agents::test_agent_def(def_id, def_id, "claude", "host", 1, "");
        store.agent_def_insert(&mut def).unwrap();
        store
    }

    #[test]
    fn a_cap_is_dollars_or_none_and_anything_else_is_refused() {
        assert_eq!(parse_request(&json!(40)), Ok(Some(40.0)));
        assert_eq!(parse_request(&json!("12.5")), Ok(Some(12.5)));
        assert_eq!(parse_request(&json!("$7")), Ok(Some(7.0)));
        assert_eq!(parse_request(&json!(0)), Ok(None));
        assert_eq!(parse_request(&json!("none")), Ok(None));
        assert!(parse_request(&json!(-1)).is_err());
        assert!(parse_request(&json!(0.004)).is_err(), "a cap under a cent would read back as none");
        assert_eq!(parse_request(&json!(0.01)), Ok(Some(0.01)));
        assert!(parse_request(&json!("lots")).is_err());
        assert!(parse_request(&json!(true)).is_err());
    }

    #[test]
    fn levels_are_the_thresholds_passed() {
        assert_eq!([0.0, 9.99, 10.0, 15.99, 16.0, 19.99, 20.0, 31.0].map(|s| level(s, 20.0)), [0, 0, 50, 50, 80, 80, 100, 100]);
    }

    #[test]
    fn an_agent_has_the_default_cap_until_one_is_set_and_none_means_no_cap() {
        let store = store_with("cap-def");
        assert_eq!(agent_cap(&store, "cap-def"), Some(DEFAULT_AGENT_CAP_USD));
        set_agent_cap(&store, "cap-def", Some(40.0)).unwrap();
        assert_eq!(agent_cap(&store, "cap-def"), Some(40.0));
        set_agent_cap(&store, "cap-def", Some(0.013)).unwrap();
        assert_eq!(agent_cap(&store, "cap-def"), Some(0.013), "kept as given, not rounded to a cent");
        set_agent_cap(&store, "cap-def", None).unwrap();
        assert_eq!(agent_cap(&store, "cap-def"), None);
    }

    #[test]
    fn a_route_has_the_default_cap_unless_the_settings_say_otherwise() {
        let mut settings = HashMap::new();
        assert_eq!(route_cap(&settings, "openrouter"), Some(DEFAULT_ROUTE_CAP_USD));
        settings.insert(format!("{ROUTE_CAP_SETTING}openrouter"), json!(250));
        assert_eq!(route_cap(&settings, "openrouter"), Some(250.0));
        settings.insert(format!("{ROUTE_CAP_SETTING}openrouter"), json!("none"));
        assert_eq!(route_cap(&settings, "openrouter"), None);
        assert_eq!(route_cap(&settings, "other"), Some(DEFAULT_ROUTE_CAP_USD));
    }

    #[test]
    fn the_agents_cap_is_reached_before_the_routes_and_the_text_says_which() {
        let store = store_with("reach-def");
        let model = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
        let used = TokenCounts { input: 1, ..Default::default() };
        let grant = Grant { agent: "reach-agent".into(), def_id: "reach-def".into(), route: "reach-route".into() };
        let now = agentmux_common::time::now_ms();
        let settings = HashMap::new();
        spend::record("reach-agent", "reach-route", model, &used, Some(19.0), now);
        assert_eq!(reached(&store, &settings, &grant, now), None);
        assert_eq!(agent_level(&store, &grant, now).0, 80);
        spend::record("reach-agent", "reach-route", model, &used, Some(1.0), now);
        let hit = reached(&store, &settings, &grant, now).expect("the agent's $20 is used");
        assert_eq!((hit.scope, hit.cap), (Scope::Agent, 20.0));
        assert!(refusal_text(&hit).starts_with("spend cap: this agent has used $20.00 of its $20.00"));

        // No cap of its own, but other agents have used the route's $100: the route's cap stops it.
        set_agent_cap(&store, "reach-def", None).unwrap();
        assert_eq!(reached(&store, &settings, &grant, now), None);
        spend::record("reach-other", "reach-route", model, &used, Some(80.0), now);
        let hit = reached(&store, &settings, &grant, now).expect("the route's $100 is used");
        assert_eq!((hit.scope, hit.cap), (Scope::Route, 100.0));
        assert!(refusal_text(&hit).contains("route's cap of $100.00"));
        // The next UTC day starts again.
        assert_eq!(reached(&store, &settings, &grant, now + 86_400_000), None);
    }
}
