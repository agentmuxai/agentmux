// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A tool result's frame and its text, measured in a real browser (#47885). A result with long lines (a Tasks claim
 * or Tasks list answer) drew its frame at the width of the preview's scroll box while the lines ran on past it: the
 * frame's right border went through the text, and scrolling sideways moved the border with the text it crossed.
 *
 * This compiles the real `agent-view.scss`, builds a tool block the way `ToolBlock` and `PreviewLines` do, and checks
 * that a framed result's frame goes around its whole text at every pane width and zoom. It skips, rather than fails,
 * on a machine with no usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "../components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = sass.compile(join(__dirname, "../agent-view.scss")).css;

const TOKENS = `:root{
  --main-text-color:#ddd;--secondary-text-color:#999;--main-bg-color:#000;--border-color:rgba(255,255,255,.4);
  --accent-color:#58c142;--error-color:#eb755d;--success-bg:#123;--error-bg:#311;--accent-bg:#131;
  --font-mono:Consolas,'DejaVu Sans Mono','Liberation Mono',monospace;--text-2xs:10px;--text-xs:11px;--text-sm:12px;
  --space-0-5:2px;--space-1:4px;--space-1-5:6px;--space-2:8px;--space-3:12px;
}
body{margin:0;background:#000;color:#ddd;font:13px Arial,sans-serif}
.case{margin:0 0 8px;overflow:hidden}`;

/** The framed results: a text result of several lines, an expanded JSON result, and a file's content. */
const FRAMED = ["agent-terminal-output", "agent-tool-compact-json", "agent-tool-read-content"] as const;
type Framed = (typeof FRAMED)[number];

const LONG_WORD = "src/app/" + "a_very_long_file_name_with_no_place_to_break_".repeat(8) + "end.ts";
const LONG_LINES = [
    "Took item 12 (first try): pass its number and the try number to every later call about it, such as update, ask, submit, complete and release.",
    "",
    "#14  open, for nobody   Sample: a list row whose title runs on well past the width of a narrow pane, with a note in brackets (and a second clause after it)",
    LONG_WORD,
    "short",
];
const SHORT_LINES = ["ok", "2 tasks"];

const lines = (text: string[]) =>
    text.map((t) => `<div class="agent-preview-line"><span class="agent-preview-text">${t}</span></div>`).join("");

/** One tool block holding one result, in a view `width` px wide on screen at `zoom` (CSS `zoom`, as on `.agent-view-zoomed`). */
function block(id: string, cls: Framed, text: string[], width: number, zoom: number): string {
    const preview = `<div class="agent-preview agent-preview--scroll ${cls}" data-frame><div class="agent-preview-lines">${lines(text)}</div></div>`;
    const result = cls === "agent-tool-read-content" ? `<div class="agent-tool-read">${preview}</div>` : `<div class="agent-tool-compact-result">${preview}</div>`;
    return `<div class="case" style="width:${width}px"><div class="agent-view" data-case="${id}" style="zoom:${zoom};width:${width / zoom}px;height:auto;display:block">
<div class="agent-tool-block"><div class="agent-tool-panel agent-tool-panel--flow"><div class="agent-tool-overlay">
<div class="agent-tool-overlay-log" data-log><div class="agent-tool-overlay-log-content">${result}</div></div>
</div></div></div></div></div>`;
}

const WIDTHS = [320, 520, 900];
const ZOOMS = [0.75, 1, 1.5];

interface Measured {
    /** The frame's own box, and the furthest right any line's text reaches (the text is measured as laid out, by a range). */
    frame: { left: number; right: number; border: number; overflowX: string };
    textRight: number;
    /** The scroll box: its visible box, and the frame's right edge once it is scrolled as far right as it goes. */
    log: { left: number; right: number; clientWidth: number; scrollWidth: number; padLeft: number; padRight: number };
    frameRightAtEnd: number;
}

