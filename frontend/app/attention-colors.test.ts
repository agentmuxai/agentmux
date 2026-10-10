// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "an agent needs you" colours stay readable in every theme
 * (docs/reports/REPORT_AGENT_ATTENTION_CTA_CONTRAST_AND_TONE_2026_10_10.md §1):
 * text on a filled attention button meets WCAG AA (4.5:1), and the fill
 * stands out from the theme's background (3:1). Read from the theme files
 * themselves, so a later tweak can't drift below the bar unnoticed.
 */

import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { LIGHT_THEME_IDS } from "@/app/menu/base-menus";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");

type Rgb = [number, number, number];

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
function token(scss: string, name: string): string | undefined {
    return scss.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1];
}

const base = read("./theme.scss");
const lightBlock = base.slice(base.indexOf('[data-theme-polarity="light"]'));

describe("the attention colours", () => {
    const themes = readdirSync(new URL("./themes/", import.meta.url))
        .filter((f) => f.endsWith(".scss") && f !== "index.scss")
        .map((f) => f.replace(/\.scss$/, ""));

    it("cover every theme", () => {
        expect(themes.length).toBeGreaterThanOrEqual(12);
    });

    for (const id of ["default", ...themes]) {
        it(`read well in ${id}`, () => {
            const own = id === "default" ? "" : read(`./themes/${id}.scss`);
            const light = LIGHT_THEME_IDS.has(id);
            // A theme's own value wins, then the light polarity's, then :root's.
            const pick = (name: string) =>
                parse(token(own, name)) ?? (light ? parse(token(lightBlock, name)) : null) ?? parse(token(base, name));
            const fill = pick("attention-color");
            const text = pick("attention-text-color");
            const bg = parse(token(own, "main-bg-color")) ?? parse(token(base, "main-bg-color"));
            expect(fill && text && bg, "attention tokens and a background resolve").toBeTruthy();
            expect(contrast(fill!, text!), "text on the filled button").toBeGreaterThanOrEqual(4.5);
            expect(contrast(fill!, bg!), "the fill against the background").toBeGreaterThanOrEqual(3);
        });
    }
});
