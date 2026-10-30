// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Typing into a field key by key (`Input.dispatchKeyEvent`), for fields that
//! ignore inserted text: input masks (SSN, phone, card and date formats) read
//! `keydown`, not the `input` event `Input.insertText` raises
//! (docs/specs/SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md
//! §4.4 step 3). The caller has already focused the field and run the secret
//! guard; this module only sends keys to wherever the focus is.

use serde_json::{json, Value};

use super::cdp::CdpSession;

/// The key events for one typed character: a `keyDown` that carries the text
/// (the page's editing then inserts it, as for a person) and the `keyUp`.
/// Letters and digits carry their key codes, since
/// mask libraries branch on them; any other character is sent as text only.
pub(super) fn key_events_for(c: char) -> Vec<Value> {
    let (key, code, vk, text) = match c {
        '0'..='9' => (c.to_string(), format!("Digit{c}"), c as i64, Some(c.to_string())),
        'a'..='z' | 'A'..='Z' => (c.to_string(), format!("Key{}", c.to_ascii_uppercase()), c.to_ascii_uppercase() as i64, Some(c.to_string())),
        ' ' => (" ".to_string(), "Space".to_string(), 32, Some(" ".to_string())),
        _ => (c.to_string(), String::new(), 0, Some(c.to_string())),
    };
    let mut down = json!({ "type": "keyDown", "key": key, "text": text, "unmodifiedText": text });
    let mut up = json!({ "type": "keyUp", "key": key });
    for event in [&mut down, &mut up] {
        if !code.is_empty() {
            event["code"] = json!(code);
        }
        if vk != 0 {
            event["windowsVirtualKeyCode"] = json!(vk);
            event["nativeVirtualKeyCode"] = json!(vk);
        }
    }
    vec![down, up]
}

/// The key events for pressing Backspace once.
fn backspace_events() -> Vec<Value> {
    ["keyDown", "keyUp"]
        .iter()
        .map(|t| json!({ "type": t, "key": "Backspace", "code": "Backspace", "windowsVirtualKeyCode": 8, "nativeVirtualKeyCode": 8 }))
        .collect()
}

async fn send(cdp: &mut CdpSession, events: Vec<Value>) -> Result<(), String> {
    for event in events {
        cdp.call("Input.dispatchKeyEvent", event).await.map_err(|e| format!("CDP Input.dispatchKeyEvent: {e}"))?;
    }
    Ok(())
}

/// Deletes the selection (the field's whole content, after `PREPARE_FILL`).
pub(super) async fn clear_selection(cdp: &mut CdpSession) -> Result<(), String> {
    send(cdp, backspace_events()).await
}

/// Text that can be typed key by key: no control characters. A line break
/// would be an Enter key, which submits the form a single-line field is in.
pub(super) fn is_typable(text: &str) -> bool {
    !text.chars().any(char::is_control)
}

/// Types `text` one key at a time where the focus is.
pub(super) async fn type_keys(cdp: &mut CdpSession, text: &str) -> Result<(), String> {
    if !is_typable(text) {
        return Err("the text has a line break or control character, which can't be typed key by key".to_string());
    }
    for c in text.chars() {
        send(cdp, key_events_for(c)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digit_is_a_key_down_with_its_text_and_code_then_a_key_up() {
        let events = key_events_for('7');
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "keyDown");
        assert_eq!(events[0]["text"], "7");
        assert_eq!(events[0]["code"], "Digit7");
        assert_eq!(events[0]["windowsVirtualKeyCode"], 55);
        assert_eq!(events[1]["type"], "keyUp");
        assert!(events[1].get("text").is_none());
    }

    #[test]
    fn a_letter_keeps_its_case_in_the_text_and_uses_the_capital_key_code() {
        let events = key_events_for('q');
        assert_eq!(events[0]["text"], "q");
        assert_eq!(events[0]["code"], "KeyQ");
        assert_eq!(events[0]["windowsVirtualKeyCode"], 81);
    }

    #[test]
    fn a_punctuation_mark_or_accented_letter_goes_as_text_only() {
        for c in ['-', 'é', '@'] {
            let events = key_events_for(c);
            assert_eq!(events[0]["text"], c.to_string());
            assert!(events[0].get("code").is_none());
            assert!(events[0].get("windowsVirtualKeyCode").is_none());
        }
    }

    #[test]
    fn text_with_a_line_break_or_control_character_is_not_typed() {
        assert!(is_typable("123-45-6789"));
        for bad in ["a\nb", "a\rb", "a\tb"] {
            assert!(!is_typable(bad));
        }
    }

    #[test]
    fn backspace_is_a_key_down_and_up_without_text() {
        let events = backspace_events();
        assert_eq!(events[0]["key"], "Backspace");
        assert!(events[0].get("text").is_none());
        assert_eq!(events[1]["type"], "keyUp");
    }
}
