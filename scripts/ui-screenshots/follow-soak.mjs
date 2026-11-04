#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Scroll-follow soak: the live acceptance run of
// docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md §7.3 (Phase 4).
//
// Streams scripted turns through each selected agent pane's real output pipeline
// (lib/one-way-soak-page.js) while it does the things that used to drop a follow: clicks in the
// transcript that are not scrolls, tool expand/collapse, focus moving in and out of the composer,
// window resizes, and real wheel gestures that must detach a pane and then re-follow it. Every
// second it checks each pane:
//   - FOLLOWING means the pane is at the bottom or converging on it: the follower eases (about
//     140 ms to cover 90 % of a step), so it may be a little above the bottom while content
//     streams, but it must arrive within 700 ms of the content going still, and never stay
//     most of a viewport away;
//   - a pane that a wheel-up detached stays detached while content streams, and a scroll back to
//     the bottom follows again.
// And it reads the `[scroll-follow]` transition log the page prints: a pane going FOLLOWING ->
// DETACHED without a user gesture of ours just before it is a failure.
// Acceptance (§7.3): 30 minutes, 3 panes, zero violations.
//
// Usage (dev builds only; the production instance on 9222 is refused):
//
//   node scripts/ui-screenshots/follow-soak.mjs --target "<title or id substr>"
//        [--port 9223] [--panes all|<blockId,...>] [--minutes 30] [--gap-ms 1500] [--seed 1]
//        [--out follow-soak-<time>.json] [--strict] [--clear-existing] [--keep]
//
// Use freshly opened agent panes, visible, and leave the window alone while it runs: the script
// does the clicking.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { assertDevPort, connectPage } from "./lib/cdp-client.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** A pane may be this far from the bottom and still be "at the bottom" (sub-pixel layout). */
export const BOTTOM_EPSILON_PX = 1.5;
/** A transition to DETACHED within this long after our own wheel/scroll input is the user's. */
export const GESTURE_WINDOW_MS = 1500;

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
    const panes = get("--panes", "all");
    return {
        target: get("--target", ""),
        port: num("--port", 9223),
        panes: panes === "all" ? [] : panes.split(",").filter(Boolean),
        minutes: num("--minutes", 30),
        gapMs: num("--gap-ms", 1500),
        seed: num("--seed", 1),
        out: get("--out", `follow-soak-${new Date().toISOString().replace(/[:.]/g, "-")}.json`),
        strict: has("--strict"),
        clearExisting: has("--clear-existing"),
        keep: has("--keep"),
        wheelOnly: has("--wheel-only"),
        allowProduction: has("--allow-production"),
    };
}

/** `[scroll-follow] pane=abc1234 following→detached cause=user-scroll gap=4px` from the console
 *  call's arguments (the logger passes them as separate strings), or null for any other line. */
export function parseTransition(args) {
    if (args[0] !== "[scroll-follow]") return null;
    const rest = args.slice(1).join(" ");
    const m = /^pane=(\S+)\s+(following|detached|suspended)→(following|detached|suspended)\s+cause=(\S+)/.exec(rest);
    return m ? { pane: m[1], from: m[2], to: m[3], cause: m[4] } : null;
}

/** Why a transition is a failure, or null. Only a drop out of FOLLOWING can be one: a pane that
 *  stops following with no user gesture of ours just before it is the bug this soak exists for. */
export function judgeTransition(t, nowMs, lastGestureMs) {
    if (t.from !== "following" || t.to !== "detached") return null;
    const ours = nowMs - (lastGestureMs ?? -Infinity) <= GESTURE_WINDOW_MS;
    if (ours && t.cause.startsWith("user-")) return null;
    return `${t.pane} dropped out of following (cause=${t.cause}) with no user gesture of ours in the last ${GESTURE_WINDOW_MS} ms`;
}

