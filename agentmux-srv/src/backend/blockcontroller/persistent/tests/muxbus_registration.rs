// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::muxbus_agent_id_from_env;
use std::collections::HashMap;

#[test]
fn resolves_agentmux_agent_id() {
    let mut env = HashMap::new();
    env.insert("AGENTMUX_AGENT_ID".to_string(), "Naki".to_string());
    assert_eq!(muxbus_agent_id_from_env(&env), Some("Naki".to_string()));
}

#[test]
fn falls_back_to_legacy_wavemux_id() {
    let mut env = HashMap::new();
    env.insert("WAVEMUX_AGENT_ID".to_string(), "clamk".to_string());
    assert_eq!(muxbus_agent_id_from_env(&env), Some("clamk".to_string()));
}

#[test]
fn prefers_agentmux_over_legacy() {
    let mut env = HashMap::new();
    env.insert("AGENTMUX_AGENT_ID".to_string(), "new".to_string());
    env.insert("WAVEMUX_AGENT_ID".to_string(), "old".to_string());
    assert_eq!(muxbus_agent_id_from_env(&env), Some("new".to_string()));
}

#[test]
fn none_when_absent_or_blank() {
    let mut env: HashMap<String, String> = HashMap::new();
    assert_eq!(muxbus_agent_id_from_env(&env), None);
    env.insert("AGENTMUX_AGENT_ID".to_string(), "   ".to_string());
    assert_eq!(muxbus_agent_id_from_env(&env), None);
}
