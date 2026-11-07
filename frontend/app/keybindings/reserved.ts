// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Keys the app can never receive, or that a desktop takes often enough to warn about, by platform
// (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §5.3). Only keys that were measured or are the
// operating system's own are listed (docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md §2, §9): a
// key with no measurement is not here, because we do not guess.

import { parseKey, sameKey, type KeyPlatform, type KeySpec } from "./keys";

/** `blocked`: cannot arrive, or would trap the person; it cannot be saved. `warned`: often taken; saved after
 *  the person confirms. */
export type ReservedTier = "blocked" | "warned";

export interface Reserved {
    tier: ReservedTier;
    /** Why, in words the person can read. */
    reason: string;
}

interface Rule extends Reserved {
    platforms: KeyPlatform[];
    test: (k: KeySpec) => boolean;
}

const spec = (key: string) => parseKey(key, "other");
const is = (...keys: string[]) => {
    const specs = keys.map(spec);
    return (k: KeySpec) => specs.some((s) => sameKey(s, k));
};
const arrows = (mods: string) => ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].map((a) => `${mods}+${a}`);
const fKeys = (mods: string) => Array.from({ length: 12 }, (_, i) => (mods ? `${mods}+F${i + 1}` : `F${i + 1}`));
const digits = (mods: string) => Array.from({ length: 9 }, (_, i) => `${mods}+${i + 1}`);
const letters = (mods: string, chars: string) => [...chars].map((c) => `${mods}+${c}`);

const GNOME = "GNOME uses it";

const RULES: Rule[] = [
    // ── Never arrives, or traps the person ──
    { tier: "blocked", platforms: ["mac", "linux", "other"], reason: "Escape closes dialogs and cancels recording; it is fixed", test: is("Escape") },
    { tier: "blocked", platforms: ["other", "linux"], reason: "Windows and Linux take Ctrl+Alt+Delete before any app", test: is("ctrl+alt+Delete") },
    { tier: "blocked", platforms: ["other"], reason: "Windows takes Alt+F4 to close the window", test: is("alt+F4") },
    { tier: "blocked", platforms: ["other"], reason: "Windows takes every Windows-key combination before an app sees it", test: (k) => k.meta },
    { tier: "blocked", platforms: ["mac"], reason: "macOS takes ⌘Tab and ⌘Space before an app sees them", test: is("meta+Tab", "meta+code:Space") },
    { tier: "blocked", platforms: ["mac"], reason: "AgentMux's own menu takes ⌘Q, ⌘H and ⌥⌘H", test: is("meta+q", "meta+h", "alt+meta+h") },

    // ── Warned: measured on this desktop or documented by the OS ──
    { tier: "warned", platforms: ["mac"], reason: "macOS uses ⌃ with an arrow key for Mission Control and Spaces", test: is(...arrows("ctrl")) },
    { tier: "warned", platforms: ["mac"], reason: "A function key needs fn unless “Use F1, F2, etc. keys as standard function keys” is on", test: is(...fKeys("")) },
    {
        tier: "warned",
        platforms: ["linux"],
        reason: `${GNOME} to switch workspace or move a window to another workspace`,
        test: is(...arrows("ctrl+alt"), ...arrows("ctrl+shift+alt"), ...arrows("meta+alt"), ...arrows("meta+shift")),
    },
    {
        tier: "warned",
        platforms: ["linux"],
        reason: `${GNOME} to switch windows or open the run prompt`,
        test: is("ctrl+alt+Tab", "ctrl+alt+Escape", "ctrl+alt+d", "ctrl+shift+alt+r", "alt+Tab", "alt+code:Backquote", "alt+Escape", "alt+code:Space", "alt+F2", "alt+F4", "alt+F6", "alt+F7", "alt+F8", "alt+F10"),
    },
    { tier: "warned", platforms: ["linux"], reason: "Linux switches to a text console with Ctrl+Alt+F1 to F12", test: is(...fKeys("ctrl+alt")) },
    {
        tier: "warned",
        platforms: ["linux"],
        reason: `${GNOME} for the Super key`,
        test: is(...digits("meta"), ...digits("meta+ctrl"), "meta+code:Space", "meta+Tab", ...letters("meta", "avmsnhdp")),
    },
    { tier: "warned", platforms: ["other"], reason: "Ctrl+Shift+0 switches the keyboard layout on some Windows setups", test: is("ctrl+shift+0") },
    // On Windows, AltGr is Ctrl+Alt and types characters on many layouts (keys.ts matchKey).
    { tier: "warned", platforms: ["other"], reason: "Ctrl+Alt+letter is AltGr on many keyboard layouts and types a character", test: (k) => k.ctrl && k.alt && !k.shift && !k.meta && !!k.letter },
];

/** What the platform does to `spec` (a key or a two-step chord), or null when it is free. The worst of the
 *  chord's steps counts, since each arrives on its own. A malformed key is not reserved: the setting rejects it. */
export function reservedFor(key: string, platform: KeyPlatform): Reserved | null {
    let worst: Reserved | null = null;
    for (const step of key.trim().split(/\s+/)) {
        let k: KeySpec;
        try {
            k = parseKey(step, platform);
        } catch {
            continue;
        }
        for (const r of RULES) {
            if (!r.platforms.includes(platform) || !r.test(k)) continue;
            if (!worst || (r.tier === "blocked" && worst.tier !== "blocked")) worst = { tier: r.tier, reason: r.reason };
        }
    }
    return worst;
}
