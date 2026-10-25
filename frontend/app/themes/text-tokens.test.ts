// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The text tokens read on every theme (the light-theme rule R3: 4.5:1 for text). Read from the theme
 * files, so a later tweak can't drop one below the bar unnoticed. Found by looking at the running
 * app on each theme: Solarized Light's secondary text was 2.9:1, and the faint grey was under 4.5:1
 * on all four light themes.
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

/** A token's value in a block of SCSS (the first match). */
const token = (scss: string, name: string) => scss.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1];

const base = read("../theme.scss");
const lightBlock = base.slice(base.indexOf('[data-theme-polarity="light"]'));

const themes = readdirSync(new URL("./", import.meta.url))
    .filter((f) => f.endsWith(".scss") && f !== "index.scss")
    .map((f) => f.replace(/\.scss$/, ""));

describe("the text tokens", () => {
    it("cover every theme", () => {
        expect(themes.length).toBeGreaterThanOrEqual(12);
    });

    for (const id of ["default", ...themes]) {
        const light = LIGHT_THEME_IDS.has(id);
        const own = id === "default" ? "" : read(`./${id}.scss`);
        // A theme's own value wins, then the light polarity's, then :root's.
        const pick = (name: string) =>
            parse(token(own, name)) ?? (light ? parse(token(lightBlock, name)) : null) ?? parse(token(base, name));

        it(`secondary text passes AA on ${id}`, () => {
            const fg = pick("secondary-text-color");
            const bg = pick("main-bg-color");
            expect(fg && bg).toBeTruthy();
            expect(contrast(fg!, bg!)).toBeGreaterThanOrEqual(4.5);
        });

        // The faint grey is a tertiary tone on the dark themes (about 3:1 there); the light themes'
        // rule is 4.5:1 for any text.
        if (light) {
            it(`the faint grey passes AA on ${id}`, () => {
                const fg = pick("grey-text-color");
                const bg = pick("main-bg-color");
                expect(fg && bg).toBeTruthy();
                expect(contrast(fg!, bg!)).toBeGreaterThanOrEqual(4.5);
            });
        }
    }
});
