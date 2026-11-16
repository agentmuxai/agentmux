// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The guide line down a lead's reports in Group by Lead, measured in a real browser (#47795): each report row has
 * the line on its left edge, and the 4 px gap above it carries the same line, so it runs unbroken from the lead
 * down the last report. It skips, rather than fails, on a machine with no usable browser.
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = sass.compile(join(__dirname, "swarm-seats.scss")).css;
const TOKENS = `:root{--accent-color:#58c142;--border-color:#333;--main-text-color:#ddd;--secondary-text-color:#999;
--text-xs:11px;--text-2xs:10px;--space-1:2px;--space-2:4px;--radius-full:999px;--tint-3:#2a2a2a;--highlight-bg-color:#333}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}
.list{display:flex;flex-direction:column;width:400px}
.row{height:36px;flex:none}`;

const HTML = `<div class="list" id="list">
<div class="row" data-swarm-key="lead">lead</div>
<div class="swarm-seat-indent swarm-seat-indent--d1"></div><div class="row" data-swarm-key="a">a</div>
<div class="swarm-seat-indent swarm-seat-indent--d1"></div><div class="row" data-swarm-key="b">b</div>
<div class="swarm-seat-indent swarm-seat-indent--d1"></div><div class="row" data-swarm-key="c">c</div>
<div class="row" data-swarm-key="loner">loner</div></div>`;

const MEASURE = `
const box = (el) => { const r = el.getBoundingClientRect(); const cs = getComputedStyle(el); return { left: r.left, top: r.top, bottom: r.bottom, line: parseFloat(cs.borderLeftWidth), color: cs.borderLeftColor }; };
const kids = [...document.getElementById('list').children];
const result = { items: kids.map((el) => ({ marker: el.classList.contains('swarm-seat-indent'), ...box(el) })) };
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Item {
    marker: boolean;
    left: number;
    top: number;
    bottom: number;
    line: number;
    color: string;
}

let items: Item[] = [];

beforeAll(async () => {
    if (!BROWSER) return;
    const r = await measureInBrowser<{ items: Item[] }>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${HTML}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 500, height: 400 }
    );
    items = r.items;
}, 90_000);

describe.skipIf(!BROWSER)("Group by Lead guide line", () => {
    it("runs unbroken from one report to the next: each gap carries the line at the row's own left edge", () => {
        const reports = items.map((it, i) => ({ it, i })).filter(({ it, i }) => !it.marker && items[i - 1]?.marker);
        expect(reports.length).toBe(3);
        for (const { it, i } of reports) {
            const gap = items[i - 1];
            expect(gap.line, "the gap has a line").toBe(2);
            expect(gap.left, "the gap's line is at the row's edge").toBeCloseTo(it.left, 1);
            expect(gap.color).toBe(it.color);
            // No space between the gap and the row, and none between the row above and the gap.
            expect(gap.bottom).toBeCloseTo(it.top, 1);
            expect(gap.top).toBeCloseTo(items[i - 2].bottom, 1);
        }
    });

    it("indents the reports one step and leaves a row with no lead where it was", () => {
        const loner = items[items.length - 1];
        const first = items.find((it) => !it.marker && it !== items[0] && it !== loner)!;
        expect(first.left).toBeCloseTo(loner.left + 12, 1);
    });
});
