// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `read_back.js`'s fallback `text` (a link, or anything else `act::READ_BACK` has nothing else to report
//! on), shortened for the one place it can run long: `act_route`, after masking (#7407).

use serde_json::{json, Value};

/// The length the text is kept to, once it is masked. Never before: a secret crossing this cut would leave
/// a fragment the host's masking (`hidden_secrets::scrub_value`, `shown_once`) no longer recognizes as the
/// complete token, where the field used to carry no text at all.
const KEEP: usize = 200;

/// Shorten `after["text"]`, if there is one, to [`KEEP`] characters. Call only after masking; no other
/// field `act`'s read-back ever sets holds free-form page text this long.
pub(super) fn shorten(mut after: Value) -> Value {
    let shortened = after.get("text").and_then(Value::as_str).and_then(|text| {
        let (cut, _) = text.char_indices().nth(KEEP)?;
        Some(format!("{}…", &text[..past_a_split_marker(text, cut)]))
    });
    if let (Some(shortened), Some(o)) = (shortened, after.as_object_mut()) {
        o.insert("text".to_string(), json!(shortened));
    }
    after
}

/// `cut` (a byte offset into `text`), moved to the end of the host's own masking placeholder (`WITHHELD`,
/// `UNREAD`) when it would otherwise land inside one: a marker reads whole, or not at all, never as a
/// meaningless fragment of itself ("[hidden t" instead of the sentence it stands for).
fn past_a_split_marker(text: &str, cut: usize) -> usize {
    use agentmux_common::secret_shape::{UNREAD, WITHHELD};
    [WITHHELD, UNREAD]
        .into_iter()
        .flat_map(|marker| text.match_indices(marker).map(move |(start, _)| (start, start + marker.len())))
        .find(|&(start, end)| start < cut && cut < end)
        .map_or(cut, |(_, end)| end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_masked_before_it_is_shortened_so_no_secret_fragment_survives_the_cut() {
        // A card's live key straddles the old, unsafe 200-character cut point (#7407): cutting here first
        // would have kept only a prefix of it ("sk_live_a..."), a fragment no masker recognizes.
        let padding = "x".repeat(190);
        let token = "sk_live_abcdefgh12";
        let text = format!("{padding} {token} more text after the secret, well past the cut");
        let token_start = text.find(token).unwrap();
        assert!(token_start < 200 && token_start + token.len() > 200, "the token must straddle the old cut point");
        let after = shorten(super::super::hidden_secrets::scrub_answer(json!({ "text": text })));
        let got = after["text"].as_str().unwrap();
        assert!(!got.contains("sk_live"), "no whole or partial secret: {got}");
        assert_eq!(got, format!("{padding} {}…", agentmux_common::secret_shape::WITHHELD));
    }

    #[test]
    fn a_masking_placeholder_itself_is_never_split_even_when_it_straddles_the_cut() {
        // Masking runs first, so the placeholder (not a secret, but a sentence) can itself now land across
        // the cut point; it is kept whole rather than shown as a meaningless fragment of itself.
        let text = format!("{} {}", "x".repeat(190), agentmux_common::secret_shape::WITHHELD);
        let after = shorten(json!({ "text": text }));
        assert!(after["text"].as_str().unwrap().ends_with(&format!("{}…", agentmux_common::secret_shape::WITHHELD)));
    }

    #[test]
    fn ordinary_long_text_with_no_placeholder_is_still_cut_to_the_cap() {
        let after = shorten(json!({ "text": "y".repeat(500) }));
        let got = after["text"].as_str().unwrap();
        assert_eq!(got.chars().count(), KEEP + 1);
        assert!(got.ends_with('…'), "{got}");
    }

    #[test]
    fn text_short_enough_is_left_exactly_as_it_is() {
        let after = shorten(json!({ "tag": "a", "text": "View invoice" }));
        assert_eq!(after, json!({ "tag": "a", "text": "View invoice" }));
    }

    #[test]
    fn text_missing_is_fine_fill_select_and_check_never_set_it() {
        let after = shorten(json!({ "tag": "input", "value": "x".repeat(400) }));
        assert_eq!(after["value"].as_str().unwrap().len(), 400, "only `text` is ever shortened here");
    }
}
