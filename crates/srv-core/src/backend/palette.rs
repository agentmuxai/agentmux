// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shared identity palette for panes, agents, window tabs and browser
//! profiles: 12 hues, then Black and White
//! (`docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md`).
//!
//! The list lives once, in `schema/color-palette.json`; the frontend imports
//! the same file (`frontend/app/theme/palette.ts`), so the two cannot drift.
//! A slot is a hue or a neutral. This side only ever needs the slot's stored
//! `#rrggbb` (what an agent, a window tab or a profile keeps) and which slot
//! an older colour means; the per-role tones are the frontend's
//! (`pane-color-scheme.ts`).

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

/// Black or White: a slot with no hue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Neutral {
    Black,
    White,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Slot {
    /// Lower-case name; what a pane's own pick and a widget's colour store.
    pub id: String,
    pub name: String,
    /// OKLCH hue, degrees. Black and White have none.
    #[serde(default)]
    pub hue: Option<f64>,
    /// Set on Black and on White.
    #[serde(default)]
    pub neutral: Option<Neutral>,
    /// The stored, theme-neutral `#rrggbb`.
    pub hex: String,
}

/// The ids and stored hexes of the 16 slots before version 2, each with the
/// id of the slot it now means.
#[derive(Deserialize)]
struct Retired {
    ids: HashMap<String, String>,
    hexes: HashMap<String, String>,
}

#[derive(Deserialize)]
struct PaletteFile {
    slots: Vec<Slot>,
    retired: Retired,
}

static PALETTE: LazyLock<PaletteFile> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../schema/color-palette.json")).expect("schema/color-palette.json parses")
});

/// A stored colour with less chroma than this is a neutral, not a hue. No slot
/// and no older palette hex is that grey.
const NEUTRAL_MAX_CHROMA: f64 = 0.02;
/// A neutral below this OKLCH lightness is Black, the rest White.
const NEUTRAL_SPLIT_LIGHTNESS: f64 = 0.5;

/// The 14 slots: the 12 hues in hue order, then Black and White.
pub fn slots() -> &'static [Slot] {
    &PALETTE.slots
}

/// The 12 hues, in hue order, each with its hue: what is handed out
/// automatically, and what a colour with a hue snaps to.
fn hue_slots() -> impl Iterator<Item = (&'static Slot, f64)> {
    slots().iter().filter_map(|s| s.hue.map(|h| (s, h)))
}

fn current(id: &str) -> Option<&'static Slot> {
    slots().iter().find(|s| s.id == id)
}

/// The slot with this id. An id retired with the 16 (`crimson`, `azure`, ...)
/// gives the slot it became.
pub fn slot_by_id(id: &str) -> Option<&'static Slot> {
    let id = id.trim().to_ascii_lowercase();
    current(&id).or_else(|| PALETTE.retired.ids.get(&id).and_then(|now| current(now)))
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A `#rrggbb` as OKLCH lightness, chroma and hue (degrees 0 to 360); `None`
/// for anything else.
fn oklch(hex: &str) -> Option<(f64, f64, f64)> {
    let hex = hex.trim();
    let b = hex.as_bytes();
    if b.len() != 7 || b[0] != b'#' || !b[1..].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map(|v| srgb_to_linear(f64::from(v) / 255.0));
    let (r, g, bl) = (ch(1).ok()?, ch(3).ok()?, ch(5).ok()?);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * bl).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * bl).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * bl).cbrt();
    let lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    let h = bb.atan2(a).to_degrees();
    Some((lightness, a.hypot(bb), if h < 0.0 { h + 360.0 } else { h }))
}

/// The OKLCH hue, in degrees 0 to 360, of a `#rrggbb`; `None` for anything
/// else. A grey has no hue worth snapping by: [`snap_hex`] reads it as Black
/// or White.
pub fn oklch_hue(hex: &str) -> Option<f64> {
    oklch(hex).map(|(_, _, h)| h)
}

