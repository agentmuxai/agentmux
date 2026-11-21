// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A short name for a model id, for a person or an agent reading a list: `claude-sonnet-5-5` is
//! "Sonnet 5.5", `claude-opus-4-1-20250805` is "Opus 4.1", `sonnet[1m]` is "Sonnet (1M)". An id this does not
//! recognise is returned as it is, so a label is never a guess.

/// The label of `model`: see the module doc.
pub fn label_of(model: &str) -> String {
    let id = model.trim();
    let (id, long) = match id.strip_suffix("[1m]") {
        Some(rest) => (rest, true),
        None => (id, false),
    };
    let label = claude_label(id).unwrap_or_else(|| id.to_string());
    if long {
        format!("{label} (1M)")
    } else {
        label
    }
}

/// `claude-<family>-<major>-<minor>[-<date>]` (or the older `claude-<major>-<minor>-<family>`), or a bare family alias.
fn claude_label(id: &str) -> Option<String> {
    let lower = id.to_ascii_lowercase();
    let rest = lower.strip_prefix("claude-").unwrap_or(&lower);
    let mut words: Vec<String> = Vec::new();
    let mut numbers: Vec<&str> = Vec::new();
    for token in rest.split('-') {
        match token {
            t if t.len() == 8 && t.bytes().all(|b| b.is_ascii_digit()) => {} // the snapshot date
            t if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) => numbers.push(t),
            t if !t.is_empty() && t.bytes().all(|b| b.is_ascii_lowercase()) => words.push(capitalised(t)),
            _ => return None,
        }
    }
    let known = ["Sonnet", "Opus", "Haiku", "Fable"];
    if words.is_empty() || !words.iter().all(|w| known.contains(&w.as_str())) {
        return None;
    }
    if rest == lower && numbers.is_empty() && words.len() != 1 {
        return None; // a bare word list that is not an alias: leave it
    }
    let mut label = words.join(" ");
    if !numbers.is_empty() {
        label.push(' ');
        label.push_str(&numbers.join("."));
    }
    Some(label)
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |c| c.to_ascii_uppercase().to_string() + chars.as_str())
}

#[cfg(test)]
mod tests {
    use super::label_of;

    #[test]
    fn a_claude_id_reads_as_its_family_and_version() {
        assert_eq!(label_of("claude-sonnet-5-5"), "Sonnet 5.5");
        assert_eq!(label_of("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(label_of("claude-haiku-5-5"), "Haiku 5.5");
        assert_eq!(label_of("claude-fable-5-2"), "Fable 5.2");
        assert_eq!(label_of("claude-opus-4-1-20250805"), "Opus 4.1", "the snapshot date is dropped");
        assert_eq!(label_of("claude-3-5-sonnet-20241022"), "Sonnet 3.5", "the older order");
    }

    #[test]
    fn an_alias_and_the_long_context_mark_read_plainly() {
        assert_eq!(label_of("sonnet"), "Sonnet");
        assert_eq!(label_of("opus"), "Opus");
        assert_eq!(label_of("sonnet[1m]"), "Sonnet (1M)");
        assert_eq!(label_of("claude-sonnet-5-5[1m]"), "Sonnet 5.5 (1M)");
    }

    #[test]
    fn an_id_it_does_not_recognise_is_returned_as_it_is() {
        assert_eq!(label_of("gpt-5.1-codex"), "gpt-5.1-codex");
        assert_eq!(label_of("deepseek-chat"), "deepseek-chat");
        assert_eq!(label_of(""), "");
        assert_eq!(label_of("claude-mystery-9"), "claude-mystery-9");
    }
}
