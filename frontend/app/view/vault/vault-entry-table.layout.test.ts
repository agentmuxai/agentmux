// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Vault list's entry rows, measured in a real browser across the widths a pane can have, at 100% and 150% pane zoom
 * (vault-view.scss: the table, the grid and the stack). Compiles the real `vault-view.scss` and the shared manager
 * chrome, and draws rows with the markup of `EntryTable` (vault-entries.tsx) with made-up entries only: a short label, a
 * long label, an unbroken variable name, several agent chips, an expiry line. The pane's zoom is put on `.vault-view`
 * the way the pane does, and the list's width is given inside it. Skips, rather than fails, on a machine with no usable
 * browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const loadPaths = [join(__dirname, "../../..")];
const css = [join(__dirname, "vault-view.scss"), join(__dirname, "../warden-shared/warden-manager-chrome.scss")]
    .map((f) => sass.compile(f, { loadPaths }).css)
    .join("\n");

const TOKENS = `:root{--main-text-color:#ddd;--secondary-text-color:#999;--warning-text-color:#e0a030;--border-color:#333;--main-bg-color:#222;
--font-sans:Arial,sans-serif;--font-mono:monospace;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--leading-tight:1.2;
--space-1:4px;--space-2:8px;--space-3:12px;--radius-full:999px;--radius-sm:3px}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}.case{margin:0 0 8px}
.vault-view{height:auto !important}.vault-list{height:auto !important}.vault-list__sections{overflow:visible !important}
.ui-pill{display:inline-flex;align-items:center;padding:0 8px;border:1px solid transparent;border-radius:999px;font-size:11px}
.ui-button{display:inline-block;width:18px;height:18px;padding:0;border:1px solid #444;background:#2a2a2a;color:#ddd}`;

const CHIPS = (names: string[]) =>
    names
        .map((n) => `<span class="vault-grant" data-origin="${n}"><span class="vault-grant__text">${n}</span><button class="ui-button" style="width:14px;height:14px"></button></span>`)
        .join("");

// A row's actions: six, or seven for a login with earlier values (vault-entries.tsx EntryActions), 18px each.
const actions = (n: number) => `<td class="vault-actions">${'<button class="ui-button" aria-label="x"></button>'.repeat(n)}</td>`;

interface Row {
    label: string;
    id: string;
    chips: string[];
    details: string;
    expiry?: string;
    /** The value cell's own markup, when it is not just the masked digits. */
    value?: string;
    /** How many action icons the row has (default 6). */
    icons?: number;
}

const ROWS: Row[] = [
    { label: "Example API key", id: "example-api-key-0001", chips: ["alpha (value)"], details: "Owner: Example Person" },
    {
        label: "A long descriptive label for a made-up service account key",
        id: "vault-entry-with-a-very-long-identifier-0002-abcdefghijklmnopqrstuvwxyz",
        chips: ["alpha (value)", "beta (value)", "gamma (value)", "delta (value)"],
        details: "A note about this made-up entry that runs to several words so that it wraps inside its own column",
        expiry: "Expires in 40 days",
    },
    { label: "EXAMPLE_ENVIRONMENT_VARIABLE_NAME_FOR_TESTING", id: "example-env-0003", chips: [], details: "https://example.com/a/very/long/path/with-no-spaces-in-it-at-all-0003" },
    // A card: the brand's pill before the masked digits (EntryValue, vault-entries.tsx).
    {
        label: "Example team card",
        id: "example-card-0004",
        chips: [],
        details: "Example Team",
        value: `<span class="ui-pill">American Express</span> <span class="vault-last-four">•••• 4321</span>`,
        icons: 7,
    },
];

function row(r: Row): string {
    const grants = r.chips.length
        ? `<div class="vault-grants"><span class="warden-manager-dim">Agents may run commands with it:</span>${CHIPS(r.chips)}</div>`
        : "";
    const expiry = r.expiry ? `<div class="vault-expiry vault-expiry-warn">${r.expiry}</div>` : "";
    return (
        `<tr><td><span class="vault-label">${r.label}</span><span class="warden-manager-mono warden-manager-dim vault-id" title="${r.id}">${r.id}</span>${grants}${expiry}</td>` +
        `<td class="vault-kind">API key</td><td class="warden-manager-dim vault-details">${r.details}</td>` +
        `<td class="warden-manager-mono vault-value">${r.value ?? '<span class="vault-last-four">•••• 4321</span>'}</td>` +
        `<td class="warden-manager-dim vault-when">3 days ago</td>${actions(r.icons ?? 6)}</tr>`
    );
}

