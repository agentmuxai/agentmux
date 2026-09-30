// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-agent color assignment — palette, deterministic pick, validation.
//!
//! Agents carry a display color in the per-agent content store
//! (`agent_content`, `content_type = "ui:color"`, value `#rrggbb`). It is
//! assigned at creation (`createagent` handler), backfilled for existing
//! agents by migration `m0020_agent_color_backfill`, and seeded into each
//! new agent block's `frame:bordercolor` meta at `agent.open` so the
//! existing pane-frame rendering displays it with no frontend changes.
//! See `docs/specs/SPEC_AGENT_COLOR_2026_08_08.md`.

/// The 14-hue palette — hex values mirror the frontend's
/// `AGENT_COLOR_PALETTE` (`frontend/app/view/agent/agent-color.ts`).
/// Duplicated deliberately: assigned colors are STORED, not derived, so
/// palette drift between the two lists cannot recolor existing agents.
/// This is a distinct, deliberately-vivid array from the frontend's tab
/// strip colors (`frontend/app/tab/tab.tsx`'s `TAB_COLORS`) — see
/// docs/specs/SPEC_TAB_COLOR_DESATURATION_2026_08_13.md.
pub const AGENT_COLOR_PALETTE: [&str; 14] = [
    "#ef4444", // Red
    "#f97316", // Orange
    "#f59e0b", // Amber
    "#eab308", // Yellow
    "#84cc16", // Lime
    "#22c55e", // Green
    "#14b8a6", // Teal
    "#06b6d4", // Cyan
    "#3b82f6", // Blue
    "#6366f1", // Indigo
    "#8b5cf6", // Violet
    "#d946ef", // Fuchsia
    "#ec4899", // Pink
    "#f43f5e", // Rose
];

/// Pick a palette color for an agent — deterministic FNV-1a hash of the
/// agent id mapped onto the palette. Effectively random across agents,
/// stable across re-runs and machines, and dependency-free (no `rand`).
pub fn pick_agent_color(agent_id: &str) -> &'static str {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in agent_id.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    AGENT_COLOR_PALETTE[(hash % AGENT_COLOR_PALETTE.len() as u64) as usize]
}

/// Strict `#rrggbb` shape check — the stored value flows into block meta
/// that the frontend applies as a CSS border-color, so a corrupt or
/// hand-edited row must not be able to inject arbitrary CSS.
pub fn is_valid_agent_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
}

/// Dimmed variant for the UNFOCUSED pane border (`frame:bordercolor`),
/// keeping the full-strength color for the focused border
/// (`frame:activebordercolor`) — the agent's color stays visible either
/// way, and focus stays distinguishable by brightness. Input must already
/// have passed [`is_valid_agent_color`]; returns the input unchanged if it
/// somehow hasn't.
pub fn dim_agent_color(hex: &str) -> String {
    if !is_valid_agent_color(hex) {
        return hex.to_string();
    }
    let channel = |s: &str| u8::from_str_radix(s, 16).unwrap_or(0);
    let r = (f32::from(channel(&hex[1..3])) * 0.55) as u8;
    let g = (f32::from(channel(&hex[3..5])) * 0.55) as u8;
    let b = (f32::from(channel(&hex[5..7])) * 0.55) as u8;
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_is_deterministic() {
        assert_eq!(pick_agent_color("abc-123"), pick_agent_color("abc-123"));
    }

    #[test]
    fn pick_returns_palette_member() {
        for id in ["a", "b", "c", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""] {
            assert!(AGENT_COLOR_PALETTE.contains(&pick_agent_color(id)));
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
    fn validation_accepts_palette_and_rejects_junk() {
        for c in AGENT_COLOR_PALETTE {
            assert!(is_valid_agent_color(c));
        }
        for bad in ["", "#fff", "red", "#gggggg", "#3b82f6; }", "3b82f6", "#3B82F66"] {
            assert!(!is_valid_agent_color(bad), "{bad} should be invalid");
        }
        // Uppercase hex is fine.
        assert!(is_valid_agent_color("#3B82F6"));
    }

    #[test]
    fn dim_scales_channels_and_stays_valid() {
        assert_eq!(dim_agent_color("#ffffff"), "#8c8c8c");
        assert_eq!(dim_agent_color("#000000"), "#000000");
        for c in AGENT_COLOR_PALETTE {
            let dimmed = dim_agent_color(c);
            assert!(is_valid_agent_color(&dimmed), "{dimmed}");
            assert_ne!(dimmed, *c);
        }
        // Invalid input passes through untouched (defensive contract).
        assert_eq!(dim_agent_color("junk"), "junk");
    }
}