/// Circular distance between two hues, degrees, 0 to 180.
pub fn hue_distance(a: f64, b: f64) -> f64 {
    ((a - b + 540.0).rem_euclid(360.0) - 180.0).abs()
}

fn nearest_hue_slot(hue: f64) -> &'static Slot {
    let mut best = &slots()[0];
    let mut best_d = f64::MAX;
    for (s, h) in hue_slots() {
        let d = hue_distance(hue, h);
        if d < best_d {
            best_d = d;
            best = s;
        }
    }
    best
}

/// The slot a stored `#rrggbb` stands for (any colour: a slot's own hex, one
/// of the 16's, an old Tailwind hex, an HSL pick). One of the 16's own hexes
/// goes where the `retired` table says: its hue alone does not settle it
/// (Indigo sits midway between Blue and Violet). A grey is Black or White; any
/// other colour is the hue slot nearest in OKLCH hue. `None` when it is not a
/// colour.
pub fn snap_hex(hex: &str) -> Option<&'static Slot> {
    let (l, c, h) = oklch(hex)?;
    if let Some(now) = PALETTE.retired.hexes.get(&hex.trim().to_ascii_lowercase()) {
        return current(now);
    }
    if c < NEUTRAL_MAX_CHROMA {
        return current(if l < NEUTRAL_SPLIT_LIGHTNESS { "black" } else { "white" });
    }
    Some(nearest_hue_slot(h))
}

fn fnv1a(id: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in id.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// How many of `taken` use each hue, in the order of [`hue_slots`]. A Black or
/// White one uses no hue, so it is not counted.
fn usage<S: AsRef<str>>(taken: impl IntoIterator<Item = S>) -> Vec<usize> {
    let hues: Vec<&'static Slot> = hue_slots().map(|(s, _)| s).collect();
    let mut counts = vec![0usize; hues.len()];
    for hex in taken {
        let Some(slot) = snap_hex(hex.as_ref()) else { continue };
        if let Some(i) = hues.iter().position(|s| s.id == slot.id) {
            counts[i] += 1;
        }
    }
    counts
}

/// The slot for a new agent: the least-used hue among `taken` (the stored
/// colours of the agents that exist), ties broken by walking forward from the
/// FNV-1a slot of the agent id, so the same id and the same set always give
/// the same colour. With nothing taken it is the plain hash. Never Black or
/// White: a neutral is only ever someone's own choice.
pub fn pick_slot<S: AsRef<str>>(agent_id: &str, taken: impl IntoIterator<Item = S>) -> &'static Slot {
    let hues: Vec<&'static Slot> = hue_slots().map(|(s, _)| s).collect();
    let n = hues.len();
    let counts = usage(taken);
    let min = counts.iter().copied().min().unwrap_or(0);
    let start = (fnv1a(agent_id) % n as u64) as usize;
    let i = (0..n).map(|k| (start + k) % n).find(|&i| counts[i] == min).unwrap_or(start);
    hues[i]
}

