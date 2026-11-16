// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A key an agent's launch environment names as `ANTHROPIC_AUTH_TOKEN=vault:<entry id>`
//! (`backend::agent_env`): the model endpoint an agent was pointed at wants one. Unlike a model
//! route's key (`route_key`), which goes only into srv's outgoing request, this value is put in
//! the agent's own process environment, where the agent can read it. So the entry must be one the
//! person has opened to that agent (the Vault's agents setting, `vaults::entry_usable`), and every
//! launch it is put in is recorded under the agent's name.

use super::{kinds, record, seal, vaults, Store};

/// The activity line for a use, listed with the other uses of a value.
pub const USED: &str = "put in an agent's launch environment";

/// What a caller is told when an entry is not one an agent may have: one answer for a missing entry, a
/// wrong kind and a closed Vault, so a refusal never says which entries exist or what they are called
/// (`vaults::resolve` keeps the same rule). The reason is in srv's log.
pub const NOT_AVAILABLE: &str = "that Vault key is not available to this agent";

/// Why `entry_id` is not an entry `agent_id` may have put in its environment, if it is not: the one
/// answer [`NOT_AVAILABLE`]. Reads no value. Used when the reference is written (for the agent that
/// writes it and the agent it is written for) and again at every launch.
pub fn problem(store: &Store, entry_id: &str, agent_id: &str) -> Option<String> {
    reason(store, entry_id, agent_id).map(|why| {
        tracing::info!(entry = %entry_id, agent = %agent_id, "a Vault key was refused for an agent's environment: {why}");
        NOT_AVAILABLE.to_string()
    })
}

fn reason(store: &Store, entry_id: &str, agent_id: &str) -> Option<String> {
    let row = match store.vault_get(entry_id) {
        Ok(Some(row)) => row,
        Ok(None) => return Some(format!("the Vault has no entry {entry_id}")),
        Err(e) => return Some(e.to_string()),
    };
    if row.kind != "api-key" {
        return Some(format!("{} is not an api-key entry", row.label));
    }
    match vaults::entry_usable(store, entry_id, Some(agent_id)) {
        Ok(true) => None,
        Ok(false) => Some(format!("{} is in a Vault the person has not opened to this agent", row.label)),
        Err(e) => Some(e),
    }
}

/// The entry's value for `agent_id`'s launch, recorded as a use by `agent_slug`.
pub fn for_launch(store: &Store, entry_id: &str, agent_id: &str, agent_slug: &str) -> Result<zeroize::Zeroizing<String>, String> {
    if let Some(why) = problem(store, entry_id, agent_id) {
        return Err(why);
    }
    let row = store.vault_get(entry_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("the Vault has no entry {entry_id}"))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {} has an unknown kind", row.id))?;
    let key = seal::key(false)?;
    let parts = seal::open(&key, &row.id, &row.sealed)?;
    let value = parts.get(kind.parts[0]).ok_or_else(|| format!("{} has no value saved", row.label))?;
    let value = zeroize::Zeroizing::new(value.to_string());
    record(store, &row.id, &row.label, agent_slug, USED, "", "done");
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(kind: &str, label: &str, part: &str) -> (Store, String) {
        crate::backend::vault::seal::use_test_key();
        let store = Store::open_in_memory().unwrap();
        store.apply_identity_schema_for_tests().unwrap();
        let entry: crate::backend::vault::NewEntry =
            serde_json::from_value(serde_json::json!({ "kind": kind, "label": label, "parts": { part: "sk-test-0123456789" } })).unwrap();
        let id = crate::backend::vault::create(&store, entry).unwrap();
        (store, id)
    }

    #[test]
    fn a_missing_entry_and_a_note_are_refused() {
        let (store, note) = store_with("note", "A note", "text");
        assert!(reason(&store, "nope", "a1").unwrap().contains("no entry"));
        assert!(reason(&store, &note, "a1").unwrap().contains("not an api-key"));
        // The caller hears the same thing for both, and for a closed Vault.
        assert_eq!(problem(&store, "nope", "a1").as_deref(), Some(NOT_AVAILABLE));
        assert_eq!(problem(&store, &note, "a1").as_deref(), Some(NOT_AVAILABLE));
        assert!(for_launch(&store, &note, "a1", "lolly").is_err());
    }

    #[test]
    fn an_api_key_in_a_vault_open_to_all_agents_is_usable() {
        let (store, id) = store_with("api-key", "Local LLM key", "value");
        assert_eq!(problem(&store, &id, "a1"), None);
    }

    #[test]
    fn a_vault_the_person_opened_to_other_agents_is_not_usable_by_this_one() {
        let (store, id) = store_with("api-key", "Local LLM key", "value");
        assert!(store.vault_set("personal", "Personal", "named", "[\"a2\"]").unwrap());
        assert!(reason(&store, &id, "a1").unwrap().contains("not opened to this agent"));
        assert_eq!(problem(&store, &id, "a1").as_deref(), Some(NOT_AVAILABLE));
        assert_eq!(problem(&store, &id, "a2"), None);
        assert!(for_launch(&store, &id, "a1", "lolly").is_err(), "no value without the person's opening");
    }

    #[test]
    fn the_value_is_read_and_recorded_under_the_agents_name() {
        let (store, id) = store_with("api-key", "Local LLM key", "value");
        let value = for_launch(&store, &id, "a1", "lolly").unwrap();
        assert_eq!(value.as_str(), "sk-test-0123456789");
        let log = crate::backend::vault::activity(&store, 10).unwrap();
        assert!(log.iter().any(|l| l.actor == "lolly" && l.action == USED));
    }
}
