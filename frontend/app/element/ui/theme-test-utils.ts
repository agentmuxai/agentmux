// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Test support: resolve a bundled theme's tokens from the SCSS files and compute WCAG contrast,
 * the way the cascade does (the theme's value, then the light polarity's, then `:root`'s). Shared
 * by pill-colors.test.ts and tag-contrast.test.ts so there is one resolver, not two.
 * Not imported by app code.
 */

import { readdirSync, readFileSync } from "node:fs";
import { LIGHT_THEME_IDS } from "@/app/menu/base-menus";

export type Rgb = [number, number, number];

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");

/** `rgb(…)`/`rgba(…)` or `#rrggbb`, ignoring alpha; null for anything else. */
export function parseColor(value: string | undefined): Rgb | null {
    if (!value) return null;
    const m = value.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/);
    if (m) return [Number(m[1]), Number(m[2]), Number(m[3])];
    const h = value.match(/#([0-9a-f]{6})\b/i);
    if (h) return [0, 2, 4].map((i) => parseInt(h[1].slice(i, i + 2), 16)) as Rgb;
    return null;
}

export function luminance([r, g, b]: Rgb): number {
    const ch = (c: number) => {
        const s = c / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}

export function contrast(a: Rgb, b: Rgb): number {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
}

const linear = (c: number) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
};

function toOklab([r, g, b]: Rgb): Rgb {
    const [R, G, B] = [linear(r), linear(g), linear(b)];
    const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
    const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
    const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
    return [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ];
}

/** How far apart two colours look: the distance in OKLab, times 100 (the Kanban stages and the Tower's kinds use it). */
export function apartOklab(a: Rgb, b: Rgb): number {
    const [x, y] = [toOklab(a), toOklab(b)];
    return 100 * Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]);
}

/** `top` laid over `under` at `alpha` (what `color-mix(in srgb, top alpha, transparent)` paints over `under`). */
export function over(top: Rgb, under: Rgb, alpha: number): Rgb {
    return top.map((c, i) => Math.round(c * alpha + under[i] * (1 - alpha))) as Rgb;
}

/** A token's value in a block of SCSS (the first match). */
function token(scss: string, name: string): string | undefined {
    return scss.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1];
}

/** Every bundled theme's id (the files in themes/ except the barrel), without `default`. */
export function bundledThemeIds(): string[] {
    return readdirSync(new URL("../../themes/", import.meta.url))
        .filter((f) => f.endsWith(".scss") && f !== "index.scss")
        .map((f) => f.replace(/\.scss$/, ""));
}

export interface ResolvedTheme {
    id: string;
    light: boolean;
    /** A token's value text as the cascade yields it, or undefined. */
    raw: (name: string) => string | undefined;
    /** A colour token as RGB; a `var(--other)` value is looked up again from the top, and a two-colour `color-mix` is resolved. */
    pick: (name: string, depth?: number) => Rgb | null;
    /** A wash token (`color-mix(in srgb, var(--x) N%, transparent)` or `rgba()`) laid over `surface`. */
    washOf: (name: string, surface: Rgb) => Rgb;
}

/** Resolve the tokens of `id` (`"default"` for the unthemed `:root`). */
export function resolveTheme(id: string): ResolvedTheme {
    const base = read("../../theme.scss");
    const lightBlock = base.slice(base.indexOf('[data-theme-polarity="light"]'));
    const own = id === "default" ? "" : read(`../../themes/${id}.scss`);
    const light = LIGHT_THEME_IDS.has(id);
    const layers = [own, ...(light ? [lightBlock] : []), base];
    const raw = (name: string): string | undefined => {
        for (const layer of layers) {
            const value = token(layer, name);
            if (value !== undefined) return value;
        }
        return undefined;
    };
    const pick = (name: string, depth = 0): Rgb | null => {
        const value = raw(name);
        if (value === undefined) return null;
        const ref = value.match(/^\s*var\(--([a-z0-9-]+)\)\s*$/i);
        if (ref && depth < 5) return pick(ref[1], depth + 1);
        // `color-mix(in srgb, var(--a) N%, var(--b))`: a over b at N%.
        const mix = value.match(/^\s*color-mix\(\s*in srgb\s*,\s*var\(--([a-z0-9-]+)\)\s+([\d.]+)%\s*,\s*var\(--([a-z0-9-]+)\)\s*\)\s*$/i);
        if (mix && depth < 5) {
            const [top, under] = [pick(mix[1], depth + 1), pick(mix[3], depth + 1)];
            return top && under ? over(top, under, Number(mix[2]) / 100) : null;
        }
        return parseColor(value);
    };
    const washOf = (name: string, surface: Rgb): Rgb => {
        const value = raw(name) ?? "";
        const mix = value.match(/color-mix\(\s*in srgb\s*,\s*var\(--([a-z0-9-]+)\)\s+([\d.]+)%/i);
        if (mix) return over(pick(mix[1])!, surface, Number(mix[2]) / 100);
        const rgba = value.match(/rgba\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*([\d.]+)\s*\)/);
        if (rgba) return over([Number(rgba[1]), Number(rgba[2]), Number(rgba[3])], surface, Number(rgba[4]));
        throw new Error(`--${name} is neither a color-mix nor an rgba(): ${value}`);
    };
    return { id, light, raw, pick, washOf };
}
