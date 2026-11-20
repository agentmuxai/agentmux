#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// UI-level layout stress: random minimize, restore, maximize, split, close, header drag, resize and reload steps
// in a running dev build, and after each step a check that what the window draws agrees with the layout srv
// stored. The unit fuzz (frontend/layout/tests/layoutStress.test.ts) covers the layout functions; this covers
// the window on top of them. Dev tool, never shipped.
//
// Usage:
//   node scripts/layout-stress.mjs [--port N] [--db PATH] [--seed N] [--steps N] [--shots DIR] [--no-drag] [--no-reload] [--drag-weight N] [--tab]
//
// Run it against your own throwaway `task dev` instance (the DevTools port is AGENTMUX_CDP_PORT, 9223 by default
// in dev), never an instance someone works in: it clicks, drags and reloads the window it finds. The input is
// synthetic DevTools input, so it takes no real mouse or keyboard. --db is that instance's objects.db (its data
// dir's `db/objects.db`); without it the newest one under `.dev-home/` in the repo is used. The run works in the
// window's active tab and closes only panes it opened itself.
//
// A drag hovers the target pane first, reads the drop guides that appear over it, and drops on one of them: the left,
// right, top or bottom spot or the centre of another pane, the wide spots at the layout's top or bottom, its own pane, a
// point on no spot, a minimized pane's slot, a minimized pane picked up, and (with --tab, which opens a second window tab)
// another tab. The run ends with a table of how many drags of each kind were picked up, had a drop on offer, dropped and
// changed the layout. --drag-weight N makes drags N times as likely as the other steps.
//
// After every step it checks, polling srv's stored layout for a few seconds to let it catch up:
//   - the panes drawn are the stored layout's leaves (same ids), none twice;
//   - a pane is drawn minimized exactly when it is stored as minimized;
//   - a maximized pane is drawn exactly when one is stored, and it is the stored one;
//   - at least one pane is expanded;
//   - the stored focused pane is a leaf;
//   - expanded panes have a size, do not overlap, and stay inside the layout area.
// A failing run prints its seed and steps, so it replays: node scripts/layout-stress.mjs --seed <seed>.
//
// Exit code: 0 all steps agree, 1 a disagreement, 2 setup error.

import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";
import { disagreements, leavesOf } from "./layout-stress-check.mjs";
import { close, connect, js, S, sleep } from "./verify-shortcuts-cdp.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const args = (() => {
    const a = { port: Number(process.env.AGENTMUX_CDP_PORT) || 9223, db: null, seed: 1, steps: 80, shots: null, drag: true, reload: true, dragWeight: 1, tab: false };
    const argv = process.argv.slice(2);
    for (let i = 0; i < argv.length; i++) {
        const k = argv[i];
        if (k === "--port") a.port = Number(argv[++i]);
        else if (k === "--db") a.db = argv[++i];
        else if (k === "--seed") a.seed = Number(argv[++i]);
        else if (k === "--steps") a.steps = Number(argv[++i]);
        else if (k === "--shots") a.shots = argv[++i];
        else if (k === "--no-drag") a.drag = false;
        else if (k === "--no-reload") a.reload = false;
        else if (k === "--drag-weight") a.dragWeight = Math.max(1, Number(argv[++i]));
        else if (k === "--tab") a.tab = true;
        else {
            console.error(`unknown argument: ${k}`);
            process.exit(2);
        }
    }
    return a;
})();

