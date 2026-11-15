// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A key a setting names as `vault:<entry id>` instead of holding it: srv's own reader of a service key
//! (voice input's Groq key, and the other settings keys the same way when they move). The value is read
//! inside srv at the moment of the call, goes into one outgoing request, and is never written to
//! settings.json, a log or the renderer. The spelling is the one `endpoint_key` resolves for an agent's
//! launch environment.
//!
//! **The grant lives on the entry, not in the settings file.** settings.json only names an entry; what lets
//! a setting read it is a grant kept in the Vault (`setting:<key>` in `db_vault_grants`, bound to the entry
//! it was given to), made by the Settings picker (the person's choice) or by AgentMux's own move of a plain
//! key. So a file edit that points a setting at another entry reads nothing: the entry has no grant for it.
//! The person sees it under the entry in the Vault and can take it back there. **One entry per setting:**
//! allowing an entry takes the setting's grant off every other entry, each removal recorded, so the file
//! alone can never point the setting back at an entry chosen before. A read records the use, once every
//! few minutes per entry, not once per call.

use super::{kinds, record, seal, NewEntry, Store};
use agentmux_common::time::now_ms;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// What a setting's value starts with when it names a Vault entry.
pub const PREFIX: &str = "vault:";
/// What a grant on an entry starts with when it lets a setting read the entry: `setting:<settings key>`.
pub const GRANT_PREFIX: &str = "setting:";
/// The settings that may name a Vault entry. A grant for any other name is refused.
pub const SETTINGS: [&str; 1] = ["voice:groqApiKey"];
/// Who is named in the Activity for what AgentMux does on its own, as opposed to the person.
pub const AGENTMUX: &str = "AgentMux";
/// The activity line for a use, listed with the other uses of a value.
pub const USED: &str = "read by AgentMux for a setting";
/// The activity line for the grant.
pub const ALLOWED: &str = "allowed for a setting";
/// The activity line for a grant taken off an entry because another entry was allowed for the setting.
pub const WITHDRAWN: &str = "no longer allowed for a setting";
/// The one answer for a missing entry, an entry of another kind, an entry the setting was not allowed to
/// read and a Vault that could not be opened, so a setting's error never says which entries exist; the
/// reason is in srv's log.
pub const NOT_AVAILABLE: &str = "the Vault entry this setting names is not there, is not an API key, or was not allowed for this setting";
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

/// The grant that lets `setting` read an entry.
pub fn grant_for(setting: &str) -> String {
    format!("{GRANT_PREFIX}{setting}")
}

/// The setting a grant on an entry is for, when it is one.
pub fn setting_of_grant(grant: &str) -> Option<&str> {
    grant.strip_prefix(GRANT_PREFIX)
}

/// Whether the Vault has an `api-key` entry with this id that `setting` may read. Opens nothing.
pub fn present(store: &Store, setting: &str, id: &str) -> bool {
    matches!(store.vault_get(id), Ok(Some(row)) if row.kind == "api-key") && store.vault_grant_check(id, &grant_for(setting)).unwrap_or(false)
}

