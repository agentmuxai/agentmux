// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Toolchain pane's status pills read in every theme. A pill is a 12% wash
 * of the theme's own success or warning colour (`--success-bg`,
 * `--warning-bg`, theme.scss) with the main text colour on it. This
 * reads the theme files themselves, resolves the tokens the way the cascade
 * does (the theme's value, then the light polarity's, then `:root`'s), and
 * checks the main text colour against the wash laid over the theme's surface, so a later
 * theme tweak can't push a pill below the bar unnoticed
 * (docs/reports/REPORT_TOOLCHAIN_PANE_THEME_INTEGRATION_2026_10_19.md).
 */

import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { LIGHT_THEME_IDS } from "@/app/menu/base-menus";

type Rgb = [number, number, number];

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");

/** `rgb(…)`/`rgba(…)` or `#rrggbb`, ignoring alpha; null for anything else. */
function parse(value: string | undefined): Rgb | null {
    if (!value) return null;
    const m = value.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/);
    if (m) return [Number(m[1]), Number(m[2]), Number(m[3])];
    const h = value.match(/#([0-9a-f]{6})\b/i);
    if (h) return [0, 2, 4].map((i) => parseInt(h[1].slice(i, i + 2), 16)) as Rgb;
    return null;
}

function luminance([r, g, b]: Rgb): number {
    const ch = (c: number) => {
        const s = c / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}

function contrast(a: Rgb, b: Rgb): number {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
}

/** `top` laid over `under` at `alpha` (what `color-mix(in srgb, top alpha, transparent)` paints). */
function over(top: Rgb, under: Rgb, alpha: number): Rgb {
    return top.map((c, i) => Math.round(c * alpha + under[i] * (1 - alpha))) as Rgb;
}

/** A token's value in a block of SCSS (the first match). */
function token(scss: string, name: string): string | undefined {
    return scss.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1];
}

const base = read("../../theme.scss");
const lightBlock = base.slice(base.indexOf('[data-theme-polarity="light"]'));

// WCAG AA for body text. The pills are 11px, so 4.5:1 is the bar. The themes'
// own success and warning colours do not reach it as text on a wash of
// themselves (about 2.6:1 on the light themes, and 3.4 to 4.4 on several dark
// ones), which is why the pill text is the main text colour and the semantic
// colour is the wash, the line and the icon.
const AA = 4.5;
// solarized-light's secondary text colour is about 2.6:1 on its own surface,
// everywhere it is used (a theme-level choice, not this pane's), so the plain
// pill, which uses it, is pinned there rather than measured against AA.
const SECONDARY_TEXT_BELOW_AA = new Set(["solarized-light"]);

describe("the Toolchain status pills", () => {
    const themes = readdirSync(new URL("../../themes/", import.meta.url))
        .filter((f) => f.endsWith(".scss") && f !== "index.scss")
        .map((f) => f.replace(/\.scss$/, ""));

    it("cover every theme", () => {
        expect(themes.length).toBeGreaterThanOrEqual(12);
    });

    for (const id of ["default", ...themes]) {
        it(`read in ${id}`, () => {
            const own = id === "default" ? "" : read(`../../themes/${id}.scss`);
            const light = LIGHT_THEME_IDS.has(id);
            // The cascade, layer by layer (the theme, the light polarity, `:root`);
            // a value that is `var(--other)` is looked up again from the top, as the
            // browser does (`--success-text-color: var(--success-color)`).
            const layers = [own, ...(light ? [lightBlock] : []), base];
            const rawValue = (name: string): string | undefined => {
                for (const layer of layers) {
                    const value = token(layer, name);
                    if (value !== undefined) return value;
                }
                return undefined;
            };
            const pick = (name: string, depth = 0): Rgb | null => {
                const value = rawValue(name);
                if (value === undefined) return null;
                const ref = value.match(/^\s*var\(--([a-z0-9-]+)\)\s*$/i);
                if (ref && depth < 5) return pick(ref[1], depth + 1);
                return parse(value);
            };
            // A wash token as the theme resolves it, laid over the theme's surface:
            // `color-mix(in srgb, var(--x) N%, transparent)` is `--x` at N%, and an
            // `rgba(r, g, b, a)` is its colour at `a`. Read from the token itself, so
            // a theme that overrides it or a shared change of its percentage is what
            // gets measured.
            const washOf = (name: string, surfaceRgb: Rgb): Rgb => {
                const value = rawValue(name) ?? "";
                const mix = value.match(/color-mix\(\s*in srgb\s*,\s*var\(--([a-z0-9-]+)\)\s+([\d.]+)%/i);
                if (mix) return over(pick(mix[1])!, surfaceRgb, Number(mix[2]) / 100);
                const rgba = value.match(/rgba\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*([\d.]+)\s*\)/);
                if (rgba) return over([Number(rgba[1]), Number(rgba[2]), Number(rgba[3])], surfaceRgb, Number(rgba[4]));
                throw new Error(`--${name} is neither a color-mix nor an rgba(): ${value}`);
            };
            const surface = pick("main-bg-color");
            expect(surface, "the theme's surface resolves").toBeTruthy();
            const text = pick("main-text-color");
            expect(text, "the theme's text colour resolves").toBeTruthy();
            // The plain pill (checking, managed, up to date, optional): the secondary
            // text colour on `--tint-3`.
            const secondary = pick("secondary-text-color");
            expect(secondary, "the secondary text colour resolves").toBeTruthy();
            const plain = contrast(secondary!, washOf("tint-3", surface!));
            if (SECONDARY_TEXT_BELOW_AA.has(id)) expect(plain).toBeLessThan(AA); // leaves the list when the theme is fixed
            else expect(plain, `plain pill text in ${id}`).toBeGreaterThanOrEqual(AA);
            for (const kind of ["success", "warning"] as const) {
                const wash = washOf(`${kind}-bg`, surface!);
                expect(contrast(text!, wash), `pill text on the ${kind} wash in ${id}`).toBeGreaterThanOrEqual(AA);
            }
        });
    }
});