function mulberry32(seed) {
    return () => {
        seed = (seed + 0x6d2b79f5) | 0;
        let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
        t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

/** The newest objects.db under the repo's .dev-home, when --db isn't given. */
function newestDb() {
    const root = path.join(REPO, ".dev-home");
    let best = null;
    const walk = (dir, depth) => {
        if (depth > 7) return;
        for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
            const p = path.join(dir, e.name);
            if (e.isDirectory()) walk(p, depth + 1);
            else if (e.name === "objects.db") {
                const m = fs.statSync(p).mtimeMs;
                if (!best || m > best.m) best = { p, m };
            }
        }
    };
    if (fs.existsSync(root)) walk(root, 0);
    return best?.p ?? null;
}

// ---------------------------------------------------------------------------------------------------------------
// What is drawn, and what srv stored.

/** Runs in the page: the active tab's panes (a tab's tiles stay mounted, hidden, when it is not shown). */
const DOM_STATE = `(() => {
    const tiles = [...document.querySelectorAll(".tile-node")].filter((t) => getComputedStyle(t).visibility !== "hidden").map((t) => {
        const r = t.getBoundingClientRect();
        return { id: t.id, minimized: t.classList.contains("tile-minimized"), hiddenByMaximize: t.classList.contains("tile-hidden"), blockId: t.querySelector("[data-blockid]")?.getAttribute("data-blockid") ?? null, rect: [r.x, r.y, r.width, r.height] };
    });
    const mag = document.querySelector(".magnify-pane [data-blockid], .magnify-container [data-blockid]");
    const area = document.querySelector(".tile-layout")?.getBoundingClientRect();
    return JSON.stringify({ tiles, maximizedBlockId: mag ? mag.getAttribute("data-blockid") : null, area: area ? [area.x, area.y, area.width, area.height] : null });
})()`;

/** Every stored layout, parsed: `[{ oid, leaves, magnifiednodeid, focusednodeid }]`. */
function readLayouts(dbPath) {
    const db = new DatabaseSync(dbPath, { readOnly: true });
    try {
        return db
            .prepare("select oid, data from db_layout")
            .all()
            .map((r) => {
                const j = JSON.parse(Buffer.from(r.data).toString());
                return { oid: r.oid, leaves: leavesOf(j.rootnode), magnifiednodeid: j.magnifiednodeid ?? "", focusednodeid: j.focusednodeid ?? "" };
            });
    } finally {
        db.close();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Input.

async function click(cdp, x, y, modifiers = 0) {
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1, modifiers });
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1, modifiers });
}

/** The centre of the first visible element matching `selector` (optionally inside `within`) with this title. */
async function centreOf(cdp, selector, within, title) {
    return cdp.evaluate(`(() => {
        const root = ${within ? `document.querySelector(${js(within)})` : "document"};
        const el = [...(root?.querySelectorAll(${js(selector)}) ?? [])].find((e) => {
            const r = e.getBoundingClientRect();
            return r.width > 0 && r.height > 0 && (!${js(title ?? null)} || e.getAttribute("title") === ${js(title ?? null)} || e.getAttribute("data-hover-tip") === ${js(title ?? null)});
        });
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    })()`);
}

async function clickIn(cdp, within, selector, title) {
    const pt = await centreOf(cdp, selector, within, title);
    if (!pt) return false;
    await click(cdp, pt.x, pt.y);
    return true;
}

/** Where the drop guides (the compass of spots over the hovered pane) are on screen: each spot's centre. */
const GUIDE_SPOTS = `JSON.stringify([...document.querySelectorAll(".drop-guide")].map((e) => { const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2, w: r.width, h: r.height, active: e.classList.contains("active") }; }).filter((s) => s.w > 0 && s.h > 0))`;

/** A header drag with the drag intercepted, as pragmatic-dnd needs: press, move, dragover the target, drop.
 *  `aim.hover` is where the pointer goes first (the guides show once it is over a pane); `aim.pick(spots)` then
 *  returns the point to drop at, from the guides now drawn (none: no such drop is offered, the drag is cancelled).
 *  Returns `{ started, spots, dropped }`. */
