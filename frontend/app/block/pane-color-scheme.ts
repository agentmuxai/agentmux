// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane colours derived from an identity slot in OKLCH. A hue gets one fixed
 * lightness and chroma per role and theme
 * (docs/reports/REPORT_PANE_TAB_COLOR_BEST_PRACTICES_2026_10_02.md §6); Black
 * and White get a lightness per role and theme, with no chroma
 * (docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md §3).
 *
 * Why OKLCH: the old derivations were HSL (`hsl(h, 65%, 52%)` …), where equal
 * "lightness" is not equal perceived lightness — a yellow identity came out
 * about 1.5× as bright as a blue one, and the active-tab underline fell below
 * WCAG's 3:1 against its own pill for red, blue, purple and pink. Here every
 * hue gets the same OKLCH L and C for a role, so no agent is louder than
 * another, and the contrast between roles is the same for every hue
 * (pane-color-scheme.test.ts asserts 3:1 for the indicators, every hue, both
 * themes).
 *
 * `PANE_COLOR_TOKENS` and `NEUTRAL_TONES` are the one place to tune the look.
 */

import { hexToRgb, oklchToHex, srgbToLinear } from "../theme/oklch";
import { slotOf, type PaletteSlot } from "../theme/palette";

export { oklchToHex };

export type PaneColorRole = "identity" | "border" | "pill" | "pillActive" | "headerTint" | "widgetTint";

export interface OklchTone {
    /** Perceived lightness, 0–1. */
    l: number;
    /** Chroma (colourfulness); reduced automatically when out of sRGB gamut. */
    c: number;
}

export const PANE_COLOR_TOKENS: Record<"dark" | "light", Record<PaneColorRole, OklchTone>> = {
    dark: {
        // Active-tab underline, the focused pane's ring, a selected Swarm row:
        // anything that marks "this is that agent" at full strength. Must
        // clear 3:1 against pill, pillActive and headerTint.
        identity: { l: 0.75, c: 0.15 },
        // An unfocused pane's border and a hovered Swarm row: the hue, dimmed.
        // Fixed L, so no hue's unfocused border is as bright as any hue's
        // focused ring (HSL had an unfocused yellow brighter than a focused blue).
        border: { l: 0.45, c: 0.09 },
        // An inactive coloured pill: the hue, quietly.
        pill: { l: 0.33, c: 0.06 },
        // The selected pill: the same hue, a step stronger.
        pillActive: { l: 0.39, c: 0.09 },
        // The header row of a pane with exactly one identity: a subtle tint at
        // the neutral header's own lightness (NON_AGENT_DEFAULT_HEADER_BG ≈ L 0.27).
        headerTint: { l: 0.27, c: 0.035 },
        // A top-bar widget icon: its type's hue, faintly, at the lightness of
        // the monochrome icon it replaces (--widget-icon-color, L ≈ 0.82).
        widgetTint: { l: 0.82, c: 0.07 },
    },
    light: {
        identity: { l: 0.52, c: 0.17 },
        // On a light surface "dimmed" means lighter: quieter than the ring.
        border: { l: 0.8, c: 0.07 },
        pill: { l: 0.91, c: 0.04 },
        pillActive: { l: 0.86, c: 0.07 },
        headerTint: { l: 0.95, c: 0.025 },
        widgetTint: { l: 0.45, c: 0.09 },
    },
};

/**
 * The OKLCH lightness (chroma 0) of each role for Black and White. A neutral cannot share the hues' tones: a black
 * line cannot be seen on dark chrome, nor a white one on light chrome. So where the neutral is the theme's own
 * colour its lines step toward the middle until they reach 3:1 (a graphite line for Black on dark, a silver one for
 * White on light), and its fills go to the neutral itself where the theme's text still reads at 4.5:1
 * (pane-color-scheme.test.ts asserts both).
 */
export const NEUTRAL_TONES: Record<"dark" | "light", Record<"black" | "white", Record<PaneColorRole, number>>> = {
    dark: {
        black: { identity: 0.56, border: 0.4, pill: 0.16, pillActive: 0, headerTint: 0.14, widgetTint: 0.56 },
        white: { identity: 0.97, border: 0.62, pill: 0.42, pillActive: 0.5, headerTint: 0.36, widgetTint: 0.97 },
    },
    light: {
        black: { identity: 0.15, border: 0.55, pill: 0.8, pillActive: 0.72, headerTint: 0.86, widgetTint: 0.15 },
        white: { identity: 0.62, border: 0.84, pill: 0.985, pillActive: 1, headerTint: 1, widgetTint: 0.62 },
    },
};

/**
 * The palette slot of a pane's identity (theme/palette.ts): from an explicit
 * pick (`frame:hue`, a legacy HSL hue) or else from the agent's persisted
 * identity colour (`frame:activebordercolor`, a hex or a slot id). Any colour
 * is snapped to its slot, so an old hex that is not in the palette still
 * draws in the palette. Undefined when the pane has neither.
 */
export function identitySlot(hslHue: number | undefined, identityHex: string | undefined): PaletteSlot | undefined {
    if (typeof hslHue === "number") return slotOf(hslHue);
    return identityHex ? slotOf(identityHex) : undefined;
}

/** The colour for `role` of a slot in a theme, as `#rrggbb`. */
export function slotRoleColor(slot: PaletteSlot, isLightTheme: boolean, role: PaneColorRole): string {
    const theme = isLightTheme ? "light" : "dark";
    if (slot.neutral) return oklchToHex(NEUTRAL_TONES[theme][slot.neutral][role], 0, 0);
    const t = PANE_COLOR_TOKENS[theme][role];
    return oklchToHex(t.l, t.c, slot.hue ?? 0);
}

/** The colour for `role` of an identity in a theme, as `#rrggbb`. */
export function paneRoleColor(
    hslHue: number | undefined,
    identityHex: string | undefined,
    isLightTheme: boolean,
    role: PaneColorRole,
): string | undefined {
    const slot = identitySlot(hslHue, identityHex);
    return slot && slotRoleColor(slot, isLightTheme, role);
}

/**
 * What a picker's swatch for a slot is painted with: a hue in its identity tone for the theme (what the ring and the
 * underline will be), Black and White as themselves in both themes. A neutral's identity tone is a grey where the
 * theme is its own colour, and a grey swatch would not say "Black".
 */
export function slotSwatchColor(slot: PaletteSlot, isLightTheme: boolean): string {
    return slot.neutral ? slot.hex : slotRoleColor(slot, isLightTheme, "identity");
}

/** WCAG 2.x contrast ratio between two `#rrggbb` colours. */
export function contrastRatio(hexA: string, hexB: string): number {
    const lum = (hex: string) => {
        const rgb = hexToRgb(hex);
        if (!rgb) return 0;
        const [r, g, b] = rgb.map(srgbToLinear);
        return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    const [hi, lo] = [lum(hexA), lum(hexB)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
}