const MEASURE = `
const out = {};
for (const view of document.querySelectorAll("[data-case]")) {
  const zoom = Number(view.style.zoom) || 1;
  const log = view.querySelector("[data-log]");
  const frame = view.querySelector("[data-frame]");
  const rect = (el) => el.getBoundingClientRect();
  let textRight = 0;
  for (const t of frame.querySelectorAll(".agent-preview-text")) {
    const r = document.createRange(); r.selectNodeContents(t);
    textRight = Math.max(textRight, r.getBoundingClientRect().right);
  }
  const fs = getComputedStyle(frame), ls = getComputedStyle(log);
  const f = rect(frame), l = rect(log);
  log.scrollLeft = log.scrollWidth;
  const frameRightAtEnd = rect(frame).right;
  log.scrollLeft = 0;
  out[view.dataset.case] = {
    frame: { left: f.left, right: f.right, border: parseFloat(fs.borderRightWidth) * zoom, overflowX: fs.overflowX },
    textRight,
    log: { left: l.left, right: l.right, clientWidth: log.clientWidth, scrollWidth: log.scrollWidth, padLeft: parseFloat(ls.paddingLeft) * zoom, padRight: parseFloat(ls.paddingRight) * zoom },
    frameRightAtEnd,
  };
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

const cases: { id: string; cls: Framed; long: boolean; width: number; zoom: number }[] = [];
for (const cls of FRAMED) for (const width of WIDTHS) for (const zoom of ZOOMS) for (const long of [true, false]) cases.push({ id: `${cls}-${width}-${zoom}-${long ? "long" : "short"}`, cls, long, width, zoom });

describe.skipIf(!BROWSER)("a tool result's frame", () => {
    let r: Record<string, Measured>;
    beforeAll(async () => {
        const body = cases.map((c) => block(c.id, c.cls, c.long ? LONG_LINES : SHORT_LINES, c.width, c.zoom)).join("");
        const html = `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${body}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        r = await measureInBrowser<Record<string, Measured>>(BROWSER!, html, { width: 1000, height: 900 });
    }, 60_000);

    // Half a pixel: the browser places text and borders on fractions of a pixel at a zoom that is not 1.
    const SLACK = 0.5;

    it.each(cases.filter((c) => c.long))("$id: goes around its whole text, however long the lines", ({ id }) => {
        const m = r[id];
        // The case reproduces the report: the lines are longer than the scroll box shows.
        expect(m.textRight, "the lines are longer than the box").toBeGreaterThan(m.log.right);
        expect(m.textRight, "text past the frame's right border").toBeLessThanOrEqual(m.frame.right - m.frame.border + SLACK);
    });

    it.each(cases.filter((c) => c.long))("$id: scrolls with its text, and its right border comes into view at the end", ({ id }) => {
        const m = r[id];
        // One scroll box, the preview's: the frame does not scroll by itself (no frame and scrollbar inside a frame).
        expect(m.frame.overflowX).toBe("visible");
        expect(m.log.scrollWidth).toBeGreaterThan(m.log.clientWidth);
        // Scrolled to the end, the frame's right border is at the scroll box's right edge (the box's own padding is not
        // added after content that overflows its wrapper), so it is in view.
        expect(m.frameRightAtEnd, "the frame's right border is inside the scroll box at the end").toBeLessThanOrEqual(m.log.right + SLACK);
        expect(m.frameRightAtEnd, "the frame ends where the scroll box does").toBeGreaterThan(m.log.right - m.log.padRight - 1);
    });

    it.each(cases.filter((c) => !c.long))("$id: fills the preview's width when its lines are short", ({ id }) => {
        const m = r[id];
        expect(m.log.scrollWidth).toBe(m.log.clientWidth);
        expect(m.frame.left).toBeCloseTo(m.log.left + m.log.padLeft, 0);
        expect(Math.abs(m.frame.right - (m.log.right - m.log.padRight))).toBeLessThanOrEqual(1);
    });
});
