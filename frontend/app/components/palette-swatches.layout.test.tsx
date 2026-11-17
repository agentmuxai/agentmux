// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the colour picker's swatches sit, measured in a real browser (#47917). With a "no colour" swatch it is alone
 * in the first column with the cell under it empty, and the 14 colours are two rows of 7, each colour of the second row
 * directly under one of the first. A picker without the swatch is two rows of 7. A context that narrows the grid with
 * its pane's width (`--palette-cols`) keeps the same shape. It skips, rather than fails, on a machine with no usable
 * browser (layout-browser.ts findBrowser).
 */
import { cleanup, render } from "@solidjs/testing-library";
import { join } from "node:path";
import * as sass from "sass";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";
import { PaletteSwatches } from "./palette-swatches";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = sass.compile(join(__dirname, "palette-swatches.scss")).css;
const TOKENS = ":root{--space-1:4px;--main-text-color:#ddd;--main-bg-color:#000;--text-2xs:10px} body{margin:8px}";

interface Case {
    id: string;
    none: boolean;
    /** What a context sets on the grid: a fixed `columns`, or `--palette-cols` in its own styles. */
    columns?: number;
    cols?: number;
}
const CASES: Case[] = [
    { id: "plain", none: false },
    { id: "none", none: true },
    { id: "none-narrow", none: true, cols: 5 },
    { id: "none-one-row", none: true, columns: 15 },
];

function markup(c: Case): string {
    const { container, unmount } = render(() => (
        <PaletteSwatches label="c" value={null} onSelect={() => {}} noneLabel={c.none ? "No colour" : undefined} columns={c.columns} />
    ));
    const html = container.innerHTML;
    unmount();
    return `<div data-case="${c.id}" data-cols="${c.cols ?? ""}">${html}</div>`;
}

// A context's own `--palette-cols` is set on the grid element, as settings.scss does.
const MEASURE = `
const out = {};
for (const root of document.querySelectorAll("[data-case]")) {
  const grid = root.querySelector(".palette-swatches");
  if (root.dataset.cols) grid.style.setProperty("--palette-cols", root.dataset.cols);
  out[root.dataset.case] = Array.from(grid.querySelectorAll(".palette-swatch-anchor")).map((a) => {
    const r = a.getBoundingClientRect();
    return { left: r.left, top: r.top, width: r.width };
  });
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

type Box = { left: number; top: number; width: number };

describe.skipIf(!BROWSER)("the colour picker's layout", () => {
    let r: Record<string, Box[]>;
    beforeAll(async () => {
        const html = `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${CASES.map(markup).join("")}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        r = await measureInBrowser<Record<string, Box[]>>(BROWSER!, html, { width: 600, height: 600 });
    }, 60_000);
    afterAll(() => cleanup());

    const colours = (id: string) => (CASES.find((c) => c.id === id)!.none ? r[id].slice(1) : r[id]);
    const columnsOf = (boxes: Box[]) => boxes.map((b) => Math.round(b.left));
    const sameRow = (boxes: Box[]) => boxes.every((b) => Math.round(b.top) === Math.round(boxes[0].top));

    it("without the none swatch: two rows of 7, each colour under one of the first row", () => {
        const c = colours("plain");
        expect(c).toHaveLength(14);
        const cols = columnsOf(c);
        expect(new Set(cols.slice(0, 7)).size).toBe(7);
        expect(cols.slice(7)).toEqual(cols.slice(0, 7));
        expect(c[7].top).toBeGreaterThan(c[0].top);
        expect(sameRow(c.slice(0, 7))).toBe(true);
    });

    it("with the none swatch: it is alone in the first column, and the cell under it is empty", () => {
        const [none] = r.none;
        const c = colours("none");
        expect(r.none).toHaveLength(15);
        for (const b of c) expect(b.left, "a colour in the none swatch's column").toBeGreaterThan(none.left + none.width - 1);
        expect(Math.round(none.top)).toBe(Math.round(c[0].top));
    });

    it("with the none swatch: the colours are two rows of 7, the 8th in the column of the 1st", () => {
        const c = colours("none");
        const cols = columnsOf(c);
        expect(new Set(cols.slice(0, 7)).size).toBe(7);
        expect(cols[7]).toBe(cols[0]);
        expect(cols.slice(7)).toEqual(cols.slice(0, 7));
        expect(c[7].top).toBeGreaterThan(c[0].top);
        expect(sameRow(c.slice(0, 7))).toBe(true);
    });

    it("narrowed by its context to 5 columns: the none swatch still alone in its column, 4 colours a row", () => {
        const [none] = r["none-narrow"];
        const c = colours("none-narrow");
        for (const b of c) expect(b.left).toBeGreaterThan(none.left + none.width - 1);
        const cols = columnsOf(c);
        expect(cols[4]).toBe(cols[0]);
        expect(c[4].top).toBeGreaterThan(c[0].top);
        expect(Math.round(c[3].top)).toBe(Math.round(c[0].top));
    });

    it("given 15 columns: the none swatch and the 14 colours are one row", () => {
        const all = r["none-one-row"];
        expect(all).toHaveLength(15);
        expect(sameRow(all)).toBe(true);
        const lefts = all.map((b) => b.left);
        expect(lefts).toEqual([...lefts].sort((a, b) => a - b));
    });
});
