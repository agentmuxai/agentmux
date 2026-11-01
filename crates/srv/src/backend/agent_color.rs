// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-agent color assignment — palette, deterministic pick, validation.
//!
//! Agents carry a display color in the per-agent content store
//! (`agent_content`, `content_type = "ui:color"`, value `#rrggbb`). It is
//! assigned at creation (`createagent` handler), backfilled for existing
//! agents by migration `m0020_agent_color_backfill`, and seeded into each
//! new agent block's `frame:activebordercolor` meta at `agent.open`.
//! The colors come from the shared palette (`backend::palette`,
//! `docs/specs/SPEC_SHARED_PERCEPTUAL_PALETTE_16_2026_11_01.md`); see also
//! `docs/specs/SPEC_AGENT_COLOR_2026_08_08.md`.

use crate::backend::palette::pick_slot;

/// Pick a palette color for an agent: the plain FNV-1a hash of the agent id
/// over the shared palette (`backend::palette`). Stable across re-runs and
/// machines, dependency-free. A caller that knows the colours the other
/// agents have uses [`pick_agent_color_avoiding`] to spread them.
pub fn pick_agent_color(agent_id: &str) -> &'static str {
    let none: [&str; 0] = [];
    pick_slot(agent_id, none).hex.as_str()
}

/// The palette color for a new agent given the colors `taken` by existing
/// agents: the least-used slot, ties broken by the id's hash
/// (`palette::pick_slot`), so no color repeats until all 16 are in use.
pub fn pick_agent_color_avoiding<S: AsRef<str>>(
    agent_id: &str,
    taken: impl IntoIterator<Item = S>,
) -> &'static str {
    pick_slot(agent_id, taken).hex.as_str()
}

/// The colors the existing agents keep (`ui:color`), for
/// [`pick_agent_color_avoiding`]. Best-effort: a store error just means fewer
/// colors are known, and the pick falls back toward the plain hash.
pub fn colors_in_use(mstore: &crate::backend::storage::store::Store) -> Vec<String> {
    let defs = mstore.agent_def_list().unwrap_or_default();
    defs.iter()
        .filter_map(|d| mstore.agent_content_get(&d.id, "ui:color").ok().flatten())
        .map(|c| c.content.trim().to_string())
        .filter(|c| is_valid_agent_color(c))
        .collect()
}

/// The color a new agent is given: the least-used slot among the colors the
/// existing agents keep.
pub fn color_for_new_agent(
    mstore: &crate::backend::storage::store::Store,
    agent_id: &str,
) -> String {
    pick_agent_color_avoiding(agent_id, colors_in_use(mstore)).to_string()
}

/// Strict `#rrggbb` shape check — the stored value flows into block meta
/// that the frontend applies as a CSS border-color, so a corrupt or
/// hand-edited row must not be able to inject arbitrary CSS.
pub fn is_valid_agent_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::palette::slots;

    #[test]
    fn pick_is_deterministic() {
        assert_eq!(pick_agent_color("abc-123"), pick_agent_color("abc-123"));
    }

    #[test]
    fn pick_returns_palette_member() {
        for id in ["a", "b", "c", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""] {
            assert!(slots().iter().any(|s| s.hex == pick_agent_color(id)));
        }
    }

    #[test]
    fn distinct_ids_spread_over_palette() {
        // Not a randomness proof — just guards against a degenerate hash
        // that maps everything to one bucket.
        let picked: std::collections::HashSet<_> =
            (0..100).map(|i| pick_agent_color(&format!("agent-{i}"))).collect();
        assert!(picked.len() > 5, "expected spread, got {}", picked.len());
    }

    #[test]
    fn avoiding_the_taken_colors_gives_a_free_one() {
        let taken: Vec<&str> = slots().iter().skip(1).map(|s| s.hex.as_str()).collect();
        assert_eq!(pick_agent_color_avoiding("anything", &taken), slots()[0].hex);
    }

    #[test]
    fn validation_accepts_palette_and_rejects_junk() {
        for s in slots() {
            assert!(is_valid_agent_color(&s.hex));
        }
        for bad in ["", "#fff", "red", "#gggggg", "#3b82f6; }", "3b82f6", "#3B82F66"] {
            assert!(!is_valid_agent_color(bad), "{bad} should be invalid");
        }
        // Uppercase hex is fine.
        assert!(is_valid_agent_color("#3B82F6"));
    }
}
