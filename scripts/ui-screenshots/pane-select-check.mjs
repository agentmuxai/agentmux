// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Real-engine check that a selection which starts in a pane stays in that pane (frontend/app/element/ui/pane-select.ts).
// Drives a task dev build over the debugging port with real mouse and key events: no scripted DOM selection. Fails (exit 1)
// on a build where the selection crosses out of its pane, which is how it fails on a build without the guard.
//
//   node scripts/ui-screenshots/pane-select-check.mjs <home dir | --port N> [--pane <blockid>]
//
// Needs at least two visible panes, one of them with selectable text (an agent transcript). It reads the page and sends
// input events; it changes nothing but the page's selection and focus. Run it only against your own isolated task dev.
import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { CdpSession, findMainTarget } from "./lib/session.mjs";

const args = process.argv.slice(2);
const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args[i + 1] : undefined;
};
let port = flag("--port");
if (!port) {
    const home = args.find((a) => !a.startsWith("--") && a !== flag("--pane"));
    if (!home) throw new Error("usage: pane-select-check.mjs <home dir | --port N> [--pane <blockid>]");
    const authPath = execSync(`bash -c 'ls ${home}/dev/*/*/data/authkey.dev'`).toString().trim().split("\n")[0];
    port = JSON.parse(readFileSync(authPath, "utf8")).debug_port;
}
const s = await CdpSession.connect((await findMainTarget(port)).webSocketDebuggerUrl);

/** What the page reports about the selection, relative to the pane `paneId`. */
const PROBE = (paneId) => `(() => {
    const P = document.querySelector('[data-blockid="${paneId}"]');
    const sel = getSelection();
    const ranges = [];
    for (let i = 0; i < sel.rangeCount; i++) ranges.push(sel.getRangeAt(i));
    const inP = (n) => !!n && P.contains(n.nodeType === 1 ? n : n.parentElement);
    const paneOfNode = (n) => (n && (n.nodeType === 1 ? n : n.parentElement)?.closest("[data-blockid]")?.getAttribute("data-blockid")) || null;
    return {
        ranges: ranges.length,
        len: String(sel).length,
        startIn: ranges.every((r) => inP(r.startContainer)),
        endIn: ranges.every((r) => inP(r.endContainer)),
        startPane: ranges[0] ? paneOfNode(ranges[0].startContainer) : null,
        endPane: ranges[0] ? paneOfNode(ranges[0].endContainer) : null,
        lock: document.documentElement.hasAttribute("data-pane-select"),
        owners: document.querySelectorAll("[data-pane-select-owner]").length,
    };
})()`;

const panes = await s.evaluate(`(() => [...document.querySelectorAll("[data-blockid]")].map((p) => {
    const r = p.getBoundingClientRect();
    return { id: p.getAttribute("data-blockid"), x: r.x, y: r.y, w: r.width, h: r.height, view: p.querySelector("[data-view]")?.getAttribute("data-view") ?? "" };
}).filter((p) => p.w > 40 && p.h > 40 && !document.querySelector('[data-blockid="' + p.id + '"]').classList.contains("block-preview")))()`);
if (panes.length < 2) throw new Error(`need at least two visible panes, saw ${panes.length}`);

/** A point on a run of visible text inside pane `paneId`, away from fields and buttons. */
const textPoint = (paneId) => s.evaluate(`(() => {
    const P = document.querySelector('[data-blockid="${paneId}"]');
    const pr = P.getBoundingClientRect();
    const walker = document.createTreeWalker(P.querySelector(".block-content") ?? P, NodeFilter.SHOW_TEXT);
    let best = null;
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
        const el = n.parentElement;
        if (!el || el.closest("input, textarea, button, select, [contenteditable], .xterm, .monaco-editor, [aria-hidden=true]")) continue;
        if (n.data.trim().length < 12) continue;
        const range = document.createRange();
        range.selectNodeContents(n);
        const r = range.getBoundingClientRect();
        if (r.width < 40 || r.height < 6 || r.top < pr.top + 40 || r.bottom > pr.bottom - 40 || r.left < pr.left + 4 || r.right > pr.right - 4) continue;
        const hit = document.elementFromPoint(r.left + 8, r.top + r.height / 2);
        if (!hit || !n.parentElement.contains(hit) && hit !== n.parentElement) continue;
        if (!best || r.top < best.y) best = { x: r.left + 8, y: r.top + r.height / 2, text: n.data.trim().slice(0, 40) };
    }
    return best;
})()`);

