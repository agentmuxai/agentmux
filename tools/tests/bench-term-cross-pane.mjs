#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// bench-term-cross-pane.mjs — Cross-pane input-delay benchmark.
//
// bench-term-echo.mjs's "busy" scenario measures a pane's OWN echo latency
// while IT is producing heavy output — that's same-pane backpressure, not
// the reported symptom. This script measures pane B's keystroke-echo
// latency while a DIFFERENT pane (A) floods output, to catch cross-pane
// starvation on the shared WS priority egress lane (see
// docs/analysis/ANALYSIS_CROSS_PANE_INPUT_DELAY_UNDER_OUTPUT_LOAD_2026_09_04.md).
//
// Scenario:
//   1. Open two terminal panes, A and B.
//   2. Measure B's echo latency with both panes quiet (baseline).
//   3. Flood A with continuous output (`yes`, unredirected — real PTY→WS
//      traffic, not suppressed by `> /dev/null`).
//   4. Measure B's echo latency again while A is flooding (cross-pane load).
//   5. Stop A's flood, confirm B recovers to baseline.
//
// Compare a run on unpatched `main` vs. a branch carrying the per-pane fair
// egress fix — the "cross-pane load" number is what the fix targets; it
// should land close to "baseline", not blow up the way the naive-FIFO
// egress lane does under a sustained flood from a different pane.
//
// Spec: docs/specs/SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md (baseline harness this borrows from)
// Auth: docs/specs/SPEC_TEST_API_ACCESS.md §5

import { WebSocket } from "ws";
import { readFileSync, writeFileSync, readdirSync, statSync } from "fs";
import { resolve, join } from "path";
import { homedir } from "os";
import { randomUUID } from "crypto";
import { execSync } from "child_process";

// ── CLI parsing ─────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
function getArg(name, fallback = undefined) {
    const i = args.indexOf(name);
    return i !== -1 ? args[i + 1] : fallback;
}

if (args.includes("--help")) {
    console.log(`
node tools/tests/bench-term-cross-pane.mjs [options]

  --ws-url <url>       WS endpoint (default: from authkey.dev)
  --auth-key <key>     Auth key   (default: from authkey.dev)
  --count <n>          Samples per scenario (default: 40)
  --warmup <n>         Warmup samples to discard (default: 5)
  --output-file <path> Save raw JSON results
  --help               Show this message
`);
    process.exit(0);
}

const COUNT = parseInt(getArg("--count", "40"), 10);
const WARMUP = parseInt(getArg("--warmup", "5"), 10);
const OUTPUT_FILE = getArg("--output-file");

// ── Auth file discovery (same convention as bench-term-echo.mjs) ────────────

function isPidAlive(pid) {
    if (!pid) return false;
    try {
        if (process.platform === "win32") {
            const out = execSync(`tasklist /FI "PID eq ${pid}" /NH /FO CSV 2>NUL`, { encoding: "utf8" });
            return out.includes(String(pid));
        }
        execSync(`kill -0 ${pid} 2>/dev/null`);
        return true;
    } catch {
        return false;
    }
}

