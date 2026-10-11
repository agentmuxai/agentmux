#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Records short clips of AgentMux being used, for the docs — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §12. The
// recording counterpart of capture.mjs: same instance, same isolation, same
// redaction, and clips declared in a suite file the way shots are, so a
// changed UI is re-recorded with one command.
//
// Usage:
//   node scripts/ui-screenshots/record.mjs [--port N] [--suite recordings] [--only id1,id2]
//       [--out DIR] [--formats gif,webp,mp4] [--width 800] [--fps 15]
//       [--ffmpeg PATH] [--redact from=to]... [--no-redact]
//
// Needs ffmpeg (--ffmpeg, $FFMPEG, or PATH). A clip: { id, title,
// description, viewport?, prep(session) → crop selector or null (not
// recorded), run(session, cursor, crop) (recorded), cleanup(session) }. The
// suite may export setup(session), run once first.

import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { CdpSession, NO_DIALOGS, findMainTarget, resolveClip } from "./lib/session.mjs";
import { Cursor, Screencast, durationOf, encode, ensureDir, findFfmpeg } from "./lib/recorder.mjs";
import { liveRedactExpression, machinePairs, parseRedactArg, stopLiveRedactionExpression } from "./redact.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const SUITES = { recordings: "./recordings.mjs" };

function parseArgs(argv) {
    const args = {
        port: Number(process.env.AGENTMUX_CDP_PORT) || 9222,
        suite: "recordings",
        formats: ["gif"],
        width: 800,
        fps: 15,
        redact: true,
        redactExtra: [],
    };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === "--port") args.port = Number(argv[++i]);
        else if (a === "--suite") args.suite = argv[++i];
        else if (a === "--only") args.only = argv[++i].split(",");
        else if (a === "--out") args.out = argv[++i];
        else if (a === "--formats") args.formats = argv[++i].split(",").map((s) => s.trim()).filter(Boolean);
        else if (a === "--width") args.width = Number(argv[++i]);
        else if (a === "--fps") args.fps = Number(argv[++i]);
        else if (a === "--ffmpeg") args.ffmpeg = argv[++i];
        else if (a === "--redact") args.redactExtra.push(parseRedactArg(argv[++i]));
        else if (a === "--no-redact") args.redact = false;
    }
    if (!SUITES[args.suite]) throw new Error(`unknown suite "${args.suite}" (known: ${Object.keys(SUITES).join(", ")})`);
    if (!args.out) args.out = join(__dirname, "..", "..", "docs", "manual", "recordings", `${args.suite}-${new Date().toISOString().replace(/[:.]/g, "-")}`);
    return args;
}

async function main() {
    const args = parseArgs(process.argv.slice(2));
    const ffmpeg = findFfmpeg(args.ffmpeg);
    const suite = await import(SUITES[args.suite]);
    const selected = args.only ? suite.recordings.filter((r) => args.only.includes(r.id)) : suite.recordings;
    if (selected.length === 0) throw new Error("no recordings selected — check --only");
    ensureDir(args.out);

    const target = await findMainTarget(args.port);
    console.log(`Main target: ${target.title}`);
    const session = await CdpSession.connect(target.webSocketDebuggerUrl);
    await session.send("Runtime.enable");
    await session.send("Page.enable").catch(() => {});
    await session.send("Page.addScriptToEvaluateOnNewDocument", { source: NO_DIALOGS }).catch(() => {});
    await session.evaluate(NO_DIALOGS);
    await session.send("Page.bringToFront").catch(() => {});
    const pairs = args.redact ? machinePairs(undefined, args.redactExtra) : [];
    // Redaction runs throughout a clip, not only before it: text the clip
    // brings up (a tab's content, a tooltip) is redacted before it's painted.
    const startRedaction = () => (pairs.length ? session.evaluate(liveRedactExpression(pairs)) : null);
    const stopRedaction = () => session.evaluate(stopLiveRedactionExpression).catch(() => {});

    if (suite.setup) {
        console.log("Setting up the suite...");
        await suite.setup(session);
    }

    const manifest = [];
    const failures = [];
    for (const rec of selected) {
        process.stdout.write(`${rec.id} ... `);
        const cursor = new Cursor(session);
        const cast = new Screencast(session);
        try {
            if (rec.viewport) await session.setViewport(rec.viewport);
            const cropSelector = rec.prep ? await rec.prep(session) : null;
            await startRedaction();
            await cursor.show(rec.cursorStart ?? { x: 60, y: 60 });
            await session.wait(300);
            await cast.start();
            await session.wait(500);
            await rec.run(session, cursor, cropSelector);
            await session.wait(rec.endHoldMs ?? 800);
            const { frames, end } = await cast.stop();
            // The crop is measured at the end, so it fits what the clip ends on.
            const crop = cropSelector ? await resolveClip(session, cropSelector, rec.padding ?? 0) : null;
            await cursor.hide();
            await stopRedaction();
            const files = encode({ frames, end, crop, out: join(args.out, rec.id), formats: args.formats, width: args.width, fps: args.fps, ffmpeg });
            const seconds = durationOf(ffmpeg, files[0]);
            manifest.push({ id: rec.id, title: rec.title, description: rec.description, files: files.map((f) => f.split(/[\\/]/).pop()), frames: frames.length, seconds });
            console.log(`ok (${frames.length} frames, ${seconds?.toFixed(1) ?? "?"} s)`);
        } catch (err) {
            await cast.stop().catch(() => {});
            await cursor.hide().catch(() => {});
            await stopRedaction();
            failures.push({ id: rec.id, error: err.message });
            console.log(`FAILED — ${err.message}`);
        }
        try {
            if (rec.cleanup) await rec.cleanup(session);
            if (rec.viewport) await session.clearViewport();
        } catch (err) {
            failures.push({ id: rec.id, error: `cleanup: ${err.message}` });
            console.log(`cleanup FAILED — ${err.message}; stopping`);
            break;
        }
    }

    writeFileSync(
        join(args.out, "manifest.json"),
        JSON.stringify({ recordedAt: new Date().toISOString(), suite: args.suite, app: target.title, redaction: pairs.length ? { names: pairs.length } : "off", recordings: manifest, failures }, null, 2)
    );
    session.close();
    console.log(`\n${manifest.length}/${selected.length} recorded to ${args.out}`);
    if (failures.length) process.exitCode = 1;
}

main().catch((err) => {
    console.error(err);
    process.exit(1);
});
