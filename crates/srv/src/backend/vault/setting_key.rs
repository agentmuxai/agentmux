// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A key a setting names as `vault:<entry id>` instead of holding it: srv's own reader of a service key
//! (voice input's Groq key, and the other settings keys the same way when they move). The value is read
//! inside srv at the moment of the call, goes into one outgoing request, and is never written to
//! settings.json, a log or the renderer. The person chose the entry in Settings, which is the grant, so
//! this asks no one; it records the use, once every few minutes per entry, not once per call. The spelling
//! is the one `endpoint_key` resolves for an agent's launch environment.

use super::{kinds, record, seal, NewEntry, Store};
use agentmux_common::time::now_ms;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// What a setting's value starts with when it names a Vault entry.
pub const PREFIX: &str = "vault:";
/// The activity line for a use, listed with the other uses of a value.
pub const USED: &str = "read by AgentMux for a setting";
/// The one answer for a missing entry, an entry of another kind and a Vault that could not be opened, so
/// a setting's error never says which entries exist; the reason is in srv's log.
pub const NOT_AVAILABLE: &str = "the Vault entry this setting names is not there, or is not an API key";
/// How often a use of the same entry is written to the activity log.
const RECORD_EVERY_MS: i64 = 10 * 60 * 1000;

/// The entry id a setting value names, when it is a `vault:<id>` reference.
pub fn entry_of(value: &str) -> Option<&str> {
    let id = value.trim().strip_prefix(PREFIX)?.trim();
    (!id.is_empty()).then_some(id)
}

/// The setting value that names entry `id`.
pub fn reference(id: &str) -> String {
    format!("{PREFIX}{id}")
}

/// Whether the Vault has an `api-key` entry with this id. Opens nothing.
pub fn present(store: &Store, id: &str) -> bool {
    matches!(store.vault_get(id), Ok(Some(row)) if row.kind == "api-key")
}

fn due(entry: &str, now: i64) -> bool {
    static LAST: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    let mut last = LAST.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    let at = last.entry(entry.to_string()).or_insert(i64::MIN);
    let due = now.saturating_sub(*at) >= RECORD_EVERY_MS;
    if due {
        *at = now;
    }
    due
}

/// The entry's value, recorded as a use by `who` for `what` (`"voice input"`).
pub fn read(store: &Store, id: &str, who: &str, what: &str) -> Result<zeroize::Zeroizing<String>, String> {
    let fail = |why: String| {
        tracing::info!(entry = %id, "a setting's Vault key could not be read: {why}");
        NOT_AVAILABLE.to_string()
    };
    let row = match store.vault_get(id) {
        Ok(Some(row)) if row.kind == "api-key" => row,
        Ok(Some(row)) => return Err(fail(format!("{} is not an api-key entry", row.label))),
        Ok(None) => return Err(fail(format!("the Vault has no entry {id}"))),
        Err(e) => return Err(fail(e.to_string())),
    };
    let kind = kinds::kind(&row.kind).ok_or_else(|| fail(format!("entry {} has an unknown kind", row.id)))?;
    let key = seal::key(false).map_err(fail)?;
    let parts = seal::open(&key, &row.id, &row.sealed).map_err(fail)?;
    let value = parts.get(kind.parts[0]).filter(|v| !v.trim().is_empty()).ok_or_else(|| fail(format!("{} has no value saved", row.label)))?;
    let value = zeroize::Zeroizing::new(value.to_string());
    if due(&row.id, now_ms()) {
        if let Err(e) = store.vault_touch(&row.id, now_ms()) {
            tracing::warn!(error = %e, "vault: couldn't note a setting key's use");
        }
        record(store, &row.id, &row.label, who, USED, what, "done");
    }
    Ok(value)
}

/// Keeps `value` as a new `api-key` entry labelled `label` in the Personal Vault and returns its id.
pub fn keep(store: &Store, label: &str, value: &str) -> Result<String, String> {
    let entry: NewEntry = serde_json::from_value(serde_json::json!({ "kind": "api-key", "label": label, "parts": { "value": value } })).map_err(|e| e.to_string())?;
    super::create(store, entry)
}

