#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Full-conversation bench — Phase 0 of
// docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md.
//
// How do streaming smoothness, typing latency and memory scale with the amount
// of history already in the agent panes? For each history size N (turns), the
// bench injects synthetic turns into every selected pane, then runs a window
// in which every pane streams a synthetic reply while (optionally) one
// composer receives OS-style key repeat, and records:
//
//   frames/fps and rAF gaps; long animation frames (total, share of time,
//   forced style+layout, render); key->paint (Event Timing); per-commit
//   document-store dispatch cost; layout/style recalc counts and durations,
//   script and task time, JS heap (CDP Performance.getMetrics); DOM size (page
//   and per pane); resident memory of the renderer, browser and GPU processes
//   (process ids from CDP SystemInfo, memory from the OS).
//
// Usage (dev builds only; the production instance on 9222 is refused):
//
//   node scripts/ui-screenshots/full-conversation-bench.mjs --target "<title or id substr>"
//        [--port 9223] [--panes all|<blockId,...>] [--history 0,25,100,200,500]
//        [--turn-kb 20] [--stream-kb 20] [--secs 10] [--type-into <paneIndex|none>]
//        [--kps 20] [--out bench.json] [--clear-existing] [--keep]
//
//   Soak: [--soak <minutes>] [--sample-every <seconds, default 60>] [--soak-turns-per-sample 1]
//         writes one JSON line per sample to --out (default soak-<time>.jsonl).
//
// Use freshly opened agent panes. Panes holding real (non-bench) content are
// refused unless --clear-existing is given, because the bench clears the panes
// it used when it finishes (unless --keep). The window must stay visible: a
// hidden page throttles rAF and timers, and any window during which it was
// hidden is reported invalid (the run aborts; a soak records and continues).

import { appendFileSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { assertDevPort, connectBrowser, connectPage } from "./lib/cdp-client.mjs";
import { processMemory } from "./lib/process-memory.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

// ── Arguments ───────────────────────────────────────────────────────────────

export function parseArgs(argv) {
    const get = (k, d) => {
        const i = argv.indexOf(k);
        return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d;
    };
    const has = (k) => argv.includes(k);
    const num = (k, d) => {
        const v = Number(get(k, d));
        if (!Number.isFinite(v) || v < 0) throw new Error(`${k} must be a non-negative number`);
        return v;
    };
    const history = String(get("--history", "0,25,100,200,500"))
        .split(",")
        .map((s) => Number(s.trim()));
    if (history.some((n) => !Number.isInteger(n) || n < 0)) throw new Error("--history must be non-negative integers");
    if (history.some((n, i) => i > 0 && n < history[i - 1]))
        throw new Error("--history must be non-decreasing (history is cumulative)");
    const panes = get("--panes", "all");
    const typeInto = get("--type-into", "0");
    return {
        port: num("--port", 9223),
        target: get("--target", ""),
        panes:
            panes === "all"
                ? []
                : panes
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean),
        history,
        turnKb: num("--turn-kb", 20),
        streamKb: num("--stream-kb", 20),
        secs: num("--secs", 10),
        typeInto: typeInto === "none" ? null : Number(typeInto),
        kps: num("--kps", 20),
        out: get("--out", null),
        clearExisting: has("--clear-existing"),
        keep: has("--keep"),
        allowProduction: has("--allow-production"),
        soakMinutes: has("--soak") ? num("--soak", 0) : null,
        sampleEvery: num("--sample-every", 60),
        soakTurns: num("--soak-turns-per-sample", 1),
    };
}

// ── Helpers ─────────────────────────────────────────────────────────────────

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...a) => console.error(new Date().toISOString().slice(11, 19), ...a);
const MB = (b) => (b == null ? null : +(b / 1024 / 1024).toFixed(1));

const METRICS = [
    "LayoutCount",
    "RecalcStyleCount",
    "LayoutDuration",
    "RecalcStyleDuration",
    "ScriptDuration",
    "TaskDuration",
    "JSHeapUsedSize",
    "JSHeapTotalSize",
    "Nodes",
    "JSEventListeners",
];

async function perfMetrics(page) {
    const { metrics } = await page.send("Performance.getMetrics");
    return Object.fromEntries(metrics.filter((m) => METRICS.includes(m.name)).map((m) => [m.name, m.value]));
}

export function metricsDelta(before, after) {
    const d = (k) => +(after[k] - before[k]).toFixed(3);
    return {
        layouts: d("LayoutCount"),
        styleRecalcs: d("RecalcStyleCount"),
        layoutMs: +(d("LayoutDuration") * 1000).toFixed(1),
        styleMs: +(d("RecalcStyleDuration") * 1000).toFixed(1),
        scriptMs: +(d("ScriptDuration") * 1000).toFixed(1),
        taskMs: +(d("TaskDuration") * 1000).toFixed(1),
        jsHeapUsedMB: MB(after.JSHeapUsedSize),
        jsHeapTotalMB: MB(after.JSHeapTotalSize),
        domNodesLive: after.Nodes,
        listeners: after.JSEventListeners,
    };
}

