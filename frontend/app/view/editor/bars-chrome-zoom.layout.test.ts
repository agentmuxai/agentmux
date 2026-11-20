// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's control bars follow the chrome's zoom and its bodies follow the pane's, in a real browser, at the four
 * corners (pane zoom 0.5 and 2, chrome zoom 0.5 and 2): the tree's toolbar, the document tab bar, the Markdown mode
 * toolbar and the viewers' bar (the same class) are the chrome's size whatever the pane's level, the tree's rows and the
 * document are the pane's whatever the chrome's, and no bar is the product of the two. Skips with no usable browser.
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
const CSS = sass.compile(join(__dirname, "editor-view.scss")).css;
const TOKENS = `:root{--space-1:4px;--space-2:8px;--space-3:12px;--border-color:#444;--block-bg-color:#111;--main-text-color:#eee}
*{box-sizing:border-box}body{margin:0;font:13px sans-serif;background:#111;color:#eee}`;

/** A probe is a box 20 px (a bar's content) or 100 px (a body) tall at no zoom. */
const probe = (name: string, px: number) => `<div data-probe="${name}" style="height:${px}px;width:${px}px"></div>`;

/** The Editor's two zoomed wrappers as editor-view.tsx draws them, at pane level `p`, in a window at chrome level `c`. */
const pills = `<span style="display:inline-block;width:40px;height:20px"></span>`.repeat(4);
/** The tree's column at its stored width (240 px at 100%), drawn `p` times wider, with four buttons in its toolbar. */
const column = (p: number, c: number) =>
    `<div data-column="p${p}c${c}" style="--zoomfactor:${c};width:${240 * p}px;overflow:hidden">` +
    `<div class="editor-tree-zoom" style="zoom:${p};--editor-tree-zoom:${p}"><div class="file-tree-toolbar" data-bar>${pills}</div></div></div>`;

const corner = (p: number, c: number) =>
    `<div data-case="p${p}c${c}" style="--zoomfactor:${c}">` +
    `<div class="editor-main-zoom" style="zoom:${p};--editor-main-zoom:${p}">` +
    `<div class="doc-tab-strip">${probe("tabs", 20)}</div>` +
    `<div class="editor-mode-toolbar">${probe("mode", 20)}</div>` +
    `<div class="editor-mode-toolbar editor-viewer-bar">${probe("viewer", 20)}</div>` +
    `${probe("document", 100)}</div>` +
    `<div class="editor-tree-zoom" style="zoom:${p};--editor-tree-zoom:${p}">` +
    `<div class="file-tree-toolbar">${probe("treebar", 20)}</div>${probe("treebody", 100)}</div></div>`;

const MEASURE = `
const out = {};
const h = (el) => Math.round(el.getBoundingClientRect().height * 100) / 100;
for (const c of document.querySelectorAll("[data-case]")) {
    const o = {};
    for (const p of c.querySelectorAll("[data-probe]")) o[p.dataset.probe] = h(p);
    out[c.dataset.case] = o;
}
for (const c of document.querySelectorAll("[data-column]")) {
    const bar = c.querySelector("[data-bar]");
    out["col-" + c.dataset.column] = { overflow: bar.scrollWidth - bar.clientWidth };
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

const LEVELS = [0.5, 2];

describe.skipIf(!BROWSER)("Editor control bars and the chrome zoom", () => {
    it("draws the bars at the chrome's level and the bodies at the pane's, at the four corners", async () => {
        const cases = LEVELS.flatMap((p) => LEVELS.map((c) => corner(p, c) + column(p, c))).join("");
        const html = `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${CSS}</style>${cases}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        const r = await measureInBrowser<Record<string, Record<string, number>>>(BROWSER!, html, { width: 600, height: 4000 });

        for (const p of LEVELS) {
            for (const c of LEVELS) {
                const m = r[`p${p}c${c}`];
                for (const bar of ["tabs", "mode", "viewer", "treebar"]) expect(m[bar] / 20, `${bar} at pane ${p}, chrome ${c}`).toBeCloseTo(c, 1);
                for (const body of ["document", "treebody"]) expect(m[body] / 100, `${body} at pane ${p}, chrome ${c}`).toBeCloseTo(p, 1);
                // A narrow tree at a large chrome zoom: the buttons wrap, nothing runs past the column.
                expect(r[`col-p${p}c${c}`].overflow, `the tree's toolbar at pane ${p}, chrome ${c}`).toBeLessThanOrEqual(1);
            }
        }
    }, 60_000);
});
