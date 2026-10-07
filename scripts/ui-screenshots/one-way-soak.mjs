#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One-way-flow soak — the acceptance test of
// docs/specs/SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md (§6).
//
// Streams scripted tool-heavy turns (lib/one-way-soak-page.js) through each
// selected agent pane's real output pipeline while the in-app one-way
// recorder (frontend/app/view/agent/scroll/one-way-recorder.ts) checks every
// painted frame: no visible row may move down (V1) and no blank space may be
// shown and then taken back (V2) while the pane follows. Prints the counts
// per pane and per cause, writes the full report to --out, and with --strict
// exits 1 on any violation.
//
// Usage (dev builds only; the production instance on 9222 is refused):
//
//   node scripts/ui-screenshots/one-way-soak.mjs --target "<title or id substr>"
//        [--port 9223] [--panes all|<blockId,...>] [--minutes 3] [--gap-ms 1500]
//        [--seed 1] [--out one-way-<time>.json] [--strict] [--clear-existing] [--keep]
//
// Use freshly opened agent panes, visible, and leave them alone while it runs:
// any input to a pane exempts the frames around it. Panes holding real
// (non-bench) content are refused unless --clear-existing; the panes used are
// cleared at the end unless --keep. Run once on the base build to get the
// baseline, then on the change.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { assertDevPort, connectPage } from "./lib/cdp-client.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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
        minutes: num("--minutes", 3),
        gapMs: num("--gap-ms", 1500),
        seed: num("--seed", 1),
        out: get("--out", `one-way-${new Date().toISOString().replace(/[:.]/g, "-")}.json`),
        strict: has("--strict"),
        clearExisting: has("--clear-existing"),
        keep: has("--keep"),
        allowProduction: has("--allow-production"),
    };
}

/** One line per pane, then the causes across all panes, most frequent first. */
export function summarize(stats) {
    const out = [];
    const causes = {};
    let violations = 0;
    for (const s of stats) {
        const v = s.down + s.overshoot;
        violations += v;
        out.push(
            [
                `pane=${s.pane}`,
                `frames=${s.framesFollowing}/${s.framesChecked}`,
                `down=${s.down}`,
                `overshoot=${s.overshoot}`,
                `maxDy=${s.maxDy.toFixed(1)}px`,
                `blankFrames=${s.blankFrames}`,
                `maxGap=${s.maxGap.toFixed(1)}px`,
            ].join("  "),
        );
        for (const [c, n] of Object.entries(s.byCause)) causes[c] = (causes[c] ?? 0) + n;
    }
    const ranked = Object.entries(causes).sort((a, b) => b[1] - a[1]);
    if (ranked.length) out.push("causes: " + ranked.map(([c, n]) => `${c}×${n}`).join("  "));
    out.push(violations === 0 ? "PASS: no one-way violations" : `FAIL: ${violations} one-way violation(s)`);
    return { text: out.join("\n"), violations };
}

async function main() {
    const opts = parseArgs(process.argv.slice(2));
    assertDevPort(opts.port, { allowProduction: opts.allowProduction });
    const { session: page } = await connectPage(opts.port, opts.target);
    let wasEnabled = false;
    let blockIds = [];
    try {
        await page.evaluate(readFileSync(join(HERE, "lib", "bench-page.js"), "utf8"));
        const init = await page.evaluate(`window.__fcb.init(${JSON.stringify(opts.panes)})`);
        if (init.visibility !== "visible") throw new Error(`page is ${init.visibility} — restore the window first`);
        if (init.panes.length === 0) throw new Error(`no matching visible agent panes (visible: ${init.available.join(", ") || "none"})`);
        const real = init.panes.filter((p) => p.nodeCount > 0 && !p.syntheticOnly);
        if (real.length && !opts.clearExisting) {
            throw new Error(`panes hold real content (${real.map((p) => p.blockId).join(", ")}); use fresh panes or --clear-existing`);
        }
        blockIds = init.panes.map((p) => p.blockId);
        await page.evaluate(`window.__fcb.clear(${JSON.stringify(blockIds)})`);
        await page.evaluate(readFileSync(join(HERE, "lib", "one-way-soak-page.js"), "utf8"));
        wasEnabled = await page.evaluate("window.__agentmuxOneWay?.isEnabled() ?? null");
        if (wasEnabled === null) throw new Error("this build has no one-way recorder (window.__agentmuxOneWay)");
        await page.evaluate("window.__agentmuxOneWay.enable(); window.__agentmuxOneWay.reset()");
        await sleep(500);
        console.log(`soak: ${blockIds.length} pane(s), ${opts.minutes} min, seed ${opts.seed}`);
        const run = await page.evaluate(
            `window.__ows.run(${JSON.stringify({ minutes: opts.minutes, gapMs: opts.gapMs, seed: opts.seed })})`,
            { timeout: (opts.minutes * 60 + 300) * 1000 },
        );
        await sleep(1000); // let the last frames be checked
        const stats = await page.evaluate("window.__agentmuxOneWay.snapshot()");
        const mine = stats.filter((s) => blockIds.some((b) => b.startsWith(s.pane)));
        const { text, violations } = summarize(mine);
        console.log(`turns=${run.turns}\n${text}`);
        writeFileSync(opts.out, JSON.stringify({ opts, run, stats: mine }, null, 2));
        console.log(`report: ${opts.out}`);
        if (opts.strict && violations > 0) process.exitCode = 1;
    } finally {
        await page.evaluate("window.__ows?.stop()").catch(() => {});
        if (!wasEnabled) await page.evaluate("window.__agentmuxOneWay?.disable()").catch(() => {});
        if (!opts.keep && blockIds.length) await page.evaluate(`window.__fcb.clear(${JSON.stringify(blockIds)})`).catch(() => {});
        await page.evaluate("window.__fcb?.dispose?.()").catch(() => {});
        await page.close().catch(() => {});
    }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
    main().catch((e) => {
        console.error(`one-way-soak: ${e.message}`);
        process.exit(2);
    });
}
