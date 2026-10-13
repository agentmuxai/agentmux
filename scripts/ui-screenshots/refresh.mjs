#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Re-takes every docs and landing screenshot and recording in one go, and
// optionally publishes them into an agentmux-docs and/or agentmux-landing
// clone — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §13. For when the
// UI has changed: one run against a capture instance (§8.4), then review the
// docs clone's diff and open a PR.
//
//   AGENTMUX_HOME_OVERRIDE=<capture home> node scripts/ui-screenshots/refresh.mjs --port N
//       [--docs <agentmux-docs clone>] [--landing <agentmux-landing clone>]
//       [--suites widgets,chrome,agent,memory,landing] [--no-recordings]
//       [--ffmpeg PATH] [--out DIR] [--redact from=to]...
//
// Runs capture.mjs once per suite, in an order that respects what they need
// (the memory suite uses an agent the agent suite creates), then record.mjs,
// each into its own folder under --out. With --docs, publish.mjs then
// replaces the images the docs already use; with --landing, it encodes the
// files the landing site's manifest lists (publish-landing.mjs). A suite that fails doesn't stop
// the others; the summary lists what failed.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { findFfmpeg } from "./lib/recorder.mjs";
import { publish } from "./publish.mjs";
import { publishLanding } from "./publish-landing.mjs";

const here = dirname(fileURLToPath(import.meta.url));

/** The suites, in the order they must run. */
export const SUITE_ORDER = ["widgets", "chrome", "agent", "memory", "landing"];

export function parseArgs(argv) {
    const args = { suites: [...SUITE_ORDER], recordings: true, redact: [] };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === "--port") args.port = argv[++i];
        else if (a === "--docs") args.docs = resolve(argv[++i]);
        else if (a === "--landing") args.landing = resolve(argv[++i]);
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
        // The docs use the GIFs; the landing site plays MP4s.
        const formats = args.landing ? ["--formats", "gif,mp4"] : [];
        list.push(["recordings", "record.mjs", ["--out", join(args.out, "recordings"), ...formats, ...(args.ffmpeg ? ["--ffmpeg", args.ffmpeg] : []), ...common]]);
    }
    return list;
}

function main() {
    const args = parseArgs(process.argv.slice(2));
    // No default port: a refresh clicks through whatever instance it reaches,
    // and a dev build opens CDP on its own.
    if (!args.port) {
        console.error("pass --port: the capture instance's CDP port (never your own instance's)");
        process.exit(2);
    }
    // The recordings use the agent suite's setup too (Codex on #4673).
    if (!process.env.AGENTMUX_HOME_OVERRIDE && (args.recordings || args.suites.some((s) => s === "agent" || s === "memory"))) {
        console.error("set AGENTMUX_HOME_OVERRIDE to the capture instance's data folder (the agent suite installs its scripted Claude Code there)");
        process.exit(2);
    }
    // Before twenty minutes of capturing, not after (muxreview on #4673).
    if (args.docs && !existsSync(join(args.docs, "src", "assets"))) {
        console.error(`--docs ${args.docs} doesn't look like an agentmux-docs clone (no src/assets)`);
        process.exit(2);
    }
    if (args.landing && !existsSync(join(args.landing, "public", "screenshots", "manifest.json"))) {
        console.error(`--landing ${args.landing} doesn't look like an agentmux-landing clone (no public/screenshots/manifest.json)`);
        process.exit(2);
    }
    mkdirSync(args.out, { recursive: true });
    const results = [];
    for (const [label, script, scriptArgs] of commands(args)) {
        console.log(`\n=== ${label} ===`);
        const dir = scriptArgs[scriptArgs.indexOf("--out") + 1];
        // An --out reused from an earlier run: drop that run's manifest, so a
        // suite that crashes now can't publish the old images (Codex on #4673).
        rmSync(join(dir, "manifest.json"), { force: true });
        const r = spawnSync(process.execPath, [join(here, script), ...scriptArgs], { stdio: "inherit", env: process.env });
        results.push({ label, ok: r.status === 0, dir });
    }
    console.log("\n=== summary ===");
    for (const r of results) console.log(`${r.ok ? "ok    " : "FAILED"}  ${r.label}`);
    // A suite that crashed before writing its manifest has nothing to publish.
    const runs = results.map((r) => r.dir).filter((d) => existsSync(join(d, "manifest.json")));
    if (args.docs) {
        console.log(`\n=== publish into ${args.docs} ===`);
        try {
            const steps = publish({ docs: args.docs, from: runs });
            // As publish.mjs itself does: an ambiguous name wasn't published.
            if (steps.some((s) => s.action === "ambiguous")) process.exitCode = 1;
            console.log("\nReview the docs clone's git diff, and look at each changed image, before you commit.");
        } catch (err) {
            console.error(`publish failed: ${err.message}`);
            process.exitCode = 1;
        }
    }
    if (args.landing) {
        console.log(`\n=== publish into ${args.landing} ===`);
        try {
            const steps = publishLanding({ landing: args.landing, from: runs, ffmpeg: findFfmpeg(args.ffmpeg) });
            if (steps.some((s) => s.action === "ambiguous")) process.exitCode = 1;
            console.log("\nReview the landing clone's git diff, and look at each changed image, before you commit.");
        } catch (err) {
            console.error(`landing publish failed: ${err.message}`);
            process.exitCode = 1;
        }
    }
    if (!args.docs && !args.landing) {
        console.log(`\nOutput: ${args.out}. Publish with: node scripts/ui-screenshots/publish.mjs --docs <agentmux-docs> --landing <agentmux-landing> --from <each folder>`);
    }
    if (results.some((r) => !r.ok)) process.exitCode = 1;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