/** Why a reading is a failure, or null. A pane that is hidden or still covered is not read. */
export function judgeSample(s) {
    if (!s.found || !s.visible || s.covered) return null;
    if (s.state === "following" && s.gap > BOTTOM_EPSILON_PX) {
        return `${s.blockId.slice(0, 7)} is following but ${s.gap.toFixed(1)} px above the bottom`;
    }
    return null;
}

/**
 * What a re-follow check found. A wheel over a tool's own scroll box ("scroll-handoff-box") is held by
 * the box's edge skid for as long as its events keep coming less than a quarter second apart, by design,
 * so a pane that never moved while the point sat over one says nothing about follow: "misaimed".
 */
export function judgeRefollow(s, overHandoffBox) {
    if (s.state === "following") return "pass";
    if (overHandoffBox && s.gap > BOTTOM_EPSILON_PX) return "misaimed";
    return "fail";
}

export function summarize(report) {
    const lines = [];
    const byKind = {};
    for (const v of report.violations) byKind[v.kind] = (byKind[v.kind] ?? 0) + 1;
    lines.push(
        `${report.minutes} min, ${report.panes} pane(s): ${report.samples} readings, ${report.transitions} follow transitions, ` +
            `${Object.entries(report.actions).map(([k, n]) => `${k}×${n}`).join(" ")}`,
    );
    lines.push(`detach checks ${report.detachChecks.pass}/${report.detachChecks.total} held (${report.detachChecks.misaimed ?? 0} wheels landed on a tool box after content moved, not counted), re-follow checks ${report.refollowChecks.pass}/${report.refollowChecks.total}`);
    for (const v of report.violations.slice(0, 20)) lines.push(`  VIOLATION [${v.kind}] t=${Math.round(v.atMs / 1000)}s ${v.text}`);
    if (report.violations.length > 20) lines.push(`  … and ${report.violations.length - 20} more`);
    lines.push(report.violations.length === 0 ? "PASS: no scroll-follow violations" : `FAIL: ${report.violations.length} violation(s) ${JSON.stringify(byKind)}`);
    return lines.join("\n");
}

// A small seeded generator, so two runs do the same things in the same order.
function rng(seed) {
    let s = seed || 1;
    return () => {
        s = (s * 1103515245 + 12345) & 0x7fffffff;
        return s / 0x7fffffff;
    };
}

