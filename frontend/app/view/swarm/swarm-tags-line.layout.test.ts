// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tags line under a Swarm row's header, measured in a real browser (docs/specs/SPEC_SWARM_MODEL_AND_SEAT_TAGS_2026_11_20.md
 * §6.2 and §8): at pane widths of 360, 300 and 240 px it is one line that never wraps and stays inside the pane; its tag
 * starts where the summary line's text does, at every Settings and chrome zoom; below 120 px it is gone with the
 * summary's indent. Compiles the real `swarm-view.scss`, `swarm-seats.scss`, `swarm-tags.scss` and the Tag styles.
 * It skips, rather than fails, on a machine with no usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const UI = join(__dirname, "../../element/ui");
const css = [join(__dirname, "swarm-view.scss"), join(__dirname, "swarm-seats.scss"), join(__dirname, "swarm-tags.scss"), join(UI, "tag.scss")]
    .map((f) => sass.compile(f).css)
    .join("\n");

const TOKENS = `:root{--main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--border-color:#333;
--font-sans:Arial,sans-serif;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--leading-tight:1.2;
--space-1:4px;--space-1-5:6px;--space-2:8px;--radius-full:999px;--radius-sm:3px;--tint-3:#2a2a2a;
--modal-bg-color:#1c1c1c;--highlight-bg-color:#333;--model-sonnet:#497bf8;--model-sonnet-edge:#9ab4fb}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}
.pane{margin:6px 0}`;

/** A pane `css` px wide (the screen width over the zoom), the row's parts as `AgentCardHead` and `AgentCardLines` lay them out. */
function pane(id: string, cssWidth: number, zoom: number, label: string): string {
    return `<div class="pane" style="width:${cssWidth}px;zoom:${zoom}"><div class="swarm-tree" id="${id}"><div class="swarm-agent-card">
<div class="swarm-agent-row"><span style="width:16px;flex:none;height:14px"></span><span class="swarm-agent-expand-spacer"></span>
<span class="swarm-agent-label">Quill</span></div>
<div class="swarm-tags-line"><span class="ui-tag-anchor"><span class="ui-tag ui-tag--chip ui-tag--model-sonnet ui-tag--sm swarm-model-tag" tabindex="0">
<i class="fa-solid fa-star-of-life" aria-hidden="true" style="display:inline-block;width:10px;height:10px"></i><span class="ui-tag-sr">Model </span><span class="swarm-model-tag__text">${label}</span></span></span></div>
<div class="swarm-activity-summary">Working on the thing</div></div></div></div>`;
}

// [screen width, zoom]: the pane's own width is the screen width over the zoom, which the container query reads.
const CASES: Array<[number, number]> = [
    [360, 1],
    [300, 1],
    [240, 1],
    [200, 1],
    [121, 1],
    [120, 1],
    [100, 1],
    [300, 1.5],
    [300, 2],
];
const LABELS: Record<string, string> = { short: "Sonnet 5.5", long: "Sonnet 5.5 Extended…" };

const MEASURE = `
const rect = (el) => el.getBoundingClientRect();
const result = {};
for (const tree of document.querySelectorAll('.swarm-tree')) {
  const line = tree.querySelector('.swarm-tags-line');
  const tag = tree.querySelector('.swarm-model-tag');
  const summary = tree.querySelector('.swarm-activity-summary');
  const shown = getComputedStyle(line).display !== 'none';
  const pane = tree.parentElement;
  const z = parseFloat(pane.style.zoom) || 1;
  const px = (n) => Math.round(n * 10) / 10;
  const textLeft = (() => { const r = document.createRange(); r.selectNodeContents(summary); return rect(r).left; })();
  result[tree.id] = {
    shown,
    // Under a CSS zoom the rects are in screen px: the pane's own edge is the reference.
    wraps: shown ? rect(tag).height > rect(tag.querySelector('.swarm-model-tag__text')).height + 8 * z : false,
    inside: shown ? rect(tag).right <= rect(pane).right + 0.5 && rect(tag).left >= rect(pane).left - 0.5 : true,
    tagLeft: shown ? px(rect(tag).left - rect(pane).left) : 0,
    summaryLeft: px(textLeft - rect(pane).left),
    lineHeight: shown ? px(rect(line).height) : 0,
  };
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    shown: boolean;
    wraps: boolean;
    inside: boolean;
    tagLeft: number;
    summaryLeft: number;
    lineHeight: number;
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const html = CASES.flatMap(([w, z]) =>
        Object.entries(LABELS).map(([name, label]) => pane(`${name}-${w}-${z}`, w / z, z, label))
    ).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 700, height: 4000 }
    );
}, 90_000);

describe.skipIf(!BROWSER)("Swarm tags line", () => {
    it("is one line that does not wrap and stays inside the pane, at 360, 300 and 240 px", () => {
        for (const w of [360, 300, 240]) {
            for (const name of Object.keys(LABELS)) {
                const m = measured[`${name}-${w}-1`];
                expect(m.shown, `${name} at ${w}px`).toBe(true);
                expect(m.wraps, `${name} wraps at ${w}px`).toBe(false);
                expect(m.inside, `${name} inside the pane at ${w}px`).toBe(true);
            }
        }
    });

    it("is gone at 120 px and under, and is there above it", () => {
        expect(measured["short-121-1"].shown).toBe(true);
        expect(measured["short-120-1"].shown).toBe(false);
        expect(measured["short-100-1"].shown).toBe(false);
    });

    it("starts where the summary's text starts, at every zoom the pane is shown at", () => {
        for (const [w, z] of CASES) {
            const m = measured[`short-${w}-${z}`];
            if (!m.shown) continue;
            expect(Math.abs(m.tagLeft - m.summaryLeft), `at ${w}px, zoom ${z}: ${JSON.stringify(m)}`).toBeLessThanOrEqual(1);
        }
    });

    it("stays one line and inside the pane under chrome zoom 150% and 200%", () => {
        for (const z of [1.5, 2]) {
            const m = measured[`long-300-${z}`];
            expect(m.shown).toBe(true);
            expect(m.wraps, `wraps at zoom ${z}`).toBe(false);
            expect(m.inside, `inside at zoom ${z}`).toBe(true);
        }
    });
});
