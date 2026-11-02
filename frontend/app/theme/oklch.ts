// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OKLab / OKLCH ⇄ sRGB (Björn Ottosson's matrices), the one colour-space
 * module: the shared palette (palette.ts) places its hues in OKLCH and the
 * pane colour scheme (block/pane-color-scheme.ts) turns a hue into a tone
 * for a role. Nothing here knows a palette or a theme.
 */

// ── OKLab / OKLCH ⇄ sRGB (Björn Ottosson's matrices) ───────────────────────

export type Rgb = [number, number, number];

export function srgbToLinear(c: number): number {
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
export function oklchHueOfSrgb(rgb: Rgb): number {
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

/** `#rrggbb` (the `#` optional) as 0–1 channels, or undefined. */
export function hexToRgb(hex: string): Rgb | undefined {
    const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
    if (!m) return undefined;
    const n = parseInt(m[1], 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

export function hslToRgb(h: number, s: number, l: number): Rgb {
    const sF = s / 100;
    const lF = l / 100;
    const k = (n: number) => (n + h / 30) % 12;
    const a = sF * Math.min(lF, 1 - lF);
    const f = (n: number) => lF - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
    return [f(0), f(8), f(4)];
}

/** The OKLCH hue, degrees 0–360, of a `#rrggbb`; undefined when it is not one. */
export function oklchHueOfHex(hex: string): number | undefined {
    const rgb = hexToRgb(hex);
    return rgb ? oklchHueOfSrgb(rgb) : undefined;
}