/** A list `width` px wide inside a pane zoomed `zoom`: three sections' worth of rows in one section. */
function pane(id: string, width: number, zoom: number): string {
    const rows = ROWS.map(row).join("");
    return (
        `<div class="case" id="${id}"><div class="vault-container" style="width:${width}px"><div class="vault-view" style="zoom:${zoom};width:${width / zoom}px">` +
        `<div class="vault-list"><div class="vault-list__sections"><section class="vault-section vault-section--api-keys">` +
        `<div class="vault-section__head"><span class="vault-section__label">API keys</span></div><div class="vault-section__body">` +
        `<table class="warden-manager-table vault-table vault-entry-table"><colgroup><col class="vault-col-label"/><col class="vault-col-kind"/><col/>` +
        `<col class="vault-col-value"/><col class="vault-col-when"/><col class="vault-col-actions"/></colgroup>` +
        `<thead class="vault-entry-columns"><tr><th>Label</th><th>Kind</th><th>Details</th><th>Value</th><th>Changed</th><th></th></tr></thead>` +
        `<tbody>${rows}</tbody></table></div></section></div></div></div></div></div>`
    );
}

// The widths from the least the list supports (160px inside the zoom) to very wide, at 100% and 150%.
const WIDTHS = [160, 240, 320, 500, 640, 700, 830, 1000, 1400, 2000];
const ZOOMS = [1, 1.5];
const CASES: Array<[number, number]> = ZOOMS.flatMap((z) => WIDTHS.map((w): [number, number] => [w, z]));
const key = (w: number, z: number) => `w${w}z${z}`;

