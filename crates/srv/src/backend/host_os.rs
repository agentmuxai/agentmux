// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The operating system an instance runs on, as the Swarm shows it next to a
//! remote machine (docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §3).
//!
//! It is one lowercase string, `std::env::consts::OS` (`windows`, `macos`,
//! `linux`, ...), advertised by each instance in its LAN record. A peer's value
//! is self-reported and used for display only, so what arrives from the network
//! is accepted only in the plain shape below and is never read for a decision.

/// The longest value accepted from a peer.
pub const MAX_OS_LEN: usize = 16;

/// This instance's own value.
pub fn local_os() -> String {
    std::env::consts::OS.to_string()
}

/// A peer's value, or `None` if it isn't a plain lowercase token. Anything else
/// (empty, long, upper case, spaces, markup) is dropped, so the frontend never
/// receives text it didn't expect.
pub fn sanitize_os(raw: &str) -> Option<String> {
    let ok = !raw.is_empty()
        && raw.len() <= MAX_OS_LEN
        && raw
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    ok.then(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_platforms_pass_unchanged() {
        for os in ["windows", "macos", "linux"] {
            assert_eq!(sanitize_os(os).as_deref(), Some(os));
        }
    }

    #[test]
    fn anything_that_is_not_a_plain_lowercase_token_is_dropped() {
        assert_eq!(sanitize_os(""), None);
        assert_eq!(sanitize_os("Windows"), None);
        assert_eq!(sanitize_os("win dows"), None);
        assert_eq!(sanitize_os("<b>linux</b>"), None);
        assert_eq!(sanitize_os("linux\n"), None);
        assert_eq!(sanitize_os("../etc"), None);
        assert_eq!(sanitize_os(&"a".repeat(MAX_OS_LEN + 1)), None);
        assert_eq!(sanitize_os(&"a".repeat(MAX_OS_LEN)).as_deref(), Some("a".repeat(MAX_OS_LEN).as_str()));
    }

    #[test]
    fn this_instance_advertises_a_value_it_would_itself_accept() {
        assert!(sanitize_os(&local_os()).is_some());
    }
}
