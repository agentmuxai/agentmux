#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Re-takes every docs screenshot and recording in one go, and optionally
// copies them into an agentmux-docs clone — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §13. For when the
// UI has changed: one run against a capture instance (§8.4), then review the
// docs clone's diff and open a PR.
//
//   AGENTMUX_HOME_OVERRIDE=<capture home> node scripts/ui-screenshots/refresh.mjs --port N
//       [--docs <agentmux-docs clone>] [--suites widgets,chrome,agent,memory] [--no-recordings]
//       [--ffmpeg PATH] [--out DIR] [--redact from=to]...
//
// Runs capture.mjs once per suite, in an order that respects what they need
// (the memory suite uses an agent the agent suite creates), then record.mjs,
// each into its own folder under --out. With --docs, publish.mjs then
// replaces the images the docs already use. A suite that fails doesn't stop
// the others; the summary lists what failed.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { publish } from "./publish.mjs";

const here = dirname(fileURLToPath(import.meta.url));

/** The suites, in the order they must run. */
export const SUITE_ORDER = ["widgets", "chrome", "agent", "memory"];

export function parseArgs(argv) {
    const args = { suites: [...SUITE_ORDER], recordings: true, redact: [] };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === "--port") args.port = argv[++i];
        else if (a === "--docs") args.docs = resolve(argv[++i]);
        else if (a === "--suites") args.suites = argv[++i].split(",").map((s) => s.trim()).filter(Boolean);
        else if (a === "--no-recordings") args.recordings = false;
        else if (a === "--ffmpeg") args.ffmpeg = argv[++i];
        else if (a === "--out") args.out = resolve(argv[++i]);
        else if (a === "--redact") args.redact.push(argv[++i]);
    }
    const unknown = args.suites.filter((s) => !SUITE_ORDER.includes(s));
    if (unknown.length) throw new Error(`unknown suite(s): ${unknown.join(", ")} (known: ${SUITE_ORDER.join(", ")})`);
    // Whatever order they were named in, run them in the order that works.
    args.suites = SUITE_ORDER.filter((s) => args.suites.includes(s));
    args.out ??= join(here, "..", "..", "docs", "manual", "refresh", new Date().toISOString().replace(/[:.]/g, "-"));
    return args;
}

/** The commands a refresh runs, as [label, script, args]. */
export function commands(args) {
    const common = [...(args.port ? ["--port", String(args.port)] : []), ...args.redact.flatMap((r) => ["--redact", r])];
    const list = args.suites.map((s) => [s, "capture.mjs", ["--suite", s, "--out", join(args.out, s), ...common]]);
    if (args.recordings) {
        list.push(["recordings", "record.mjs", ["--out", join(args.out, "recordings"), ...(args.ffmpeg ? ["--ffmpeg", args.ffmpeg] : []), ...common]]);
    }
    return list;
}

function main() {
    const args = parseArgs(process.argv.slice(2));
    if (!process.env.AGENTMUX_HOME_OVERRIDE && args.suites.some((s) => s === "agent" || s === "memory")) {
        console.error("set AGENTMUX_HOME_OVERRIDE to the capture instance's data folder (the agent suite installs its scripted Claude Code there)");
        process.exit(2);
    }
    mkdirSync(args.out, { recursive: true });
    const results = [];
    for (const [label, script, scriptArgs] of commands(args)) {
        console.log(`\n=== ${label} ===`);
        const r = spawnSync(process.execPath, [join(here, script), ...scriptArgs], { stdio: "inherit", env: process.env });
        results.push({ label, ok: r.status === 0, dir: scriptArgs[scriptArgs.indexOf("--out") + 1] });
    }
    console.log("\n=== summary ===");
    for (const r of results) console.log(`${r.ok ? "ok    " : "FAILED"}  ${r.label}`);
    if (args.docs) {
        console.log(`\n=== publish into ${args.docs} ===`);
        // A suite that crashed before writing its manifest has nothing to publish.
        publish({ docs: args.docs, from: results.map((r) => r.dir).filter((d) => existsSync(join(d, "manifest.json"))) });
        console.log("\nReview the docs clone's git diff, and look at each changed image, before you commit.");
    } else {
        console.log(`\nOutput: ${args.out}. Publish with: node scripts/ui-screenshots/publish.mjs --docs <agentmux-docs> --from <each folder>`);
    }
    if (results.some((r) => !r.ok)) process.exitCode = 1;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