async function processMemoryByType(browser) {
    try {
        const { processInfo } = await browser.send("SystemInfo.getProcessInfo");
        const mem = processMemory(processInfo.map((p) => p.id));
        const byType = {};
        for (const p of processInfo) {
            const m = mem.get(p.id);
            if (!m) continue;
            const t = (byType[p.type] ??= { count: 0, rssMB: 0, privateMB: 0 });
            t.count++;
            t.rssMB = +(t.rssMB + MB(m.rssBytes)).toFixed(1);
            if (m.privateBytes != null) t.privateMB = +(t.privateMB + MB(m.privateBytes)).toFixed(1);
        }
        return byType;
    } catch (e) {
        return { error: e.message };
    }
}

// ── One measurement window ──────────────────────────────────────────────────

async function measureWindow(page, browser, opts, typingBlock) {
    const before = await perfMetrics(page);
    await page.evaluate(`window.__fcb.startWindow(${JSON.stringify({ streamKb: opts.streamKb, secs: opts.secs })})`);
    let typing = null;
    if (typingBlock) {
        await page.evaluate(`window.__fcb.armTyping(${JSON.stringify(typingBlock)})`);
        typing = { keys: 0, abortedAt: null };
        const kc = 70; // "f"
        const interval = 1000 / opts.kps;
        const t0 = Date.now();
        const n = Math.round(opts.secs * opts.kps);
        try {
            for (let i = 0; i < n; i++) {
                const w = t0 + i * interval - Date.now();
                if (w > 0) await sleep(w);
                if (
                    i > 0 &&
                    i % Math.max(1, Math.round(opts.kps)) === 0 &&
                    !(await page.evaluate("window.__fcb.typingOk()"))
                ) {
                    typing.abortedAt = i;
                    break;
                }
                // Fire-and-forget, like OS key repeat: a slow frame must not throttle input.
                page.send("Input.dispatchKeyEvent", {
                    type: "keyDown",
                    key: "f",
                    code: "KeyF",
                    windowsVirtualKeyCode: kc,
                    nativeVirtualKeyCode: kc,
                    text: "f",
                    unmodifiedText: "f",
                    autoRepeat: i > 0,
                }).catch(() => {});
                page.send("Input.dispatchKeyEvent", {
                    type: "keyUp",
                    key: "f",
                    code: "KeyF",
                    windowsVirtualKeyCode: kc,
                    nativeVirtualKeyCode: kc,
                }).catch(() => {});
                typing.keys++;
            }
        } finally {
            const r = await page
                .evaluate("window.__fcb.restoreTyping()")
                .catch((e) => ({ ok: false, error: e.message }));
            typing.restore = r;
        }
    }
    const win = await page.evaluate("window.__fcb.finishWindow()");
    const after = await perfMetrics(page);
    const dom = await page.evaluate("window.__fcb.domCounts()");
    const memory = await processMemoryByType(browser);
    const invalid =
        win.hiddenDuring || win.visibilityAtRead !== "visible"
            ? "page hidden during the window (rAF/timers throttled)"
            : typing && (typing.abortedAt !== null || (typing.restore && typing.restore.stray > 0))
              ? "typing left the composer during the window"
              : null;
    return { invalid, window: win, metrics: metricsDelta(before, after), dom, memory, typing };
}

function summaryLine(n, r) {
    const w = r.window;
    const k = w.keyToPaintMs;
    const rend = r.memory?.renderer?.rssMB ?? "?";
    return [
        `N=${String(n).padStart(3)}`,
        `fps=${String(w.fps).padStart(5)}`,
        `long=${(w.longFrames.shareOfTime * 100).toFixed(0).padStart(3)}%`,
        `forcedLayout=${String(w.longFrames.forcedLayoutMs).padStart(6)}ms`,
        `gap p95=${w.rafGapMs.p95}ms`,
        k ? `key->paint p50/p95/max=${k.p50}/${k.p95}/${k.max}ms` : "no typing",
        `dispatch p95=${w.dispatchMs.p95}ms`,
        `DOM=${r.dom.total}`,
        `renderer=${rend}MB`,
        r.invalid ? `INVALID: ${r.invalid}` : "",
    ].join("  ");
}

// ── Main ────────────────────────────────────────────────────────────────────