async function main() {
    const opts = parseArgs(process.argv.slice(2));
    assertDevPort(opts.port, { allowProduction: opts.allowProduction });
    // A run that has just ended may still hold the page's debugger session for a moment, and the
    // page is missing from the target list until it lets go: try again for a few seconds.
    let connected;
    for (let i = 0; ; i++) {
        try {
            connected = await connectPage(opts.port, opts.target);
            break;
        } catch (e) {
            if (i >= 15) throw e;
            await sleep(2000);
        }
    }
    const { session: page } = connected;
    const t0 = Date.now();
    const now = () => Date.now() - t0;
    const rnd = rng(opts.seed);
    const violations = [];
    const actions = {};
    const lastGesture = new Map(); // pane tag -> ms
    const report = {
        opts,
        minutes: opts.minutes,
        panes: 0,
        samples: 0,
        transitions: 0,
        actions,
        detachChecks: { pass: 0, total: 0, misaimed: 0 },
        refollowChecks: { pass: 0, total: 0 },
        violations,
    };
    let blockIds = [];
    let streaming = null;
    const fail = (kind, text) => violations.push({ kind, text, atMs: now() });
    const tag = (blockId) => blockId.slice(0, 7);
    const followLog = []; // the page's recent follow transitions, for a failure's context
    const transitionsOf = (blockId, windowMs) =>
        followLog
            .filter((l) => l.pane === blockId.slice(0, 7) && now() - l.atMs <= windowMs)
            .map((l) => `${((l.atMs - now()) / 1000).toFixed(1)}s ${l.text}`)
            .join("; ") || "none";
    const recent = []; // the last few things we did, for a failure's context
    const note = (kind) => {
        actions[kind] = (actions[kind] ?? 0) + 1;
        recent.push(`${kind}@${Math.round(now() / 1000)}s`);
        if (recent.length > 6) recent.shift();
    };
    const gesture = (blockId) => lastGesture.set(tag(blockId), now());

    const wheel = async (point, deltaY) => {
        await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: point.x, y: point.y });
        await page.send("Input.dispatchMouseEvent", { type: "mouseWheel", x: point.x, y: point.y, deltaX: 0, deltaY });
    };
    const click = async (point) => {
        await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: point.x, y: point.y });
        await page.send("Input.dispatchMouseEvent", { type: "mousePressed", x: point.x, y: point.y, button: "left", clickCount: 1 });
        await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: point.x, y: point.y, button: "left", clickCount: 1 });
    };
    const sampleOf = (blockId) => page.evaluate(`window.__fsk.sample(${JSON.stringify(blockId)})`);
    const pointOf = (blockId, kind, pick = 0) => page.evaluate(`window.__fsk.point(${JSON.stringify(blockId)}, ${JSON.stringify(kind)}, ${pick})`);

    try {
        await page.send("Runtime.enable");
        page.on("Runtime.consoleAPICalled", (e) => {
            const args = (e.args ?? []).map((a) => a.value ?? a.description ?? "");
            const t = parseTransition(args);
            if (!t) return;
            report.transitions++;
            followLog.push({ atMs: now(), pane: t.pane, text: `${t.from}→${t.to} ${t.cause}` });
            if (followLog.length > 60) followLog.shift();
            const why = judgeTransition(t, now(), lastGesture.get(t.pane));
            if (why) fail("transition", why);
        });

        await page.evaluate(readFileSync(join(HERE, "lib", "bench-page.js"), "utf8"));
        // A window that has just reloaded shows its agent panes a few seconds later: wait for them.
        let init = await page.evaluate(`window.__fcb.init(${JSON.stringify(opts.panes)})`);
        for (let i = 0; i < 20 && init.panes.length === 0 && init.visibility === "visible"; i++) {
            await sleep(2000);
            init = await page.evaluate(`window.__fcb.init(${JSON.stringify(opts.panes)})`);
        }
        if (init.visibility !== "visible") throw new Error(`page is ${init.visibility} — restore the window first`);
        if (init.panes.length === 0) throw new Error(`no matching visible agent panes (visible: ${init.available.join(", ") || "none"})`);
        const real = init.panes.filter((p) => p.nodeCount > 0 && !p.syntheticOnly);
        if (real.length && !opts.clearExisting) throw new Error(`panes hold real content (${real.map((p) => p.blockId).join(", ")}); use fresh panes or --clear-existing`);
        blockIds = init.panes.map((p) => p.blockId);
        report.panes = blockIds.length;
        await page.evaluate(`window.__fcb.clear(${JSON.stringify(blockIds)})`);
        await page.evaluate(readFileSync(join(HERE, "lib", "one-way-soak-page.js"), "utf8"));
        await page.evaluate(readFileSync(join(HERE, "lib", "follow-soak-page.js"), "utf8"));
        await page.evaluate("window.__fsk.watchWheels()");
        const toolRun = page
            .evaluate(`window.__fsk.runTools(${JSON.stringify({ minutes: opts.minutes })})`, { timeout: (opts.minutes * 60 + 300) * 1000 })
            .catch((e) => fail("stream", `the tool streamer stopped: ${e.message}`));
        console.log(`follow soak: ${blockIds.length} pane(s), ${opts.minutes} min, seed ${opts.seed}`);

        streaming = page
            .evaluate(`window.__ows.run(${JSON.stringify({ minutes: opts.minutes, gapMs: opts.gapMs, seed: opts.seed })})`, { timeout: (opts.minutes * 60 + 300) * 1000 })
            .catch((e) => fail("stream", `the streamer stopped: ${e.message}`));

        const endAt = Date.now() + opts.minutes * 60_000;
        const busy = new Set(); // panes inside a detach/refollow check: not also clicked on

        // The readings: every second, every pane.
        const reader = (async () => {
            while (Date.now() < endAt) {
                for (const id of blockIds) {
                    const s = await sampleOf(id).catch(() => null);
                    if (!s) continue;
                    report.samples++;
                    // The follower eases toward the bottom, so a following pane is a little above it while
                    // content streams. It is a failure when it stops converging (page: K.stuck).
                    if (!busy.has(id) && judgeSample(s)) {
                        const r = await page.evaluate(`window.__fsk.stuck(${JSON.stringify(id)}, ${BOTTOM_EPSILON_PX})`).catch(() => null);
                        if (r?.stuck) fail("not-at-bottom", `${tag(id)} is following but ${r.reason}`);
                    }
                }
                await sleep(1000);
            }
        })();

        // A wheel-up must detach and the pane must stay detached while content streams; scrolling
        // back down must follow again.
        const detachCycle = async (id) => {
            busy.add(id);
            try {
                const before = await sampleOf(id);
                if (!before.found || !before.visible || before.state !== "following" || before.scrollHeight <= before.clientHeight + 300) return;
                // On the transcript itself: a wheel over a tool preview's own scroll box scrolls that box
                // (and rightly does not detach the pane).
                const pick = Math.floor(rnd() * 27);
                const p = await page.evaluate(`window.__fsk.wheelPoint(${JSON.stringify(id)}, ${pick})`);
                if (!p) return;
                note("wheel-up");
                gesture(id);
                const watching = page.evaluate(`window.__fsk.watch(${JSON.stringify(id)}, 700)`);
                await wheel(p, -400);
                const seen = await watching;
                const after = await sampleOf(id);
                // The wheel was over plain transcript text, so it must have scrolled the transcript up and
                // detached it. (A wheel over a tool's own scroll box is not sent: it says nothing about follow.)
                // Content moves under the cursor between aiming and the wheel landing: judge only a wheel
                // that landed on plain text.
                const aimed = (await page.evaluate(`window.__fsk.lastWheelPlain(${JSON.stringify(id)})`)) !== false;
                if (!aimed) {
                    report.detachChecks.misaimed++;
                    if (after.state !== "detached") return;
                } else {
                    report.detachChecks.total++;
                    if (after.state === "detached") report.detachChecks.pass++;
                    else {
                        const arrived = await page.evaluate(`window.__fsk.wheelsSince(${JSON.stringify(id)}, 1500)`);
                        const at = await page.evaluate(`window.__fsk.elementAt(${p.x}, ${p.y})`);
                        fail(
                            "detach",
                            `${tag(id)} did not detach on a wheel-up over plain text (scrolled ${seen.upScrolled.toFixed(0)} px up at an unchanged height, ended "following", gap=${after.gap?.toFixed(1)}); the pane saw ${arrived} wheel event(s); over ${at.blockId?.slice(0, 7)} ${at.tag}.${at.cls}; last actions: ${recent.join(", ")}; the pane's transitions in the last 4 s: ${transitionsOf(id, 4000)}; events: ${seen.trace.slice(0, 24).join(" | ")}`,
                        );
                    }
                }
                if (after.state !== "detached") return;
                // Held while content streams.
                for (let i = 0; i < 4; i++) {
                    await sleep(1000);
                    const s = await sampleOf(id);
                    if (s.state === "following" && s.gap > 40) {
                        fail("detach", `${tag(id)} was following again ${s.gap.toFixed(0)} px above the bottom with no input`);
                        break;
                    }
                }
                // Back to the bottom: wheel down until it is there.
                report.refollowChecks.total++;
                let s = await sampleOf(id);
                // Aimed afresh each time: content moves under the pointer, and a tool's scroll box holds the wheel.
                let aim = p;
                for (let i = 0; i < 120 && s.state === "detached" && s.gap > BOTTOM_EPSILON_PX; i++) {
                    gesture(id);
                    aim = (await page.evaluate(`window.__fsk.wheelPoint(${JSON.stringify(id)}, ${pick})`)) ?? aim;
                    await wheel(aim, 1500);
                    await sleep(100);
                    s = await sampleOf(id);
                }
                await sleep(500);
                s = await sampleOf(id);
                const at = s.state === "following" ? null : await page.evaluate(`window.__fsk.elementAt(${aim.x}, ${aim.y})`);
                const verdict = judgeRefollow(s, at?.inHandoffBox === true);
                if (verdict === "pass") report.refollowChecks.pass++;
                else if (verdict === "misaimed") report.refollowChecks.total--;
                else {
                    fail(
                        "refollow",
                        `${tag(id)} did not follow again after scrolling to the bottom (state=${s.state}, gap=${s.gap?.toFixed(1)}); the last wheel point is over ${at.blockId?.slice(0, 7)} ${at.tag}.${at.cls}; last actions: ${recent.join(", ")}; the pane's transitions in the last 20 s: ${transitionsOf(id, 20000)}`,
                    );
                }
            } finally {
                busy.delete(id);
            }
        };

        // The things that are not scrolling.
        const chaos = async () => {
            const id = blockIds[Math.floor(rnd() * blockIds.length)];
            if (busy.has(id)) return;
            const roll = opts.wheelOnly ? 1 : rnd();
            if (roll < 0.22) {
                const p = await pointOf(id, "text", Math.floor(rnd() * 50));
                if (p) (note("click"), await click(p));
            } else if (roll < 0.42) {
                const p = await pointOf(id, "expander", Math.floor(rnd() * 20));
                if (p) (note("expand"), await click(p));
            } else if (roll < 0.62) {
                note("focus");
                await page.evaluate(`window.__fsk.focusComposer(${JSON.stringify(id)}, true)`);
                await sleep(400 + rnd() * 800);
                await page.evaluate(`window.__fsk.focusComposer(${JSON.stringify(id)}, false)`);
            } else if (roll < 0.8) {
                note("resize");
                const v = await page.evaluate("window.__fsk.viewport()");
                await page.send("Emulation.setDeviceMetricsOverride", { width: Math.max(700, v.width - 120 - Math.floor(rnd() * 160)), height: v.height, deviceScaleFactor: 0, mobile: false });
                await sleep(1200);
                await page.send("Emulation.clearDeviceMetricsOverride");
            } else {
                await detachCycle(id);
            }
        };

        const driver = (async () => {
            await sleep(15_000); // let the first turns put real content in the panes
            while (Date.now() < endAt - 8000) {
                await chaos().catch((e) => fail("harness", `an action failed: ${e.message}`));
                await sleep(1500 + rnd() * 4500);
            }
        })();

        await Promise.all([reader, driver]);
        await Promise.race([Promise.all([streaming, toolRun]), sleep(120_000)]);
        await sleep(1000);

        const text = summarize(report);
        console.log(text);
        writeFileSync(opts.out, JSON.stringify(report, null, 2));
        console.log(`report: ${opts.out}`);
        if (opts.strict && violations.length > 0) process.exitCode = 1;
    } finally {
        await page.evaluate("window.__ows?.stop(); window.__fsk?.stopTools()").catch(() => {});
        await page.send("Emulation.clearDeviceMetricsOverride").catch(() => {});
        if (!opts.keep && blockIds.length) await page.evaluate(`window.__fcb.clear(${JSON.stringify(blockIds)})`).catch(() => {});
        await page.evaluate("window.__fcb?.dispose?.()").catch(() => {});
        await page.close().catch(() => {});
    }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
    main().catch((e) => {
        console.error(`follow-soak: ${e.message}`);
        process.exit(2);
    });
}