async function dragPane(cdp, from, aim) {
    await cdp.send("Input.setInterceptDrags", { enabled: true });
    let data = null;
    const off = cdp.on("Input.dragIntercepted", (p) => (data = p.data));
    const mouse = (type, p, extra = {}) => cdp.send("Input.dispatchMouseEvent", { type, x: p.x, y: p.y, button: "left", clickCount: 1, ...extra });
    const dragTo = (type, p) => cdp.send("Input.dispatchDragEvent", { type, x: p.x, y: p.y, data });
    await mouse("mouseMoved", from, { button: "none", clickCount: 0 });
    await mouse("mousePressed", from);
    for (let i = 1; i <= 4; i++) await mouse("mouseMoved", { x: from.x + i * 6, y: from.y + i * 3 }, { buttons: 1 });
    await sleep(150);
    const result = { started: Boolean(data), spots: 0, dropped: false };
    try {
        if (!data) return result;
        await dragTo("dragEnter", aim.hover);
        await dragTo("dragOver", aim.hover);
        await sleep(250);
        const spots = aim.pick ? JSON.parse(await cdp.evaluate(GUIDE_SPOTS)) : [];
        result.spots = spots.length;
        const to = aim.pick ? aim.pick(spots) : aim.hover;
        // No such drop on offer: put the pane back where it was. A drag released over nothing tears the pane off
        // into a window of its own, which is another test than this one.
        if (!to) {
            const home = aim.home ?? aim.hover;
            await dragTo("dragOver", home);
            await sleep(150);
            await dragTo("dragOver", home);
            await dragTo("drop", home);
            return result;
        }
        if (to !== aim.hover) {
            await dragTo("dragOver", to);
            await sleep(150);
        }
        for (const type of ["dragOver", "drop"]) {
            await dragTo(type, to);
            await sleep(80);
        }
        result.dropped = true;
        return result;
    } finally {
        await mouse("mouseReleased", from).catch(() => {});
        await cdp.send("Input.setInterceptDrags", { enabled: false });
        off();
    }
}

/** The app windows open on the DevTools port: every page but a pre-warmed one waiting in the pool. */
async function openWindows(port) {
    const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    return targets.filter((t) => t.type === "page" && !/[?&](pool|pane-pool)=1/.test(t.url));
}

const centre = (r) => ({ x: r[0] + r[2] / 2, y: r[1] + r[3] / 2 });
/** The guide spot nearest `want` among `spots` (each `{x, y}`); the compass is small, so nearest is the one meant. */
const nearest = (spots, want) => spots.reduce((best, s) => (!best || Math.hypot(s.x - want.x, s.y - want.y) < Math.hypot(best.x - want.x, best.y - want.y) ? s : best), null);

/** The kinds of drop target a drag is aimed at. `edge-*` are the compass spots on the left, right, top and bottom of
 *  another pane, `centre` its middle spot (the swap), `outer-*` the wide spots at the layout's top or bottom, `own` the
 *  dragged pane itself (a release there offers the tear-off), `off-spot` a point on another pane that is on no spot, `minimized` a minimized pane's slot,
 *  `chip` a minimized pane picked up and dropped on a pane's spot, `tab-strip` another pane's tab strip (the
 *  dragged pane joins it as a tab), `tab` another window tab. */
const KINDS = ["edge-left", "edge-right", "edge-top", "edge-bottom", "centre", "outer-top", "outer-bottom", "own", "off-spot", "minimized", "chip", "tab-strip", "tab"];

