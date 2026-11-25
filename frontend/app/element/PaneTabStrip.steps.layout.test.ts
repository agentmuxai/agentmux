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
import {
    ACTIVE_FLOOR_PX,
    ACTIVE_LABEL_MIN_PX,
    ACTIVE_NO_ICON_FLOOR_PX,
    minTabsRoom,
    RESTING_FLOOR_PX,
    RESTING_NO_ICON_FLOOR_PX,
} from "./pane-tab-steps";

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

function row(id: string, width: number, tabs: number, step: string, zoom: number, renaming = false): string {
    // What pane-tab-strip-steps.ts sets: the room the range leaves the strip, never under the tabs' floors.
    const total = width - OTHERS_PX - GAP_PX * 3; // three other items (the spacer too), three gaps
    // A row of five or more has a tab with no icon (index 4), which gives the strip the wider floors.
    const noIcon = tabs > 4;
    const floors = noIcon ? [RESTING_NO_ICON_FLOOR_PX, ACTIVE_NO_ICON_FLOOR_PX] : [RESTING_FLOOR_PX, ACTIVE_FLOOR_PX];
    const limit = Math.max(total, minTabsRoom(tabs, floors[0], floors[1]) * zoom + ADD_PX * zoom);
    const pills = Array.from({ length: tabs }, (_, i) => {
        const active = i === 1 ? " pane-tab--active" : "";
        const attention = i === 2 ? " pane-tab--attention" : "";
        const noIcon = i === 4 ? " doc-tab--no-icon" : ""; // a tab with no icon of its own keeps its label
        // The inline rename input (PaneTabRenameInput) takes the selected tab's label's place.
        const label =
            renaming && i === 1
                ? `<input class="pane-tab-label pane-tab-rename-input" value="${LABELS[i]}">`
                : `<span class="pane-tab-label">${LABELS[i % LABELS.length]}</span>`;
        return (
            `<div class="pane-tab-tip"><div class="pane-tab${active}${attention}${noIcon}" data-tab-id="t${i}">` +
            `<span class="pane-tab-icon"><i>*</i></span>${label}` +
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

/**
 * The strip of a pane header whose selected tab (the first) has a long title or a short one, with a long-titled tab among the
 * others: the case where the selected tab was cut to its icon while an unselected tab kept its whole title (it carried the
 * pane's buttons then; they are rows of the pane's menu now, and the rule is the same for the tab's icon, title and close). The selected tab's floor is written the way pane-tab-strip-steps.ts does: what is
 * not its title, and the start of its title, read from where the items stand once laid out unsqueezed (`FLOOR_SCRIPT`).
 */
const SELECTED_TITLES = ["Camper the agent", "Cl", "Agent two", "/workspace/handoff/DRAFT_REPORT_WITH_A_LONG_NAME.md", "Atlas"];
function selectedRow(id: string, width: number, tabs: number, step: string, zoom: number): string {
    const total = width - OTHERS_PX - GAP_PX * 3;
    const pills = Array.from({ length: tabs }, (_, i) => {
        const active = i === 0 ? " pane-tab--active" : "";
        return (
            `<div class="pane-tab-tip"><div class="pane-tab${active}" data-tab-id="t${i}">` +
            `<span class="pane-tab-icon"><i>*</i></span><span class="pane-tab-label">${SELECTED_TITLES[i % SELECTED_TITLES.length]}</span>` +
            `<button class="pane-tab-close" aria-label="Close tab">x</button></div></div>`
        );
    }).join("");
    return (
        `<div class="row" id="${id}" data-selected-first="1" style="--w:${width}px"><div class="block-frame-header-scroll" style="width:${width}px">` +
        `<div class="block-frame-header-content"><div style="width:${OTHERS_PX / 2}px;flex:0 0 auto"></div>` +
        `<div class="block-frame-default-header-tabstrip"><div class="pane-tab-strip" data-step="${step}" data-total="${total}" data-add="${ADD_PX}" style="--pane-tab-strip-zoom:${zoom};--pane-tab-room:${Math.max(total, 1)}px">` +
        `<div class="pane-tab-strip-inner">${pills}<div class="pane-tab-strip-add-tip"><button class="pane-tab-strip-add"><span class="pane-tab-strip-add-glyph"></span></button></div></div></div></div>` +
        `<div class="block-frame-header-spacer"></div><div style="width:${OTHERS_PX / 2}px;flex:0 0 auto"></div></div></div></div>`
    );
}

// [id, header width, tab count, step, zoom]
const SELECTED_CASES: Array<[string, number, number, string, number]> = [
    ["agentFull", 540, 5, "full", 1],
    ["agentIcon", 540, 5, "icon", 1],
    ["agentNarrow", 400, 8, "icon", 1],
    ["agentZoomed", 640, 5, "full", 1.5],
    ["editorFull", 600, 5, "full", 1],
    ["editorIcon", 480, 8, "icon-close", 1],
];

// [id, header width, tab count, step, zoom]
// With a tab that has no icon (a row of five or more) the others keep the wider floors too, so these rows are wide enough
// to hold them: a narrower header scrolls instead (the "floors" row).
const CASES: Array<[string, number, number, string, number, boolean?]> = [
    ["renaming", 400, 8, "icon", 1, true],
    ["fit", 900, 3, "full", 1],
    ["clipped", 700, 6, "full", 1],
    ["noLabel", 600, 7, "icon-close", 1],
    ["iconOnly", 640, 8, "icon", 1],
    ["floors", 260, 8, "icon", 1],
    ["zoomed", 700, 7, "icon-close", 1.25],
];

/**
 * What pane-tab-strip-steps.ts `activeTabNeeds` and `activeFloor` do, for the rows whose selected tab is first: the strip
 * is first laid out wide, the pill's items are read, and the floor (what is not the title, and 56 px of it) is written on the strip.
 */
const FLOOR_SCRIPT = `
for (const r of document.querySelectorAll('.row[data-selected-first]')) {
  const strip = r.querySelector('.pane-tab-strip');
  const zoom = parseFloat(strip.style.getPropertyValue('--pane-tab-strip-zoom')) || 1;
  const pill = r.querySelector('.pane-tab--active');
  const items = [...pill.children].filter((el) => getComputedStyle(el).position !== 'absolute');
  const first = items[0].getBoundingClientRect(), last = items[items.length - 1].getBoundingClientRect();
  const label = pill.querySelector('.pane-tab-label');
  const pillBox = pill.getBoundingClientRect();
  const labelBox = label.getBoundingClientRect();
  const fixed = (last.right - first.left - labelBox.width + Math.max(0, first.left - pillBox.left)) / zoom + 5;
  const floor = Math.max(${ACTIVE_FLOOR_PX}, fixed + Math.min(label.scrollWidth / zoom, ${ACTIVE_LABEL_MIN_PX}));
  strip.style.setProperty('--pane-tab-active-floor', floor + 'px');
  const limit = Math.max(Number(strip.dataset.total), (${ACTIVE_FLOOR_PX} > floor ? ${ACTIVE_FLOOR_PX} : floor) * zoom + ${RESTING_FLOOR_PX} * zoom * (r.querySelectorAll('.pane-tab-tip').length - 1) + Number(strip.dataset.add) * zoom);
  strip.style.setProperty('--pane-tab-room', limit + 'px');
}`;

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
    activeLabelCut: (() => { const l = r.querySelector('.pane-tab--active .pane-tab-label'); return l.scrollWidth > l.clientWidth + 1; })(),
    closeWidth: tips.map((t) => w(t.querySelector('.pane-tab-close'))),
    closeOpacity: tips.map((t) => parseFloat(getComputedStyle(t.querySelector('.pane-tab-close')).opacity)),
    overlap: tips.some((t, i) => i > 0 && t.getBoundingClientRect().left < tips[i - 1].getBoundingClientRect().right - 0.5),
    pillRight: r.querySelector('.pane-tab--active').getBoundingClientRect().right,
    closeRight: r.querySelector('.pane-tab--active .pane-tab-close').getBoundingClientRect().right,
    floorVar: parseFloat(strip.style.getPropertyValue('--pane-tab-active-floor')) || null,
    centreHitsClose: tips.map((t) => { const r = t.getBoundingClientRect(); const el = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); return !!(el && el.closest('.pane-tab-close')); }),
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
    activeLabelCut: boolean;
    closeWidth: number[];
    closeOpacity: number[];
    overlap: boolean;
    pillRight: number;
    closeRight: number;
    floorVar: number | null;
    centreHitsClose: boolean[];
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const html =
        CASES.map(([id, w, n, step, z, renaming]) => row(id, w, n, step, z, renaming)).join("") +
        SELECTED_CASES.map(([id, w, n, step, z]) => selectedRow(id, w, n, step, z)).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>window.addEventListener("load",()=>{${FLOOR_SCRIPT}\n${MEASURE}})</script>`,
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

    it("hides the labels of tabs that are not selected at the steps, and keeps the start of the selected tab's title", () => {
        for (const id of ["noLabel", "iconOnly"]) {
            const m = measured[id];
            m.labels.forEach((l, i) => {
                if (i === 4) return; // no icon: see below
                if (i === m.active) expect(l, `selected label in ${id}`).toBeGreaterThan(8);
                else expect(l, `label ${i} in ${id}`).toBeLessThanOrEqual(1);
            });
        }
    });

    it("shows the selected tab's close button, not only on hover, at every width", () => {
        for (const id of ["fit", "clipped", "noLabel", "iconOnly", "floors", "zoomed"]) {
            const m = measured[id];
            expect(m.closeOpacity[m.active], `selected tab's close in ${id}`).toBe(1);
        }
    });

    it("keeps the label of a tab that has no icon, so it can still be told apart", () => {
        for (const id of ["noLabel", "iconOnly"]) expect(measured[id].labels[4], id).toBeGreaterThan(1);
    });

    it("keeps the selected tab above its floor, with room for its close button, and wider than a resting tab with an icon", () => {
        for (const id of ["noLabel", "iconOnly"]) {
            const m = measured[id];
            expect(m.tips[m.active], id).toBeGreaterThanOrEqual(ACTIVE_FLOOR_PX - 1);
            expect(m.tips[m.active], id).toBeGreaterThan(Math.max(...m.tips.filter((_, i) => i !== m.active && i !== 4)));
            expect(m.closeWidth[m.active], `selected tab's close button in ${id}`).toBeGreaterThan(8);
        }
    });

    it("keeps resting tabs at or above the floor, as wide as icon and padding", () => {
        for (const id of ["noLabel", "iconOnly"]) {
            const m = measured[id];
            m.tips.forEach((t, i) => i !== m.active && expect(t, `tab ${i} in ${id}`).toBeGreaterThanOrEqual(RESTING_FLOOR_PX - 1));
        }
    });

    it("keeps the rename input of the selected tab visible and wide enough to type in, at the icon step", () => {
        const m = measured.renaming;
        expect(m.labels[m.active], JSON.stringify(m)).toBeGreaterThan(60);
        expect(m.tips[m.active]).toBeGreaterThanOrEqual(110);
        expect(m.overlap).toBe(false);
    });

    it("leaves the middle of a resting tab to select it at the icon steps: only the selected tab's close is under the pointer", () => {
        for (const id of ["noLabel", "iconOnly"]) {
            const m = measured[id];
            m.centreHitsClose.forEach((hit, i) => {
                if (i !== m.active) expect(hit, `middle of tab ${i} in ${id}`).toBe(false);
            });
        }
    });

    it("scrolls the header, as it always did, once the floors no longer fit", () => {
        expect(measured.floors.rangeScroll).toBeGreaterThan(1);
        expect(measured.floors.stripWidth).toBeGreaterThanOrEqual(minTabsRoom(8, RESTING_NO_ICON_FLOOR_PX, ACTIVE_NO_ICON_FLOOR_PX));
    });

    it("scales the floors with the strip's zoom", () => {
        const m = measured.zoomed;
        m.tips.forEach((t, i) => i !== m.active && expect(t).toBeGreaterThanOrEqual(RESTING_FLOOR_PX * 1.25 - 1));
    });

    it("gives the selected tab its room first: its close button whole, and the start of its title, when the others are squeezed", () => {
        for (const id of ["agentFull", "agentIcon", "agentZoomed", "editorFull"]) {
            const m = measured[id];
            expect(m.closeRight, `close button inside the selected tab in ${id}`).toBeLessThanOrEqual(m.pillRight + 0.5);
            expect(m.overlap, `tabs overlap in ${id}`).toBe(false);
        }
    });

    it("never leaves the selected tab narrower than its floor, and never leaves a tab that is not selected wider than a selected tab whose title is cut", () => {
        for (const [id, , , , zoom] of SELECTED_CASES) {
            const m = measured[id];
            expect(m.floorVar, `floor written in ${id}`).not.toBeNull();
            const selected = m.tips[m.active];
            expect(selected, `selected tab in ${id}`).toBeGreaterThanOrEqual(m.floorVar! * zoom - 1);
            // A selected tab at its own width, its title whole, may be narrower than a long-titled tab with room; once its
            // title is cut, no tab that is not selected is wider than it.
            if (m.activeLabelCut) {
                m.tips.forEach((t, i) => {
                    if (i !== m.active) expect(t, `tab ${i} wider than the selected in ${id}`).toBeLessThanOrEqual(selected + 0.5);
                });
            }
        }
    });

    it("cuts the long title of a tab that is not selected before it cuts the selected tab's", () => {
        const m = measured.agentFull;
        // The long-titled tab (the fourth) is cut, and the selected tab keeps its title.
        expect(m.labels[m.active], JSON.stringify(m)).toBeGreaterThanOrEqual(ACTIVE_LABEL_MIN_PX - 2);
        // The long title of the tab that is not selected is the one that gives way (it is under its natural 240 px), and the selected title is whole.
        expect(m.tips[3], JSON.stringify(m)).toBeLessThan(239);
        expect(m.activeLabelCut, JSON.stringify(m)).toBe(false);
    });

    it("keeps the header from scrolling while the floors fit", () => {
        for (const id of ["agentFull", "agentIcon", "agentZoomed", "editorFull", "editorIcon"]) expect(measured[id].rangeScroll, id).toBeLessThanOrEqual(1);
    });
});
