// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Settings section that flows into columns, measured in a real browser (jsdom has no layout): the number of columns
 * for a pane's width at Settings zoom 100% and 150%, no row split across columns, a group header never alone at the
 * foot of a column, a single section staying one capped column, and the rules that ask a row (widget colors) firing for
 * a narrow column. Skips with no usable browser.
 * docs/specs/SPEC_SETTINGS_WIDE_PANE_COLUMNS_2026_11_20.md 3.2, 3.3, 3.5 and 5.
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
const ui = sass.compile(join(__dirname, "../../element/ui/ui.scss")).css;
const settings = sass.compile(join(__dirname, "settings.scss")).css;
const TOKENS = `:root{--text-xs:11px;--text-sm:12px;--space-0-5:2px;--space-1:4px;--space-1-5:6px;--space-2:8px;--space-3:12px;
--space-4:16px;--space-6:24px;--success-text-color:#2e7d32;--main-text-color:#111;--secondary-text-color:#555;
--border-color:#ccc;--duration-linger:1500ms;--ease-standard:ease;--font-weight-medium:500}body{margin:0;font:13px sans-serif}`;

const field = (label: string, description: string, control: string, cls = "setting-row") =>
    `<div class="${cls}"><div class="ui-field ui-field--inline"><div class="ui-field-text">` +
    `<div class="ui-field-label-row"><label class="ui-field-label">${label}</label></div>` +
    `<div class="ui-field-description">${description}</div></div><div class="ui-field-control">${control}</div></div></div>`;
const TOGGLE = `<button class="ui-switch" role="switch" aria-checked="false"><span class="ui-switch-thumb"></span></button>`;
const SELECT = `<select class="ui-input setting-select"><option>Default</option></select>`;
const TEXT = `<input class="ui-input setting-text" type="text" value="">`;
const SWATCHES =
    `<div class="setting-widget-color"><span>Chip</span><div class="palette-swatches">${"<i></i>".repeat(15)}</div>` +
    `<button class="ui-button">Reset</button></div>`;

/** A section of 6 groups: a header, then 3 to 5 rows, so a column could end on a header if nothing kept it with its row. */
function flowSection(rows = 24): string {
    let html = "";
    let n = 0;
    for (let g = 0; n < rows; g++) {
        html += `<div class="settings-subheader" data-kind="header">Group ${g}</div>`;
        for (let i = 0; i < 3 + (g % 3) && n < rows; i++, n++) {
            const description = "Words that explain what this setting does, long enough to take two lines in a narrow column. ".repeat(1 + (n % 2));
            html += field(`Setting ${n}`, description, n % 3 === 0 ? SELECT : n % 3 === 1 ? TEXT : TOGGLE);
        }
    }
    return html;
}

/** One pane: the container the section is queried against, at `width` screen px under Settings zoom `zoom`. */
function pane(name: string, width: number, zoom: number, body: string, bodyClass = "settings-section-body"): string {
    return (
        `<div class="settings-view-container" data-case="${name}" style="width:${width / zoom}px;height:auto;zoom:${zoom};` +
        `overflow:hidden;margin-bottom:8px"><div class="settings-body"><div class="settings-scroll">` +
        `<div class="${bodyClass}">${body}</div></div></div></div>`
    );
}

