// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The narrow Kanban's bar of section links, measured in a real browser (kanban-section-bar.tsx): it stays at the top of the
 * list while the list scrolls, wraps to more lines when its links do not fit, never scrolls the list sideways, and the
 * sections' own sticky heads sit under it, not behind it. Compiles the real `kanban.scss`, the bar's styles and the shared
 * coloured section. The bar's height is put in `--kanban-bar-h` the way the component does. It skips, rather than fails, on
 * a machine with no usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = [join(__dirname, "kanban.scss"), join(__dirname, "kanban-section-bar.scss")].map((f) => sass.compile(f).css).join("\n");

const TOKENS = `:root{--main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--border-color:#333;--main-bg-color:#222;
--font-sans:Arial,sans-serif;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--text-2xl:20px;--leading-tight:1.2;
--space-1:4px;--space-2:8px;--space-3:12px;--radius-full:999px;--radius-sm:3px;--z-tab-name:3;--kanban-stage:#4a90d9;
--duration-quick:0s}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}.pane{margin:6px 0}`;

const NAMES = ["Needs you", "Backlog", "In Research", "Spec'd", "In progress", "Review", "Merged", "Released"];

/** A narrow list `width` px wide: the bar, then eight sections each tall enough to scroll the list. */
function pane(id: string, width: number, zoom: number): string {
    const links = NAMES.map(
        (n, i) =>
            `<a class="kanban-section-bar__link" href="#s${i}"><span class="kanban-section-bar__name">${n}</span><span class="kanban-section-bar__count">${i + 3}</span></a>`
    ).join("");
    const sections = NAMES.map(
        (n, i) =>
            `<section class="kanban-column" data-stage-id="s${i}"><div class="kanban-column__head"><span class="kanban-column__label">${n}</span><span class="kanban-column__count">${i + 3}</span></div>` +
            `<div class="kanban-column__list"><div style="height:140px">cards</div></div></section>`
    ).join("");
    return (
        `<div class="pane" style="zoom:${zoom}"><div class="kanban-container" id="${id}" style="width:${width / zoom}px;height:${360 / zoom}px">` +
        `<div class="kanban" style="display:flex;flex-direction:column;height:100%"><div class="kanban-board">` +
        `<nav class="kanban-section-bar"><div class="kanban-section-bar__links">${links}</div><button class="kanban-section-bar__all">Collapse all</button></nav>` +
        `${sections}</div></div></div></div>`
    );
}

const CASES: Array<[number, number]> = [
    [480, 1],
    [300, 1],
    [220, 1],
    [300, 1.5],
];

const MEASURE = `
const result = {};
for (const c of document.querySelectorAll('.kanban-container')) {
  const board = c.querySelector('.kanban-board');
  const bar = c.querySelector('.kanban-section-bar');
  const link = c.querySelector('.kanban-section-bar__link');
  board.style.setProperty('--kanban-bar-h', bar.offsetHeight + 'px');
  const before = { barTop: bar.getBoundingClientRect().top - board.getBoundingClientRect().top, barHeight: bar.getBoundingClientRect().height, linkHeight: link.getBoundingClientRect().height };
  board.scrollTop = 400;
  const boardRect = board.getBoundingClientRect(), barRect = bar.getBoundingClientRect();
  const barAfter = bar.getBoundingClientRect();
  // A section whose top is 40px above the bar's bottom edge and whose end is far below: its head is stuck, so it
  // must sit at the bar's bottom edge (a head at the list's top edge would be behind the bar).
  const target = c.querySelectorAll('.kanban-column')[3];
  board.scrollTop += target.getBoundingClientRect().top - barAfter.bottom + 40;
  const head = target.querySelector('.kanban-column__head').getBoundingClientRect();
  const headGap = head.top - bar.getBoundingClientRect().bottom;
  result[c.id] = {
    ...before,
    scrolled: board.scrollTop > 0,
    barTopAfter: barRect.top - boardRect.top,
    headGap,
    sideways: board.scrollWidth - board.clientWidth,
  };
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    barTop: number;
    barHeight: number;
    linkHeight: number;
    scrolled: boolean;
    barTopAfter: number;
    headGap: number;
    sideways: number;
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const html = CASES.map(([w, z]) => pane(`w${w}z${z}`, w, z)).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 700, height: 2400 }
    );
}, 90_000);

describe.skipIf(!BROWSER)("Kanban section bar", () => {
    it("stays at the top of the list while it scrolls", () => {
        for (const [w, z] of CASES) {
            const m = measured[`w${w}z${z}`];
            expect(m.scrolled, `list scrolled at ${w}px, zoom ${z}`).toBe(true);
            expect(Math.abs(m.barTopAfter), `bar's top after scrolling at ${w}px, zoom ${z}`).toBeLessThanOrEqual(1);
        }
    });

    it("wraps its links onto more lines when they do not fit", () => {
        expect(measured["w300z1"].barHeight).toBeGreaterThan(measured["w300z1"].linkHeight * 1.5);
        expect(measured["w220z1"].barHeight).toBeGreaterThanOrEqual(measured["w300z1"].barHeight);
    });

    it("never scrolls the list sideways", () => {
        for (const [w, z] of CASES) expect(measured[`w${w}z${z}`].sideways, `${w}px, zoom ${z}`).toBeLessThanOrEqual(1);
    });

    it("keeps a section's stuck head just under it, not behind it", () => {
        for (const [w, z] of CASES) expect(Math.abs(measured[`w${w}z${z}`].headGap), `head's top against the bar's bottom at ${w}px, zoom ${z}`).toBeLessThanOrEqual(1);
    });
});
