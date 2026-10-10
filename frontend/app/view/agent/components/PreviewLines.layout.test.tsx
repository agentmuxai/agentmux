// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool previews, measured in a real browser: the definition of "previews are
 * always clean" (docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §7).
 *
 * Every earlier wrap and tab fix passed its tests because jsdom has no layout.
 * This renders real previews through the preview text stage and PreviewLines,
 * compiles the real `_preview.scss`, loads them in headless Chromium and
 * measures: one screen row per line in scroll mode, a hanging indent in wrap
 * mode, equal indentation steps whatever the gutter width, diff backgrounds
 * across the line, and the same box for streamed and finished output.
 *
 * It skips, rather than fails, on a machine with no usable browser
 * (layout-browser.ts findBrowser: on macOS, Chrome's headless shell only).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { join } from "node:path";
import * as sass from "sass";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { chunksDoc, codeDoc, diffDocFromSides, outputDoc, proseDoc } from "../preview-text/docs";
import type { PreviewDoc, PreviewMode } from "../preview-text/types";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "./layout-browser";
import { PreviewLines } from "./PreviewLines";

// No Shiki in this test: the plain text is what's measured.
vi.mock("./shiki-highlighter", () => ({
    codeToTokens: async () => {
        throw new Error("not in this test");
    },
}));
vi.spyOn(console, "warn").mockImplementation(() => {});

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);
const WIDTH = 400;

const TOKENS = `:root{--font-mono:Menlo,Consolas,"DejaVu Sans Mono",monospace;--main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--error-color:#f87171}
body{margin:0;background:#111;color:#ddd;font-size:12px}
.box{width:${WIDTH}px;overflow:auto;margin:8px 0}`;

/** Runs in the page. For each fixture box: rows per line, where each line's
 *  first non-space glyph sits (in ch from the text column), the text column's
 *  left edge, line widths, and the box's scroll width. */
