// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A value an agent had the Vault read is not trusted until the person has seen it
//! (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md R11).
//!
//! An `import` reads with the host's own AWS setup and a `capture` reads a page or a command's output, and neither asks the
//! person first. So the entry they write is marked unseen: a *use* of it never goes ahead on silence and offers no *Always*,
//! and the card says who set the value and when. The mark goes when the person marks the change seen, or answers a use of
//! the entry with a yes. Marking also drops the entry's *Always* grants, which skip the card on a use. A value the Vault made (`generate`, `rotate`) is not marked: nobody had it before.

use agentmux_common::time::now_ms;

use super::uses::UseRequest;
use crate::backend::storage::store::Store;

/// Marks the value of `entry_id` as set by `agent` and not yet seen by the person. A failure to mark is logged: the write
/// it follows is already made, and the history line records it.
pub fn mark(store: &Store, entry_id: &str, agent: &str) {
    let marked = store.vault_get(entry_id).ok().flatten().map(|row| store.vault_unseen_mark(entry_id, row.created_at, agent, now_ms()));
    if !matches!(marked, Some(Ok(()))) {
        tracing::warn!(entry = entry_id, "vault: couldn't mark an agent's value as unseen");
    }
}

/// The person has seen the entry's value, or answered a use of it: it is trusted as any other.
pub fn clear(store: &Store, entry_id: &str) -> bool {
    store.vault_unseen_clear(entry_id).unwrap_or(false)
}

/// Tells `request` that its entry's value was set by an agent and not seen, when it was: who and when. A use of it then never
/// goes ahead on silence and offers no *Always*. Leaves a use of an entry the person has seen as it is.
pub fn apply(store: &Store, request: &mut UseRequest) {
    if request.entry_id.is_empty() {
        return;
    }
    if let Ok(Some(unseen)) = store.vault_unseen_get(&request.entry_id) {
        request.set_by = unseen.by_agent;
        request.set_ms = u64::try_from(unseen.at).unwrap_or(0);
        request.offer_always = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::vault::uses::goes_ahead;

    fn store() -> Store {
        crate::backend::vault::seal::use_test_key();
        let store = Store::open_in_memory().unwrap();
        store.apply_identity_schema_for_tests().unwrap();
        store
    }

    fn entry(store: &Store, id: &str) {
        let entry: crate::backend::vault::NewEntry = serde_json::from_value(serde_json::json!({ "kind": "api-key", "label": "Mail key", "parts": { "value": "sk_test_invented_value" } })).unwrap();
        assert_eq!(crate::backend::vault::create(store, entry).unwrap(), id);
    }

    fn use_of(id: &str) -> UseRequest {
        UseRequest { entry_id: id.into(), kind: "api-key".into(), offer_always: true, ..Default::default() }
    }

    #[test]
    fn an_unseen_value_never_goes_ahead_on_silence_and_offers_no_always_until_it_is_seen() {
        let store = store();
        entry(&store, "api-key-mail-key");
        let mut plain = use_of("api-key-mail-key");
        apply(&store, &mut plain);
        assert!(goes_ahead(&plain) && plain.offer_always && plain.set_by.is_empty(), "a value nobody marked is used as before");
        mark(&store, "api-key-mail-key", "builder");
        let mut unseen = use_of("api-key-mail-key");
        apply(&store, &mut unseen);
        assert_eq!(unseen.set_by, "builder");
        assert!(unseen.set_ms > 0 && !unseen.offer_always && !goes_ahead(&unseen), "silence is a no, and there is no Always");
        assert!(clear(&store, "api-key-mail-key"));
        let mut seen = use_of("api-key-mail-key");
        apply(&store, &mut seen);
        assert!(goes_ahead(&seen), "seen, it is used as any other");
        assert!(!clear(&store, "api-key-mail-key"), "nothing left to clear");
    }

    #[test]
    fn marking_a_value_unseen_drops_the_always_grants_given_for_the_value_the_person_saw() {
        let store = store();
        entry(&store, "api-key-mail-key");
        let created = store.vault_get("api-key-mail-key").unwrap().unwrap().created_at;
        store.vault_grant_add("api-key-mail-key", "https://mail.example", 1, created).unwrap();
        assert!(store.vault_grant_check("api-key-mail-key", "https://mail.example").unwrap());
        mark(&store, "api-key-mail-key", "builder");
        assert!(!store.vault_grant_check("api-key-mail-key", "https://mail.example").unwrap(), "a use on a grant skips the card, so the grant goes");
    }

    #[test]
    fn a_mark_belongs_to_the_entry_it_was_made_on_and_not_to_one_made_again_under_its_id() {
        let store = store();
        entry(&store, "api-key-mail-key");
        mark(&store, "api-key-mail-key", "builder");
        store.vault_delete("api-key-mail-key").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        entry(&store, "api-key-mail-key");
        let mut again = use_of("api-key-mail-key");
        apply(&store, &mut again);
        assert!(again.set_by.is_empty(), "the new entry was made by the person, not by that agent");
    }
}
