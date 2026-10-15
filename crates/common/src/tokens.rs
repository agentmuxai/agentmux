// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The token estimate every AgentMux surface shows, labelled "~N tok (est.)":
//! ⌈Unicode scalar values ÷ 4⌉. The frontend's `estimateTokenCount`
//! (`frontend/util/format-count.ts`) counts the same way, and both test the
//! same vector, so the Memory pane, the context delivery card and the
//! reinjection card agree to the token (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md
//! §4.1).

/// Estimated tokens in `text`.
pub fn estimate_tokens(text: &str) -> u64 {
    tokens_for_chars(text.chars().count() as u64)
}

/// Estimated tokens in a text of `chars` Unicode scalar values.
pub fn tokens_for_chars(chars: u64) -> u64 {
    chars.div_ceil(4)
}

/// Lines in `text`, as `str::lines` counts them: a final line needs no
/// newline, and an empty text has none.
pub fn count_lines(text: &str) -> u64 {
    text.lines().count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vector `frontend/util/format-count.test.ts` checks too.
    pub(crate) const VECTOR: &[(&str, u64)] = &[
        ("", 0),
        ("abcd", 1),
        ("abcde", 2),
        ("héllo", 2),
        ("日本語テキスト", 2),
        ("🙂🙂🙂🙂", 1),
        ("e\u{301}e\u{301}", 1),
        ("# Memory\n\nOne line.\n", 5),
    ];

    #[test]
    fn the_estimate_is_scalar_values_over_four_rounded_up() {
        for (text, tokens) in VECTOR {
            assert_eq!(estimate_tokens(text), *tokens, "{text:?}");
        }
    }

    #[test]
    fn lines_count_like_str_lines() {
        assert_eq!(count_lines(""), 0);
        assert_eq!(count_lines("a"), 1);
        assert_eq!(count_lines("a\n"), 1);
        assert_eq!(count_lines("a\nb"), 2);
        assert_eq!(count_lines("a\r\nb\n"), 2);
        assert_eq!(count_lines("\n"), 1);
    }
}
