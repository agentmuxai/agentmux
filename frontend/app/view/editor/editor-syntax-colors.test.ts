// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The editor's syntax colours (`--syntax-*`, theme.scss) read on every theme.
 * The dark palette is One Dark's and the light one is its own, so this reads
 * the theme files for each theme's surface and checks the palette meant for
 * that theme's polarity against it. The editor used to take One Dark's colours
 * on every theme, which left code pale on the four light ones.
 */

import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { LIGHT_THEME_IDS } from "@/app/menu/base-menus";

type Rgb = [number, number, number];

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");

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

const token = (scss: string, name: string) => scss.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1];

const base = read("../../theme.scss");
const lightIndex = base.indexOf('[data-theme-polarity="light"]');
const darkBlock = base.slice(0, lightIndex);
const lightBlock = base.slice(lightIndex);

const NAMES = ["keyword", "name", "function", "constant", "plain", "type", "operator", "comment", "string", "invalid"];

// The text colours are 4.5:1 or better on a light page. One Dark's are not all
// that high on a dark one (its comment grey is the lowest, deliberately
// brightened from the original), so the dark bar is the 3:1 of large and
// non-body text, which the current palette clears on every dark theme.
const BAR = { light: 4.5, dark: 3 };

const themes = readdirSync(new URL("../../themes/", import.meta.url))
    .filter((f) => f.endsWith(".scss") && f !== "index.scss")
    .map((f) => {
        const id = f.replace(/\.scss$/, "");
        return { id, surface: parse(token(read(`../../themes/${f}`), "main-bg-color")), light: LIGHT_THEME_IDS.has(id) };
    });

describe("editor syntax colours", () => {
    it("finds every theme's surface", () => {
        expect(themes.length).toBeGreaterThanOrEqual(12);
        for (const t of themes) expect(t.surface, t.id).not.toBeNull();
    });

    for (const t of themes) {
        it(`reads on ${t.id}`, () => {
            const source = t.light ? lightBlock : darkBlock;
            for (const name of NAMES) {
                const colour = parse(token(source, `syntax-${name}`));
                expect(colour, `--syntax-${name}`).not.toBeNull();
                const ratio = contrast(colour!, t.surface!);
                expect(ratio, `--syntax-${name} on ${t.id}: ${ratio.toFixed(2)}`).toBeGreaterThanOrEqual(t.light ? BAR.light : BAR.dark);
            }
        });
    }
});
