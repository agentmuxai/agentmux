// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Codex App Server turn is given: its pictures, then its words
//! (docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.6).
//!
//! Every provider is handed the numbered `<attached_files>` list in the text. The App Server's model also sees
//! each picture, as a `localImage` input with the path of the file on this machine, ahead of the text input.

use serde::Serialize;

/// One item of a `turn/start` input.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(super) enum UserInput {
    /// A picture the model sees, by the path of a file on this machine.
    #[serde(rename = "localImage")]
    LocalImage { path: String },
    #[serde(rename = "text")]
    Text { text: String },
}

/// A turn's input: its pictures first, then its words, as the App Server takes them.
pub(super) fn turn_input(text: String, images: Vec<String>) -> Vec<UserInput> {
    images.into_iter().map(|path| UserInput::LocalImage { path }).chain(std::iter::once(UserInput::Text { text })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_turns_pictures_go_before_its_words_as_local_images_and_a_turn_without_any_is_unchanged() {
        let with = serde_json::to_value(turn_input("what is in these?".into(), vec!["/s/aa.send.png".into(), "/s/dd.send.jpg".into()])).unwrap();
        assert_eq!(
            with,
            json!([
                {"type": "localImage", "path": "/s/aa.send.png"},
                {"type": "localImage", "path": "/s/dd.send.jpg"},
                {"type": "text", "text": "what is in these?"},
            ])
        );
        let without = serde_json::to_value(turn_input("hello".into(), Vec::new())).unwrap();
        assert_eq!(without, json!([{"type": "text", "text": "hello"}]));
    }
}
