// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words on an error-red fill (the TEAR OFF place a dragged pane leaves, tilelayout.scss) read at 3:1 or
 * better on every theme's error colour, large bold text's bar. White on the default red does; on the pale reds of
 * Catppuccin and Tokyo Night it was 2.3:1 and 2.7:1, so those themes set their dark base as `--on-error-color`.
 * Read from the theme files, so a later tweak can't drop one below the bar.
 */

import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

type Rgb = [number, number, number];

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");

/** `rgb(…)`/`rgba(…)` or `#rrggbb`, ignoring alpha; null for anything else. */
function parse(value: string | undefined): Rgb | null {
    if (!value) return null;
    const m = value.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/);
    if (m) return [Number(m[1]), Number(m[2]), Number(m[3])];
    const h = value.match(/#([0-9a-f]{3}|[0-9a-f]{6})\b/i);
    if (!h) return null;
    const hex = h[1].length === 3 ? [...h[1]].map((c) => c + c).join("") : h[1];
    return [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
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

describe("the words on an error fill", () => {
    for (const id of ["default", ...themes]) {
        it(`read at 3:1 on ${id}`, () => {
            const own = id === "default" ? "" : read(`./${id}.scss`);
            const pick = (name: string) => parse(token(own, name)) ?? parse(token(base, name));
            const words = pick("on-error-color");
            const fill = pick("error-color");
            expect(words && fill, "the words and the fill resolve").toBeTruthy();
            expect(contrast(words!, fill!)).toBeGreaterThanOrEqual(3);
        });
    }

    it("has a lighter edge wherever the words are dark, and a dark one where they are white", () => {
        for (const id of ["default", ...themes]) {
            const own = id === "default" ? "" : read(`./${id}.scss`);
            const pick = (name: string) => parse(token(own, name)) ?? parse(token(base, name));
            const [words, edge] = [pick("on-error-color")!, pick("on-error-edge")!];
            expect(luminance(words) > 0.5, id).toBe(luminance(edge) < 0.5);
        }
    });
});
