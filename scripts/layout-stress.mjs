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
//   node scripts/layout-stress.mjs [--port N] [--db PATH] [--seed N] [--steps N] [--shots DIR] [--no-drag] [--no-reload]
//
// Run it against your own throwaway `task dev` instance (the DevTools port is AGENTMUX_CDP_PORT, 9223 by default
// in dev), never an instance someone works in: it clicks, drags and reloads the window it finds. The input is
// synthetic DevTools input, so it takes no real mouse or keyboard. --db is that instance's objects.db (its data
// dir's `db/objects.db`); without it the newest one under `.dev-home/` in the repo is used. The run works in the
// window's active tab and closes only panes it opened itself.
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
    const a = { port: Number(process.env.AGENTMUX_CDP_PORT) || 9223, db: null, seed: 1, steps: 80, shots: null, drag: true, reload: true };
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

/** A header drag with the drag intercepted, as pragmatic-dnd needs: press, move, dragover the target, drop. */
async function dragPane(cdp, from, to) {
    await cdp.send("Input.setInterceptDrags", { enabled: true });
    let data = null;
    const off = cdp.on("Input.dragIntercepted", (p) => (data = p.data));
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: from.x, y: from.y });
    await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: from.x, y: from.y, button: "left", clickCount: 1 });
    for (let i = 1; i <= 4; i++) await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: from.x + i * 6, y: from.y + i * 3, button: "left", buttons: 1 });
    await sleep(150);
    if (!data) {
        await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: from.x, y: from.y, button: "left", clickCount: 1 });
        await cdp.send("Input.setInterceptDrags", { enabled: false });
        off();
        return false;
    }
    for (const type of ["dragEnter", "dragOver", "dragOver", "drop"]) {
        await cdp.send("Input.dispatchDragEvent", { type, x: to.x, y: to.y, data });
        await sleep(80);
    }
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: to.x, y: to.y, button: "left", clickCount: 1 });
    await cdp.send("Input.setInterceptDrags", { enabled: false });
    off();
    return true;
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

    // Start: the window as it is, which must already agree.
    const start = await settleAgree();
    if (start.length) await fail("the window and the stored layout disagree before the first step", start);
    const counts = {};
    const changed = {};

    for (let step = 0; step < args.steps; step++) {
        const dom = await domNow();
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
            if (args.drag && dom.tiles.length > 1) ops.push("drag");
            ops.push("resize");
        } else {
            ops.push("restoreMax", "restoreMax", "minimizeMax", "closeMaxedMine");
        }
        if (args.reload && step > 0 && step % 25 === 0) ops.push("reload", "reload", "reload");
        const op = pick(ops);
        let note = op;
        let acted = true;
        if (op === "splitRight" || op === "splitDown") {
            const t = pick(dom.tiles.filter((x) => !x.minimized));
            if (!t?.blockId) continue;
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
            const a = pick(dom.tiles.filter((x) => !x.minimized));
            const b = pick(dom.tiles.filter((x) => x.id !== a?.id && !x.minimized));
            if (a && b) {
                const edge = pick([[0.5, 0.15], [0.5, 0.85], [0.15, 0.5], [0.85, 0.5]]);
                const from = await centreOf(cdp, ".block-frame-default-header", `.tile-node[id="${a.id}"]`);
                const to = { x: b.rect[0] + b.rect[2] * edge[0], y: b.rect[1] + b.rect[3] * edge[1] };
                acted = Boolean(from) && (await dragPane(cdp, { x: from.x - 40 > 0 ? from.x - 40 : from.x, y: from.y }, to));
                note = `drag ${a.id.slice(0, 8)} onto ${b.id.slice(0, 8)} edge ${edge.join(",")}${acted ? "" : " (not started)"}`;
            } else acted = false;
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
        if (acted && shape(await domNow()) !== shape(dom)) changed[op] = (changed[op] ?? 0) + 1;
    }

    console.log(`seed ${args.seed}: ${args.steps} steps, window and stored layout agree after each.`);
    console.log("steps run (changed what is drawn): " + Object.entries(counts).map(([k, v]) => `${k} ${v} (${changed[k] ?? 0})`).join(", "));
    const noop = log.filter((l) => l.endsWith("[no-op]")).length;
    if (noop) console.log(`${noop} step(s) found nothing to click or drag and did nothing.`);
    await close(cdp);
}

main().catch((e) => {
    console.error(e.stack ?? e);
    process.exit(2);
});
