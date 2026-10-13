#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reusable, cropped screenshot capture for the AgentMux UI manual.
// See docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md for the
// full design — this is the runner; docs/specs' companion `shots.mjs`
// (loaded from this same directory) is the declarative manifest of what
// to capture. Edit shots.mjs to add/change shots; this file shouldn't
// need to change for the common case.
//
// Usage:
//   node scripts/ui-screenshots/capture.mjs [--port N] [--out DIR] [--only id1,id2]
//       [--suite manual|widgets|chrome|agent|memory|landing] [--sizes small,medium,large|all]
//       [--redact from=to]... [--no-redact]
//
// --suite picks the manifest: `manual` (shots.mjs, the default), `widgets`
// (widget-shots.mjs, every widget in every size) or `chrome` (chrome-shots.mjs:
// menus, the status bar and Settings, spec §9) or `agent` (agent-shots.mjs: the
// agent pane, driven by a scripted Claude Code, spec §10) or `memory`
// (memory-shots.mjs: Memory, Connectors and a widget approval, spec §11).
// --sizes limits which of
// sizes.mjs's sizes a shot with `sizes: true` is captured in (default: all);
// see the spec's §8.
//
// Before each capture the capturing machine's user name and hostname are
// replaced in the page with neutral ones (redact.mjs, spec §9); --redact adds
// a pair, --no-redact turns it off. After each shot's `prep`, its `verify`
// (verify.mjs) must hold or the shot fails.
//
// Talks directly to the target AgentMux instance's CDP remote-debugging
// port — NOT the agentmux-mcp UIScreenshot/UIClick/UIQuery tools, which are
// scoped to the calling agent's own pane and can't reach a separate
// instance's window. See the spec's §2 for why. Node's native WebSocket
// (no external dependency) is used throughout.
//
// A RELEASE instance only runs that port when launched with
// AGENTMUX_CDP_PORT set (#3681) — e.g. `AGENTMUX_CDP_PORT=9222` for both the
// app and this script. Dev builds run it by default.

import { writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { SIZES, parseSizes, shotFilename } from "./sizes.mjs";
import { machinePairs, parseRedactArg, redactExpression } from "./redact.mjs";
import { checkVerify } from "./verify.mjs";
import { CdpSession, NO_DIALOGS, findMainTarget, resolveClip } from "./lib/session.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));

/** Suite name → manifest module (relative to this directory). */
const SUITES = { manual: "./shots.mjs", widgets: "./widget-shots.mjs", chrome: "./chrome-shots.mjs", agent: "./agent-shots.mjs", memory: "./memory-shots.mjs", landing: "./landing-shots.mjs" };

function parseArgs(argv) {
    const args = {
        port: Number(process.env.AGENTMUX_CDP_PORT) || 9222,
        out: null,
        only: null,
        suite: "manual",
        sizes: null,
        redact: true,
        redactExtra: [],
    };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === "--port") args.port = Number(argv[++i]);
        else if (a === "--out") args.out = argv[++i];
        else if (a === "--only") args.only = argv[++i].split(",").map((s) => s.trim());
        else if (a === "--suite") args.suite = argv[++i];
        else if (a === "--sizes") args.sizes = argv[++i];
        else if (a === "--redact") args.redactExtra.push(parseRedactArg(argv[++i]));
        else if (a === "--no-redact") args.redact = false;
    }
    if (!SUITES[args.suite]) throw new Error(`unknown suite "${args.suite}" (known: ${Object.keys(SUITES).join(", ")})`);
    args.sizes = parseSizes(args.sizes);
    if (!args.out) {
        const stamp = new Date().toISOString().replace(/[:.]/g, "-");
        args.out = join(__dirname, "..", "..", "docs", "manual", "screenshots", `${args.suite}-${stamp}`);
    }
    return args;
}

