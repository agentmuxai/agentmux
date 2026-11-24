// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane header's tab strip as its tabs run out of room, measured in a real browser: the tabs share the squeeze down to
 * their floors, the labels of tabs that are not selected go at the steps (pane-tab-steps.ts), the selected tab keeps its
 * label and close button, and past the floors the header's range scrolls as it always did. Compiles the real strip and
 * header styles; the room and the step are set on the strip the way pane-tab-strip-steps.ts does (that logic is covered by
 * its own tests). It skips, rather than fails, on a machine with no usable browser (layout-browser.ts findBrowser).
 */
import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";
import { ACTIVE_FLOOR_PX, minTabsRoom, RESTING_FLOOR_PX } from "./pane-tab-steps";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = [join(__dirname, "PaneTabStrip.scss"), join(__dirname, "../block/block.scss")].map((f) => sass.compile(f).css).join("\n");

const TOKENS = `:root{--main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--border-color:#333;--main-bg-color:#222;--block-bg-color:#222;
--font-sans:Arial,sans-serif;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--leading-tight:1.2;--warning-color:#e0b956;
--space-1:4px;--space-2:8px;--space-3:12px;--radius-full:999px;--radius-sm:3px;--pane-tab-strip-height:28px;--tint-1:rgba(255,255,255,.06);
--duration-decorative-instant:0s;--ease-standard:linear;--z-behind:-1;--pane-ring-width:2px}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}
.pane-tab-close{width:16px;height:16px;flex:0 0 16px;border:0;padding:0;background:none;color:inherit}
.row{width:var(--w);margin:6px 0;display:flex}`;

const LABELS = ["Camper", "Build checks", "Reviewer for the long task", "Terminal", "Research", "Notes", "Deploy", "Docs"];
const OTHERS_PX = 90; // the header's other controls inside the range (an icon button, the grip, the aux controls)
const GAP_PX = 8;
const ADD_PX = 28;

function row(id: string, width: number, tabs: number, step: string, zoom: number): string {
    // What pane-tab-strip-steps.ts sets: the room the range leaves the strip, never under the tabs' floors.
    const total = width - OTHERS_PX - GAP_PX * 3; // three other items (the spacer too), three gaps
    const limit = Math.max(total, minTabsRoom(tabs) * zoom + ADD_PX * zoom);
    const pills = Array.from({ length: tabs }, (_, i) => {
        const active = i === 1 ? " pane-tab--active" : "";
        const attention = i === 2 ? " pane-tab--attention" : "";
        return (
            `<div class="pane-tab-tip"><div class="pane-tab${active}${attention}" data-tab-id="t${i}">` +
            `<span class="pane-tab-icon"><i>*</i></span><span class="pane-tab-label">${LABELS[i % LABELS.length]}</span>` +
            `<button class="pane-tab-close" aria-label="Close tab">x</button></div></div>`
        );
    }).join("");
    return (
        `<div class="row" id="${id}" style="--w:${width}px"><div class="block-frame-header-scroll" style="width:${width}px">` +
        `<div class="block-frame-header-content"><div style="width:${OTHERS_PX / 2}px;flex:0 0 auto"></div>` +
        `<div class="block-frame-default-header-tabstrip"><div class="pane-tab-strip" data-step="${step}" style="--pane-tab-strip-zoom:${zoom};--pane-tab-room:${limit}px">` +
        `<div class="pane-tab-strip-inner">${pills}<div class="pane-tab-strip-add-tip"><button class="pane-tab-strip-add"><span class="pane-tab-strip-add-glyph"></span></button></div></div></div></div>` +
        `<div class="block-frame-header-spacer"></div><div style="width:${OTHERS_PX / 2}px;flex:0 0 auto"></div></div></div></div>`
    );
}

// [id, header width, tab count, step, zoom]
const CASES: Array<[string, number, number, string, number]> = [
    ["fit", 900, 3, "full", 1],
    ["clipped", 700, 6, "full", 1],
    ["noLabel", 480, 7, "icon-close", 1],
    ["iconOnly", 400, 8, "icon", 1],
    ["floors", 260, 8, "icon", 1],
    ["zoomed", 480, 7, "icon-close", 1.25],
];