let P;
const want = flag("--pane");
for (const p of want ? panes.filter((q) => q.id === want) : panes) {
    const pt = await textPoint(p.id);
    if (pt) {
        P = { ...p, pt };
        break;
    }
}
if (!P) throw new Error("no pane with a run of selectable text; open an agent transcript or an Editor with text");
const others = panes.filter((p) => p.id !== P.id);
const vp = await s.evaluate(`({ w: innerWidth, h: innerHeight })`);
console.log("pane", P.id, P.view, "press at", Math.round(P.pt.x), Math.round(P.pt.y), `"${P.pt.text}"`, "others", others.map((o) => o.id).join(","));

const mouse = (type, x, y, buttons, clickCount = 1, modifiers = 0) =>
    s.send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons, clickCount, modifiers });
const key = async (k, code, vk, modifiers, commands) => {
    const ev = { key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers, ...(commands ? { commands } : {}) };
    await s.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...ev });
    await s.send("Input.dispatchKeyEvent", { type: "keyUp", ...ev });
};
const SHIFT = 8;
const CTRL = 2;

const failures = [];
const check = (name, cond, detail) => {
    console.log(`${cond ? "PASS" : "FAIL"}  ${name}${cond ? "" : "  " + JSON.stringify(detail)}`);
    if (!cond) failures.push(name);
};
const clearSelection = async () => {
    await s.evaluate(`getSelection().removeAllRanges()`);
    await mouse("mouseMoved", P.pt.x, P.pt.y, 0);
};

// Where the held pointer goes: the centre of each other pane, the window's chrome (top, bottom), and its edges.
const targets = [
    ...others.map((o) => ({ name: `over pane ${o.id} (${o.view})`, x: o.x + o.w / 2, y: o.y + o.h / 2 })),
    { name: "over the top of the window", x: P.pt.x, y: 4 },
    { name: "over the bottom of the window (status bar)", x: P.pt.x, y: vp.h - 3 },
    { name: "out of the window, left", x: 1, y: P.pt.y },
    { name: "out of the window, right", x: vp.w - 1, y: P.pt.y },
];

for (const clickCount of [1, 2, 3]) {
    const label = ["a drag", "a double-click drag", "a triple-click drag"][clickCount - 1];
    await clearSelection();
    await mouse("mouseMoved", P.pt.x, P.pt.y, 0);
    await mouse("mousePressed", P.pt.x, P.pt.y, 1, clickCount);
    for (const t of targets) {
        // In steps, so the engine sees a drag and not a jump.
        const steps = 6;
        for (let i = 1; i <= steps; i++) {
            await mouse("mouseMoved", P.pt.x + ((t.x - P.pt.x) * i) / steps, P.pt.y + ((t.y - P.pt.y) * i) / steps, 1, clickCount);
        }
        await s.wait(60);
        const r = await s.evaluate(PROBE(P.id));
        check(`${label}, held ${t.name}: the selection is inside the pane`, r.ranges === 0 || (r.startIn && r.endIn), r);
    }
    const held = await s.evaluate(PROBE(P.id));
    check(`${label}: the root is locked and the pane owns the press while held`, held.lock && held.owners === 1, held);
    await mouse("mouseReleased", P.pt.x, P.pt.y, 0, clickCount);
    await s.wait(80);
    const after = await s.evaluate(PROBE(P.id));
    check(`${label}: the lock is gone after the release, and the selection stays`, !after.lock && after.owners === 0 && after.len > 0, after);
}

