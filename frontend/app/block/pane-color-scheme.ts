// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane colours derived from an identity hue in OKLCH, one fixed lightness and
 * chroma per role and theme (docs/reports/REPORT_PANE_TAB_COLOR_BEST_PRACTICES_2026_10_02.md §6).
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
 * `PANE_COLOR_TOKENS` is the one place to tune the look.
 */

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

// ── OKLab / OKLCH ⇄ sRGB (Björn Ottosson's matrices) ───────────────────────

type Rgb = [number, number, number];

function srgbToLinear(c: number): number {
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function linearToSrgb(c: number): number {
    return c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
}

function linearRgbToOklab([r, g, b]: Rgb): Rgb {
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    return [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ];
}

function oklabToLinearRgb([L, a, b]: Rgb): Rgb {
    const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
    return [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
    ];
}

/** OKLCH hue (degrees) of an sRGB colour given as 0–1 channels. */
function oklchHueOfSrgb(rgb: Rgb): number {
    const [, a, b] = linearRgbToOklab(rgb.map(srgbToLinear) as Rgb);
    const h = (Math.atan2(b, a) * 180) / Math.PI;
    return h < 0 ? h + 360 : h;
}

function inGamut(rgb: Rgb): boolean {
    return rgb.every((c) => c >= -1e-4 && c <= 1 + 1e-4);
}

/** OKLCH → `#rrggbb`, lowering chroma (keeping L and h) until it fits sRGB. */
export function oklchToHex(l: number, c: number, h: number): string {
    const rad = (h * Math.PI) / 180;
    const at = (chroma: number): Rgb => oklabToLinearRgb([l, chroma * Math.cos(rad), chroma * Math.sin(rad)]);
    let lin = at(c);
    if (!inGamut(lin)) {
        let lo = 0;
        let hi = c;
        for (let i = 0; i < 24; i++) {
            const mid = (lo + hi) / 2;
            if (inGamut(at(mid))) lo = mid;
            else hi = mid;
        }
        lin = at(lo);
    }
    const toHex = (x: number) =>
        Math.round(Math.min(1, Math.max(0, linearToSrgb(Math.min(1, Math.max(0, x))))) * 255)
            .toString(16)
            .padStart(2, "0");
    return `#${toHex(lin[0])}${toHex(lin[1])}${toHex(lin[2])}`;
}

function hexToRgb(hex: string): Rgb | undefined {
    const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
    if (!m) return undefined;
    const n = parseInt(m[1], 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

function hslToRgb(h: number, s: number, l: number): Rgb {
    const sF = s / 100;
    const lF = l / 100;
    const k = (n: number) => (n + h / 30) % 12;
    const a = sF * Math.min(lF, 1 - lF);
    const f = (n: number) => lF - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
    return [f(0), f(8), f(4)];
}

/**
 * The OKLCH hue of a pane's identity: from an explicit "Pane Color" pick
 * (`frame:hue`, an HSL hue, converted through the colour the picker shows,
 * `hsl(h, 65%, 52%)`) or else from the agent's persisted identity hex
 * (`frame:activebordercolor`). Undefined when the pane has neither.
 */
export function identityOklchHue(hslHue: number | undefined, identityHex: string | undefined): number | undefined {
    if (typeof hslHue === "number") return oklchHueOfSrgb(hslToRgb(hslHue, 65, 52));
    if (identityHex) {
        const rgb = hexToRgb(identityHex);
        if (rgb) return oklchHueOfSrgb(rgb);
    }
    return undefined;
}

/** The colour for `role` of an identity hue in a theme, as `#rrggbb`. */
export function paneRoleColor(
    hslHue: number | undefined,
    identityHex: string | undefined,
    isLightTheme: boolean,
    role: PaneColorRole,
): string | undefined {
    const h = identityOklchHue(hslHue, identityHex);
    if (h === undefined) return undefined;
    const t = PANE_COLOR_TOKENS[isLightTheme ? "light" : "dark"][role];
    return oklchToHex(t.l, t.c, h);
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