const MEASURE = `
const result = {};
for (const r of document.querySelectorAll('.row')) {
  const range = r.querySelector('.block-frame-header-scroll');
  const strip = r.querySelector('.pane-tab-strip');
  const tips = [...r.querySelectorAll('.pane-tab-tip')];
  const pill = (t) => t.querySelector('.pane-tab');
  const w = (el) => Math.round(el.getBoundingClientRect().width * 10) / 10;
  result[r.id] = {
    stripWidth: w(strip),
    room: parseFloat(strip.style.getPropertyValue('--pane-tab-room')),
    rangeScroll: range.scrollWidth - range.clientWidth,
    tips: tips.map(w),
    active: tips.findIndex((t) => pill(t).classList.contains('pane-tab--active')),
    labels: tips.map((t) => w(t.querySelector('.pane-tab-label'))),
    closeWidth: tips.map((t) => w(t.querySelector('.pane-tab-close'))),
    overlap: tips.some((t, i) => i > 0 && t.getBoundingClientRect().left < tips[i - 1].getBoundingClientRect().right - 0.5),
  };
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    stripWidth: number;
    room: number;
    rangeScroll: number;
    tips: number[];
    active: number;
    labels: number[];
    closeWidth: number[];
    overlap: boolean;
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const html = CASES.map(([id, w, n, step, z]) => row(id, w, n, step, z)).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 1000, height: 600 }
    );
}, 90_000);

describe.skipIf(!BROWSER)("a header's tab strip as its tabs run out of room", () => {
    it("takes the room it is given and no more, and the header does not scroll while the floors fit", () => {
        for (const id of ["fit", "clipped", "noLabel", "iconOnly", "zoomed"]) {
            const m = measured[id];
            expect(m.stripWidth, `strip width in ${id}`).toBeLessThanOrEqual(m.room + 1);
            expect(m.rangeScroll, `header scroll in ${id}`).toBeLessThanOrEqual(1);
            expect(m.overlap, `tabs overlap in ${id}`).toBe(false);
        }
    });

    it("clips labels with an ellipsis before it hides any, keeping every tab above its floor", () => {
        const m = measured.clipped;
        m.tips.forEach((t, i) => expect(t, `tab ${i}`).toBeGreaterThanOrEqual(i === m.active ? ACTIVE_FLOOR_PX : RESTING_FLOOR_PX - 1));
        expect(m.labels.every((l) => l > 1), JSON.stringify(m)).toBe(true);
    });

    it("leaves a strip that fits as it was: no tab shrinks", () => {
        const m = measured.fit;
        expect(m.stripWidth).toBeLessThan(m.room);
        expect(m.labels.every((l) => l > 20)).toBe(true);
    });

    it("hides the labels of tabs that are not selected at the steps, never the selected tab's", () => {
        for (const id of ["noLabel", "iconOnly"]) {
            const m = measured[id];
            m.labels.forEach((l, i) => (i === m.active ? expect(l, `selected label in ${id}`).toBeGreaterThan(8) : expect(l, `label ${i} in ${id}`).toBeLessThanOrEqual(1)));
        }
    });

    it("keeps the selected tab wider than the others and above its floor, with room for its close button", () => {
        for (const id of ["noLabel", "iconOnly"]) {
            const m = measured[id];
            expect(m.tips[m.active], id).toBeGreaterThanOrEqual(ACTIVE_FLOOR_PX - 1);
            expect(m.tips[m.active], id).toBeGreaterThan(Math.max(...m.tips.filter((_, i) => i !== m.active)));
            expect(m.closeWidth[m.active], `selected tab's close button in ${id}`).toBeGreaterThan(8);
        }
    });

    it("keeps resting tabs at or above the floor, as wide as icon and padding", () => {
        for (const id of ["noLabel", "iconOnly"]) {
            const m = measured[id];
            m.tips.forEach((t, i) => i !== m.active && expect(t, `tab ${i} in ${id}`).toBeGreaterThanOrEqual(RESTING_FLOOR_PX - 1));
        }
    });

    it("scrolls the header, as it always did, once the floors no longer fit", () => {
        expect(measured.floors.rangeScroll).toBeGreaterThan(1);
        expect(measured.floors.stripWidth).toBeGreaterThanOrEqual(minTabsRoom(8));
    });

    it("scales the floors with the strip's zoom", () => {
        const m = measured.zoomed;
        m.tips.forEach((t, i) => i !== m.active && expect(t).toBeGreaterThanOrEqual(RESTING_FLOOR_PX * 1.25 - 1));
    });
});
