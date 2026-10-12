// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent's identity variables in its spawn environment, beyond what
//! `input.rs`'s `split_agent_identity_env` and `carry_agent_uid_env` set.

use std::collections::HashMap;

/// `AGENTMUX_AGENT_ID` (and its `MUXBUS_AGENT_ID` mirror) when the launch's
/// `cmd:env` didn't carry one. It is the canonical, app-wide agent-identity
/// variable (MCP tool routing, native memory, shell OSC titling, jekt
/// auto-registration, gh-agent; see this repo's CLAUDE.md Naming Conventions
/// table), and a `cmd:env` value takes precedence, unlike the reserved
/// variables. The fallback is the row's slug, as every launch path sets it
/// and as the name-keyed signing keys are filed (identity M4d,
/// `agent_self_keys.rs`); for a pane with no row (quick launch), its display
/// name as a route name, since a name that isn't a valid agent id (a fork's
/// "Loap #3") fails every registration
/// (REPORT_FORK_AGENT_ISOLATION_2026_10_12.md F3). `MUXBUS_AGENT_ID` mirrors
/// it so muxbus-client picks it up under the prefix it checks first; scoped to
/// that one hand-off point, not a second source of truth (ARCH-002).
pub(crate) fn carry_agent_id_env(
    env_vars: &mut HashMap<String, String>,
    display_name: &str,
    persisted_slug: Option<&str>,
) {
    if !env_vars.contains_key("AGENTMUX_AGENT_ID") {
        let id = persisted_slug
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| (!display_name.trim().is_empty()).then(|| agentmux_common::slug::agent_route_name(display_name)));
        if let Some(id) = id {
            env_vars.insert("AGENTMUX_AGENT_ID".to_string(), id);
        }
    }
    if !env_vars.contains_key("MUXBUS_AGENT_ID") {
        if let Some(agent_id) = env_vars.get("AGENTMUX_AGENT_ID").cloned() {
            env_vars.insert("MUXBUS_AGENT_ID".to_string(), agent_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::carry_agent_id_env;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// With no `AGENTMUX_AGENT_ID` in `cmd:env`, the row's slug: what the
    /// launch sets and the signing keys are filed under. A fork's display
    /// name is never used when the row is known.
    #[test]
    fn a_missing_agent_id_is_the_rows_slug() {
        let mut e = env(&[]);
        carry_agent_id_env(&mut e, "Loap #3", Some("loap-loap-2-loap-3"));
        assert_eq!(e["AGENTMUX_AGENT_ID"], "loap-loap-2-loap-3");
        assert_eq!(e["MUXBUS_AGENT_ID"], "loap-loap-2-loap-3");
    }

    /// A pane with no row (quick launch) falls back to its display name, as
    /// a valid id: "Loap #3" used to fail every registration.
    #[test]
    fn a_rowless_panes_agent_id_is_its_route_name() {
        let mut e = env(&[]);
        carry_agent_id_env(&mut e, "Loap #3", None);
        assert_eq!(e["AGENTMUX_AGENT_ID"], "Loap-3");
        let mut plain = env(&[]);
        carry_agent_id_env(&mut plain, "AgentY", None);
        assert_eq!(plain["AGENTMUX_AGENT_ID"], "AgentY", "a valid name keeps its casing");
        let mut none = env(&[]);
        carry_agent_id_env(&mut none, "  ", None);
        assert!(!none.contains_key("AGENTMUX_AGENT_ID"));
    }

    /// A launch's own value wins, and its mirror is kept too.
    #[test]
    fn a_carried_agent_id_is_kept() {
        let mut e = env(&[("AGENTMUX_AGENT_ID", "agenty-2"), ("MUXBUS_AGENT_ID", "bus-y")]);
        carry_agent_id_env(&mut e, "AgentY", Some("agenty-3"));
        assert_eq!(e["AGENTMUX_AGENT_ID"], "agenty-2");
        assert_eq!(e["MUXBUS_AGENT_ID"], "bus-y");
    }
}