/// Lets `setting` read entry `id`, recorded under `who`: the person's choice in Settings, or AgentMux's own
/// move of a key that sat in the file. Only an `api-key` entry, and only for a setting that is on the list.
/// One entry per setting: the setting's grant comes off every other entry, each removal recorded under `who`.
pub fn allow(store: &Store, id: &str, setting: &str, who: &str) -> Result<(), String> {
    if !SETTINGS.contains(&setting) {
        return Err(format!("{setting} is not a setting that can read a Vault key"));
    }
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    if row.kind != "api-key" {
        return Err(format!("{} is not an API key, so a setting can't read it", row.label));
    }
    let grant = grant_for(setting);
    store.vault_grant_add(&row.id, &grant, now_ms(), row.created_at).map_err(|e| e.to_string())?;
    record(store, &row.id, &row.label, who, ALLOWED, setting, "done");
    // The new grant is in before the old ones go: a failure part way leaves the setting readable, never unreadable.
    let mut others: Vec<String> = store.vault_grants().map_err(|e| e.to_string())?.into_iter().filter(|(entry, grants)| *entry != row.id && grants.contains(&grant)).map(|(entry, _)| entry).collect();
    others.sort();
    for other in others {
        if store.vault_grant_revoke(&other, &grant).map_err(|e| e.to_string())? {
            let label = store.vault_get(&other).ok().flatten().map(|r| r.label).unwrap_or_default();
            record(store, &other, &label, who, WITHDRAWN, setting, "done");
        }
    }
    Ok(())
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

/// The entry's value for `setting`, recorded as a use by `who` for `what` (`"voice input"`). Only an entry
/// that `setting` was allowed to read: the setting file's word alone is not enough.
pub fn read(store: &Store, setting: &str, id: &str, who: &str, what: &str) -> Result<zeroize::Zeroizing<String>, String> {
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
    if !store.vault_grant_check(&row.id, &grant_for(setting)).unwrap_or(false) {
        return Err(fail(format!("{} was not allowed for {setting}", row.label)));
    }
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
    // The grant first: a reference with no grant would read nothing, and a failed grant leaves no entry.
    if let Err(e) = allow(store, &id, setting, AGENTMUX) {
        let _ = store.vault_delete(&id);
        return Err(e);
    }
    let mut keys = serde_json::Map::new();
    keys.insert(setting.to_string(), serde_json::Value::String(reference(&id)));
    keys.insert(format!("{setting}Moved"), serde_json::Value::Bool(true));
    if let Err(e) = write(keys) {
        if let Err(del) = store.vault_delete(&id) {
            tracing::warn!(entry = %id, error = %del, "vault: a key kept for a setting could not be taken back after the setting failed to save");
        }
        return Err(e);
    }
    // AgentMux did this on its own at start, not the person.
    record(store, &id, label, AGENTMUX, "kept from a setting", setting, "done");
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTING: &str = "voice:groqApiKey";

    fn store() -> Store {
        seal::use_test_key();
        let store = Store::open_in_memory().unwrap();
        store.apply_identity_schema_for_tests().unwrap();
        store
    }

    /// A kept key the setting was allowed to read, as the Settings picker makes one.
    fn allowed_key(store: &Store, value: &str) -> String {
        let id = keep(store, "Groq key (voice input)", value).unwrap();
        allow(store, &id, SETTING, super::super::PERSON).unwrap();
        id
    }

    #[test]
    fn a_reference_is_vault_colon_and_an_id() {
        assert_eq!(entry_of("vault:abc-1"), Some("abc-1"));
        assert_eq!(entry_of("  vault: abc-1 "), Some("abc-1"));
        assert_eq!(entry_of("vault:"), None);
        assert_eq!(entry_of("gsk_plain_value"), None);
        assert_eq!(entry_of(&reference("e1")), Some("e1"));
        assert_eq!(setting_of_grant(&grant_for(SETTING)), Some(SETTING));
        assert_eq!(setting_of_grant("command:a:value"), None);
    }

    #[test]
    fn an_allowed_key_is_read_back_and_the_use_is_recorded_under_who_used_it() {
        let store = store();
        // Its own label: `due` keeps one process-wide clock per entry id, and ids come from kind and label,
        // so a test that reads a key under the shared label first would make this one's first read "not due".
        let id = keep(&store, "Groq key read-back test", "gsk_test_0123456789").unwrap();
        allow(&store, &id, SETTING, super::super::PERSON).unwrap();
        assert!(present(&store, SETTING, &id));
        assert_eq!(read(&store, SETTING, &id, "AgentMux voice input", "voice input").unwrap().as_str(), "gsk_test_0123456789");
        let log = crate::backend::vault::activity(&store, 10).unwrap();
        assert!(log.iter().any(|l| l.actor == "AgentMux voice input" && l.action == USED), "{:?}", log.iter().map(|l| (&l.actor, &l.action)).collect::<Vec<_>>());
        let again = crate::backend::vault::activity(&store, 10).unwrap().len();
        read(&store, SETTING, &id, "AgentMux voice input", "voice input").unwrap();
        assert_eq!(crate::backend::vault::activity(&store, 10).unwrap().len(), again, "a second call in the window writes no second line");
    }

    /// The file only names an entry: an entry the setting was not allowed to read is not read, whatever
    /// the file says (a tidying agent, a bad merge).
    #[test]
    fn an_entry_a_setting_was_not_allowed_to_read_is_not_read_whatever_the_file_names() {
        let store = store();
        let other = keep(&store, "A payment key", "sk_live_payment_0123").unwrap();
        assert!(!present(&store, SETTING, &other));
        assert_eq!(read(&store, SETTING, &other, "w", "x").unwrap_err(), NOT_AVAILABLE);
        // Allowed for one setting is not allowed for another, and a grant is for its own entry only.
        let mine = allowed_key(&store, "gsk_mine_0123456789");
        assert!(present(&store, SETTING, &mine) && !present(&store, "messaging:discord:token", &mine));
        assert_eq!(read(&store, SETTING, &other, "w", "x").unwrap_err(), NOT_AVAILABLE, "still refused beside an allowed one");
        // Taking the grant back in the Vault takes the reading with it.
        assert!(store.vault_grant_revoke(&mine, &grant_for(SETTING)).unwrap());
        assert_eq!(read(&store, SETTING, &mine, "w", "x").unwrap_err(), NOT_AVAILABLE);
        // Deleting an entry takes its grant, so an entry made again under the same id starts with none.
        let again = allowed_key(&store, "gsk_again_0123456789");
        crate::backend::vault::delete(&store, &again).unwrap();
        assert!(!store.vault_grant_check(&again, &grant_for(SETTING)).unwrap());
    }

    #[test]
    fn allowing_another_entry_takes_the_setting_off_the_old_one_and_records_it() {
        let store = store();
        let old = keep(&store, "Old groq key", "gsk_old_0123456789").unwrap();
        let new = keep(&store, "New groq key", "gsk_new_0123456789").unwrap();
        allow(&store, &old, SETTING, "you").unwrap();
        assert_eq!(read(&store, SETTING, &old, "w", "x").unwrap().as_str(), "gsk_old_0123456789");
        allow(&store, &new, SETTING, "you").unwrap();
        // The old entry is refused on read, as an entry that was never allowed; the new one reads.
        assert!(!present(&store, SETTING, &old) && present(&store, SETTING, &new));
        assert_eq!(read(&store, SETTING, &old, "w", "x").unwrap_err(), NOT_AVAILABLE);
        assert_eq!(read(&store, SETTING, &new, "w", "x").unwrap().as_str(), "gsk_new_0123456789");
        assert!(!store.vault_grant_check(&old, &grant_for(SETTING)).unwrap() && store.vault_grant_check(&new, &grant_for(SETTING)).unwrap());
        // The removal is a line in the Vault's history, under who chose and on the old entry.
        let log = crate::backend::vault::activity(&store, 20).unwrap();
        let line = log.iter().find(|l| l.action == WITHDRAWN).expect("the removal is recorded");
        assert_eq!((line.actor.as_str(), line.target.as_str(), line.entry_id.as_str()), ("you", SETTING, old.as_str()));
        assert_eq!(log.iter().filter(|l| l.action == WITHDRAWN).count(), 1);
        // Choosing the same entry again takes nothing off and records no removal.
        allow(&store, &new, SETTING, "you").unwrap();
        assert!(store.vault_grant_check(&new, &grant_for(SETTING)).unwrap());
        assert_eq!(crate::backend::vault::activity(&store, 20).unwrap().iter().filter(|l| l.action == WITHDRAWN).count(), 1);
    }

    #[test]
    fn a_grant_that_is_not_the_settings_is_left_alone_when_the_setting_moves_to_another_entry() {
        let store = store();
        let key = keep(&store, "A groq key", "gsk_a_0123456789").unwrap();
        let other = keep(&store, "Another key", "gsk_b_0123456789").unwrap();
        let created = store.vault_get(&other).unwrap().unwrap().created_at;
        store.vault_grant_add(&other, "command:deploy:value", now_ms(), created).unwrap();
        allow(&store, &other, SETTING, "you").unwrap();
        allow(&store, &key, SETTING, "you").unwrap();
        assert!(store.vault_grant_check(&other, "command:deploy:value").unwrap(), "the other grant stays");
        assert!(!store.vault_grant_check(&other, &grant_for(SETTING)).unwrap());
    }

    #[test]
    fn only_an_api_key_and_only_a_listed_setting_can_be_allowed() {
        let store = store();
        let note: NewEntry = serde_json::from_value(serde_json::json!({ "kind": "note", "label": "A note", "parts": { "text": "x" } })).unwrap();
        let note = crate::backend::vault::create(&store, note).unwrap();
        assert!(allow(&store, &note, SETTING, "you").unwrap_err().contains("not an API key"));
        let key = keep(&store, "A key", "gsk_x_0123456789").unwrap();
        assert!(allow(&store, &key, "term:fontsize", "you").unwrap_err().contains("not a setting"));
        assert!(allow(&store, "nope", SETTING, "you").is_err());
        assert!(!present(&store, SETTING, &key), "nothing was allowed");
    }

    #[test]
    fn a_grant_is_recorded_under_whoever_made_it() {
        let store = store();
        let id = keep(&store, "A key", "gsk_x_0123456789").unwrap();
        allow(&store, &id, SETTING, "you").unwrap();
        let log = crate::backend::vault::activity(&store, 10).unwrap();
        assert!(log.iter().any(|l| l.actor == "you" && l.action == ALLOWED && l.target == SETTING));
    }

    #[test]
    fn a_plain_key_moves_into_the_vault_with_its_grant_and_the_setting_becomes_a_reference() {
        let store = store();
        let mut written = None;
        let id = move_setting(&store, SETTING, "Groq key (voice input)", Some("  gsk_plain_0123  "), |keys| {
            written = Some(keys);
            Ok(())
        })
        .unwrap()
        .expect("moved");
        let keys = written.unwrap();
        assert_eq!(keys["voice:groqApiKey"], serde_json::json!(reference(&id)));
        assert_eq!(keys["voice:groqApiKeyMoved"], serde_json::json!(true));
        assert!(!serde_json::to_string(&keys).unwrap().contains("gsk_plain_0123"), "the value is not in what the settings file is given");
        assert_eq!(read(&store, SETTING, &id, "t", "t").unwrap().as_str(), "gsk_plain_0123", "trimmed, kept in the Vault, and readable: the move made the grant");
        // The start-up move is AgentMux's own doing, not the person's.
        let log = crate::backend::vault::activity(&store, 10).unwrap();
        let moved = log.iter().find(|l| l.action == "kept from a setting").expect("the move is recorded");
        assert_eq!(moved.actor, AGENTMUX);
        let allowed = log.iter().find(|l| l.action == ALLOWED).expect("so is the grant");
        assert_eq!(allowed.actor, AGENTMUX);
        assert!(log.iter().all(|l| l.actor != super::super::PERSON || l.action != "kept from a setting"));
    }

    #[test]
    fn nothing_moves_when_there_is_no_plain_key_and_a_reference_is_not_moved_again() {
        let store = store();
        for current in [None, Some(""), Some("   "), Some("vault:abc")] {
            let moved = move_setting(&store, SETTING, "Groq", current, |_| panic!("nothing to write")).unwrap();
            assert_eq!(moved, None, "{current:?}");
        }
        assert!(store.vault_list().unwrap().is_empty(), "no entry was made");
    }

    #[test]
    fn a_settings_write_that_fails_leaves_no_entry_behind_and_the_plain_key_in_place() {
        let store = store();
        let err = move_setting(&store, SETTING, "Groq", Some("gsk_x"), |_| Err("settings.json was not changed".into())).unwrap_err();
        assert!(err.contains("not changed"));
        assert!(store.vault_list().unwrap().is_empty(), "the entry made for it was taken back");
        assert!(store.vault_grants().unwrap().is_empty(), "and its grant went with it");
    }

    #[test]
    fn a_missing_entry_and_a_note_get_the_same_answer() {
        let store = store();
        let note: NewEntry = serde_json::from_value(serde_json::json!({ "kind": "note", "label": "A note", "parts": { "text": "x" } })).unwrap();
        let note = crate::backend::vault::create(&store, note).unwrap();
        assert!(!present(&store, SETTING, &note) && !present(&store, SETTING, "nope"));
        assert_eq!(read(&store, SETTING, "nope", "w", "x").unwrap_err(), NOT_AVAILABLE);
        assert_eq!(read(&store, SETTING, &note, "w", "x").unwrap_err(), NOT_AVAILABLE);
    }
}
