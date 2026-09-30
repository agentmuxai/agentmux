// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! POSIX shell quoting, shared by srv and the CEF host.
//! (bashwrap's `hook.rs` has its own platform-aware variant that also leaves
//! safe strings bare; it is deliberately different.)

/// Wrap `s` in single quotes for bash/sh/zsh, escaping each `'` as `'\''`.
/// Always quotes, so the result is one word whatever `s` contains.
pub fn posix_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_every_input_as_one_word() {
        assert_eq!(posix_single_quote("plain"), "'plain'");
        assert_eq!(posix_single_quote(""), "''");
        assert_eq!(posix_single_quote("a b $HOME `x`"), "'a b $HOME `x`'");
        assert_eq!(posix_single_quote("it's"), "'it'\\''s'");
    }
}
