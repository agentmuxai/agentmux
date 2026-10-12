// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Name → slug / id / path-component rules, one named function per rule.
//!
//! These rules differ ON PURPOSE (a display slug and a path component are
//! different things), so they are not merged. They used to be written inline
//! at each call site, which is how the same rule was derived two different
//! ways (PR #2901: a work dir computed two ways broke Personal Bundle) and how
//! a key purge that must reproduce every naming rule ever used missed some
//! (#3633). Here each rule has a name, one body, and a table test that runs
//! every rule over the same inputs so the differences are stated, not
//! discovered (docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §5.1 #2).
//!
//! Changing a rule's output moves existing agents' directories, registry files
//! or signing-key names. Don't.

/// The longest agent id the registry accepts.
pub const AGENT_ID_MAX_LEN: usize = 64;

/// A valid agent id: 1 to 64 ASCII alphanumerics, `-` or `_`. What the
/// registry, MuxBus and a message's target accept.
pub fn is_agent_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= AGENT_ID_MAX_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// ASCII alphanumerics, `-` and `_` kept, everything else `-`; runs of dashes
/// collapsed and trimmed; at most 64 chars; `"agent"` if nothing is left.
fn dashed(name: &str) -> String {
    let filtered: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let collapsed: String = filtered
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let trimmed: String = collapsed.chars().take(AGENT_ID_MAX_LEN).collect();
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed
    }
}

/// A definition's slug: lowercase; ASCII alphanumerics, `-` and `_` kept,
/// everything else `-`; runs of dashes collapsed and trimmed; at most 64
/// chars; `"agent"` if nothing is left.
pub fn definition_slug(name: &str) -> String {
    dashed(&name.to_lowercase())
}

/// The name an agent is registered and addressed by: `AGENTMUX_AGENT_ID`, the
/// registry's display binding, a message's target. A valid agent id stays as
/// it is, natural casing included (`Agent2`, which PR titles and git authors
/// carry); anything else gets `definition_slug`'s rule without lowercasing, so
/// a fork's automatic name `Loap #3` routes as `Loap-3` instead of being
/// refused (REPORT_FORK_AGENT_ISOLATION_2026_10_12.md F3).
pub fn agent_route_name(name: &str) -> String {
    if is_agent_id(name) {
        name.to_string()
    } else {
        dashed(name)
    }
}

/// A path component, and `agent.open`'s id for a definition with no slug:
/// lowercase; Unicode alphanumerics, `-` and `_` kept, everything else `-`;
/// no collapsing, no cap. `Zed Bot` → `zed-bot`, `Ä` → `ä`.
pub fn path_slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// A registry file stem: `path_slug`'s rule, but everything else becomes `_`,
/// so it can never introduce a path separator or `..`.
pub fn file_stem(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The frontend's id for a launch with no slug: JavaScript's
/// `name.toLowerCase().replace(/[^a-z0-9-_]/g, "-")`. ASCII only, unlike
/// `path_slug`; no collapsing. The regex has no `u` flag, so it replaces each
/// UTF-16 **code unit**: a character outside the BMP becomes two dashes
/// (`Agent 🚀` → `agent---`), which signing-key names depend on (Codex P1 on
/// #3633).
pub fn js_ascii_slug(name: &str) -> String {
    let mut id = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
            id.push(c);
        } else {
            id.extend(std::iter::repeat('-').take(c.len_utf16()));
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every rule over the same inputs: the differences, stated.
    #[test]
    fn the_rules_side_by_side() {
        // (input, definition_slug, path_slug, file_stem, js_ascii_slug, agent_route_name)
        let table: &[(&str, &str, &str, &str, &str, &str)] = &[
            ("Zed Bot", "zed-bot", "zed-bot", "zed_bot", "zed-bot", "Zed-Bot"),
            ("Ä", "agent", "ä", "ä", "-", "agent"),
            ("Agent 🚀", "agent", "agent--", "agent__", "agent---", "Agent"),
            ("--x--", "x", "--x--", "--x--", "--x--", "--x--"),
            ("a/b\\c..d", "a-b-c-d", "a-b-c--d", "a_b_c__d", "a-b-c--d", "a-b-c-d"),
            ("", "agent", "", "", "", "agent"),
            (
                "My_Agent-2",
                "my_agent-2",
                "my_agent-2",
                "my_agent-2",
                "my_agent-2",
                "My_Agent-2",
            ),
            ("Loap #3", "loap-3", "loap--3", "loap__3", "loap--3", "Loap-3"),
        ];
        for (input, def, path, stem, js, route) in table {
            assert_eq!(definition_slug(input), *def, "definition_slug({input:?})");
            assert_eq!(path_slug(input), *path, "path_slug({input:?})");
            assert_eq!(file_stem(input), *stem, "file_stem({input:?})");
            assert_eq!(js_ascii_slug(input), *js, "js_ascii_slug({input:?})");
            assert_eq!(agent_route_name(input), *route, "agent_route_name({input:?})");
            assert!(is_agent_id(&agent_route_name(input)), "agent_route_name({input:?}) is a valid id");
        }
    }

    #[test]
    fn agent_route_name_caps_at_64_chars_and_keeps_valid_ids_as_they_are() {
        assert_eq!(agent_route_name(&"Research Assistant #3 ".repeat(5)).len(), AGENT_ID_MAX_LEN);
        assert_eq!(agent_route_name("Agent2"), "Agent2");
        assert!(!is_agent_id(&"a".repeat(65)));
    }

    #[test]
    fn definition_slug_caps_at_64_chars() {
        assert_eq!(definition_slug(&"a".repeat(100)).len(), 64);
    }

    // The bodies these replace, frozen as they were at each call site, so the
    // move is proven not to change any output.
    mod oracle {
        pub fn default_agent_working_dir_slug(n: &str) -> String {
            n.to_lowercase()
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect()
        }
        pub fn registry_safe(n: &str) -> String {
            n.to_lowercase()
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        }
    }

    #[test]
    fn identical_to_the_inline_copies_they_replace() {
        let inputs = [
            "Zed Bot",
            "Ä",
            "Agent 🚀",
            "--x--",
            "a/b\\c..d",
            "",
            "ÉCOLE Ñ ß",
            "tab\there",
            "日本語",
            "x".repeat(80).leak(),
            "İstanbul",
            "ǅ",
            "..",
            "/",
            "a\u{0301}",
        ];
        for n in inputs {
            assert_eq!(
                path_slug(n),
                oracle::default_agent_working_dir_slug(n),
                "path_slug({n:?})"
            );
            assert_eq!(file_stem(n), oracle::registry_safe(n), "file_stem({n:?})");
        }
    }
}