const MEASURE = `
const result = {};
try {
for (const c of document.querySelectorAll('.case')) {
  const view = c.querySelector('.vault-view');
  const zoom = parseFloat(view.style.zoom) || 1;
  const table = c.querySelector('.vault-entry-table');
  const fs = parseFloat(getComputedStyle(table).fontSize);
  const box = c.querySelector('.vault-container').getBoundingClientRect();
  const rows = [...c.querySelectorAll('tbody tr')].map((tr) => {
    const tds = [...tr.children];
    const first = tds[0].getBoundingClientRect();
    const label = tr.querySelector('.vault-label');
    const lh = parseFloat(getComputedStyle(label).lineHeight) || fs * 1.3;
    const chips = [...tr.querySelectorAll('.vault-grant')].map((g) => g.getBoundingClientRect());
    const buttons = [...tr.querySelectorAll('.vault-actions .ui-button')].map((b) => b.getBoundingClientRect());
    const rect = tr.getBoundingClientRect();
    return {
      height: rect.height / zoom,
      labelCellWidth: first.width / zoom,
      labelHeight: label.getBoundingClientRect().height / zoom,
      lineHeight: lh,
      chipHeights: chips.map((r) => r.height / zoom),
      chipWidths: chips.map((r) => r.width / zoom),
      buttonLines: new Set(buttons.map((b) => Math.round(b.top))).size,
      buttonsRight: buttons.length ? Math.max(...buttons.map((b) => b.right)) - box.right : -999,
      buttonsLeft: buttons.length ? Math.min(...buttons.map((b) => b.left)) - box.left : 999,
      detailsLeft: tr.querySelector('.vault-details').getBoundingClientRect().left / zoom,
      valueLeft: tr.querySelector('.vault-value').getBoundingClientRect().left / zoom,
      idHeight: tr.querySelector('.vault-id').getBoundingClientRect().height / zoom,
      valueOverflow: (tr.querySelector('.vault-value').scrollWidth - tr.querySelector('.vault-value').clientWidth) / zoom,
    };
  });
  const body = c.querySelector('.vault-section__body');
  result[c.id] = { fs, rows, sideways: Math.max(body.scrollWidth - body.clientWidth, c.querySelector('.vault-view').scrollWidth - c.querySelector('.vault-view').clientWidth) };
}
} catch (e) { result.error = String(e && e.stack || e); }
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    fs: number;
    sideways: number;
    rows: Array<{
        height: number;
        labelCellWidth: number;
        labelHeight: number;
        lineHeight: number;
        chipHeights: number[];
        chipWidths: number[];
        buttonLines: number;
        buttonsRight: number;
        buttonsLeft: number;
        detailsLeft: number;
        valueLeft: number;
        idHeight: number;
        valueOverflow: number;
    }>;
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const html = CASES.map(([w, z]) => pane(key(w, z), w, z)).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 2100, height: 4000 }
    );
}, 90_000);

describe.skipIf(!BROWSER)("Vault entry rows across widths", () => {
    it("never scrolls the list sideways", () => {
        for (const [w, z] of CASES) expect(measured[key(w, z)].sideways, `${w}px, zoom ${z}`).toBeLessThanOrEqual(1);
    });

    it("gives the label a least width of 14em, or the whole row when the row has less, and never a few letters", () => {
        // Under 160px of list the pane says it is too narrow and draws no rows (vault-view.scss, `vault-pane`).
        for (const [w, z] of CASES.filter(([cw, cz]) => cw / cz >= 160)) {
            const m = measured[key(w, z)];
            for (const r of m.rows) {
                // The section's frame and the row's padding take about 60px of the list's width.
                const least = Math.min(14 * m.fs, w / z - 60);
                expect(r.labelCellWidth, `label cell at ${w}px, zoom ${z}`).toBeGreaterThanOrEqual(Math.min(least, 120) - 1);
            }
        }
    });

    it("keeps a short label on one or two lines, and a row from standing hundreds of pixels tall, from 240px up", () => {
        for (const [w, z] of CASES.filter(([cw]) => cw >= 240)) {
            const r = measured[key(w, z)].rows[0];
            expect(r.labelHeight, `short label at ${w}px, zoom ${z}`).toBeLessThanOrEqual(r.lineHeight * 2.2);
            expect(r.height, `short entry's row at ${w}px, zoom ${z}`).toBeLessThanOrEqual(20 * measured[key(w, z)].fs);
        }
    });

    it("keeps an agent's chip whole on one line, wider than a letter", () => {
        for (const [w, z] of CASES.filter(([cw]) => cw >= 240)) {
            const m = measured[key(w, z)];
            for (const r of m.rows) {
                for (const h of r.chipHeights) expect(h, `chip height at ${w}px, zoom ${z}`).toBeLessThanOrEqual(3 * m.fs);
                for (const cw of r.chipWidths) expect(cw, `chip width at ${w}px, zoom ${z}`).toBeGreaterThanOrEqual(3 * m.fs);
            }
        }
    });

    it("keeps the id to one line", () => {
        for (const [w, z] of CASES) {
            const m = measured[key(w, z)];
            for (const r of m.rows) expect(r.idHeight, `id at ${w}px, zoom ${z}`).toBeLessThanOrEqual(2 * m.fs);
        }
    });

    it("keeps every action icon inside the list's box", () => {
        for (const [w, z] of CASES) {
            for (const r of measured[key(w, z)].rows) {
                expect(r.buttonsRight, `icons' right edge at ${w}px, zoom ${z}`).toBeLessThanOrEqual(1);
                expect(r.buttonsLeft, `icons' left edge at ${w}px, zoom ${z}`).toBeGreaterThanOrEqual(-1);
            }
        }
    });

    it("keeps a card's brand and masked digits inside the value cell, wrapping between them when it must", () => {
        for (const [w, z] of CASES.filter(([cw, cz]) => cw / cz >= 160)) {
            for (const r of measured[key(w, z)].rows) expect(r.valueOverflow, `value cell at ${w}px, zoom ${z}`).toBeLessThanOrEqual(1);
        }
    });

    it("keeps all of a row's action icons on one line where the row is a table or a grid, at most seven of them", () => {
        for (const [w, z] of CASES) {
            const m = measured[key(w, z)];
            if (w / z - 60 < 53 * m.fs) continue;
            for (const r of m.rows) expect(r.buttonLines, `action icons at ${w}px, zoom ${z}`).toBe(1);
        }
    });

    it("aligns the details and the value from row to row where the row is a table or a grid (52em and more)", () => {
        for (const [w, z] of CASES) {
            const m = measured[key(w, z)];
            // The list is the pane less the section's frame (about 60px): the grid starts at 52em of the list.
            if (w / z - 60 < 53 * m.fs) continue;
            for (const r of m.rows.slice(1)) {
                expect(Math.abs(r.detailsLeft - m.rows[0].detailsLeft), `details' left at ${w}px, zoom ${z}`).toBeLessThanOrEqual(1);
                expect(Math.abs(r.valueLeft - m.rows[0].valueLeft), `value's left at ${w}px, zoom ${z}`).toBeLessThanOrEqual(1);
            }
        }
    });
});
