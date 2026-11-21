// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A selection drag out of a text field, in a real browser: a pane tile carries `transform: translate3d`, and
 * under a transformed ancestor the engine maps a pointer that has left the field to the wrong character. The
 * guard (`text-drag.ts`) sets the selection from where the pointer is. Each case runs the same drag twice, once
 * in the tile as the app lays it out (moved by a transform, the guard installed) and once beside it moved by a
 * margin with no guard, where the engine is right, and the selections must match at every point of the drag.
 * (The cause is the engine's, not the app's: in a bare page with no app styles, a textarea in a box moved by
 * `transform: translateX(600px)` mis-selects the same way while one moved by `margin-left: 600px` does not.)
 * Skips with no usable browser.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { transformSync } from "esbuild";
import { describe, expect, it } from "vitest";
import { findBrowser } from "@/app/view/agent/components/layout-browser";
import { driveInBrowser, type DrivenPage } from "@/app/view/agent/components/layout-drive";

const BROWSER = findBrowser();
const GUARD = transformSync(readFileSync(join(__dirname, "text-drag.ts"), "utf8"), { loader: "ts", format: "iife", globalName: "TD" }).code;
// The stylesheet rules the guard turns its marker into: the real ones, not a copy.
const RULES = (readFileSync(join(__dirname, "../../app.scss"), "utf8").match(/html\[data-text-drag\][^{]*\{[^}]*\}/g) ?? []).join("\n");

interface Case {
    kind: "textarea" | "input";
    /** Where the pane's left edge is: 0 is the leftmost pane. */
    x: number;
    /** The pane's `zoom`. */
    zoom: number;
}

const TEXT = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";

/** The agent pane's shape: a tile moved by a transform (or, for the control, a margin) around a slot that sets `pointer-events: auto` inline. */
function fixture(c: Case, moved: "transform" | "margin"): string {
    const shift = moved === "transform" ? `transform:translate3d(${c.x}px,0,0)` : `margin-left:${c.x}px`;
    const field =
        c.kind === "textarea"
            ? `<textarea id="f" style="display:block;width:260px;height:64px;font:13px monospace;resize:none;overflow:hidden">${TEXT}</textarea>`
            : `<input id="f" style="display:block;width:260px;font:13px monospace" value="alpha beta gamma delta eps">`;
    return `<!doctype html><meta charset="utf-8"><style>html,body{margin:0;overflow:hidden}${RULES}</style>
<div id="tile" style="position:absolute;left:0;top:40px;width:520px;height:300px;${shift}">
<div id="slot" style="position:absolute;inset:0;pointer-events:auto"><div style="zoom:${c.zoom};padding:20px">${field}</div></div></div>
<script>${moved === "transform" ? `${GUARD};TD.installTextDragGuard(window);` : ""}</script>`;
}

type Sel = [number, number, string];
interface Run {
    steps: Record<string, Sel>;
    /** Whether the slot was under the pointer, at the far left of the field, with the drag marked. */
    slotHit: boolean;
}

/** Press in the field's first line, drag out left, right, above and below, then out of the window, and release. */
async function drag(p: DrivenPage): Promise<Run> {
    for (let i = 0; i < 50 && !(await p.eval<boolean>(`!!document.getElementById("f")`)); i++) await new Promise((res) => setTimeout(res, 100));
    if (!(await p.eval<boolean>(`!!document.getElementById("f")`))) {
        throw new Error("the fixture field never appeared: " + (await p.eval<string>(`location.href + " " + document.readyState + " " + String(document.body?.innerHTML.length)`)));
    }
    const r = await p.eval<{ left: number; right: number; top: number; bottom: number }>(
        `(() => { const b = document.getElementById("f").getBoundingClientRect(); return { left: b.left, right: b.right, top: b.top, bottom: b.bottom }; })()`
    );
    const sel = () => p.eval<Sel>(`(() => { const f = document.getElementById("f"); return [f.selectionStart, f.selectionEnd, f.selectionDirection]; })()`);
    await p.eval(`document.getElementById("f").focus()`);
    const line = r.top + 9;
    const mid = (r.left + r.right) / 2;
    let at = { x: r.left + (r.right - r.left) * 0.7, y: line };
    await p.mouse("mouseMoved", at.x, line);
    await p.mouse("mousePressed", at.x, line);
    const to = async (x: number, y: number): Promise<void> => {
        for (let i = 1; i <= 6; i++) {
            await p.mouse("mouseMoved", at.x + ((x - at.x) * i) / 6, at.y + ((y - at.y) * i) / 6);
            await new Promise((res) => setTimeout(res, 20));
        }
        at = { x, y };
        await new Promise((res) => setTimeout(res, 60));
    };
    const steps: Record<string, Sel> = {};
    const left = Math.max(r.left - 15, 1); // still over the pane's slot
    const farLeft = Math.max(r.left - 150, 1); // over whatever is left of the pane
    await to(mid, line);
    steps.inside = await sel();
    await to(left, line);
    steps.left = await sel();
    const slotHit = await p.eval<boolean>(`(() => { const e = document.elementFromPoint(${left}, ${line}); return !!e && document.getElementById("slot").contains(e); })()`);
    await to(r.right + 60, line);
    steps.right = await sel();
    await to(mid, r.top - 15);
    steps.above = await sel();
    await to(mid, r.bottom + 15);
    steps.below = await sel();
    await to(farLeft, r.bottom + 15);
    steps.belowLeft = await sel();
    await to(farLeft, line);
    steps.farLeft = await sel();
    await to(-20, -20);
    steps.outsideWindow = await sel();
    await p.mouse("mouseReleased", at.x, at.y);
    await new Promise((res) => setTimeout(res, 100));
    steps.released = await sel();
    return { steps, slotHit };
}

const SIZE = { width: 1000, height: 600 };
const run = async (c: Case, moved: "transform" | "margin"): Promise<Run> => driveInBrowser(BROWSER!, fixture(c, moved), SIZE, drag);

describe.skipIf(!BROWSER)("a selection drag out of a text field", () => {
    const cases: Case[] = [
        { kind: "textarea", x: 0, zoom: 1 },
        { kind: "textarea", x: 400, zoom: 1 },
        { kind: "textarea", x: 400, zoom: 1.25 },
        { kind: "input", x: 400, zoom: 1 },
        { kind: "input", x: 400, zoom: 1.25 },
    ];
    for (const c of cases) {
        it(`selects what the pointer is over, in a pane moved by a transform: ${c.kind}, pane at ${c.x}, zoom ${c.zoom}`, async (ctx) => {
            const moved = await run(c, "transform");
            const control = await run(c, "margin");
            // Chrome's headless shell does not drag-select in a single-line input from synthetic mouse events (the caret
            // just follows the pointer, with or without a transform): with no oracle there, the case needs a full Chrome.
            if (c.kind === "input" && control.steps.inside[0] === control.steps.inside[1]) ctx.skip();
            expect(moved.steps).toEqual(control.steps);
            // The pane's slot sets `pointer-events: auto` inline: during the drag nothing in the pane is under the pointer.
            expect(moved.slotHit).toBe(false);
        }, 120_000);
    }
});