// Shift+click into another pane, from a selection that began in P.
await clearSelection();
await mouse("mousePressed", P.pt.x, P.pt.y, 1);
await mouse("mouseReleased", P.pt.x, P.pt.y, 0);
await mouse("mouseMoved", P.pt.x + 120, P.pt.y, 0);
await mouse("mousePressed", P.pt.x + 120, P.pt.y, 1, 1, SHIFT);
await mouse("mouseReleased", P.pt.x + 120, P.pt.y, 0, 1, SHIFT);
for (const o of others) {
    const c = { x: o.x + o.w / 2, y: o.y + o.h / 2 };
    await mouse("mouseMoved", c.x, c.y, 0, 1, SHIFT);
    await mouse("mousePressed", c.x, c.y, 1, 1, SHIFT);
    await mouse("mouseReleased", c.x, c.y, 0, 1, SHIFT);
    await s.wait(60);
    const r = await s.evaluate(PROBE(P.id));
    check(`Shift+click in pane ${o.id} (${o.view}) does not extend a selection from ${P.id} into it`, !(r.ranges > 0 && r.startPane === P.id && r.endPane === o.id), r);
    // Put a selection back in P for the next pane.
    await mouse("mousePressed", P.pt.x, P.pt.y, 1);
    await mouse("mouseReleased", P.pt.x, P.pt.y, 0);
    await mouse("mousePressed", P.pt.x + 120, P.pt.y, 1, 1, SHIFT);
    await mouse("mouseReleased", P.pt.x + 120, P.pt.y, 0, 1, SHIFT);
}

// Shift+arrows from a click in the pane's text, far past the pane's own content.
await clearSelection();
await mouse("mousePressed", P.pt.x, P.pt.y, 1);
await mouse("mouseReleased", P.pt.x, P.pt.y, 0);
for (let i = 0; i < 400; i++) await key("ArrowDown", "ArrowDown", 40, SHIFT, ["moveDownAndModifySelection"]);
await s.wait(100);
const down = await s.evaluate(PROBE(P.id));
check("Shift+Down, 400 times: the selection stays inside the pane", down.ranges === 0 || (down.startIn && down.endIn), down);
await mouse("mousePressed", P.pt.x, P.pt.y, 1);
await mouse("mouseReleased", P.pt.x, P.pt.y, 0);
for (let i = 0; i < 400; i++) await key("ArrowUp", "ArrowUp", 38, SHIFT, ["moveUpAndModifySelection"]);
await s.wait(100);
const up = await s.evaluate(PROBE(P.id));
check("Shift+Up, 400 times: the selection stays inside the pane", up.ranges === 0 || (up.startIn && up.endIn), up);

// Ctrl+A with the caret in the pane's text.
await clearSelection();
await mouse("mousePressed", P.pt.x, P.pt.y, 1);
await mouse("mouseReleased", P.pt.x, P.pt.y, 0);
// With the focus in a text field (a Settings pane's search line, a composer), Select all is the field's own: it selects
// the field's text and leaves the page's selection alone, so there is nothing to check about the pane's content.
const fieldFocused = await s.evaluate(`(() => { const a = document.activeElement; return !!a && (/^(INPUT|TEXTAREA|SELECT)$/.test(a.tagName) || a.isContentEditable); })()`);
await key("a", "KeyA", 65, CTRL, ["selectAll"]);
await s.wait(100);
const all = await s.evaluate(PROBE(P.id));
if (fieldFocused) console.log(`SKIP  Ctrl+A selects the pane's own content: the focus is in a text field, whose own Select all runs  ${JSON.stringify(all)}`);
else check("Ctrl+A selects the pane's own content, not the window's", all.ranges > 0 && all.len > 0 && all.startIn && all.endIn, all);

s.close();
console.log(failures.length ? `\n${failures.length} FAILED` : "\nall passed");
process.exit(failures.length ? 1 : 0);
