// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The wheel over a row that shows a hover panel, in a real browser (#47491). A person reported the wheel going dead
 * while the time/token panel was showing, with the pointer on the row and the panel just below it. This builds the
 * transcript's scroller and the panel as `PeekOverlay` places and styles it (the real `computePeekVertical`,
 * `computePeekHorizontal`, `peekFloatingStyle` and the real `agent-view.scss`), sends the browser real wheel input
 * (`Input.dispatchMouseEvent`, the same pipeline as a mouse), and checks:
 *  - a panel that fits is clear of the pointer and takes no pointer events, so a wheel at the pointer moves the
 *    transcript by the wheel's amount;
 *  - a panel that was cut and scrolls takes the wheel on itself, and a wheel at the pointer (on the row) still
 *    moves the transcript.
 * It skips, rather than fails, on a machine with no usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, NO_BROWSER_HINT } from "./layout-browser";
import { withInputPage, type InputPage } from "./layout-browser-input";
import { peekFloatingStyle } from "./peek-floating-style";
import { CURSOR_GAP_PX, computePeekHorizontal, computePeekVertical } from "./peek-placement";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = sass.compile(join(__dirname, "../agent-view.scss")).css;

const VIEWPORT = { width: 800, height: 600 };
/** The transcript: a column 600 wide and 400 tall, from the window's top left. */
const CONTAINER = { top: 0, bottom: 400 };
const ROW_HEIGHT = 30;
/** The pointer: on row 6 (y 180 to 210), at its right half. */
const POINTER = { x: 450, y: 200 };
const ROW = { left: 0, right: 600, top: 180, bottom: 210 };

const TOKENS = `:root{
  --main-text-color:#ddd;--secondary-text-color:#999;--main-bg-color:#222;--border-color:rgba(255,255,255,.4);
  --font-mono:Consolas,'DejaVu Sans Mono','Liberation Mono',monospace;--text-base:13px;--text-xs:11px;
  --space-0-5:2px;--space-1:4px;--space-2:8px;--z-node-peek:50;--leading-normal:1.4;
}
body{margin:0;background:#000;color:#ddd;font:13px Arial,sans-serif}`;

const styleAttr = (style: object) =>
    Object.entries(style)
        .map(([k, v]) => `${k}:${v}`)
        .join(";");

function fixture(panelStyle: object, panelLines: number): string {
    const rows = Array.from(
        { length: 60 },
        (_, i) => `<div class="agent-document-row" style="height:${ROW_HEIGHT}px">row ${i}</div>`
    ).join("");
    const lines = Array.from({ length: panelLines }, (_, i) => `<div>12:00:${String(i % 60).padStart(2, "0")} · 340 tokens</div>`).join("");
    return `<!doctype html><meta charset="utf-8"><style>${css}${TOKENS}</style>
<div id="scroller" class="agent-document" style="position:fixed;left:0;top:0;width:600px;height:${CONTAINER.bottom}px;overflow-y:auto">${rows}</div>
<div id="panel" class="agent-node-peek-overlay" style="${styleAttr(panelStyle)}">${lines}</div>`;
}

/** Wait for the scroller to stop moving, then return where it is. */
async function settledScrollTop(page: InputPage): Promise<number> {
    let last = -1;
    for (let i = 0; i < 40; i++) {
        const now = await page.evaluate<number>("document.getElementById('scroller').scrollTop");
        if (now === last) return now;
        last = now;
        await new Promise((r) => setTimeout(r, 60));
    }
    return last;
}

const WHEEL = 100;

describe.skipIf(!BROWSER)("the wheel over a row that shows a hover panel", () => {
    it("a panel that fits sits above the pointer, takes no pointer events, and the wheel scrolls the transcript", async () => {
        const naturalWidth = 170;
        const naturalHeight = 22;
        const hz = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth });
        const vt = computePeekVertical({ row: ROW, mouseY: POINTER.y, container: CONTAINER, viewport: VIEWPORT, naturalHeight });
        expect(vt.scrolls).toBe(false);
        const html = fixture(peekFloatingStyle(hz, vt, false), 1);
        const out = await withInputPage(BROWSER!, html, VIEWPORT, async (page) => {
            await page.move(POINTER.x, POINTER.y);
            const panel = await page.evaluate<{ top: number; bottom: number; left: number; right: number; events: string }>(
                `(() => { const p = document.getElementById('panel'); const r = p.getBoundingClientRect();
                  return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, events: getComputedStyle(p).pointerEvents }; })()`
            );
            const hit = await page.evaluate<string[]>(
                `document.elementsFromPoint(${POINTER.x}, ${POINTER.y}).map((e) => e.id || e.className)`
            );
            await page.wheel(POINTER.x, POINTER.y, WHEEL);
            return { panel, hit, scrollTop: await settledScrollTop(page) };
        });
        expect(out.panel.events).toBe("none");
        // Clear of the pointer, by the gap, above it.
        expect(out.panel.bottom).toBeLessThanOrEqual(POINTER.y - CURSOR_GAP_PX);
        expect(out.hit).not.toContain("panel");
        expect(out.scrollTop).toBe(WHEEL);
    }, 90_000);

    it("a panel that fits and went below the pointer for want of room above still lets the wheel through", async () => {
        const row = { left: 0, right: 600, top: 0, bottom: 30 };
        const naturalHeight = 22;
        const hz = computePeekHorizontal({ row, viewport: VIEWPORT, naturalWidth: 170 });
        const vt = computePeekVertical({ row, mouseY: 10, container: CONTAINER, viewport: VIEWPORT, naturalHeight });
        expect(vt.top).toBeGreaterThan(10);
        const html = fixture(peekFloatingStyle(hz, vt, false), 1);
        const scrollTop = await withInputPage(BROWSER!, html, VIEWPORT, async (page) => {
            await page.move(450, 10);
            await page.wheel(450, 10, WHEEL);
            return settledScrollTop(page);
        });
        expect(scrollTop).toBe(WHEEL);
    }, 90_000);

    it("a panel that was cut scrolls on itself, and a wheel at the pointer on the row moves the transcript", async () => {
        const naturalHeight = 5000;
        const hz = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth: 170 });
        const vt = computePeekVertical({
            row: ROW,
            mouseY: POINTER.y,
            container: CONTAINER,
            viewport: VIEWPORT,
            naturalHeight,
            flushToRow: true,
        });
        expect(vt.scrolls).toBe(true);
        const html = fixture(peekFloatingStyle(hz, vt, true), 200);
        const out = await withInputPage(BROWSER!, html, VIEWPORT, async (page) => {
            const r = await page.evaluate<{ x: number; y: number; events: string }>(
                `(() => { const p = document.getElementById('panel'); const r = p.getBoundingClientRect();
                  return { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2, events: getComputedStyle(p).pointerEvents }; })()`
            );
            await page.wheel(POINTER.x, POINTER.y, WHEEL);
            const onRow = await settledScrollTop(page);
            await page.wheel(r.x, r.y, WHEEL);
            const after = await settledScrollTop(page);
            const onPanel = await page.evaluate<number>("document.getElementById('panel').scrollTop");
            return { events: r.events, onRow, onPanel, after };
        });
        expect(out.events).toBe("auto");
        expect(out.onRow).toBe(WHEEL);
        expect(out.onPanel).toBeGreaterThan(0);
        // The panel took that notch: the transcript stayed where the first one left it.
        expect(out.after).toBe(WHEEL);
    }, 90_000);
});
