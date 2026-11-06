// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The key of a model route (`backend::model_gateway`), read for the gateway to add upstream. The value
//! goes into one outgoing request and is not returned to any caller that could show it to an agent.
//! The person approved the agent's use of the route when it was made, so this asks no one; it records
//! the use under the agent's name, once per agent and route every few minutes, not once per request.

use super::{kinds, record, seal, Store};
use agentmux_common::time::now_ms;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The activity line for a use, listed with the other uses of a value.
pub const USED: &str = "sent to a model route";
/// How often a use by the same agent of the same route is written to the activity log.
const RECORD_EVERY_MS: i64 = 10 * 60 * 1000;

pub struct RouteKey {
    pub value: zeroize::Zeroizing<String>,
    pub label: String,
}

fn last_recorded() -> &'static Mutex<HashMap<(String, String), i64>> {
    static LAST: OnceLock<Mutex<HashMap<(String, String), i64>>> = OnceLock::new();
    LAST.get_or_init(Default::default)
}

fn due(agent: &str, route: &str, now: i64) -> bool {
    let mut last = last_recorded().lock().unwrap();
    let at = last.entry((agent.to_string(), route.to_string())).or_insert(i64::MIN);
    let due = now.saturating_sub(*at) >= RECORD_EVERY_MS;
    if due {
        *at = now;
    }
    due
}

/// The `api-key` entry whose label starts with `label_prefix` (case does not matter), and its value.
pub fn for_route(store: &Store, label_prefix: &str, agent: &str, route: &str) -> Result<RouteKey, String> {
    let prefix = label_prefix.to_lowercase();
    let rows = store.vault_list().map_err(|e| e.to_string())?;
    let row = rows
        .into_iter()
        .find(|r| r.kind == "api-key" && r.label.to_lowercase().starts_with(&prefix))
        .ok_or_else(|| format!("the Vault has no api-key entry labelled \"{label_prefix}\""))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {} has an unknown kind", row.id))?;
    let key = seal::key(false)?;
    let parts = seal::open(&key, &row.id, &row.sealed)?;
    let value = parts.get(kind.parts[0]).ok_or_else(|| format!("{} has no value saved", row.label))?;
    let value = zeroize::Zeroizing::new(value.to_string());
    if due(agent, route, now_ms()) {
        if let Err(e) = store.vault_touch(&row.id, now_ms()) {
            tracing::warn!(error = %e, "vault: couldn't note a route key's use");
        }
        record(store, &row.id, &row.label, agent, USED, route, "done");
    }
    Ok(RouteKey { value, label: row.label })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_use_is_written_down_once_per_window() {
        let t = 1_000_000_000;
        assert!(due("rk-agent", "openrouter", t));
        assert!(!due("rk-agent", "openrouter", t + 1_000));
        assert!(due("rk-other", "openrouter", t + 1_000), "another agent has its own line");
        assert!(due("rk-agent", "openrouter", t + RECORD_EVERY_MS));
    }
}