/** The drag one step makes: `{ kind, from, aim, note }`, or null when this layout has nothing of that kind. */
async function planDrag(cdp, dom, kind, pick, area) {
    const expanded = dom.tiles.filter((t) => !t.minimized);
    const chips = dom.tiles.filter((t) => t.minimized);
    const headerOf = async (t) => {
        const h = await centreOf(cdp, ".block-frame-default-header", `.tile-node[id="${t.id}"]`);
        return h ? { x: h.x - 40 > 0 ? h.x - 40 : h.x, y: h.y } : null;
    };
    const others = (a) => expanded.filter((x) => x.id !== a.id);
    const side = { "edge-left": [-1, 0], "edge-right": [1, 0], "edge-top": [0, -1], "edge-bottom": [0, 1], centre: [0, 0] };
    if (kind === "chip") {
        if (!chips.length || !expanded.length) return null;
        const c = pick(chips);
        const b = pick(expanded);
        const want = pick(Object.values(side));
        const r = b.rect;
        return { kind, from: centre(c.rect), aim: { home: centre(c.rect), hover: centre(r), pick: (spots) => nearest(spots, { x: centre(r).x + want[0] * r[2] / 2, y: centre(r).y + want[1] * r[3] / 2 }) }, note: `chip ${c.id.slice(0, 8)} onto ${b.id.slice(0, 8)} side ${want.join(",")}` };
    }
    const a = expanded.length ? pick(expanded) : null;
    if (!a) return null;
    const from = await headerOf(a);
    if (!from) return null;
    if (kind === "own") return { kind, from, aim: { hover: centre(a.rect) }, note: `drag ${a.id.slice(0, 8)} onto itself` };
    if (kind === "minimized") {
        if (!chips.length) return null;
        const c = pick(chips);
        return { kind, from, aim: { home: centre(a.rect), hover: centre(c.rect), pick: (spots) => (spots.length ? pick(spots) : null) }, note: `drag ${a.id.slice(0, 8)} onto minimized ${c.id.slice(0, 8)}` };
    }
    if (kind === "tab") {
        const tab = await cdp.evaluate(`(() => { const t = [...document.querySelectorAll(".tab")].find((e) => !e.className.includes("active") && e.getBoundingClientRect().width > 0); if (!t) return null; const r = t.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
        if (!tab || expanded.length + chips.length < 2) return null;
        return { kind, from, aim: { home: centre(a.rect), hover: tab }, note: `drag ${a.id.slice(0, 8)} onto another window tab` };
    }
    if (kind === "outer-top" || kind === "outer-bottom") {
        if (!area) return null;
        const up = kind === "outer-top";
        const hover = { x: area[0] + area[2] / 2, y: up ? area[1] + 24 : area[1] + area[3] - 24 };
        return { kind, from, aim: { home: centre(a.rect), hover, pick: (spots) => (spots.length ? pick(spots) : null) }, note: `drag ${a.id.slice(0, 8)} to the layout's ${up ? "top" : "bottom"} spots` };
    }
    const b = others(a).length ? pick(others(a)) : null;
    if (!b) return null;
    const c = centre(b.rect);
    if (kind === "tab-strip") {
        const strip = await centreOf(cdp, ".pane-tab-strip", `.tile-node[id="${b.id}"]`);
        if (!strip) return null;
        return { kind, from, aim: { home: centre(a.rect), hover: strip }, note: `drag ${a.id.slice(0, 8)} onto the tab strip of ${b.id.slice(0, 8)}` };
    }
    if (kind === "off-spot") {
        // A point in the pane's corner: no spot stands there.
        const hover = { x: b.rect[0] + b.rect[2] * 0.06, y: b.rect[1] + b.rect[3] * 0.06 };
        return { kind, from, aim: { home: centre(a.rect), hover, pick: (spots) => (spots.some((s) => Math.abs(s.x - hover.x) < s.w && Math.abs(s.y - hover.y) < s.h) ? null : hover) }, note: `drag ${a.id.slice(0, 8)} to the corner of ${b.id.slice(0, 8)}` };
    }
    const [sx, sy] = side[kind];
    const want = { x: c.x + (sx * b.rect[2]) / 2, y: c.y + (sy * b.rect[3]) / 2 };
    return { kind, from, aim: { home: centre(a.rect), hover: c, pick: (spots) => nearest(spots, want) }, note: `drag ${a.id.slice(0, 8)} onto ${b.id.slice(0, 8)} ${kind}` };
}

/** Drags a resize handle by (dx, dy) in small moves. False when there is no handle. */
async function resizeBy(cdp, rand) {
    const handles = await cdp.evaluate(`JSON.stringify([...document.querySelectorAll(".resize-handle")].map((e) => { const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2, row: e.classList.contains("flex-row"), w: r.width, h: r.height }; }).filter((h) => h.w > 0 && h.h > 0))`);
    const list = JSON.parse(handles);
    if (!list.length) return null;
    const h = list[Math.floor(rand() * list.length)];
    const delta = Math.round((rand() - 0.5) * 240);
    const to = h.row ? { x: h.x + delta, y: h.y } : { x: h.x, y: h.y + delta };
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: h.x, y: h.y });
    await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: h.x, y: h.y, button: "left", clickCount: 1 });
    for (let i = 1; i <= 6; i++) await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: h.x + ((to.x - h.x) * i) / 6, y: h.y + ((to.y - h.y) * i) / 6, button: "left", buttons: 1 });
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: to.x, y: to.y, button: "left", clickCount: 1 });
    return `${h.row ? "dx" : "dy"} ${delta}`;
}

