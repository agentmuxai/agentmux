// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shared identity palette: 16 hues for panes, agents, window tabs and
//! browser profiles (`docs/specs/SPEC_SHARED_PERCEPTUAL_PALETTE_16_2026_11_01.md`).
//!
//! The list lives once, in `schema/color-palette.json`; the frontend imports
//! the same file (`frontend/app/theme/palette.ts`), so the two cannot drift.
//! A slot is a hue. This side only ever needs the slot's stored `#rrggbb`
//! (what an agent, a window tab or a profile keeps) and which slot an older
//! colour is nearest to; the per-role tones are the frontend's
//! (`pane-color-scheme.ts`).

use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Slot {
    /// Lower-case name; what a pane's own pick and a widget's colour store.
    pub id: String,
    pub name: String,
    /// OKLCH hue, degrees.
    pub hue: f64,
    /// The stored, theme-neutral `#rrggbb`.
    pub hex: String,
}

#[derive(Deserialize)]
struct PaletteFile {
    slots: Vec<Slot>,
}

static SLOTS: LazyLock<Vec<Slot>> = LazyLock::new(|| {
    let file: PaletteFile = serde_json::from_str(include_str!("../../../../schema/color-palette.json"))
        .expect("schema/color-palette.json parses");
    file.slots
});

/// The 16 slots, in hue order.
pub fn slots() -> &'static [Slot] {
    &SLOTS
}

pub fn slot_by_id(id: &str) -> Option<&'static Slot> {
    let id = id.trim().to_ascii_lowercase();
    slots().iter().find(|s| s.id == id)
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The OKLCH hue, in degrees 0 to 360, of a `#rrggbb`; `None` for anything
/// else. A grey has no hue worth snapping by, and comes out as 0.
pub fn oklch_hue(hex: &str) -> Option<f64> {
    let b = hex.trim().as_bytes();
    if b.len() != 7 || b[0] != b'#' || !b[1..].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&hex.trim()[i..i + 2], 16).map(|v| srgb_to_linear(f64::from(v) / 255.0));
    let (r, g, bl) = (ch(1).ok()?, ch(3).ok()?, ch(5).ok()?);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * bl).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * bl).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * bl).cbrt();
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    let h = bb.atan2(a).to_degrees();
    Some(if h < 0.0 { h + 360.0 } else { h })
}

/// Circular distance between two hues, degrees, 0 to 180.
pub fn hue_distance(a: f64, b: f64) -> f64 {
    ((a - b + 540.0).rem_euclid(360.0) - 180.0).abs()
}

