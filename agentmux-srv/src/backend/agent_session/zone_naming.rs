// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Zone-name validation + builders for agent session zones.

/// Returns true if `s` matches `[A-Za-z0-9_-]+`. Rejects empty.
///
/// We're embedding `definition_id` into a zone name (a string the
/// frontend can supply via RPC), so anything outside the safe set would
/// let an attacker write/read arbitrary zones. UUIDs (the production
/// definition_id shape) are a strict subset of this character class.
pub fn is_valid_definition_id(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `agent:<definition_id>:current`. Panics in debug if `definition_id`
/// is invalid; callers should `validate_definition_id` first in release.
pub fn agent_current_zone(definition_id: &str) -> String {
    debug_assert!(
        is_valid_definition_id(definition_id),
        "agent_current_zone: invalid definition_id"
    );
    format!("agent:{}:current", definition_id)
}

/// `agent:<definition_id>:archive:<ts_ms>`.
pub fn agent_archive_zone(definition_id: &str, ts_ms: u64) -> String {
    debug_assert!(
        is_valid_definition_id(definition_id),
        "agent_archive_zone: invalid definition_id"
    );
    format!("agent:{}:archive:{}", definition_id, ts_ms)
}

/// Convenience: validate + build the current-zone string. Returns
/// `Err` with a stable error prefix on bad input so RPC callers see a
/// consistent message.
pub fn validate_and_current(definition_id: &str) -> Result<String, String> {
    if !is_valid_definition_id(definition_id) {
        return Err(format!(
            "INVALID_DEFINITION_ID: must match [A-Za-z0-9_-]+, got {:?}",
            definition_id
        ));
    }
    Ok(agent_current_zone(definition_id))
}