/// Moves a key that sits in settings.json as plain text into the Vault: keeps it as an `api-key` entry
/// labelled `label`, then has `write` replace the setting with its `vault:<id>` reference and set
/// `<setting>Moved` (so Settings can say so once). `current` is the setting's value as the file has it.
/// Returns the new entry's id, or `None` when there was nothing to move (unset, blank, or already a
/// reference). Whether it already ran is read from the value alone, never from a marker. When `write`
/// fails the new entry is deleted again and the plain key stays where it was, so a later start tries
/// again and nothing is kept twice.
pub fn move_setting(
    store: &Store,
    setting: &str,
    label: &str,
    current: Option<&str>,
    write: impl FnOnce(serde_json::Map<String, serde_json::Value>) -> Result<(), String>,
) -> Result<Option<String>, String> {
    let Some(plain) = current.map(str::trim).filter(|v| !v.is_empty() && entry_of(v).is_none()) else { return Ok(None) };
    let id = keep(store, label, plain)?;
    let mut keys = serde_json::Map::new();
    keys.insert(setting.to_string(), serde_json::Value::String(reference(&id)));
    keys.insert(format!("{setting}Moved"), serde_json::Value::Bool(true));
    if let Err(e) = write(keys) {
        if let Err(del) = store.vault_delete(&id) {
            tracing::warn!(entry = %id, error = %del, "vault: a key kept for a setting could not be taken back after the setting failed to save");
        }
        return Err(e);
    }
    record(store, &id, label, super::PERSON, "kept from a setting", setting, "done");
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        seal::use_test_key();
        let store = Store::open_in_memory().unwrap();
        store.apply_identity_schema_for_tests().unwrap();
        store
    }

    #[test]
    fn a_reference_is_vault_colon_and_an_id() {
        assert_eq!(entry_of("vault:abc-1"), Some("abc-1"));
        assert_eq!(entry_of("  vault: abc-1 "), Some("abc-1"));
        assert_eq!(entry_of("vault:"), None);
        assert_eq!(entry_of("gsk_plain_value"), None);
        assert_eq!(entry_of(&reference("e1")), Some("e1"));
    }

    #[test]
    fn a_kept_key_is_read_back_and_the_use_is_recorded_under_who_used_it() {
        let store = store();
        let id = keep(&store, "Groq key (voice input)", "gsk_test_0123456789").unwrap();
        assert!(present(&store, &id));
        assert_eq!(read(&store, &id, "AgentMux voice input", "voice input").unwrap().as_str(), "gsk_test_0123456789");
        let log = crate::backend::vault::activity(&store, 10).unwrap();
        assert!(log.iter().any(|l| l.actor == "AgentMux voice input" && l.action == USED), "{:?}", log.iter().map(|l| (&l.actor, &l.action)).collect::<Vec<_>>());
        let again = crate::backend::vault::activity(&store, 10).unwrap().len();
        read(&store, &id, "AgentMux voice input", "voice input").unwrap();
        assert_eq!(crate::backend::vault::activity(&store, 10).unwrap().len(), again, "a second call in the window writes no second line");
    }

    #[test]
    fn a_plain_key_moves_into_the_vault_and_the_setting_becomes_a_reference() {
        let store = store();
        let mut written = None;
        let id = move_setting(&store, "voice:groqApiKey", "Groq key (voice input)", Some("  gsk_plain_0123  "), |keys| {
            written = Some(keys);
            Ok(())
        })
        .unwrap()
        .expect("moved");
        let keys = written.unwrap();
        assert_eq!(keys["voice:groqApiKey"], serde_json::json!(reference(&id)));
        assert_eq!(keys["voice:groqApiKeyMoved"], serde_json::json!(true));
        assert!(!serde_json::to_string(&keys).unwrap().contains("gsk_plain_0123"), "the value is not in what the settings file is given");
        assert_eq!(read(&store, &id, "t", "t").unwrap().as_str(), "gsk_plain_0123", "trimmed, and kept in the Vault");
    }

    #[test]
    fn nothing_moves_when_there_is_no_plain_key_and_a_reference_is_not_moved_again() {
        let store = store();
        for current in [None, Some(""), Some("   "), Some("vault:abc")] {
            let moved = move_setting(&store, "voice:groqApiKey", "Groq", current, |_| panic!("nothing to write")).unwrap();
            assert_eq!(moved, None, "{current:?}");
        }
        assert!(store.vault_list().unwrap().is_empty(), "no entry was made");
    }

    #[test]
    fn a_settings_write_that_fails_leaves_no_entry_behind_and_the_plain_key_in_place() {
        let store = store();
        let err = move_setting(&store, "voice:groqApiKey", "Groq", Some("gsk_x"), |_| Err("settings.json was not changed".into())).unwrap_err();
        assert!(err.contains("not changed"));
        assert!(store.vault_list().unwrap().is_empty(), "the entry made for it was taken back");
    }

    #[test]
    fn a_missing_entry_and_a_note_get_the_same_answer() {
        let store = store();
        let note: NewEntry = serde_json::from_value(serde_json::json!({ "kind": "note", "label": "A note", "parts": { "text": "x" } })).unwrap();
        let note = crate::backend::vault::create(&store, note).unwrap();
        assert!(!present(&store, &note) && !present(&store, "nope"));
        assert_eq!(read(&store, "nope", "w", "x").unwrap_err(), NOT_AVAILABLE);
        assert_eq!(read(&store, &note, "w", "x").unwrap_err(), NOT_AVAILABLE);
    }
}
