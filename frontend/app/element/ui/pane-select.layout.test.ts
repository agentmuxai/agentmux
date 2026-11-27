// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A selection that starts in a pane stays in that pane, in a real browser: the engine's own selection handling
 * under real mouse and key events, which jsdom cannot do. A page of three panes side by side with a tab strip above
 * and a status bar below (the panes' other content carries `user-select: text` of its own, as a transcript or a card
 * does); a press in the first pane's text, held and moved over the others, the strip, the bar and out of the window,
 * for a drag, a double-click drag and a triple-click drag; Shift+click into another pane; Shift+Down; Ctrl+A.
 *
 * Each case runs twice: with the guard (`pane-select.ts` and the stylesheet rules from `pane-select.scss`, the real
 * ones) where the selection must stay in the first pane at every point, and as a control with neither, where the
 * engine carries the selection out of the pane: the cases prove nothing unless the control crosses.
 * Skips with no usable browser.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";
import { findBrowser } from "@/app/view/agent/components/layout-browser";
import { driveInBrowser, type DrivenPage } from "@/app/view/agent/components/layout-drive";

const BROWSER = findBrowser();
// The guard's own source, compiled to a script that leaves its exports on `PS` (its stylesheet import is the rules below).
const compile = (file: string) =>
    ts.transpileModule(readFileSync(join(__dirname, file), "utf8"), { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
// `outside.ts`, which the guard imports, goes in first and answers its `require`.
const GUARD = `var OUT = {}; (function (exports) {${compile("outside.ts")}})(OUT); (function (exports) {${compile("drag-lag.ts")}})(OUT); var PS = {}; (function (exports, require) {${compile("pane-select.ts").replace('require("./pane-select.scss");', "")}})(PS, function () { return OUT; });`;
// The stylesheet's rules, as it is written (SCSS here is plain CSS with line comments).
const RULES = readFileSync(join(__dirname, "pane-select.scss"), "utf8").replace(/^\s*\/\/.*$/gm, "");

const SHIFT = 8;
// Ctrl+A is Cmd+A on macOS.
const SELECT_ALL_MOD = process.platform === "darwin" ? 4 : 2;

const PARAGRAPH = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega ";

function fixture(guarded: boolean): string {
    const text = (id: string) => `<p id="${id}" style="margin:0 0 10px;font:14px sans-serif">${PARAGRAPH.repeat(3)}</p>`;
    return `<!doctype html><meta charset="utf-8"><style>html,body{margin:0;overflow:hidden;font:14px sans-serif}${guarded ? RULES : ""}</style>
<div id="chrome" style="position:absolute;left:0;top:0;width:1000px;height:30px">tab strip: one two three four five</div>
<div class="block block-focused" data-blockid="a" id="a" style="position:absolute;left:0;top:40px;width:300px;height:400px;overflow:hidden">
  <div class="block-content">${text("a1")}${text("a2")}${text("a3")}</div></div>
<div class="block" data-blockid="b" id="b" style="position:absolute;left:320px;top:40px;width:300px;height:400px;overflow:hidden;user-select:text">
  <div class="block-content" style="user-select:text"><p id="b1" class="card" style="user-select:text;margin:0 0 10px">kanban card one: ${PARAGRAPH}</p><p id="b2" style="user-select:text;margin:0">kanban card two: ${PARAGRAPH}</p></div></div>
<div class="block" data-blockid="c" id="c" style="position:absolute;left:640px;top:40px;width:300px;height:400px;overflow:hidden">
  <div class="block-content"><p id="c1" style="margin:0;user-select:text">editor image area caption ${PARAGRAPH}</p></div></div>
<div id="status" style="position:absolute;left:0;top:560px;width:1000px;height:30px">status bar: ready, three agents working</div>
<script>${guarded ? `${GUARD};PS.installPaneSelectGuard(window);` : ""}</script>`;
}

/** The first pane `a` after a pane `s` in the document, `s` to its right on screen, with a field that is `user-select: none`. */
function fixtureBehind(): string {
    const text = (id: string) => `<p id="${id}" style="margin:0 0 10px;font:14px sans-serif">${PARAGRAPH.repeat(3)}</p>`;
    return `<!doctype html><meta charset="utf-8"><style>html,body{margin:0;overflow:hidden;font:14px sans-serif}${RULES}</style>
<div class="block" data-blockid="s" id="s" style="position:absolute;left:320px;top:40px;width:300px;height:400px;overflow:hidden">
  <div class="block-content"><textarea id="sf" placeholder="Broadcast to agents" style="user-select:none;width:280px;height:60px"></textarea><p id="s1" style="margin:0;font:14px sans-serif">swarm text ${PARAGRAPH}</p></div></div>
<div class="block block-focused" data-blockid="a" id="a" style="position:absolute;left:0;top:40px;width:300px;height:400px;overflow:hidden">
  <div class="block-content">${text("a1")}${text("a2")}${text("a3")}</div></div>
<script>${GUARD};PS.installPaneSelectGuard(window);</script>`;
}

interface Probe {
    ranges: number;
    len: number;
    /** Every range starts and ends in the first pane. */
    inA: boolean;
    /** A range starts in one pane (or the strip, or the bar) and ends in another. */
    crossing: boolean;
    locked: boolean;
}

const PROBE = `(() => {
    const where = (n) => { const e = n && (n.nodeType === 1 ? n : n.parentElement); return (e && e.closest("[data-blockid]")?.id) || (e && e.closest("#chrome, #status")?.id) || "none"; };
    const s = getSelection();
    const rs = []; for (let i = 0; i < s.rangeCount; i++) rs.push(s.getRangeAt(i));
    return {
        ranges: rs.length,
        len: String(s).length,
        inA: rs.every((r) => where(r.startContainer) === "a" && where(r.endContainer) === "a"),
        crossing: rs.some((r) => where(r.startContainer) !== where(r.endContainer)),
        locked: document.querySelector("[data-pane-select-off]") !== null,
    };
})()`;

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const probe = (p: DrivenPage) => p.eval<Probe>(PROBE);

/** The middle of the first line of text in `id`. */
async function textPoint(p: DrivenPage, id: string): Promise<{ x: number; y: number }> {
    return p.eval(`(() => { const r = document.createRange(); r.selectNodeContents(document.getElementById("${id}").firstChild); const b = r.getClientRects()[0]; return { x: b.left + 40, y: b.top + b.height / 2 }; })()`);
}

async function centre(p: DrivenPage, id: string): Promise<{ x: number; y: number }> {
    return p.eval(`(() => { const b = document.getElementById("${id}").getBoundingClientRect(); return { x: b.left + b.width / 2, y: b.top + b.height / 2 }; })()`);
}

/** Where the held pointer goes: over each other pane, the strip, the bar, and out of the window. */
async function targets(p: DrivenPage): Promise<Array<[string, { x: number; y: number }]>> {
    return [
        ["over the second pane", await centre(p, "b1")],
        ["over the third pane", await centre(p, "c1")],
        ["over the tab strip", await centre(p, "chrome")],
        ["over the status bar", await centre(p, "status")],
        ["out of the window", { x: -20, y: -20 }],
    ];
}

/** Press in the first pane's text (with `clickCount` presses before the drag), then drag to each target, probing at each. */
async function dragFromA(p: DrivenPage, clickCount: 1 | 2 | 3): Promise<Array<[string, Probe]>> {
    const start = await textPoint(p, "a1");
    await p.mouse("mouseMoved", start.x, start.y);
    for (let n = 1; n <= clickCount; n++) {
        await p.mouse("mousePressed", start.x, start.y, { clickCount: n });
        if (n < clickCount) await p.mouse("mouseReleased", start.x, start.y, { clickCount: n });
    }
    let at = start;
    const seen: Array<[string, Probe]> = [];
    for (const [name, to] of await targets(p)) {
        for (let i = 1; i <= 6; i++) {
            await p.mouse("mouseMoved", at.x + ((to.x - at.x) * i) / 6, at.y + ((to.y - at.y) * i) / 6, { clickCount });
            await wait(15);
        }
        at = to;
        await wait(60);
        seen.push([name, await probe(p)]);
    }
    await p.mouse("mouseReleased", at.x, at.y, { clickCount });
    await wait(80);
    seen.push(["after the release", await probe(p)]);
    return seen;
}

const SIZE = { width: 1000, height: 600 };
/** The fixture page, once its text is there (the page can still be loading when the browser first answers). */
const drive = <T>(guarded: boolean, run: (p: DrivenPage) => Promise<T>): Promise<T> =>
    driveInBrowser(BROWSER!, fixture(guarded), SIZE, async (p) => {
        for (let i = 0; i < 50 && !(await p.eval<boolean>(`!!document.getElementById("c1")`)); i++) await wait(100);
        return run(p);
    });

describe.skipIf(!BROWSER)("a selection that starts in a pane", () => {
    for (const clickCount of [1, 2, 3] as const) {
        const what = ["a drag", "a double-click drag", "a triple-click drag"][clickCount - 1];

        it(`stays in the pane through ${what}, over the other panes, the strip, the bar and out of the window`, async () => {
            const seen = await drive(true, (p) => dragFromA(p, clickCount));
            for (const [name, s] of seen) expect(s.inA, `${what}, ${name}: ${JSON.stringify(s)}`).toBe(true);
            // The drag took something: the selection is not empty once the pointer has left the first pane's text.
            expect(seen.at(-1)![1].len).toBeGreaterThan(0);
            // The lock is on while the button is held, and off after the release.
            expect(seen[0][1].locked).toBe(true);
            expect(seen.at(-1)![1].locked).toBe(false);
        }, 60_000);

        it(`is carried out of the pane by ${what} without the guard (the control)`, async () => {
            const seen = await drive(false, (p) => dragFromA(p, clickCount));
            expect(seen.some(([, s]) => !s.inA || s.crossing), `${what} never left the pane: ${JSON.stringify(seen)}`).toBe(true);
        }, 60_000);
    }

    /**
     * The pane the drag began in comes AFTER another pane in the document (the Swarm and CPU panes sit before an agent
     * pane in DOM order, to its right on screen), and that pane holds a field and a handle that are `user-select: none`.
     * Held over such a thing the engine snaps the end of the selection to the nearest position it can select, which
     * can be before the pane in document order: the selection leaps backwards, over the other pane.
     */
    const behind = (p: DrivenPage) =>
        p.eval<{ sf: { x: number; y: number }; s1: { x: number; y: number } }>(
            `(() => { const c = (id) => { const b = document.getElementById(id).getBoundingClientRect(); return { x: b.left + b.width / 2, y: b.top + b.height / 2 }; }; return { sf: c("sf"), s1: c("s1") }; })()`,
        );
    const dragOverFieldBefore = async (p: DrivenPage): Promise<Array<[string, Probe]>> => {
        const start = await textPoint(p, "a1");
        const over = await behind(p);
        const edge = { x: 296, y: over.sf.y };
        await p.mouse("mouseMoved", start.x, start.y);
        await p.mouse("mousePressed", start.x, start.y, { clickCount: 1 });
        const seen: Array<[string, Probe]> = [];
        let at = start;
        for (const [name, to] of [["at the pane's right edge", edge], ["over a field before the pane", over.sf], ["over text before the pane", over.s1]] as const) {
            for (let i = 1; i <= 8; i++) {
                await p.mouse("mouseMoved", at.x + ((to.x - at.x) * i) / 8, at.y + ((to.y - at.y) * i) / 8, { clickCount: 1 });
                await wait(15);
            }
            at = to;
            await wait(80);
            seen.push([name, await probe(p)]);
        }
        await p.mouse("mouseReleased", at.x, at.y, { clickCount: 1 });
        return seen;
    };

    it("holds the selection where it was in the pane while the pointer is over a field or text of a pane that comes before it in the document", async () => {
        const seen = await driveInBrowser(BROWSER!, fixtureBehind(), SIZE, async (p) => {
            for (let i = 0; i < 50 && !(await p.eval<boolean>(`!!document.getElementById("s1")`)); i++) await wait(100);
            return dragOverFieldBefore(p);
        });
        // Where the pointer last was over the pane is where the selection stays.
        const held = seen[0][1].len;
        expect(held).toBeGreaterThan(0);
        for (const [name, s] of seen) {
            expect(s.inA, `${name}: ${JSON.stringify(s)}`).toBe(true);
            expect(s.len, `${name}: ${JSON.stringify(s)}`).toBe(held);
        }
    }, 60_000);

    /** Click in the first pane's text, then Shift+click in the second pane's. */
    const shiftClick = async (p: DrivenPage): Promise<Probe> => {
        const a = await textPoint(p, "a1");
        const b = await centre(p, "b1");
        await p.mouse("mouseMoved", a.x, a.y);
        await p.mouse("mousePressed", a.x, a.y);
        await p.mouse("mouseReleased", a.x, a.y);
        await p.mouse("mouseMoved", b.x, b.y, { modifiers: SHIFT });
        await p.mouse("mousePressed", b.x, b.y, { modifiers: SHIFT });
        await p.mouse("mouseReleased", b.x, b.y, { modifiers: SHIFT });
        await wait(80);
        return probe(p);
    };

    it("is not extended into another pane by Shift+click", async () => {
        const s = await drive(true, shiftClick);
        expect(s.crossing, JSON.stringify(s)).toBe(false);
    }, 60_000);

    it("is extended into another pane by Shift+click without the guard (the control)", async () => {
        const s = await drive(false, shiftClick);
        expect(s.crossing, JSON.stringify(s)).toBe(true);
    }, 60_000);

    /** Select a word in the first pane's text (Shift+arrows extend a selection that is not just a caret), then Shift+Down far past its last line. */
    const shiftDown = async (p: DrivenPage): Promise<Probe> => {
        const a = await textPoint(p, "a1");
        await p.mouse("mouseMoved", a.x, a.y);
        await p.mouse("mousePressed", a.x, a.y);
        await p.mouse("mouseReleased", a.x, a.y);
        await p.mouse("mousePressed", a.x, a.y, { clickCount: 2 });
        await p.mouse("mouseReleased", a.x, a.y, { clickCount: 2 });
        for (let i = 0; i < 60; i++) await p.key("ArrowDown", "ArrowDown", 40, SHIFT, ["moveDownAndModifySelection"]);
        await wait(120);
        return probe(p);
    };

    it("stops at the pane's content edge for Shift+Down", async () => {
        const s = await drive(true, shiftDown);
        expect(s.len, JSON.stringify(s)).toBeGreaterThan(0);
        expect(s.inA, JSON.stringify(s)).toBe(true);
    }, 60_000);

    it("runs on into the bar below for Shift+Down without the guard (the control)", async () => {
        const s = await drive(false, shiftDown);
        expect(s.inA, JSON.stringify(s)).toBe(false);
    }, 60_000);

    /** Click in the first pane's text, then Select all. */
    const selectAll = async (p: DrivenPage): Promise<Probe> => {
        const a = await textPoint(p, "a1");
        await p.mouse("mouseMoved", a.x, a.y);
        await p.mouse("mousePressed", a.x, a.y);
        await p.mouse("mouseReleased", a.x, a.y);
        await p.key("a", "KeyA", 65, SELECT_ALL_MOD, ["selectAll"]);
        await wait(120);
        return probe(p);
    };

    it("selects the pane's own content for Ctrl+A / Cmd+A", async () => {
        const s = await drive(true, selectAll);
        expect(s.ranges).toBe(1);
        expect(s.len).toBeGreaterThan(0);
        expect(s.inA, JSON.stringify(s)).toBe(true);
    }, 60_000);

    it("selects the whole window for Ctrl+A / Cmd+A without the guard (the control)", async () => {
        const s = await drive(false, selectAll);
        expect(s.inA, JSON.stringify(s)).toBe(false);
    }, 60_000);
});