/** Reads width/height straight out of a PNG's IHDR chunk (bytes 16-23,
 *  big-endian uint32 each) — the actual captured pixel dimensions,
 *  independent of any CSS-pixel/device-scale-factor math, and available
 *  whether or not the shot used a `clip` (a `selector`). */
function pngDimensions(buffer) {
    return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

/** Captures one PNG: redacts the machine's names, measures the shot's
 *  selector (a string, an array of them, or a function returning either, for
 *  a pane found during `prep`) and crops to it. `redaction`, when set, is
 *  `{ pairs, replacements }`; the count of replacements made is added to it. */
async function captureOne(session, shot, redaction) {
    // A menu that opens on hover closes when the pointer leaves it; such a
    // shot sets `keepPointer` and leaves the pointer where its prep put it.
    if (!shot.keepPointer) await session.parkMouse();
    if (redaction && redaction.pairs.length > 0) {
        // Before measuring: a shorter name changes the text's width.
        redaction.replacements += await session.evaluate(redactExpression(redaction.pairs));
        await session.evaluate("new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))");
    }
    let clip;
    const selector = typeof shot.selector === "function" ? shot.selector() : shot.selector;
    if (selector) {
        clip = await resolveClip(session, selector, shot.padding ?? 0);
        if (!clip) throw new Error(`selector not found: ${selector}`);
        if (process.env.SHOTS_DEBUG) console.log(`\n  clip ${JSON.stringify(clip)} for ${selector}`);
    }
    // A capture waits for the next compositor frame, which can be slow to come
    // when the window is behind others; give it longer, and one retry.
    const params = { format: "png", ...(clip ? { clip } : {}) };
    let lastErr;
    for (let attempt = 0; attempt < 2; attempt++) {
        try {
            const { data } = await session.send("Page.captureScreenshot", params, 20000);
            return Buffer.from(data, "base64");
        } catch (err) {
            lastErr = err;
            await session.send("Page.bringToFront").catch(() => {});
            await session.wait(500);
        }
    }
    throw lastErr;
}

async function main() {
    const args = parseArgs(process.argv.slice(2));
    const suite = await import(SUITES[args.suite]);
    const { shots } = suite;
    const selected = args.only ? shots.filter((s) => args.only.includes(s.id)) : shots;
    if (selected.length === 0) {
        console.error("No shots selected — check --only against shots.mjs's ids.");
        process.exit(1);
    }

    mkdirSync(args.out, { recursive: true });
    console.log(`Connecting to CDP on port ${args.port}...`);
    const target = await findMainTarget(args.port);
    console.log(`Main target: ${target.title} — ${target.url}`);
    const session = await CdpSession.connect(target.webSocketDebuggerUrl);
    await session.send("Runtime.enable");
    await session.send("Page.bringToFront").catch(() => {});
    // A confirm()/alert() blocks the page, and CEF draws it where CDP can't
    // answer it, so every later command hangs (spec §11.1). In this page, and
    // any it reloads into, they don't open: confirm answers Cancel.
    await session.send("Page.enable").catch(() => {});
    await session.send("Page.addScriptToEvaluateOnNewDocument", { source: NO_DIALOGS }).catch(() => {});
    await session.evaluate(NO_DIALOGS);

    // The names themselves never reach the manifest or the console: only how
    // many pairs there were and how many replacements were made.
    const redaction = args.redact ? { pairs: machinePairs(undefined, args.redactExtra), replacements: 0 } : null;
    console.log(redaction ? `Redacting ${redaction.pairs.length} name(s) before each capture.` : "Redaction is off (--no-redact).");

    // A suite may prepare the instance first (the agent suite seeds agents
    // and installs its scripted CLI); a failed setup ends the run.
    if (suite.setup) {
        console.log("Setting up the suite...");
        await suite.setup(session);
    }

    const manifest = [];
    const failures = [];

    for (let i = 0; i < selected.length; i++) {
        const shot = selected[i];
        const n = String(i + 1).padStart(2, "0");
        process.stdout.write(`[${n}/${selected.length}] ${shot.id} ... `);
        const variants = shot.sizes ? args.sizes : [null];
        // A shot may allow `retries` (UI timing makes the odd attempt fail);
        // each attempt is cleaned up before the next, and only the images of
        // the attempt that succeeded go into the manifest.
        const attempts = (shot.retries ?? 0) + 1;
        let entries = [];
        let lastErr = null;
        let stop = false;
        for (let attempt = 1; attempt <= attempts; attempt++) {
            entries = [];
            lastErr = null;
            try {
                if (shot.prep) await shot.prep(session);
                await session.wait(shot.settleMs ?? 400);
                await checkVerify(session, shot);

                // One capture, or one per size: the viewport is set to the size
                // and the selector re-measured, so each crop fits that layout.
                for (const size of variants) {
                    if (size) {
                        await session.setViewport(SIZES[size]);
                        await session.wait(shot.sizeSettleMs ?? 600);
                    }
                    const pngBuffer = await captureOne(session, shot, redaction);
                    const filename = shotFilename(n, shot.id, size);
                    writeFileSync(join(args.out, filename), pngBuffer);
                    const { width, height } = pngDimensions(pngBuffer);
                    entries.push({
                        id: shot.id,
                        title: shot.title,
                        description: shot.description,
                        ...(size ? { size, viewport: SIZES[size] } : {}),
                        // Whether a human must check the image for this machine's
                        // data before it's published (spec §7/§8): true, false or "review".
                        ...(shot.containsWorkspaceData !== undefined ? { containsWorkspaceData: shot.containsWorkspaceData } : {}),
                        file: filename,
                        capturedAt: new Date().toISOString(),
                        width,
                        height,
                    });
                }
            } catch (err) {
                lastErr = err;
            }
            // SHOTS_DEBUG=keep leaves the shot's state in place to inspect.
            if (process.env.SHOTS_DEBUG === "keep") break;
            // Undo the shot's setup (a tab it opened, a viewport it set) even
            // when it failed, so the next attempt or shot starts from the same
            // state.
            try {
                if (shot.cleanup) await shot.cleanup(session);
                else if (shot.sizes) await session.clearViewport();
            } catch (err) {
                // State the shot left behind (a tab that didn't close) can
                // change every later shot, so a failed cleanup fails the shot
                // and ends the run (Codex on #4471).
                lastErr = new Error(`cleanup: ${err.message}`);
                stop = true;
                break;
            }
            if (!lastErr) break;
            if (attempt < attempts) process.stdout.write(`retrying (${lastErr.message}) ... `);
        }
        if (lastErr) {
            console.log(`FAILED — ${lastErr.message}${stop ? "; stopping the run" : ""}`);
            failures.push({ id: shot.id, error: lastErr.message });
        } else {
            manifest.push(...entries);
            console.log(shot.sizes ? `ok (${variants.join(", ")})` : "ok");
        }
        if (stop) break;
    }

    writeFileSync(
        join(args.out, "manifest.json"),
        JSON.stringify(
            {
                capturedAt: new Date().toISOString(),
                suite: args.suite,
                app: target.title,
                redaction: redaction ? { names: redaction.pairs.length, replacements: redaction.replacements } : "off",
                shots: manifest,
                failures,
            },
            null,
            2
        )
    );
    session.close();

    // Shots with at least one image (a failure can come from cleanup after a
    // capture, and a stopped run leaves later shots unrun).
    const done = new Set(manifest.map((m) => m.id)).size;
    console.log(`\n${done}/${selected.length} shots captured (${manifest.length} images) to ${args.out}`);
    if (failures.length) {
        console.log(`${failures.length} FAILED:`);
        for (const f of failures) console.log(`  - ${f.id}: ${f.error}`);
        process.exitCode = 1;
    }
}

main().catch((err) => {
    console.error(err);
    process.exit(1);
});
