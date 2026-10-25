// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The status colours as text (`--error-text-color`, `--warning-text-color`, `--success-text-color`)
 * pass AA (4.5:1) on every theme's surface and on the colour's own 12% wash (a notice's background).
 * Found by looking at the running app on each theme: the colours themselves, which 287 rules used as
 * text, were 2.3 to 4.3:1 on several themes (warning on Catppuccin Latte 2.3, error on Gruvbox 2.7).
 * Each theme's text token is its colour with the lightness moved until it passes; read here from the
 * theme files, so a later tweak can't drop one below the bar.
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

/** `fg` at `alpha` over `bg`. */
const over = (fg: Rgb, bg: Rgb, alpha: number): Rgb => fg.map((c, i) => c * alpha + bg[i] * (1 - alpha)) as Rgb;

/** A token's value in a block of SCSS (the first match). */
const token = (scss: string, name: string) => scss.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1]?.trim();

const base = read("../theme.scss");
const lightBlock = base.slice(base.indexOf('[data-theme-polarity="light"]'));

const themes = readdirSync(new URL("./", import.meta.url))
    .filter((f) => f.endsWith(".scss") && f !== "index.scss")
    .map((f) => f.replace(/\.scss$/, ""));

const KINDS = ["error", "warning", "success"] as const;

describe("the status colours as text", () => {
    for (const id of ["default", ...themes]) {
        const light = LIGHT_THEME_IDS.has(id);
        const own = id === "default" ? "" : read(`./${id}.scss`);
        // A theme's own value wins, then the light polarity's, then :root's; `var(--x)` follows to that token.
        const resolve = (name: string, depth = 0): Rgb | null => {
            const raw = token(own, name) ?? (light ? token(lightBlock, name) : undefined) ?? token(base, name);
            const alias = raw?.match(/^var\(--([\w-]+)\)$/);
            if (alias) return depth < 5 ? resolve(alias[1], depth + 1) : null;
            return parse(raw);
        };

        for (const kind of KINDS) {
            it(`${kind} text passes AA on ${id}, on the surface and on its own wash`, () => {
                const text = resolve(`${kind}-text-color`);
                const color = resolve(`${kind}-color`);
                const bg = resolve("main-bg-color");
                expect(text && color && bg, "the tokens resolve").toBeTruthy();
                expect(contrast(text!, bg!)).toBeGreaterThanOrEqual(4.5);
                expect(contrast(text!, over(color!, bg!, 0.12))).toBeGreaterThanOrEqual(4.5);
            });
        }
    }

    it("covers every theme", () => {
        expect(themes.length).toBeGreaterThanOrEqual(12);
    });
});