/// The slot for a new browser profile: the least-used hue, then the one
/// farthest in hue from the hues already used, then the lowest. Never Black or
/// White.
pub fn farthest_free_slot<S: AsRef<str>>(
    used: impl IntoIterator<Item = S>,
) -> &'static Slot {
    let all: Vec<(&'static Slot, f64)> = hue_slots().collect();
    let counts = usage(used);
    let min = counts.iter().copied().min().unwrap_or(0);
    let taken: Vec<f64> = (0..all.len()).filter(|&i| counts[i] > 0).map(|i| all[i].1).collect();
    let nearest = |hue: f64| taken.iter().map(|t| hue_distance(hue, *t)).fold(f64::MAX, f64::min);
    let mut best = 0;
    let mut best_d = -1.0;
    for i in (0..all.len()).filter(|&i| counts[i] == min) {
        let d = if taken.is_empty() { 0.0 } else { nearest(all[i].1) };
        if d > best_d + 1e-9 {
            best_d = d;
            best = i;
        }
    }
    all[best].0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 16 slots before version 2 (id, stored hex) and the slot each became (spec §5.1).
    const THE_16: [(&str, &str, &str); 16] = [
        ("crimson", "#e8777f", "red"),
        ("red", "#e87b64", "red"),
        ("orange", "#e28247", "orange"),
        ("amber", "#d48d24", "amber"),
        ("yellow", "#bb9c12", "yellow"),
        ("lime", "#9aa932", "yellow"),
        ("green", "#75b255", "green"),
        ("jade", "#47b777", "jade"),
        ("teal", "#00b89e", "teal"),
        ("cyan", "#00b4be", "teal"),
        ("sky", "#00aedd", "sky"),
        ("azure", "#53a3f2", "blue"),
        ("indigo", "#8297f5", "violet"),
        ("violet", "#ae88e7", "violet"),
        ("magenta", "#ce7cc8", "magenta"),
        ("pink", "#e276a0", "rose"),
    ];

    fn id_of(hex: &str) -> Option<&'static str> {
        snap_hex(hex).map(|s| s.id.as_str())
    }

    #[test]
    fn twelve_hues_in_hue_order_then_black_and_white() {
        let s = slots();
        assert_eq!(s.len(), 14);
        let hues: Vec<f64> = hue_slots().map(|(_, h)| h).collect();
        assert_eq!(hues.len(), 12);
        for w in hues.windows(2) {
            assert!(w[0] < w[1], "{} before {}", w[0], w[1]);
        }
        assert!(s[..12].iter().all(|x| x.hue.is_some() && x.neutral.is_none()), "the hues come first");
        assert_eq!((s[12].id.as_str(), s[12].neutral, s[12].hex.as_str()), ("black", Some(Neutral::Black), "#000000"));
        assert_eq!((s[13].id.as_str(), s[13].neutral, s[13].hex.as_str()), ("white", Some(Neutral::White), "#ffffff"));
        assert!(s[12..].iter().all(|x| x.hue.is_none()));
        let mut ids: Vec<_> = s.iter().map(|x| x.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 14, "ids are unique");
        for x in s {
            assert!(oklch_hue(&x.hex).is_some(), "{} has a valid hex", x.id);
            assert_eq!(x.id, x.name.to_ascii_lowercase());
        }
    }

    #[test]
    fn each_hex_snaps_back_to_its_own_slot_and_a_hue_within_half_a_degree() {
        for x in slots() {
            assert_eq!(id_of(&x.hex), Some(x.id.as_str()));
            if let Some(hue) = x.hue {
                assert!(hue_distance(oklch_hue(&x.hex).unwrap(), hue) < 0.5, "{}", x.id);
            }
        }
    }

    #[test]
    fn the_sixteen_before_go_where_the_spec_says_by_hex_and_by_id() {
        for (id, hex, now) in THE_16 {
            assert_eq!(id_of(hex), Some(now), "{id} {hex}");
            assert_eq!(id_of(&hex.to_uppercase()), Some(now), "{id} in upper case");
            assert_eq!(slot_by_id(id).map(|s| s.id.as_str()), Some(now), "{id}");
        }
        assert_eq!(slot_by_id(" Azure ").map(|s| s.id.as_str()), Some("blue"));
        assert!(slot_by_id("mauve").is_none());
        // Every entry of the retired table names a slot that exists.
        for now in PALETTE.retired.ids.values().chain(PALETTE.retired.hexes.values()) {
            assert!(current(now).is_some(), "{now}");
        }
        assert_eq!(PALETTE.retired.hexes.len(), 16);
    }

    #[test]
    fn the_older_tailwind_colours_keep_their_meaning() {
        let old = [
            "#ef4444", "#f97316", "#f59e0b", "#eab308", "#84cc16", "#22c55e", "#14b8a6", "#06b6d4", "#3b82f6",
            "#6366f1", "#8b5cf6", "#d946ef", "#ec4899", "#f43f5e",
        ];
        for hex in old {
            assert!(snap_hex(hex).unwrap().hue.is_some(), "{hex} is a hue");
        }
        // The operator's browser profiles keep their meaning.
        assert_eq!(id_of("#14b8a6"), Some("teal"));
        assert_eq!(id_of("#ec4899"), Some("rose"));
        assert_eq!(id_of("#a855f7"), Some("violet"));
        assert_eq!(id_of("#f59e0b"), Some("amber"));
        assert_eq!(id_of("#3b82f6"), Some("blue"));
    }

    #[test]
    fn a_grey_is_black_or_white_and_a_faint_colour_is_still_a_hue() {
        assert_eq!(id_of("#000000"), Some("black"));
        assert_eq!(id_of("#ffffff"), Some("white"));
        assert_eq!(id_of("#FFFFFF"), Some("white"));
        assert_eq!(id_of("#1a1a1a"), Some("black"));
        assert_eq!(id_of("#808080"), Some("white"), "a mid grey is nearer White in lightness");
        assert_eq!(id_of("#5b5b5b"), Some("black"));
        // The slate of the old profile list: chroma 0.04, so it is a hue.
        assert_eq!(id_of("#64748b"), Some("blue"));
    }

    #[test]
    fn junk_is_not_a_colour() {
        for bad in ["", "#fff", "red", "#gggggg", "#3b82f6; }", "3b82f6", "#3B82F66"] {
            assert!(snap_hex(bad).is_none(), "{bad}");
        }
        assert!(snap_hex("#3B82F6").is_some(), "upper case is fine");
    }

    #[test]
    fn pick_is_deterministic_and_never_a_neutral() {
        let none: [&str; 0] = [];
        for id in ["a", "b", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""] {
            let p = pick_slot(id, none);
            assert_eq!(p.id, pick_slot(id, none).id);
            assert!(p.hue.is_some(), "{id} got {}", p.id);
        }
        for i in 0..200 {
            assert!(pick_slot(&format!("agent-{i}"), none).neutral.is_none());
        }
    }

    #[test]
    fn twenty_five_agents_use_no_hue_more_than_three_times() {
        let mut taken: Vec<String> = Vec::new();
        for i in 0..25 {
            taken.push(pick_slot(&format!("agent-{i}"), &taken).hex.clone());
        }
        let counts = usage(&taken);
        assert_eq!(counts.len(), 12);
        assert!(counts.iter().all(|c| *c <= 3), "{counts:?}");
        // The first 12 all differ.
        assert_eq!(usage(&taken[..12]).iter().filter(|c| **c == 1).count(), 12);
    }

    #[test]
    fn a_black_or_white_agent_counts_against_no_hue() {
        assert_eq!(usage(["#000000", "#ffffff", "#808080"]), vec![0; 12]);
        let none: [&str; 0] = [];
        assert_eq!(pick_slot("someone", ["#000000", "#ffffff"]).id, pick_slot("someone", none).id);
    }

    #[test]
    fn a_new_profile_takes_an_unused_hue_far_from_the_used_ones() {
        let none: [&str; 0] = [];
        assert_eq!(farthest_free_slot(none).id, "rose");
        let first = farthest_free_slot(none);
        let second = farthest_free_slot([first.hex.as_str()]);
        assert!(hue_distance(first.hue.unwrap(), second.hue.unwrap()) > 150.0, "{} then {}", first.id, second.id);
        // A deleted middle profile leaves a gap that is filled, never a repeat.
        let used: Vec<&str> = slots().iter().filter(|s| s.id != "jade").map(|s| s.hex.as_str()).collect();
        assert_eq!(farthest_free_slot(used).id, "jade");
        // Never a neutral, even when every hue is in use and neither neutral is.
        let hues: Vec<&str> = hue_slots().map(|(s, _)| s.hex.as_str()).collect();
        assert!(farthest_free_slot(hues).hue.is_some());
        assert!(farthest_free_slot(["#000000"]).hue.is_some());
    }

    #[test]
    fn hue_distance_wraps() {
        assert!((hue_distance(357.0, 17.0) - 20.0).abs() < 1e-9);
        assert!((hue_distance(10.0, 190.0) - 180.0).abs() < 1e-9);
    }
}