// ---------------------------------------------------------------------------------------------------------------

async function main() {
    const dbPath = args.db ?? newestDb();
    if (!dbPath || !fs.existsSync(dbPath)) {
        console.error("no objects.db: pass --db (the dev instance's data dir, db/objects.db)");
        process.exit(2);
    }
    let cdp;
    try {
        cdp = await connect(args.port);
    } catch (e) {
        console.error(e.message);
        process.exit(2);
    }
    // The run drives the window it connected to and has to be sure which that is: one window, no strays (a drag
    // that tears a pane off leaves another one behind, and a run that picked it up would test the wrong tab).
    const known = new Set((await openWindows(args.port)).map((w) => w.id));
    if (known.size !== 1) {
        console.error(`${known.size} app windows are open on port ${args.port}: close all but one, then run again`);
        await close(cdp);
        process.exit(2);
    }
    await cdp.send("Page.bringToFront").catch(() => {});
    const rand = mulberry32(args.seed);
    const pick = (xs) => xs[Math.floor(rand() * xs.length)];
    const domNow = async () => JSON.parse(await cdp.evaluate(DOM_STATE));
    const log = [];
    const mine = new Set(); // block ids of panes this run split off: the only ones it closes
    let oid = null;

    /** The stored layout of the tab drawn: the one whose leaves are the panes shown. */
    const layoutOf = (dom) => {
        const layouts = readLayouts(dbPath);
        if (oid) return layouts.find((l) => l.oid === oid);
        const ids = new Set(dom.tiles.map((t) => t.id));
        const found = layouts.find((l) => l.leaves.length === ids.size && l.leaves.every((x) => ids.has(x.id)));
        if (found) oid = found.oid;
        return found;
    };

    /** Waits for the window and srv's stored layout to agree; the disagreements if they still don't after ~5 s. */
    const settleAgree = async () => {
        let last = ["no stored layout matches the panes drawn"];
        for (let waited = 0; waited < 5000; waited += 400) {
            await sleep(400);
            const dom = await domNow();
            const layout = layoutOf(dom);
            if (layout) {
                last = disagreements(dom, layout);
                if (!last.length) return [];
            }
        }
        return last;
    };

    const fail = async (why, problems) => {
        console.error(`\nFAIL after step ${log.length} (seed ${args.seed}): ${why}`);
        for (const p of problems) console.error(`  - ${p}`);
        console.error("steps:");
        log.forEach((s, i) => console.error(`  ${i + 1}. ${s}`));
        if (args.shots) {
            fs.mkdirSync(args.shots, { recursive: true });
            const shot = await cdp.send("Page.captureScreenshot", { format: "png" }).catch(() => null);
            if (shot?.result?.data) fs.writeFileSync(path.join(args.shots, `layout-stress-seed${args.seed}-step${log.length}.png`), Buffer.from(shot.result.data, "base64"));
        }
        await close(cdp);
        process.exit(1);
    };

    // A second window tab for drops on a tab (--tab): opened and left, so the run starts in the tab it was in.
    if (args.tab && (await cdp.evaluate(`document.querySelectorAll(".tab").length`)) < 2) {
        await cdp.evaluate(`${S}.run("tab:new")`);
        await sleep(1500);
        await cdp.evaluate(`${S}.run("tab:prev")`);
        await sleep(1500);
    }
    // Start: the window as it is, which must already agree.
    const start = await settleAgree();
    if (start.length) await fail("the window and the stored layout disagree before the first step", start);
    const counts = {};
    const changed = {};
    const dragStats = {};
    /** Closes every window but the one the run started in (a pane torn off by a drag gets its own, which shows a
     *  moment after the drop) and returns how many it closed. The pane goes with its window. */
    const reapWindows = async () => {
        let closed = 0;
        for (let i = 0; i < 8; i++) {
            await sleep(250);
            for (const w of (await openWindows(args.port)).filter((x) => !known.has(x.id))) {
                await fetch(`http://127.0.0.1:${args.port}/json/close/${w.id}`).catch(() => {});
                closed++;
            }
            if (closed && i >= 3) break;
        }
        return closed;
    };
    let dragKind = null;

    for (let step = 0; step < args.steps; step++) {
        let dom = await domNow();
        const maxed = Boolean(dom.maximizedBlockId);
        const expanded = dom.tiles.filter((t) => !t.minimized);
        const minimized = dom.tiles.filter((t) => t.minimized);
        const ops = [];
        if (!maxed) {
            if (dom.tiles.length < 8) ops.push("splitRight", "splitDown", "splitRight");
            if (expanded.length > 1) ops.push("minimize", "minimize");
            if (minimized.length) ops.push("restore", "restore");
            if (dom.tiles.length > 1) ops.push("maximize");
            if ([...mine].some((b) => dom.tiles.some((t) => t.blockId === b)) && dom.tiles.length > 2) ops.push("close");
            if (args.drag && dom.tiles.length > 1) for (let i = 0; i < args.dragWeight; i++) ops.push("drag");
            ops.push("resize");
        } else {
            ops.push("restoreMax", "restoreMax", "minimizeMax", "closeMaxedMine");
        }
        if (args.reload && step > 0 && step % 25 === 0) ops.push("reload", "reload", "reload");
        const op = pick(ops);
        dragKind = null;
        let note = op;
        let acted = true;
        if (op === "splitRight" || op === "splitDown") {
            // The app splits a pane at any size (only a resize drag keeps a pane over 128 px), so a run that
            // splits freely ends in panes with no header to take hold of: keep each half big enough to use.
            const room = (x) => x.rect[op === "splitRight" ? 2 : 3] >= 2 * 128;
            const t = pick(dom.tiles.filter((x) => !x.minimized && room(x)));
            if (!t?.blockId) {
                log.push(`${op} (every pane is too small to split) [no-op]`);
                continue;
            }
            const before = new Set(dom.tiles.map((x) => x.id));
            await cdp.evaluate(`${S}.run(${js(op === "splitRight" ? "split:right" : "split:down")}, ${js(t.blockId)})`);
            await sleep(500);
            const after = await domNow();
            for (const n of after.tiles) if (!before.has(n.id) && n.blockId) mine.add(n.blockId);
            note = `${op} on ${t.id.slice(0, 8)}`;
        } else if (op === "minimize") {
            const t = pick(expanded);
            acted = await clickIn(cdp, `.tile-node[id="${t.id}"]`, ".block-frame-minimize", "Minimize");
            note = `minimize ${t.id.slice(0, 8)}${acted ? "" : " (no button)"}`;
        } else if (op === "restore") {
            const t = pick(minimized);
            acted = await clickIn(cdp, `.tile-node[id="${t.id}"]`, ".block-frame-minimize", "Restore");
            note = `restore ${t.id.slice(0, 8)}${acted ? "" : " (no button)"}`;
        } else if (op === "maximize") {
            const t = pick(expanded);
            acted = await clickIn(cdp, `.tile-node[id="${t.id}"]`, ".block-frame-magnify", "Maximize");
            note = `maximize ${t.id.slice(0, 8)}${acted ? "" : " (no button)"}`;
        } else if (op === "restoreMax") {
            acted = await clickIn(cdp, ".magnify-pane, .magnify-container", ".block-frame-magnify", "Restore");
        } else if (op === "minimizeMax") {
            acted = await clickIn(cdp, ".magnify-pane, .magnify-container", ".block-frame-minimize", "Minimize");
        } else if (op === "closeMaxedMine") {
            acted = false; // closing from a maximized pane is left to a person
        } else if (op === "close") {
            const t = pick(dom.tiles.filter((x) => x.blockId && mine.has(x.blockId)));
            acted = await clickIn(cdp, `.tile-node[id="${t.id}"]`, ".block-frame-default-close", "Close");
            if (acted) mine.delete(t.blockId);
            note = `close ${t.id.slice(0, 8)}${acted ? "" : " (no button)"}`;
        } else if (op === "drag") {
            // Kinds this layout can offer; a minimized pane's slot and a picked-up chip count three times when there is one.
            const offered = KINDS.filter((k) => (k !== "minimized" && k !== "chip") || minimized.length).flatMap((k) => (k === "minimized" || k === "chip" ? [k, k, k] : [k]));
            const kind = pick(offered);
            await sleep(400); // let the last step's animation finish, so a change seen after is the drag's
            dom = await domNow();
            const plan = await planDrag(cdp, dom, kind, pick, dom.area);
            if (!plan) {
                acted = false;
                note = `drag ${kind} (none in this layout)`;
            } else {
                const r = await dragPane(cdp, plan.from, plan.aim);
                const torn = (await reapWindows()) > 0;
                const t = (dragStats[kind] ??= { tried: 0, started: 0, aimed: 0, dropped: 0, changed: 0 });
                t.tried++;
                if (r.started) t.started++;
                if (r.started && (r.spots > 0 || !plan.aim.pick)) t.aimed++;
                if (r.dropped) t.dropped++;
                if (torn) t.torn = (t.torn ?? 0) + 1;
                dragKind = kind;
                acted = r.dropped;
                note = `${plan.note}${r.dropped ? "" : r.started ? " (no such drop offered)" : " (not started)"}${torn ? " (tore a window off)" : ""}`;
            }
        } else if (op === "resize") {
            const r = await resizeBy(cdp, rand);
            acted = r !== null;
            note = `resize ${r ?? "(no handle)"}`;
        } else if (op === "reload") {
            await cdp.send("Page.reload");
            await sleep(2000);
            for (let i = 0; i < 40; i++) {
                const ready = await cdp.evaluate(`typeof window.__agentmux_shortcuts === "object" && document.querySelectorAll(".tile-node").length > 0`).catch(() => false);
                if (ready) break;
                await sleep(500);
            }
            await cdp.send("Page.bringToFront").catch(() => {});
        }
        log.push(`${note}${acted ? "" : " [no-op]"}`);
        counts[op] = (counts[op] ?? 0) + 1;
        const problems = await settleAgree();
        if (problems.length) await fail(note, problems);
        // Did the step change anything drawn? (A drag that starts but drops nowhere changes nothing.)
        const shape = (d) => JSON.stringify(d.tiles.map((t) => [t.id, t.minimized, ...t.rect.map(Math.round)]).sort());
        if (acted && shape(await domNow()) !== shape(dom)) {
            changed[op] = (changed[op] ?? 0) + 1;
            if (op === "drag" && dragKind) dragStats[dragKind].changed++;
            // A drop on no spot is meant to leave the layout as it was. (A release on the pane's own place offers the
            // tear-off, which moves the pane to a window of its own: that is `own`'s expected change.)
            if (op === "drag" && dragKind === "off-spot") console.log(`  note: step ${log.length} (${log[log.length - 1]}) changed the layout; a drop on no spot should not`);
        }
    }

    console.log(`seed ${args.seed}: ${args.steps} steps, window and stored layout agree after each.`);
    console.log("steps run (changed what is drawn): " + Object.entries(counts).map(([k, v]) => `${k} ${v} (${changed[k] ?? 0})`).join(", "));
    const noop = log.filter((l) => l.endsWith("[no-op]")).length;
    if (noop) console.log(`${noop} step(s) found nothing to click or drag and did nothing.`);
    if (Object.keys(dragStats).length) {
        console.log("drags by target kind (tried / picked up / a drop on offer / dropped / changed the layout):");
        for (const k of KINDS) if (dragStats[k]) console.log(`  ${k.padEnd(14)} ${["tried", "started", "aimed", "dropped", "changed"].map((f) => String(dragStats[k][f]).padStart(3)).join(" ")}${dragStats[k].torn ? `  (${dragStats[k].torn} tore a window off)` : ""}`);
    }
    await close(cdp);
}

main().catch((e) => {
    console.error(e.stack ?? e);
    process.exit(2);
});
