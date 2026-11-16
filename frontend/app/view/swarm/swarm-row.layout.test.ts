// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Swarm row in a narrow pane, measured in a real browser (#47761): an agent that runs with no pane carries a
 * "No pane" mark, which used to take the room the seat badge needed, so the badge read "Wor…" or was missing.
 * Who an agent is (its name and role) now outlasts the mark, the age and the context size.
 *
 * This compiles the real `swarm-view.scss`, `swarm-seats.scss` and the Tag and Pill styles, lays a row's parts
 * out the way `AgentCardHead` does, and measures what is left at each width. It skips, rather than fails, on a
 * machine with no usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const UI = join(__dirname, "../../element/ui");
const css = [join(__dirname, "swarm-view.scss"), join(__dirname, "swarm-seats.scss"), join(UI, "tag.scss"), join(UI, "pill.scss")]
    .map((f) => sass.compile(f).css)
    .join("\n");

const TOKENS = `:root{--main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--border-color:#333;
--font-sans:Arial,sans-serif;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--leading-tight:1.2;
--space-1:2px;--space-1-5:3px;--space-2:4px;--radius-full:999px;--radius-sm:3px;--tint-3:#2a2a2a;--success-color:#58c142;
--success-edge:#3a7a2c;--modal-bg-color:#1c1c1c;--highlight-bg-color:#333}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}
.pane{margin:6px 0}`;

/** `parked`: the row carries the "No pane" mark. Widths are the Swarm pane's, which the row is measured against. */
function row(id: string, width: number, parked: boolean): string {
    const mark = parked
        ? `<span class="swarm-nopane-slot"><span class="ui-tag ui-tag--chip ui-tag--ok ui-tag--sm">No pane</span></span>`
        : "";
    return `<div class="pane" style="width:${width}px"><div class="swarm-tree" id="${id}"><div class="swarm-agent-row">
<span style="width:16px;flex:none;height:14px"></span>
<span class="swarm-agent-expand-spacer"></span>
<span class="swarm-agent-icon"><i style="display:inline-block;width:14px;height:14px;background:#888"></i></span>
<span class="swarm-agent-label">Quill</span>
<span class="swarm-seat-slot"><span class="ui-pill swarm-seat-badge">Worker</span></span>
${mark}
<span class="swarm-ctx-size">63k</span>
<span class="swarm-last-active">1m ago</span>
<span class="swarm-status-chip" style="display:inline-flex;align-items:center;gap:4px;padding:0 6px;border:1px solid #444;white-space:nowrap"><i style="display:inline-block;width:6px;height:6px;background:#58c142;border-radius:3px"></i><span class="swarm-status-label">Waiting for an answer</span></span>
<span class="swarm-supervisor-chip" style="display:inline-flex;flex:none;gap:2px"><i style="display:inline-block;width:14px;height:14px;background:#777"></i><span class="swarm-supervisor-mode">auto</span></span>
</div></div></div>`;
}

// Every 10 px: the squeeze that cut a badge to "Wor…" sits in a narrow band, and which width it is depends on the
// fonts, so the test sweeps instead of naming one.
const WIDTHS = Array.from({ length: 33 }, (_, i) => 240 + i * 10);

const MEASURE = `
const fits = (el, whole) => {
  if (!el || getComputedStyle(el).display === 'none') return false;
  const w = whole.getBoundingClientRect();
  if (w.width <= 0) return false;
  const r = document.createRange(); r.selectNodeContents(el);
  const rects = [...r.getClientRects()].filter((x) => x.width > 0);
  return rects.length > 0 && rects.every((x) => x.left >= w.left - 0.5 && x.right <= w.right + 0.5 && x.top >= w.top - 0.5 && x.bottom <= w.bottom + 0.5);
};
const result = {};
for (const tree of document.querySelectorAll('.swarm-tree')) {
  const r = tree.querySelector('.swarm-agent-row');
  const q = (cls) => r.querySelector(cls);
  const badge = q('.swarm-seat-slot .swarm-seat-badge');
  result[tree.id] = {
    row: { scrollWidth: r.scrollWidth, clientWidth: r.clientWidth },
    name: fits(q('.swarm-agent-label'), q('.swarm-agent-label')) && q('.swarm-agent-label').scrollWidth <= q('.swarm-agent-label').clientWidth + 0.5,
    badge: fits(badge, q('.swarm-seat-slot')) && badge.scrollWidth <= badge.clientWidth + 0.5,
    mark: fits(q('.swarm-nopane-slot .ui-tag'), q('.swarm-nopane-slot')),
    ctx: fits(q('.swarm-ctx-size'), q('.swarm-ctx-size')) && q('.swarm-ctx-size').getBoundingClientRect().width > 8,
    age: fits(q('.swarm-last-active'), q('.swarm-last-active')) && q('.swarm-last-active').getBoundingClientRect().width > 8,
  };
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    row: { scrollWidth: number; clientWidth: number };
    name: boolean;
    badge: boolean;
    mark: boolean;
    age: boolean;
    ctx: boolean;
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const html = WIDTHS.flatMap((w) => [row(`plain-${w}`, w, false), row(`parked-${w}`, w, true)]).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 700, height: 3000 }
    );
}, 90_000);

describe.skipIf(!BROWSER)("Swarm row in a narrow pane", () => {
    it("shows every part when there is room", () => {
        expect(measured["parked-560"]).toMatchObject({ name: true, badge: true, mark: true, age: true, ctx: true });
    });

    it("gives up the age, then the context size, then the No pane mark, then the role badge", () => {
        // At 240 px and under the badge is dropped by its own container step, whatever else is left.
        for (const w of WIDTHS.filter((v) => v > 240)) {
            const m = measured[`parked-${w}`];
            const at = `at ${w}px: ${JSON.stringify(m)}`;
            if (m.age) expect(m.ctx, `context size kept with the age ${at}`).toBe(true);
            if (m.ctx) expect(m.mark, `mark kept with the context size ${at}`).toBe(true);
            if (m.mark) expect(m.badge, `badge kept with the mark ${at}`).toBe(true);
        }
    });

    it("loses the mark before the badge: some width has the whole badge and no mark", () => {
        const widths = WIDTHS.filter((w) => measured[`parked-${w}`].badge && !measured[`parked-${w}`].mark);
        expect(widths.length).toBeGreaterThan(0);
    });

    it("keeps the name whole wherever the badge is", () => {
        for (const w of WIDTHS) {
            const m = measured[`parked-${w}`];
            if (m.badge) expect(m.name, `name with the badge at ${w}px`).toBe(true);
        }
    });

    it("never grows past the pane", () => {
        for (const w of WIDTHS) {
            for (const key of [`plain-${w}`, `parked-${w}`]) {
                expect(measured[key].row.scrollWidth, key).toBeLessThanOrEqual(measured[key].row.clientWidth + 1);
            }
        }
    });
});
