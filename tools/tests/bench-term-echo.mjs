#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// bench-term-echo.mjs — Terminal input echo-latency benchmark.
//
// Measures the wall-clock interval from when a command is sent into a
// PTY via the AgentMux App API until its echo appears in the PTY output
// stream. Uses the sentinel-echo pattern (echo __BENCH_N__\r) to avoid
// false matches from concurrent output.
//
// Requires a running dev instance (task dev). Auth is read from
// ~/.agentmux/dev/<branch>/data/authkey.dev — the same file used by
// the PowerShell harnesses in this directory.
//
// Spec: docs/specs/SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md
// Auth: docs/specs/SPEC_TEST_API_ACCESS.md §5
//
// Usage: node tools/tests/bench-term-echo.mjs [options]
//   --ws-url <url>       Override WS endpoint
//   --auth-key <key>     Override auth key
//   --count <n>          Samples per scenario (default 60)
//   --warmup <n>         Warmup samples to discard (default 5)
//   --busy               Also run busy-terminal scenario
//   --output-file <path> Write raw JSON results here
//   --help               Show this message

import { WebSocket } from "ws";
import { readFileSync, writeFileSync, readdirSync, statSync } from "fs";
import { join, resolve } from "path";
import { homedir } from "os";
import { randomUUID } from "crypto";
import { execSync } from "child_process";

// ── CLI parsing ─────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
function getArg(name, fallback = undefined) {
    const i = args.indexOf(name);
    return i !== -1 ? args[i + 1] : fallback;
}
function hasFlag(name) { return args.includes(name); }

if (hasFlag("--help")) {
    console.log(`
node tools/tests/bench-term-echo.mjs [options]

  --ws-url <url>       WS endpoint (default: from authkey.dev)
  --auth-key <key>     Auth key   (default: from authkey.dev)
  --count <n>          Samples per scenario (default: 60)
  --warmup <n>         Warmup samples to discard (default: 5)
  --busy               Run busy-terminal scenario too
  --output-file <path> Save raw JSON results
  --help               Show this message

Requires a running dev instance started with \`task dev\`.
Auth is read automatically from ~/.agentmux/dev/<branch>/data/authkey.dev.
`);
    process.exit(0);
}

const COUNT = parseInt(getArg("--count", "60"), 10);
const WARMUP = parseInt(getArg("--warmup", "5"), 10);
const RUN_BUSY = hasFlag("--busy");
const OUTPUT_FILE = getArg("--output-file");

// ── Auth file discovery ──────────────────────────────────────────────────────

