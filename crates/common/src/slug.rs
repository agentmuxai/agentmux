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

/// A definition's slug: lowercase; ASCII alphanumerics, `-` and `_` kept,
/// everything else `-`; runs of dashes collapsed and trimmed; at most 64
/// chars; `"agent"` if nothing is left.
pub fn definition_slug(name: &str) -> String {
    let filtered: String = name
        .to_lowercase()
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
    let trimmed: String = collapsed.chars().take(64).collect();
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed
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
        // (input, definition_slug, path_slug, file_stem, js_ascii_slug)
        let table: &[(&str, &str, &str, &str, &str)] = &[
            ("Zed Bot", "zed-bot", "zed-bot", "zed_bot", "zed-bot"),
            ("Ä", "agent", "ä", "ä", "-"),
            ("Agent 🚀", "agent", "agent--", "agent__", "agent---"),
            ("--x--", "x", "--x--", "--x--", "--x--"),
            ("a/b\\c..d", "a-b-c-d", "a-b-c--d", "a_b_c__d", "a-b-c--d"),
            ("", "agent", "", "", ""),
            (
                "My_Agent-2",
                "my_agent-2",
                "my_agent-2",
                "my_agent-2",
                "my_agent-2",
            ),
        ];
        for (input, def, path, stem, js) in table {
            assert_eq!(definition_slug(input), *def, "definition_slug({input:?})");
            assert_eq!(path_slug(input), *path, "path_slug({input:?})");
            assert_eq!(file_stem(input), *stem, "file_stem({input:?})");
            assert_eq!(js_ascii_slug(input), *js, "js_ascii_slug({input:?})");
        }
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