const MEASURE = `
const ch = (() => { const s = document.createElement('span'); s.style.cssText = 'font-family:var(--font-mono);font-size:12px;position:absolute;visibility:hidden;white-space:pre'; s.textContent = '0'.repeat(100); document.body.appendChild(s); const w = s.getBoundingClientRect().width / 100; s.remove(); return w; })();
const rowsOf = (el) => { const r = document.createRange(); r.selectNodeContents(el); return new Set([...r.getClientRects()].filter((x) => x.width > 0).map((x) => Math.round(x.top))).size; };
const firstGlyphCh = (el) => {
  const text = el.textContent; const i = text.search(/\\S/); if (i < 0) return null;
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT); let n, acc = 0;
  while ((n = walker.nextNode())) { if (acc + n.textContent.length > i) { const r = document.createRange(); r.setStart(n, i - acc); r.setEnd(n, i - acc + 1); return Math.round((r.getBoundingClientRect().left - el.getBoundingClientRect().left) / ch * 10) / 10; } acc += n.textContent.length; }
  return null;
};
const rowLefts = (el) => { const r = document.createRange(); r.selectNodeContents(el); const byTop = new Map(); for (const x of r.getClientRects()) { if (x.width <= 0) continue; const t = Math.round(x.top); if (!byTop.has(t)) byTop.set(t, x.left); } return [...byTop.values()].map((l) => Math.round((l - el.getBoundingClientRect().left) / ch * 10) / 10); };
const result = {};
for (const box of document.querySelectorAll('.box')) {
  const texts = [...box.querySelectorAll('.agent-preview-text')];
  result[box.id] = {
    rows: texts.map(rowsOf),
    glyph: texts.map(firstGlyphCh),
    textLeft: texts.map((t) => Math.round(t.getBoundingClientRect().left - box.getBoundingClientRect().left)),
    rowLefts: texts.map(rowLefts),
    lineWidths: [...box.querySelectorAll('.agent-preview-line')].map((l) => Math.round(l.getBoundingClientRect().width)),
    box: { scrollWidth: box.scrollWidth, clientWidth: box.clientWidth, height: Math.round(box.getBoundingClientRect().height) },
  };
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Measured {
    rows: number[];
    glyph: (number | null)[];
    textLeft: number[];
    rowLefts: number[][];
    lineWidths: number[];
    box: { scrollWidth: number; clientWidth: number; height: number };
}

const goLines = [
    "package main",
    "",
    "func main() {",
    "\tfor i := 0; i < 3; i++ {",
    "\t\tif i%2 == 0 {",
    "\t\t\tfmt.Println(i)",
    "\t\t}",
    "\t}",
    "}",
];
const asRead = (start: number) => goLines.map((l, i) => `${String(start + i).padStart(6)}\t${l}`).join("\n");
const LONG =
    "frontend/app/view/agent/components/tool-renderers/builtins.tsx:221:const capped=content?capText(content,MAX_TOOL_OUTPUT_LINES,'head'):null;return<FilePreview/>";
const OUTPUT = `first line\nsecond\tcolumn\n${LONG}\nlast`;

const fixtures: { id: string; doc: PreviewDoc; mode?: PreviewMode }[] = [
    { id: "read-1", doc: codeDoc(asRead(1), { path: "a.go", gutter: true }) },
    { id: "read-95", doc: codeDoc(asRead(95), { path: "a.go", gutter: true }) },
    { id: "read-995", doc: codeDoc(asRead(995), { path: "a.go", gutter: true }) },
    { id: "output", doc: outputDoc(OUTPUT, { from: "tail" }) },
    {
        id: "streamed",
        doc: chunksDoc([
            { kind: "stdout", content: "first li" },
            { kind: "stdout", content: "ne\nsecond\tcol" },
            { kind: "stdout", content: `umn\n${LONG.slice(0, 50)}` },
            { kind: "stdout", content: `${LONG.slice(50)}\nlast` },
        ]),
    },
    {
        id: "prose",
        doc: proseDoc(
            `A sentence long enough that it has to wrap onto a second row in a ${WIDTH}px box, several times over in fact.\nshort`
        ),
    },
    {
        id: "diff",
        doc: diffDocFromSides(
            "const a = 1;\nconst b = 2;",
            "const a = 1;\nconst b = 3; // a longer replacement line than any other here",
            "a.ts"
        ),
    },
];

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const css = sass.compile(join(__dirname, "..", "styles", "_preview.scss")).css;
    const html = fixtures
        .map((f) => {
            const { container } = render(() => <PreviewLines doc={f.doc} mode={f.mode} />);
            const out = `<div class="box" id="${f.id}">${container.innerHTML}</div>`;
            cleanup();
            return out;
        })
        .join("\n");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 700, height: 1600 }
    );
}, 90_000);

afterEach(() => cleanup());

describe.skipIf(!BROWSER)("tool previews in a real browser", () => {
    it("indents a tab-indented Read 2 columns a level, whatever the gutter width", () => {
        const levels = [0, null, 0, 2, 4, 6, 4, 2, 0];
        for (const id of ["read-1", "read-95", "read-995"]) {
            expect(measured[id].glyph, id).toEqual(levels);
            // The text column starts at the same x on every line of a preview.
            expect(new Set(measured[id].textLeft).size, id).toBe(1);
        }
    });

    it("scroll mode: every line is one row; a long line widens the box instead of wrapping", () => {
        expect(measured.output.rows).toEqual([1, 1, 1, 1]);
        expect(measured.output.box.scrollWidth).toBeGreaterThan(measured.output.box.clientWidth);
    });

    it("expands an output tab to the next 8-column stop", () => {
        // "second" is 6 columns, so the tab is 2 wide and "column" starts at 8.
        expect(fixtures.find((f) => f.id === "output")!.doc.lines[1].text).toBe("second  column");
    });

    it("streamed output renders exactly like the finished output", () => {
        expect(measured.streamed.rows).toEqual(measured.output.rows);
        expect(measured.streamed.box).toEqual(measured.output.box);
    });

    it("wrap mode: prose wraps, and its wrapped rows sit 3ch in from the first", () => {
        const [rows] = measured.prose.rows;
        expect(rows).toBeGreaterThan(1);
        const [first, ...rest] = measured.prose.rowLefts[0];
        for (const left of rest) expect(left - first).toBeCloseTo(3, 0);
        expect(measured.prose.box.scrollWidth).toBe(measured.prose.box.clientWidth);
    });

    it("a diff line's background spans the whole line, as wide as the longest line", () => {
        const widths = measured.diff.lineWidths;
        expect(new Set(widths).size).toBe(1);
        expect(widths[0]).toBeGreaterThanOrEqual(measured.diff.box.clientWidth);
    });
});