function findAuthFile(overrideWsUrl, overrideAuthKey) {
    if (overrideWsUrl && overrideAuthKey) {
        return { ws_endpoint: overrideWsUrl.replace(/^wss?:\/\//, "").replace(/\/.*/, ""), auth_key: overrideAuthKey, instance: "manual", host_pid: 0 };
    }

    const home = homedir();
    const searchRoots = [
        join(home, ".agentmux", "dev"),
        join(home, ".agentmux", "versions"),
    ];

    const candidates = [];
    for (const root of searchRoots) {
        let entries;
        try { entries = readdirSync(root); } catch { continue; }
        for (const entry of entries) {
            const candidate = join(root, entry, "data", "authkey.dev");
            try {
                const st = statSync(candidate);
                candidates.push({ path: candidate, mtime: st.mtimeMs });
            } catch { /* not found */ }
        }
    }

    if (candidates.length === 0) {
        throw new Error(
            `No authkey.dev found under ~/.agentmux/dev/*/data/ or ~/.agentmux/versions/*/data/.\n` +
            `Start an instance: task dev  (dev)  or launch the portable build.\n` +
            `See docs/specs/SPEC_TEST_API_ACCESS.md §5`
        );
    }

    // Newest first, pick the first one whose host_pid is alive.
    candidates.sort((a, b) => b.mtime - a.mtime);
    for (const { path } of candidates) {
        let auth;
        try { auth = JSON.parse(readFileSync(path, "utf8")); } catch { continue; }
        if (!isPidAlive(auth.host_pid)) {
            process.stderr.write(`Stale authkey.dev (pid ${auth.host_pid} dead): ${path}\n`);
            continue;
        }
        const mode = path.includes(`${join("", ".agentmux", "dev")}`) ? "dev" : "portable";
        process.stderr.write(`Using authkey.dev: ${path} (instance=${auth.instance}, pid=${auth.host_pid}, mode=${mode})\n`);
        return auth;
    }

    throw new Error("Found authkey.dev file(s) but none belong to a live agentmux-cef process.");
}

function isPidAlive(pid) {
    if (!pid) return false;
    try {
        if (process.platform === "win32") {
            const out = execSync(`tasklist /FI "PID eq ${pid}" /NH /FO CSV 2>NUL`, { encoding: "utf8" });
            return out.includes(String(pid));
        } else {
            execSync(`kill -0 ${pid} 2>/dev/null`);
            return true;
        }
    } catch { return false; }
}

// ── WS helpers ───────────────────────────────────────────────────────────────

function openWs(wsEndpoint, authKey) {
    const url = `ws://${wsEndpoint}/ws?authkey=${encodeURIComponent(authKey)}`;
    const ws = new WebSocket(url);
    const pending = new Map();      // reqid → {resolve, reject}
    const eventHandlers = [];       // (msg) => void

    ws.on("message", (raw) => {
        const msg = JSON.parse(raw.toString("utf8"));
        if (msg.wscommand === "eventrecv") {
            for (const h of eventHandlers) h(msg);
            return;
        }
        if (msg.reqid && pending.has(msg.reqid)) {
            const { resolve } = pending.get(msg.reqid);
            pending.delete(msg.reqid);
            resolve(msg);
        }
    });

    function rpc(command, data) {
        return new Promise((resolve, reject) => {
            const reqid = randomUUID();
            pending.set(reqid, { resolve, reject });
            ws.send(JSON.stringify({ wscommand: "rpc", message: { command, reqid, data } }));
        });
    }

    function sendInput(blockId, text) {
        const inputdata64 = Buffer.from(text, "utf8").toString("base64");
        ws.send(JSON.stringify({ wscommand: "blockinput", blockid: blockId, inputdata64 }));
    }

    function onEvent(handler) {
        eventHandlers.push(handler);
        return () => {
            const i = eventHandlers.indexOf(handler);
            if (i !== -1) eventHandlers.splice(i, 1);
        };
    }

    function close() { ws.close(); }

    const ready = new Promise((resolve, reject) => {
        ws.on("open", resolve);
        ws.on("error", reject);
    });

    return { ready, rpc, sendInput, onEvent, close };
}

// ── Timing helpers ────────────────────────────────────────────────────────────

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function percentile(sorted, p) {
    const idx = Math.min(Math.floor(sorted.length * p), sorted.length - 1);
    return sorted[idx];
}

function stats(samples) {
    const sorted = [...samples].sort((a, b) => a - b);
    return {
        p50: percentile(sorted, 0.50),
        p95: percentile(sorted, 0.95),
        p99: percentile(sorted, 0.99),
        min: sorted[0],
        max: sorted[sorted.length - 1],
        mean: sorted.reduce((a, b) => a + b, 0) / sorted.length,
        n: sorted.length,
        raw: sorted,
    };
}

// ── Benchmark core ────────────────────────────────────────────────────────────

async function waitForPattern(client, pattern, timeoutMs = 5000) {
    return new Promise((resolve, reject) => {
        let buf = "";
        let unsub;
        const timer = setTimeout(() => {
            unsub?.();
            reject(new Error(`Timeout waiting for: ${pattern}`));
        }, timeoutMs);
        const handler = (msg) => {
            if (msg.data?.data64) {
                buf += Buffer.from(msg.data.data64, "base64").toString("utf8");
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
    process.stdout.write("  Waiting for shell prompt...");

    // Wait for initial prompt
    client.sendInput(blockId, "\r");
    await waitForPattern(client, "$", 8000).catch(() => {});
    process.stdout.write(" ok\n");

    const allSamples = [];

    for (let i = 0; i < count + warmup; i++) {
        const sentinel = `__BENCH_${i}__`;

        // Record send time, then send
        const t0 = performance.now();
        client.sendInput(blockId, `echo ${sentinel}\r`);

        // Wait until sentinel appears in PTY output
        await waitForPattern(client, sentinel, 5000);
        const dt = performance.now() - t0;

        if (i >= warmup) {
            allSamples.push(dt);
            process.stdout.write(`\r  Sample ${i - warmup + 1}/${count} ...`);
        }

        // Small gap between samples to let shell settle
        await sleep(50);
    }

    process.stdout.write("\n");
    const s = stats(allSamples);
    process.stdout.write(
        `  p50: ${s.p50.toFixed(1)} ms  ` +
        `p95: ${s.p95.toFixed(1)} ms  ` +
        `p99: ${s.p99.toFixed(1)} ms  ` +
        `max: ${s.max.toFixed(1)} ms\n`
    );
    return s;
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
    const auth = findAuthFile(getArg("--ws-url"), getArg("--auth-key"));
    const wsEndpoint = auth.ws_endpoint;
    const authKey = auth.auth_key;

    console.log(`\nAgentMux terminal echo-latency benchmark`);
    console.log(`Instance: ${auth.instance}  WS: ws://${wsEndpoint}/ws`);

    const client = openWs(wsEndpoint, authKey);
    await client.ready;

    let blockId;
    try {
        // Open a fresh terminal pane
        process.stdout.write("\nOpening terminal pane...");
        const paneResp = await client.rpc("pane.open", { view: "term" });
        blockId = paneResp.data?.block_id;
        if (!blockId) throw new Error(`pane.open failed: ${JSON.stringify(paneResp)}`);
        process.stdout.write(` block=${blockId}\n`);

        // Subscribe to PTY output events
        await client.rpc("eventsub", {
            event: "blockfile",
            scopes: [`block:${blockId}`],
            allscopes: false,
        });

        // Allow the PTY to initialise (shell startup)
        await sleep(1200);

        const results = {};

        // ── Quiet scenario ─────────────────────────────────
        results.quiet = await measureEchoLatency(
            client, blockId, COUNT, WARMUP,
            "Quiet terminal"
        );

        // ── Busy scenario ──────────────────────────────────
        if (RUN_BUSY) {
            process.stdout.write("\n  Launching background load (seq 1 50000)...\n");
            client.sendInput(blockId, "seq 1 50000 > /dev/null &\r");
            await sleep(200);

            results.busy = await measureEchoLatency(
                client, blockId, COUNT, WARMUP,
                "Busy terminal (concurrent seq 1 50000)"
            );

            // Kill any leftover background jobs
            client.sendInput(blockId, "kill %1 2>/dev/null; wait\r");
            await sleep(300);
        }

        // ── Results ───────────────────────────────────────
        if (OUTPUT_FILE) {
            const payload = {
                instance: auth.instance,
                timestamp: new Date().toISOString(),
                ws_endpoint: wsEndpoint,
                count: COUNT,
                warmup: WARMUP,
                ...results,
            };
            writeFileSync(OUTPUT_FILE, JSON.stringify(payload, null, 2));
            console.log(`\nResults saved to ${resolve(OUTPUT_FILE)}`);
        }

    } finally {
        // Best-effort pane cleanup — don't leave dangling terminal panes
        if (blockId) {
            try {
                await client.rpc("object.DeleteBlock", { blockId });
            } catch { /* ignore cleanup errors */ }
        }
        client.close();
    }
}

main().catch((err) => {
    console.error("\nError:", err.message);
    process.exit(1);
});
