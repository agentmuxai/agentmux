// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Redact secret settings keys from the full-config JSON before it is sent to
//! any renderer/frontend connection.
//!
//! Found in PR #2751 review: `voice:groqApiKey`'s own doc comment claims
//! "Read server-side only, never sent to the renderer," but `get_full_config()`'s
//! outbound JSON (the initial `config` event, the `getfullconfig` RPC, and the
//! post-`setconfig` broadcast — all three call sites in `server/websocket.rs`)
//! was never actually filtered, so the real plaintext key has always reached
//! every renderer connection's JS memory. PR #2751 introduced a masked-dot
//! Settings UI for this key (`MaskedKeyField`) that visually implies the
//! opposite. This module makes the doc comment's claim actually true.
//!
//! The renderer's UI only needs to know a value IS set, never what it is, so
//! redaction replaces the real value with a fixed non-empty placeholder
//! (distinguishable from "unset") rather than merely blanking it. The write
//! path (`setconfig` → `merge_settings_to_disk`/`merge_settings_into_current`)
//! is untouched — this only filters the server → renderer read direction.

use serde_json::Value;

/// Settings keys whose values must never reach a renderer connection.
/// Extend this list as more secret-shaped fields get a Settings UI (e.g. the
/// messaging-bridge bot tokens proposed in
/// docs/specs/SPEC_SETTINGS_MESSAGING_BRIDGES_SECTION_2026_08_22.md).
const REDACTED_SETTINGS_KEYS: &[&str] = &["voice:groqApiKey"];

/// Placeholder written in place of a redacted value. Any renderer code that
/// merely checks "is a key set" (e.g. `MaskedKeyField`'s locked-state gate)
/// keeps working unchanged; nothing should ever display this string as if it
/// were the real value.
const REDACTED_PLACEHOLDER: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// Redact known-secret keys in-place on an already-serialized full-config
/// JSON value. Call this on every path that sends config to a renderer;
/// never on the in-memory `ConfigState` state or the on-disk file.
pub fn redact_full_config_for_renderer(v: &mut Value) {
    let Some(settings) = v.get_mut("settings").and_then(|s| s.as_object_mut()) else {
        return;
    };
    for key in REDACTED_SETTINGS_KEYS {
        let is_set = settings
            .get(*key)
            .map(|existing| match existing {
                Value::String(s) => !s.is_empty(),
                Value::Null => false,
                _ => true,
            })
            .unwrap_or(false);
        if is_set {
            settings.insert((*key).to_string(), Value::String(REDACTED_PLACEHOLDER.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_a_set_secret_key() {
        let mut v = json!({ "settings": { "voice:groqApiKey": "sk-real-secret-value" } });
        redact_full_config_for_renderer(&mut v);
        assert_eq!(v["settings"]["voice:groqApiKey"], REDACTED_PLACEHOLDER);
        assert_ne!(v["settings"]["voice:groqApiKey"], "sk-real-secret-value");
    }

    #[test]
    fn leaves_an_unset_secret_key_alone() {
        let mut v = json!({ "settings": { "term:fontsize": 14 } });
        redact_full_config_for_renderer(&mut v);
        assert!(v["settings"].get("voice:groqApiKey").is_none());
    }

    #[test]
    fn does_not_touch_unrelated_settings() {
        let mut v = json!({ "settings": { "voice:groqApiKey": "sk-real", "term:fontsize": 14 } });
        redact_full_config_for_renderer(&mut v);
        assert_eq!(v["settings"]["term:fontsize"], 14);
    }

    #[test]
    fn handles_missing_settings_object_gracefully() {
        let mut v = json!({ "mimetypes": {} });
        redact_full_config_for_renderer(&mut v); // must not panic
        assert_eq!(v, json!({ "mimetypes": {} }));
    }

    #[test]
    fn treats_empty_string_as_unset() {
        let mut v = json!({ "settings": { "voice:groqApiKey": "" } });
        redact_full_config_for_renderer(&mut v);
        assert_eq!(v["settings"]["voice:groqApiKey"], "");
    }
}
