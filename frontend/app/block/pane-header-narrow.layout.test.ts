// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane header in a narrow pane, measured in a real browser: the buttons that are the only way in to something (the
 * agent's Messages, the terminal's Start and Restart) can still be reached when the pane is too narrow for the
 * selected tab to show its own buttons (#48360, after #48252). The real `block.scss`, tab strip and tab actions
 * styles are compiled; the markup is the header's (blockframe.tsx, PaneTabStrip.tsx, pane-tab-actions.tsx); the width
 * fit is the real one (element/width-fit.ts with header-fit-measure.ts), bundled into the page and run on each header
 * as the window runs it. A button is reached when it is drawn inside the pane's box and is the topmost element at its
 * centre. It skips, rather than fails, on a machine with no usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import { buildSync } from "esbuild";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const ROOT = join(__dirname, "..", "..", "..");
const css = ["block/block.scss", "element/PaneTabStrip.scss", "block/pane-tab-actions.scss"].map((f) => sass.compile(join(ROOT, "frontend/app", f)).css).join("\n");

/** The real width fit and the header's measure, as one script that puts `__fit(header)` on the page. */
const FIT_SCRIPT = buildSync({
    stdin: {
        contents: `import { fitToWidth } from "./frontend/app/element/width-fit.ts";
import { headerOverrunPx } from "./frontend/app/block/header-fit-measure.ts";
window.__fit = (el) => fitToWidth(el, ["tabs", "trim", "min"], headerOverrunPx);
window.__clip = headerOverrunPx;`,
        resolveDir: ROOT,
        loader: "ts",
    },
    bundle: true,
    format: "iife",
    platform: "browser",
    write: false,
}).outputFiles[0].text;

const TOKENS = `:root{--main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--border-color:#333;--main-bg-color:#222;
--block-bg-color:#1d1d1d;--block-border-radius:6px;--header-height:30px;--header-font:12px Arial,sans-serif;--font-sans:Arial,sans-serif;
--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-md:13px;--text-base:13px;--space-0-5:2px;--space-1:4px;--space-2:8px;--space-3:12px;
--radius-sm:3px;--radius-full:999px;--pane-tab-strip-height:28px;--pane-tab-strip-zoom:1;--header-icon-size:13px;--header-icon-width:16px;
--duration-quick:0s;--z-tab-name:3}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}.pane{margin:6px 0;position:relative}
.ctl{width:22px;height:22px;padding:0;flex:none;display:flex;align-items:center;justify-content:center}`;

type View = "agent" | "term";

/** One pane `width` px wide whose header is the agent's (Find and Messages) or a terminal's (Start) at chrome zoom `zoom`. */
function pane(id: string, view: View, width: number, zoom: number): string {
    const actions =
        view === "agent"
            ? `<button class="pane-tab-action ui-button--chrome" data-btn="find">F</button><button class="pane-tab-action ui-button--chrome block-frame-end-keep" data-btn="keep">M</button>`
            : `<button class="pane-tab-action ui-button--chrome block-frame-end-keep" data-btn="keep">S</button>`;
    const kept = `<button class="ctl block-frame-end-keep" data-kept="keep">${view === "agent" ? "M" : "S"}</button>`;
    return (
        `<div class="pane" id="${id}" style="width:${width}px;--zoomfactor:${zoom}"><div class="block block-frame-default"><div class="block-frame-default-inner">` +
        `<div class="block-frame-default-header block-frame-default-header--has-tabstrip" data-role="block-header">` +
        `<div class="block-frame-header-scroll"><div class="block-frame-header-content">` +
        `<div class="block-frame-default-header-tabstrip"><div class="pane-tab-strip"><div class="pane-tab-strip-inner">` +
        `<div class="pane-tab-tip"><div class="pane-tab pane-tab--active"><span class="pane-tab-label">${view === "agent" ? "Loap-3" : "Terminal"}</span>` +
        `<div class="pane-tab-actions" role="group">${actions}</div><button class="pane-tab-close ctl" data-btn="close-tab">x</button></div></div>` +
        `<button class="pane-tab-strip-add ctl" data-btn="add">+</button></div></div></div>` +
        `<div class="block-frame-textelems-wrapper"></div><div class="block-frame-header-spacer"></div>` +
        `<div class="block-frame-end-kept">${kept}</div></div></div>` +
        `<div class="block-frame-end-icons"><span class="pane-grip ctl" data-btn="grip">g</span><button class="block-frame-minimize ctl" data-btn="min">-</button>` +
        `<button class="block-frame-magnify ctl" data-btn="max">+</button><button class="block-frame-default-close ctl" data-btn="close">x</button></div>` +
        `</div></div></div></div>`
    );
}

const WIDTHS = [125, 197, 240, 320, 600];
const CASES: Array<[View, number, number]> = [];
for (const view of ["agent", "term"] as View[]) for (const zoom of [1, 1.5]) for (const w of WIDTHS) CASES.push([view, w, zoom]);
const idOf = (view: View, w: number, z: number) => `${view}-${w}-${z}`;

const MEASURE = `
const result = {};
const reached = (el, box) => {
  if (!el) return false;
  const cs = getComputedStyle(el);
  const r = el.getBoundingClientRect();
  if (cs.display === 'none' || cs.visibility === 'hidden' || r.width === 0 || r.height === 0) return false;
  if (r.left < box.left - 0.5 || r.right > box.right + 0.5) return false;
  const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  return !!top && (top === el || el.contains(top));
};
for (const p of document.querySelectorAll('.pane')) {
  const header = p.querySelector('.block-frame-default-header');
  window.__fit(header);
  const box = p.getBoundingClientRect();
  const q = (s) => p.querySelector(s);
  result[p.id] = {
    fit: header.getAttribute('data-fit'),
    keep: reached(q('[data-btn="keep"]'), box) || reached(q('[data-kept="keep"]'), box),
    keepInTab: reached(q('[data-btn="keep"]'), box),
    find: reached(q('[data-btn="find"]'), box),
    close: reached(q('[data-btn="close"]'), box),
    scrollsSideways: header.scrollWidth - header.clientWidth,
    clipped: window.__clip(header),
    stripBox: (() => { const r = q('.block-frame-default-header-tabstrip').getBoundingClientRect(); return [Math.round(r.left), Math.round(r.right)]; })(),
    rangeBox: (() => { const r = q('.block-frame-header-scroll'); const b = r.getBoundingClientRect(); return [Math.round(b.left), Math.round(b.right), r.scrollWidth, r.clientWidth]; })(),
    endBox: (() => { const r = q('.block-frame-end-icons').getBoundingClientRect(); return [Math.round(r.left), Math.round(r.right)]; })(),
    actionsBox: (() => { const e = q('.pane-tab-actions'); if (!e) return null; const r = e.getBoundingClientRect(); return [Math.round(r.left), Math.round(r.right)]; })(),
  };
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    fit: string | null;
    keep: boolean;
    keepInTab: boolean;
    find: boolean;
    close: boolean;
    scrollsSideways: number;
    clipped: number;
    stripBox: number[];
    actionsBox: number[] | null;
    rangeBox: number[];
    endBox: number[];
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const html = CASES.map(([view, w, z]) => pane(idOf(view, w, z), view, w, z)).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>${FIT_SCRIPT}</script><script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 800, height: 2400 }
    );
}, 120_000);

describe.skipIf(!BROWSER)("a pane header in a narrow pane", () => {
    it("keeps the agent's Messages reachable at every width, in the tab or in the header", () => {
        for (const [view, w, z] of CASES.filter(([v]) => v === "agent")) {
            expect(measured[idOf(view, w, z)].keep, `Messages at ${w}px, chrome zoom ${z}: ${JSON.stringify(measured[idOf(view, w, z)])}`).toBe(true);
        }
    });

    it("keeps the terminal's Start and Restart reachable at every width", () => {
        for (const [view, w, z] of CASES.filter(([v]) => v === "term")) {
            expect(measured[idOf(view, w, z)].keep, `Start at ${w}px, chrome zoom ${z}: ${JSON.stringify(measured[idOf(view, w, z)])}`).toBe(true);
        }
    });

    it("keeps Close reachable at every width", () => {
        for (const [view, w, z] of CASES) expect(measured[idOf(view, w, z)].close, `Close at ${w}px, ${view}, chrome zoom ${z}`).toBe(true);
    });

    it("leaves a wide pane alone: the buttons stay in the selected tab and no tier is set", () => {
        for (const [view, w, z] of CASES.filter(([, w]) => w === 600)) {
            const m = measured[idOf(view, w, z)];
            expect(m.fit, `${view} at ${w}px, chrome zoom ${z}`).toBeNull();
            expect(m.keepInTab, `${view} at ${w}px, chrome zoom ${z}`).toBe(true);
        }
        for (const z of [1, 1.5]) expect(measured[idOf("agent", 600, z)].find, `Find at 600px, chrome zoom ${z}`).toBe(true);
    });
});
