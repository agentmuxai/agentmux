// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Constant-time comparison for secrets presented by a caller.
//!
//! `==` on strings returns at the first differing byte, so how long a
//! rejection takes tells a caller how many leading bytes of its guess were
//! right. Every check of a caller-supplied secret against the real one (srv's
//! auth key and LAN key, the CEF host's IPC token, a re-registering host's
//! token, a webhook verify token) goes through [`secret_eq`] instead.
//!
//! The length is not hidden: a length mismatch returns early. Every secret
//! compared here has a fixed, public length (UUIDs, fixed-size tokens), so
//! the length reveals nothing.

/// True when `supplied` equals `expected`, in time that depends only on the
/// length, never on where the first difference is.
pub fn secret_eq(supplied: &[u8], expected: &[u8]) -> bool {
    if supplied.len() != expected.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in supplied.iter().zip(expected.iter()) {
        diff |= x ^ y;
    }
    // Keep the optimizer from turning the loop back into an early exit.
    std::hint::black_box(diff) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_secrets_match() {
        assert!(secret_eq(b"3f2b9c1e-auth-key", b"3f2b9c1e-auth-key"));
    }

    #[test]
    fn same_length_different_secrets_do_not_match() {
        assert!(!secret_eq(b"3f2b9c1e-auth-key", b"3f2b9c1e-auth-kez"));
        assert!(!secret_eq(b"Xf2b9c1e-auth-key", b"3f2b9c1e-auth-key"));
    }

    #[test]
    fn different_lengths_do_not_match() {
        assert!(!secret_eq(b"short", b"a-much-longer-secret"));
        assert!(!secret_eq(b"3f2b9c1e-auth-key", b"3f2b9c1e-auth-key-"));
    }

    #[test]
    fn empty_matches_only_empty() {
        assert!(secret_eq(b"", b""));
        assert!(!secret_eq(b"", b"x"));
    }
}
