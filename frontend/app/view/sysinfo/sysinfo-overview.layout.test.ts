// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Overview's four tiles, measured in a real browser as the pane is narrowed a few pixels at a time: one row of
 * four, then two by two, never three and one; and the width at which four stop fitting follows the pane's zoom.
 * Skips with no usable browser.
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
const css = sass.compile(join(__dirname, "sysinfo-overview.scss")).css;
const tile = (name: string) => `<div class="sysinfo-tile"><div class="sysinfo-tile-name">${name}</div></div>`;
const MEASURE = `
const out = {};
for (const zoom of [1, 1.5]) {
  const pane = document.querySelector(".sysinfo-pane");
  pane.style.setProperty("--sysinfo-zoom", String(zoom));
  const rows = [];
  for (let w = 120; w <= 1400; w += 5) {
    pane.style.width = w + "px";
    const tops = [...document.querySelectorAll(".sysinfo-tile")].map((t) => Math.round(t.getBoundingClientRect().top));
    const shape = [...new Set(tops)].map((top) => tops.filter((t) => t === top).length);
    rows.push([w, shape.join("+")]);
  }
  out[zoom] = rows;
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

/** The widest pane at which the shape is `shape`, among the measured `rows` (width, shape). */
const widest = (rows: Array<[number, string]>, shape: string) => Math.max(...rows.filter(([, s]) => s === shape).map(([w]) => w));

describe.skipIf(!BROWSER)("Sysinfo Overview tiles", () => {
    it("go from one row of four to two by two as the pane narrows, never three and one", async () => {
        const html =
            `<!doctype html><meta charset="utf-8"><style>body{margin:0;font:13px sans-serif}.sysinfo-pane{--sysinfo-gap:2px}${css}</style>` +
            `<div class="sysinfo-pane"><div class="sysinfo-overview-tiles">${["CPU", "Memory", "Network", "Disk"].map(tile).join("")}</div></div>` +
            `<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        const r = await measureInBrowser<Record<string, Array<[number, string]>>>(BROWSER!, html, { width: 1500, height: 400 });
        for (const zoom of ["1", "1.5"]) {
            const shapes = new Set(r[zoom].map(([, s]) => s));
            // Only the shapes a person wants: all four in a row, two and two, or one column. No row with a tile alone beside others.
            expect([...shapes].filter((s) => s !== "4" && s !== "2+2" && s !== "1+1+1+1"), `zoom ${zoom}`).toEqual([]);
            expect(shapes.has("4") && shapes.has("2+2"), `zoom ${zoom}`).toBe(true);
            // Monotone: a wider pane never has fewer columns than a narrower one.
            const order = ["1+1+1+1", "2+2", "4"];
            const idx = r[zoom].map(([, s]) => order.indexOf(s));
            expect(idx, `zoom ${zoom}`).toEqual([...idx].sort((a, b) => a - b));
        }
        // Four need room for four least-width tiles, and that room grows with the zoom.
        expect(widest(r["1.5"], "2+2")).toBeGreaterThan(widest(r["1"], "2+2"));
        // The single column keeps its place (two 150px tiles side by side) whatever the zoom.
        expect(widest(r["1"], "1+1+1+1")).toBe(widest(r["1.5"], "1+1+1+1"));
        expect(widest(r["1"], "1+1+1+1")).toBeLessThan(302);
    });
});
