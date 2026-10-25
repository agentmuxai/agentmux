// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The accent colour read as text (`--accent-fg-color`: an accent-toned control's label) passes AA
 * (4.5:1) on every theme's surface. Found by looking at the running app on each theme: the accent
 * button labels ("Add", "New task") were 4.4:1 on Light, 4.0:1 on Catppuccin Latte and 3.2:1 on
 * Solarized Light. Read from the theme files, so a later tweak can't drop one below the bar.
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

const themes = readdirSync(new URL("./", import.meta.url))
    .filter((f) => f.endsWith(".scss") && f !== "index.scss")
    .map((f) => f.replace(/\.scss$/, ""));

describe("the accent colour as text", () => {
    for (const id of ["default", ...themes]) {
        it(`passes AA on ${id}`, () => {
            const own = id === "default" ? "" : read(`./${id}.scss`);
            const pick = (name: string) => parse(token(own, name)) ?? parse(token(base, name));
            // The base declares it as `var(--accent-color)`: a theme that doesn't override it uses its accent.
            const fg = parse(token(own, "accent-fg-color")) ?? pick("accent-color");
            const bg = pick("main-bg-color");
            expect(fg && bg, "the accent and the surface resolve").toBeTruthy();
            expect(contrast(fg!, bg!)).toBeGreaterThanOrEqual(4.5);
        });
    }

    it("is darker than the accent on the light themes that need it", () => {
        for (const id of themes.filter((t) => LIGHT_THEME_IDS.has(t))) {
            const own = read(`./${id}.scss`);
            expect(token(own, "accent-fg-color"), id).toBeTruthy();
        }
    });
});
