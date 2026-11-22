// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A card's pull request line, measured in a real browser (kanban-prs.tsx, kanban.scss): the card keeps the width its
 * column gives it (the container query on the card must not shrink it), the line is as tall with no badge as with one
 * (so data arriving late does not move the card), and the chips' words go below a narrow card's width while their
 * icons stay. Compiles the real `kanban.scss` and the pill's styles. It skips, rather than fails, on a machine with no
 * usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";

import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = [join(__dirname, "kanban.scss"), join(__dirname, "../../element/ui/pill.scss")].map((f) => sass.compile(f).css).join("\n");

const TOKENS = `:root{--main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--border-color:#333;--main-bg-color:#222;
--font-sans:Arial,sans-serif;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--leading-tight:1.2;
--space-1:4px;--space-2:8px;--space-3:12px;--radius-full:999px;--radius-sm:3px;--duration-fast:0s;--duration-quick:0s}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}`;

const WIDTHS = [160, 200, 300, 420];

/** One column `width` px wide with a card that has the line (a badge and two chips) and one whose line is empty. */
function column(width: number): string {
    const chip = (icon: string, word: string) =>
        `<span class="ui-pill kanban-card__flow-chip" title="${word}"><i class="${icon}"></i><span class="kanban-card__flow-word">${word}</span></span>`;
    const full = `<div class="kanban-card__flow"><span class="ui-pill">Waits on the author</span>${chip("a", "approved")}${chip("b", "checks failing")}</div>`;
    const empty = `<div class="kanban-card__flow"></div>`;
    const card = (id: string, flow: string) => `<div class="kanban-card" id="${id}"><div class="kanban-card__title">task</div>${flow}</div>`;
    return `<div class="col" style="width:${width}px">${card(`full${width}`, full)}${card(`empty${width}`, empty)}</div>`;
}

const MEASURE = `
const result = {};
for (const w of ${JSON.stringify(WIDTHS)}) {
  const full = document.getElementById('full' + w), empty = document.getElementById('empty' + w);
  const word = full.querySelector('.kanban-card__flow-word');
  result[w] = {
    cardWidth: full.getBoundingClientRect().width,
    flowFull: full.querySelector('.kanban-card__flow').getBoundingClientRect().height,
    flowEmpty: empty.querySelector('.kanban-card__flow').getBoundingClientRect().height,
    wordShown: getComputedStyle(word).display !== 'none',
  };
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    cardWidth: number;
    flowFull: number;
    flowEmpty: number;
    wordShown: boolean;
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${WIDTHS.map(column).join("")}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 900, height: 900 }
    );
}, 90_000);

describe.skipIf(!BROWSER)("A card's pull request line", () => {
    it("leaves the card the width its column gives it", () => {
        for (const w of WIDTHS) expect(measured[w].cardWidth, `card in a ${w}px column`).toBeGreaterThanOrEqual(w - 1);
    });

    it("is reserved: as tall with nothing to show as with a badge, until the badge needs more lines", () => {
        expect(measured[420].flowEmpty).toBeGreaterThan(10);
        expect(measured[420].flowFull).toBe(measured[420].flowEmpty);
    });

    it("drops the chips' words below a narrow card and keeps them above it", () => {
        expect(measured[160].wordShown).toBe(false);
        expect(measured[200].wordShown).toBe(false);
        expect(measured[300].wordShown).toBe(true);
        expect(measured[420].wordShown).toBe(true);
    });
});