function findAuthFile(overrideWsUrl, overrideAuthKey) {
    if (overrideWsUrl && overrideAuthKey) {
        return {
            ws_endpoint: overrideWsUrl.replace(/^wss?:\/\//, "").replace(/\/.*/, ""),
            auth_key: overrideAuthKey,
            instance: "manual",
            host_pid: 0,
        };
    }

    const home = homedir();
    const searchRoots = [join(home, ".agentmux", "dev"), join(home, ".agentmux", "versions")];
    const candidates = [];
    for (const root of searchRoots) {
        let entries;
        try {
            entries = readdirSync(root);
        } catch {
            continue;
        }
        for (const entry of entries) {
            const candidate = join(root, entry, "data", "authkey.dev");
            try {
                const st = statSync(candidate);
                candidates.push({ path: candidate, mtime: st.mtimeMs });
            } catch {
                /* not found */
            }
        }
    }

    if (candidates.length === 0) {
        throw new Error(
            `No authkey.dev found under ~/.agentmux/dev/*/data/ or ~/.agentmux/versions/*/data/.\n` +
                `Start an instance: task dev  (dev)  or launch the portable build.\n` +
                `See docs/specs/SPEC_TEST_API_ACCESS.md §5`,
        );
    }

    candidates.sort((a, b) => b.mtime - a.mtime);
    for (const { path } of candidates) {
        let auth;
        try {
            auth = JSON.parse(readFileSync(path, "utf8"));
        } catch {
            continue;
        }
        if (!isPidAlive(auth.host_pid)) continue;
        return auth;
    }

    throw new Error("Found authkey.dev file(s) but none belong to a live agentmux-cef process.");
}

// ── WS client ────────────────────────────────────────────────────────────────

function openWs(wsEndpoint, authKey) {
    const url = `ws://${wsEndpoint}/ws?authkey=${encodeURIComponent(authKey)}`;
    const ws = new WebSocket(url);
    const pending = new Map();
    const eventHandlers = [];

    ws.on("message", (raw) => {
        const outer = JSON.parse(raw.toString("utf8"));
        if (outer.eventtype !== "rpc" || !outer.data) return;
        const rpc = outer.data;
        if (rpc.command === "eventrecv") {
            for (const h of eventHandlers) h(rpc);
            return;
        }
        const matchId = rpc.resid || rpc.reqid;
        if (matchId && pending.has(matchId)) {
            const { resolve: res } = pending.get(matchId);
            pending.delete(matchId);
            res(rpc);
        }
    });

    function rpc(command, data) {
        return new Promise((res, rej) => {
            const reqid = randomUUID();
            pending.set(reqid, { resolve: res, reject: rej });
            ws.send(JSON.stringify({ wscommand: "rpc", message: { command, reqid, data } }));
        });
    }

    function rpcFire(command, data) {
        const reqid = randomUUID();
        ws.send(JSON.stringify({ wscommand: "rpc", message: { command, reqid, data } }));
    }

    function onEvent(handler) {
        eventHandlers.push(handler);
        return () => {
            const i = eventHandlers.indexOf(handler);
            if (i !== -1) eventHandlers.splice(i, 1);
        };
    }

    const ready = new Promise((res, rej) => {
        ws.on("open", res);
        ws.on("error", rej);
    });

    return { ready, rpc, rpcFire, onEvent, close: () => ws.close() };
}

// Per-block seq counters — controllerinput's reorder buffer is keyed per
// block, so each pane needs its own independent, monotonically-increasing
// sequence (mirrors TermViewModel, one instance per pane).
const seqByBlock = new Map();
function nextSeq(blockId) {
    const n = seqByBlock.get(blockId) ?? 0;
    seqByBlock.set(blockId, n + 1);
    return n;
}

function sendInputFire(client, blockId, text) {
    const inputdata64 = Buffer.from(text, "utf8").toString("base64");
    client.rpcFire("controllerinput", { blockid: blockId, inputdata64, seq: nextSeq(blockId) });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function percentile(sorted, p) {
    const idx = Math.min(Math.floor(sorted.length * p), sorted.length - 1);
    return sorted[idx];
}

function stats(samples) {
    const sorted = [...samples].sort((a, b) => a - b);
    return {
        p50: percentile(sorted, 0.5),
        p95: percentile(sorted, 0.95),
        p99: percentile(sorted, 0.99),
        min: sorted[0],
        max: sorted[sorted.length - 1],
        mean: sorted.reduce((a, b) => a + b, 0) / sorted.length,
        n: sorted.length,
    };
}

// Waits for `pattern` in the blockfile output for ONE specific blockId —
// critical for this bench, since two panes' events interleave on the same
// WS connection and a naive "any event" wait would false-positive on A's
// flood output.
function waitForPatternOnBlock(client, blockId, pattern, timeoutMs = 8000) {
    return new Promise((resolve, reject) => {
        let buf = "";
        let unsub;
        const timer = setTimeout(() => {
            unsub?.();
            reject(new Error(`Timeout waiting for "${pattern}" on block ${blockId}`));
        }, timeoutMs);
        const handler = (rpc) => {
            // rpc is the eventrecv's `data` field, i.e. the WaveEvent itself:
            // { event, scopes, data: { data64, ... } } (see bench-term-echo.mjs's
            // waitForPattern comment for the same shape, single-pane case).
            if (!rpc.data?.scopes?.includes(`block:${blockId}`)) return;
            const data64 = rpc.data?.data?.data64;
            if (data64) {
                buf += Buffer.from(data64, "base64").toString("utf8");
                if (buf.includes(pattern)) {
                    clearTimeout(timer);
                    unsub?.();
                    resolve(buf);
                }
            }
        };
        unsub = client.onEvent(handler);
    });
}

async function measureEchoLatency(client, blockId, count, warmup, label) {
    process.stdout.write(`\n=== ${label} (${count} samples, ${warmup} warmup) ===\n`);
    const runTag = Math.floor(Math.random() * 99999).toString().padStart(5, "0");
    const samples = [];

    for (let i = 0; i < count + warmup; i++) {
        const idx = i.toString().padStart(5, "0");
        const sentinel = `XPANE${idx}_r${runTag}_END`;

        // Send the command text and the Enter keypress as two separate
        // controllerinput messages, with a settle gap in between (NOT
        // included in the timed measurement). PSReadLine's own
        // syntax-highlighting re-render needs to finish processing the typed
        // text before Enter — sending "text\r" as one combined chunk races
        // that re-render and the line is never submitted, hanging the wait
        // below indefinitely. What's timed here is Enter → sentinel-in-output,
        // i.e. exactly the round trip through the shared WS egress lane this
        // bench exists to measure — the settle gap is identical across the
        // baseline/cross-pane-load/recovered scenarios, so it cancels out of
        // any before/after comparison.
        sendInputFire(client, blockId, `echo ${sentinel}`);
        await sleep(250);

        const t0 = performance.now();
        sendInputFire(client, blockId, "\r");
        await waitForPatternOnBlock(client, blockId, sentinel);
        const dt = performance.now() - t0;

        if (i >= warmup) {
            samples.push(dt);
            process.stdout.write(`\r  Sample ${i - warmup + 1}/${count} — last: ${dt.toFixed(1)} ms   `);
        }
        await sleep(30);
    }

    process.stdout.write("\n");
    const s = stats(samples);
    process.stdout.write(
        `  p50: ${s.p50.toFixed(1)} ms  p95: ${s.p95.toFixed(1)} ms  p99: ${s.p99.toFixed(1)} ms  max: ${s.max.toFixed(1)} ms\n`,
    );
    return s;
}

async function openTermPane(client) {
    const paneResp = await client.rpc("pane.open", { view: "term" });
    const blockId = paneResp.data?.block_id;
    if (!blockId) throw new Error(`pane.open failed: ${JSON.stringify(paneResp)}`);
    await client.rpc("eventsub", { event: "blockfile", scopes: [`block:${blockId}`], allscopes: false });
    // Fixed settle instead of probing for a "$" prompt: not every configured
    // shell uses a "$"-terminated prompt (e.g. a themed PowerShell prompt
    // ends in a Unicode glyph, never "$"), so a prompt-pattern probe silently
    // times out and adds pure dead time here anyway. A flat settle is honest
    // about that and cheaper.
    await sleep(3000);
    return blockId;
}

async function main() {
    const auth = findAuthFile(getArg("--ws-url"), getArg("--auth-key"));
    console.log(`\nAgentMux cross-pane input-delay benchmark`);
    console.log(`Instance: ${auth.instance}  WS: ws://${auth.ws_endpoint}/ws\n`);

    const client = openWs(auth.ws_endpoint, auth.auth_key);
    await client.ready;

    let blockA, blockB;
    try {
        process.stdout.write("Opening pane A (will be flooded)...");
        blockA = await openTermPane(client);
        process.stdout.write(` block=${blockA}\n`);

        process.stdout.write("Opening pane B (measured)...");
        blockB = await openTermPane(client);
        process.stdout.write(` block=${blockB}\n`);

        await sleep(500);

        const results = {};

        results.baseline = await measureEchoLatency(
            client,
            blockB,
            COUNT,
            WARMUP,
            "Pane B echo latency — BOTH PANES QUIET (baseline)",
        );

        process.stdout.write("\n  Flooding pane A with a continuous output loop...\n");
        // `pane.open` spawns a platform-appropriate default shell — PowerShell
        // on Windows (detect_local_shell_path_windows), $SHELL (bash/zsh) on
        // macOS/Linux (lifecycle.rs) — so the flood generator must match
        // whichever shell THIS process's OS would get, or pane A stays quiet
        // and the benchmark silently reports a false "no regression" result
        // (caught in review: a PowerShell-only command is a syntax error in
        // bash/zsh). `yes` is the canonical flood on POSIX shells; PowerShell
        // has no `yes`, so it gets an equivalent infinite-output loop. Text
        // and Enter sent separately, same PSReadLine-safety reason as
        // measureEchoLatency above (harmless no-op split on POSIX shells).
        const floodCmd = process.platform === "win32" ? "while($true){'y'}" : "yes";
        sendInputFire(client, blockA, floodCmd);
        await sleep(250);
        sendInputFire(client, blockA, "\r");
        await sleep(500);

        results.cross_pane_load = await measureEchoLatency(
            client,
            blockB,
            COUNT,
            WARMUP,
            "Pane B echo latency — PANE A FLOODING (cross-pane load)",
        );

        sendInputFire(client, blockA, "\x03"); // Ctrl+C stops `yes`
        await sleep(500);

        results.recovered = await measureEchoLatency(
            client,
            blockB,
            COUNT,
            WARMUP,
            "Pane B echo latency — AFTER A's flood stops (recovery)",
        );

        process.stdout.write("\n── Summary ─────────────────────────────────────────────\n");
        for (const [name, s] of Object.entries(results)) {
            process.stdout.write(`  ${name.padEnd(16)} p50=${s.p50.toFixed(1)} ms  p95=${s.p95.toFixed(1)} ms  p99=${s.p99.toFixed(1)} ms\n`);
        }
        const regression = results.cross_pane_load.p95 / Math.max(results.baseline.p95, 1);
        process.stdout.write(
            `\n  Cross-pane p95 is ${regression.toFixed(1)}x the quiet baseline p95.\n` +
                `  (A well-isolated egress path should stay close to 1x; a shared-FIFO\n` +
                `  egress lane starved by pane A's flood will show a large multiple.)\n`,
        );

        if (OUTPUT_FILE) {
            writeFileSync(
                OUTPUT_FILE,
                JSON.stringify(
                    { instance: auth.instance, timestamp: new Date().toISOString(), count: COUNT, warmup: WARMUP, ...results, regression_factor: regression },
                    null,
                    2,
                ),
            );
            console.log(`\nResults saved to ${resolve(OUTPUT_FILE)}`);
        }
    } finally {
        if (blockA) {
            try {
                sendInputFire(client, blockA, "\x03");
            } catch {
                /* ignore */
            }
            try {
                await client.rpc("object.DeleteBlock", { blockId: blockA });
            } catch {
                /* ignore */
            }
        }
        if (blockB) {
            try {
                await client.rpc("object.DeleteBlock", { blockId: blockB });
            } catch {
                /* ignore */
            }
        }
        client.close();
    }
}

main().catch((err) => {
    console.error("\nError:", err.message);
    process.exit(1);
});