fn slot_index_of_hue(hue: f64) -> usize {
    let mut best = 0;
    let mut best_d = f64::MAX;
    for (i, s) in slots().iter().enumerate() {
        let d = hue_distance(hue, s.hue);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// The slot whose hue is nearest a stored `#rrggbb` (any colour: a slot's own
/// hex, an old Tailwind hex, an HSL pick). `None` when it is not a colour.
pub fn snap_hex(hex: &str) -> Option<&'static Slot> {
    oklch_hue(hex).map(|h| &slots()[slot_index_of_hue(h)])
}

fn fnv1a(id: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in id.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn usage<S: AsRef<str>>(taken: impl IntoIterator<Item = S>) -> Vec<usize> {
    let mut counts = vec![0usize; slots().len()];
    for hex in taken {
        if let Some(h) = oklch_hue(hex.as_ref()) {
            counts[slot_index_of_hue(h)] += 1;
        }
    }
    counts
}

/// The slot for a new agent: the least-used one among `taken` (the stored
/// colours of the agents that exist), ties broken by walking forward from the
/// FNV-1a slot of the agent id, so the same id and the same set always give
/// the same colour. With nothing taken it is the plain hash.
pub fn pick_slot<S: AsRef<str>>(agent_id: &str, taken: impl IntoIterator<Item = S>) -> &'static Slot {
    let n = slots().len();
    let counts = usage(taken);
    let min = counts.iter().copied().min().unwrap_or(0);
    let start = (fnv1a(agent_id) % n as u64) as usize;
    let i = (0..n).map(|k| (start + k) % n).find(|&i| counts[i] == min).unwrap_or(start);
    &slots()[i]
}

/// The slot for a new browser profile: the least-used one, then the one
/// farthest in hue from the slots already used, then the lowest.
pub fn farthest_free_slot<S: AsRef<str>>(
    used: impl IntoIterator<Item = S>,
) -> &'static Slot {
    let all = slots();
    let counts = usage(used);
    let min = counts.iter().copied().min().unwrap_or(0);
    let taken: Vec<f64> = (0..all.len()).filter(|&i| counts[i] > 0).map(|i| all[i].hue).collect();
    let nearest = |hue: f64| taken.iter().map(|t| hue_distance(hue, *t)).fold(f64::MAX, f64::min);
    let mut best = 0;
    let mut best_d = -1.0;
    for i in (0..all.len()).filter(|&i| counts[i] == min) {
        let d = if taken.is_empty() { 0.0 } else { nearest(all[i].hue) };
        if d > best_d + 1e-9 {
            best_d = d;
            best = i;
        }
    }
    &all[best]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_valid_slots_in_hue_order() {
        let s = slots();
        assert_eq!(s.len(), 16);
        for w in s.windows(2) {
            assert!(w[0].hue < w[1].hue, "{} before {}", w[0].id, w[1].id);
        }
        let mut ids: Vec<_> = s.iter().map(|x| x.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 16, "ids are unique");
        for x in s {
            assert!(oklch_hue(&x.hex).is_some(), "{} has a valid hex", x.id);
            assert_eq!(x.id, x.name.to_ascii_lowercase());
        }
    }

    #[test]
    fn each_hex_snaps_back_to_its_own_slot_within_half_a_degree() {
        for x in slots() {
            assert_eq!(snap_hex(&x.hex).map(|s| s.id.as_str()), Some(x.id.as_str()));
            assert!(hue_distance(oklch_hue(&x.hex).unwrap(), x.hue) < 0.5, "{}", x.id);
        }
    }

    #[test]
    fn todays_agent_colours_land_on_fourteen_different_slots() {
        let old = [
            "#ef4444", "#f97316", "#f59e0b", "#eab308", "#84cc16", "#22c55e", "#14b8a6", "#06b6d4", "#3b82f6",
            "#6366f1",
            "#8b5cf6", "#d946ef", "#ec4899", "#f43f5e",
        ];
        let mut hit: Vec<_> = old.iter().map(|h| snap_hex(h).unwrap().id.clone()).collect();
        hit.sort();
        hit.dedup();
        assert_eq!(hit.len(), 14);
        // The operator's browser profiles keep their meaning.
        assert_eq!(snap_hex("#14b8a6").unwrap().id, "teal");
        assert_eq!(snap_hex("#ec4899").unwrap().id, "pink");
        assert_eq!(snap_hex("#a855f7").unwrap().id, "violet");
        assert_eq!(snap_hex("#f59e0b").unwrap().id, "amber");
    }

    #[test]
    fn junk_is_not_a_colour() {
        for bad in ["", "#fff", "red", "#gggggg", "#3b82f6; }", "3b82f6", "#3B82F66"] {
            assert!(snap_hex(bad).is_none(), "{bad}");
        }
        assert!(snap_hex("#3B82F6").is_some(), "upper case is fine");
    }

    #[test]
    fn pick_is_deterministic_and_a_member() {
        for id in ["a", "b", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""] {
            let none: [&str; 0] = [];
            let p = pick_slot(id, none);
            assert_eq!(p.id, pick_slot(id, none).id);
            assert!(slots().iter().any(|s| s.id == p.id));
        }
    }

    #[test]
    fn twenty_five_agents_use_no_slot_more_than_twice() {
        let mut taken: Vec<String> = Vec::new();
        for i in 0..25 {
            taken.push(pick_slot(&format!("agent-{i}"), &taken).hex.clone());
        }
        let counts = usage(&taken);
        assert!(counts.iter().all(|c| *c <= 2), "{counts:?}");
        // The first 16 all differ.
        assert_eq!(usage(&taken[..16]).iter().filter(|c| **c == 1).count(), 16);
    }

    #[test]
    fn a_new_profile_takes_an_unused_slot_far_from_the_used_ones() {
        let none: [&str; 0] = [];
        assert_eq!(farthest_free_slot(none).id, "crimson");
        let first = farthest_free_slot(none);
        let second = farthest_free_slot([first.hex.as_str()]);
        assert!(hue_distance(first.hue, second.hue) > 150.0, "{} then {}", first.id, second.id);
        // A deleted middle profile leaves a gap that is filled, never a repeat.
        let used: Vec<&str> = slots().iter().filter(|s| s.id != "jade").map(|s| s.hex.as_str()).collect();
        assert_eq!(farthest_free_slot(used).id, "jade");
    }

    #[test]
    fn hue_distance_wraps() {
        assert!((hue_distance(357.0, 17.0) - 20.0).abs() < 1e-9);
        assert!((hue_distance(10.0, 190.0) - 180.0).abs() < 1e-9);
    }
}
