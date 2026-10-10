// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Removing an agent's dependent rows when the agent itself is deleted.

use super::*;

#[cfg(test)]
pub(crate) fn purge_agent_dependents_for_tests(
    conn: &rusqlite::Connection,
    id: &str,
    tokens: Option<&super::super::agent_tokens::TokenIndex>,
) -> Result<usize, StoreError> {
    purge_agent_dependents(conn, id, tokens)
}

pub(super) fn purge_agent_dependents(
    conn: &rusqlite::Connection,
    id: &str,
    tokens: Option<&super::super::agent_tokens::TokenIndex>,
) -> Result<usize, StoreError> {
    const BY_AGENT_ID: &[&str] = &[
        "db_agent_content",
        "db_agent_skills",
        "db_agent_history",
        "db_agent_identity_links",
        "db_agent_skills_ref",
        "db_agent_mcp_ref",
        "db_agent_credentials",
        "db_agent_native_memory",
        "db_agent_native_memory_versions",
        "db_agent_jekt_keys",
        "db_agent_lan_keys",
        // v37, identity M1: the agent's local identity token dies with it.
        "db_agent_tokens",
        // The agent's Bundles list.
        "db_agent_bundles",
    ];
    let present: std::collections::HashSet<String> = {
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut removed = 0usize;
    for table in BY_AGENT_ID {
        if !present.contains(*table) {
            continue;
        }
        // Table names are compile-time constants from the list above, never
        // caller input — the id itself is still bound as a parameter.
        removed += conn.execute(&format!("DELETE FROM {table} WHERE agent_id=?1"), params![id])?;
        // Identity M4a: the index follows the token row's own delete, not
        // the purge's overall result (spec §6.5.3).
        if *table == "db_agent_tokens" {
            if let Some(tokens) = tokens {
                tokens.forget_uid(id);
            }
        }
    }
    // A channel's cached cloud credentials sit behind its `<channel>/` prefix
    // when channels share the store (`muxbus::agent_credential_prefix_for`), so
    // the plain delete above misses them: a deleted agent's client secret must
    // not stay on disk in any channel's rows.
    if present.contains("db_agent_credentials") {
        removed += conn.execute(
            "DELETE FROM db_agent_credentials WHERE instr(agent_id, '/') > 0 AND substr(agent_id, instr(agent_id, '/') + 1) = ?1",
            params![id],
        )?;
    }
    // Identity M4d-2: the UID-keyed signing keys, keyed by `uid`.
    for table in ["db_agent_lan_keys_by_uid", "db_agent_wan_keys_by_uid"] {
        if present.contains(table) {
            removed += conn.execute(&format!("DELETE FROM {table} WHERE uid=?1"), params![id])?;
        }
    }
    // The two that key on the agent differently.
    if present.contains("db_conversation_trust_grants") {
        removed += conn.execute(
            "DELETE FROM db_conversation_trust_grants WHERE agent_id=?1 OR granted_peer_agent_id=?1",
            params![id],
        )?;
    }
    if present.contains("db_agent_activity_summaries") {
        removed += conn.execute(
            "DELETE FROM db_agent_activity_summaries WHERE definition_id=?1",
            params![id],
        )?;
    }
    // v40, durable jekt: messages held for the deleted agent die with it —
    // their bodies must not linger, nor be delivered if the UID is ever
    // recreated (Codex P1 on #3632).
    if present.contains("db_jekt_held") {
        removed += conn.execute("DELETE FROM db_jekt_held WHERE target_uid=?1", params![id])?;
    }
    Ok(removed)
}