const MEASURE = `
const out = {};
for (const pane of document.querySelectorAll("[data-case]")) {
    const body = pane.querySelector(".settings-section-body");
    const items = [...body.children];
    const xs = new Set(items.map((el) => Math.round(el.getBoundingClientRect().left)));
    const rows = items.filter((el) => el.classList.contains("setting-row"));
    const headers = items.filter((el) => el.classList.contains("settings-subheader"));
    const colWidth = Math.max(...items.map((el) => el.getBoundingClientRect().width));
    const split = items.filter((el) => el.getClientRects().length > 1).length;
    // A header is alone at a column's foot when what follows it starts in another column.
    const aloneHeaders = headers.filter((h) => {
        const next = h.nextElementSibling;
        return next ? Math.abs(next.getBoundingClientRect().left - h.getBoundingClientRect().left) > 1 : false;
    }).length;
    const widget = pane.querySelector(".setting-widget-color");
    const swatches = widget && widget.querySelector(".palette-swatches");
    out[pane.dataset.case] = {
        columns: xs.size,
        split,
        aloneHeaders,
        rows: rows.length,
        colWidth: Math.round(colWidth * 100) / 100,
        order: swatches ? getComputedStyle(swatches).order : null,
        cols: swatches ? getComputedStyle(swatches).getPropertyValue("--palette-cols").trim() : null,
        tops: items.slice(0, 3).map((el) => Math.round((el.getBoundingClientRect().top - pane.getBoundingClientRect().top) * 10) / 10),
    };
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

interface Measured {
    columns: number;
    split: number;
    aloneHeaders: number;
    rows: number;
    colWidth: number;
    order: string | null;
    cols: string | null;
    tops: number[];
}

async function measure(panes: string[]): Promise<Record<string, Measured>> {
    const html =
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${ui}\n${settings}</style>${panes.join("")}` +
        `<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
    return measureInBrowser<Record<string, Measured>>(BROWSER!, html, { width: 3600, height: 4000 });
}

// The spec's table (3.3): n columns from a pane width of n x 28rem; at Settings zoom z the pane's own width is the
// screen width divided by z.
const COUNTS: Array<[number, number, number]> = [
    // [pane width in screen px, Settings zoom, columns]
    [480, 1, 1],
    [895, 1, 1],
    [896, 1, 2],
    [1343, 1, 2],
    [1344, 1, 3],
    [1800, 1, 4],
    [2200, 1, 4],
    [3400, 1, 7],
    [900, 1.5, 1],
    [1400, 1.5, 2],
    [3400, 1.5, 5],
];

describe.skipIf(!BROWSER)("settings columns", () => {
    it("counts the columns from the pane's own width, at zoom 100% and 150%", async () => {
        const r = await measure(COUNTS.map(([w, z]) => pane(`w${w}z${z}`, w, z, flowSection())));
        for (const [w, z, columns] of COUNTS) {
            const m = r[`w${w}z${z}`];
            expect(m.columns, `${w}px at ${z * 100}%`).toBe(columns);
            // A column is never narrower than 26rem (416px) nor, with the gaps, wider than the pane's share of it.
            if (columns > 1) expect(m.colWidth).toBeGreaterThanOrEqual(416 - 0.5);
        }
    }, 60_000);

    it("never splits a row and never leaves a group header alone at the foot of a column", async () => {
        const widths = [896, 1344, 2200, 3400];
        const r = await measure(widths.map((w) => pane(`w${w}`, w, 1, flowSection(30))));
        for (const w of widths) {
            expect(r[`w${w}`].rows, `${w}px`).toBe(30);
            expect(r[`w${w}`].split, `${w}px: a row in two pieces`).toBe(0);
            expect(r[`w${w}`].aloneHeaders, `${w}px: a header ending a column`).toBe(0);
        }
    }, 60_000);

    it("keeps a single section in one column, capped at 720px from a pane of 1024px", async () => {
        const r = await measure([
            pane("single-480", 480, 1, flowSection(), "settings-section-body settings-section-body--single"),
            pane("single-3400", 3400, 1, flowSection(), "settings-section-body settings-section-body--single"),
        ]);
        expect(r["single-480"].columns).toBe(1);
        expect(r["single-3400"].columns).toBe(1);
        expect(r["single-3400"].colWidth).toBeLessThanOrEqual(720);
    }, 60_000);

    it("lays the widget-color swatches under the chip, and in fewer columns, as the row narrows", async () => {
        const body = (extra: string) =>
            field("Appearance", "Words", TOGGLE) +
            field("Widget colors", "Words", SWATCHES, "setting-row setting-row--indent") + extra;
        // One column of 700px: the row's content is 700 - 32 - 24 = 644px, over the 583px the swatches need beside the chip.
        // 1344px is three columns of about 435px: the row's content is about 387px, so the swatches wrap, in 8 columns.
        // 400px is the narrowest one-column pane: a content width of 344px.
        const r = await measure([pane("wide", 700, 1, body("")), pane("column", 1344, 1, body("")), pane("narrow", 400, 1, body(""))]);
        expect(r.wide.order).not.toBe("3");
        expect(r.column.order).toBe("3");
        expect(r.column.cols).toBe("8");
        expect(r.narrow.order).toBe("3");
    }, 60_000);

    it("keeps the rows above a row that grows where they were", async () => {
        const plain = flowSection(12);
        const grown = plain.replace("Setting 3</label></div>", "Setting 3</label></div><div style='height:60px'></div>");
        const r = await measure([pane("plain", 1344, 1, plain), pane("grown", 1344, 1, grown)]);
        expect(r.grown.tops).toEqual(r.plain.tops);
    }, 60_000);
});