async function main() {
    const opts = parseArgs(process.argv.slice(2));
    assertDevPort(opts.port, { allowProduction: opts.allowProduction });

    const { session: page, target } = await connectPage(opts.port, opts.target);
    const browser = await connectBrowser(opts.port);
    log(`page: ${target.title}`);
    await page.send("Performance.enable");
    await page.evaluate(readFileSync(join(HERE, "lib", "bench-page.js"), "utf8"));

    const init = await page.evaluate(`window.__fcb.init(${JSON.stringify(opts.panes)})`);
    if (init.visibility !== "visible") throw new Error(`page is ${init.visibility} — restore the window first`);
    if (init.panes.length === 0)
        throw new Error(`no matching visible agent panes (visible: ${init.available.join(", ") || "none"})`);
    const real = init.panes.filter((p) => p.nodeCount > 0 && !p.syntheticOnly);
    if (real.length && !opts.clearExisting) {
        throw new Error(
            `pane(s) ${real.map((p) => p.blockId.slice(0, 8)).join(", ")} hold real content; open fresh agent panes, or pass --clear-existing (the bench clears the panes it uses when it finishes)`
        );
    }
    const blockIds = init.panes.map((p) => p.blockId);
    const typingBlock = opts.typeInto == null ? null : blockIds[opts.typeInto];
    if (opts.typeInto != null && !typingBlock)
        throw new Error(`--type-into ${opts.typeInto}: only ${blockIds.length} pane(s)`);
    log(
        `panes: ${blockIds.map((b) => b.slice(0, 8)).join(", ")}${typingBlock ? `; typing into ${typingBlock.slice(0, 8)} at ${opts.kps}/s` : ""}`
    );

    let cleaned = false;
    const cleanup = async () => {
        if (cleaned) return;
        cleaned = true;
        const r = await page.evaluate("window.__fcb.restoreTyping()").catch(() => null);
        if (r && r.ok === false) {
            console.error(
                `\nCOULD NOT RESTORE the composer draft of block ${r.blockId}: its composer was not mounted within 30 s.`
            );
            console.error(
                `The composer may show synthetic "f" characters. Your original draft was:\n---\n${r.draft}\n---`
            );
            process.exitCode = 4;
        }
        if (!opts.keep) await page.evaluate(`window.__fcb.clear(${JSON.stringify(blockIds)})`).catch(() => {});
        await page.close().catch(() => {});
        await browser.close().catch(() => {});
    };
    const onSignal = (code) => async () => {
        log("interrupted — restoring the composer and clearing bench content");
        await cleanup();
        process.exit(code);
    };
    process.once("SIGINT", onSignal(130));
    process.once("SIGTERM", onSignal(143));

    try {
        if (opts.clearExisting && real.length) await page.evaluate(`window.__fcb.clear(${JSON.stringify(blockIds)})`);
        if (opts.soakMinutes != null) {
            await soak(page, browser, opts, typingBlock);
        } else {
            await sweep(page, browser, opts, typingBlock, blockIds);
        }
    } finally {
        await cleanup();
    }
}

async function sweep(page, browser, opts, typingBlock, blockIds) {
    const results = [];
    let have = 0;
    for (const n of opts.history) {
        if (n > have) {
            // Inject in chunks so one giant dispatch doesn't dominate the page.
            for (let left = n - have; left > 0; left -= 25)
                await page.evaluate(`window.__fcb.inject(${Math.min(25, left)}, ${opts.turnKb})`);
            have = n;
        }
        await sleep(1500); // let layout and measurement settle
        const r = await measureWindow(page, browser, opts, typingBlock);
        results.push({ historyTurns: n, ...r });
        log(summaryLine(n, r));
        if (r.invalid) {
            log(`aborting: ${r.invalid}`);
            break;
        }
    }
    const report = {
        bench: "full-conversation",
        at: new Date().toISOString(),
        opts: { ...opts, panes: blockIds },
        results,
    };
    const out = opts.out ?? `full-conversation-bench-${Date.now()}.json`;
    writeFileSync(out, JSON.stringify(report, null, 1));
    log(`wrote ${out}`);
    if (results.some((r) => r.invalid)) process.exitCode = 3;
}

async function soak(page, browser, opts, typingBlock) {
    const out = opts.out ?? `soak-${Date.now()}.jsonl`;
    const end = Date.now() + opts.soakMinutes * 60_000;
    let turns = 0;
    let invalid = 0;
    log(`soak for ${opts.soakMinutes} min, one sample every ${opts.sampleEvery}s -> ${out}`);
    while (Date.now() < end) {
        const t0 = Date.now();
        await page.evaluate(`window.__fcb.inject(${opts.soakTurns}, ${opts.turnKb})`);
        turns += opts.soakTurns;
        let sample;
        try {
            sample = await measureWindow(page, browser, opts, typingBlock);
        } catch (e) {
            sample = { invalid: `window failed: ${e.message}` };
        }
        if (sample.invalid) invalid++;
        appendFileSync(
            out,
            JSON.stringify({
                at: new Date().toISOString(),
                elapsedMin: +((Date.now() - (end - opts.soakMinutes * 60_000)) / 60_000).toFixed(2),
                historyTurns: turns,
                ...sample,
            }) + "\n"
        );
        if (sample.window) log(summaryLine(turns, sample));
        const wait = opts.sampleEvery * 1000 - (Date.now() - t0);
        if (wait > 0) await sleep(wait);
    }
    log(`soak done: ${invalid} invalid sample(s); ${out}`);
}

function isEntryPoint() {
    if (!process.argv[1]) return false;
    try {
        const self = realpathSync(fileURLToPath(import.meta.url));
        const invoked = realpathSync(resolve(process.argv[1]));
        return process.platform === "win32" ? self.toLowerCase() === invoked.toLowerCase() : self === invoked;
    } catch {
        return false;
    }
}

if (isEntryPoint()) {
    main().catch((e) => {
        console.error(`full-conversation-bench: ${e.message}`);
        process.exitCode = process.exitCode || 1;
    });
}
