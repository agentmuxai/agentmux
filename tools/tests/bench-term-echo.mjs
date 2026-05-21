#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// bench-term-echo.mjs — Terminal input echo-latency benchmark.
//
// Two test modes:
//
//   Echo latency (default): Sends `echo SENTINEL\r` via controllerinput+seq
//   (same path as frontend TermViewModel). Times from RPC dispatch until the
//   sentinel appears in PTY output = full input→PTY→echo round-trip.
//
//   Stream throughput (--stream): Sends STREAM_LEN 'a' characters one at a time
//   with CHAR_DELAY ms between each (default 0 = as fast as possible). Bookended
//   by unique sentinels to measure full-burst latency and detect ordering violations.
//
// Spec: docs/specs/SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md
// Auth: docs/specs/SPEC_TEST_API_ACCESS.md §5

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
  --stream             Run character-stream throughput test too
  --stream-len <n>     Characters per stream burst (default: 200)
  --char-delay <ms>    Delay between chars in stream test (default: 0)
  --output-file <path> Save raw JSON results
  --help               Show this message

Both --busy and --stream test the controllerinput+seq path (same as the
frontend TermViewModel), NOT the blockinput path.
`);
    process.exit(0);
}

const COUNT      = parseInt(getArg("--count",      "60"),  10);
const WARMUP     = parseInt(getArg("--warmup",      "5"),   10);
const RUN_BUSY   = hasFlag("--busy");
const RUN_STREAM = hasFlag("--stream");
const STREAM_LEN = parseInt(getArg("--stream-len", "200"), 10);
const CHAR_DELAY = parseInt(getArg("--char-delay", "0"),   10);
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

// ── WS client ────────────────────────────────────────────────────────────────

function openWs(wsEndpoint, authKey) {
    const url = `ws://${wsEndpoint}/ws?authkey=${encodeURIComponent(authKey)}`;
    const ws = new WebSocket(url);
    const pending = new Map();
    const eventHandlers = [];

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

    // Fire-and-forget RPC — no response awaited.
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

    function close() { ws.close(); }

    const ready = new Promise((resolve, reject) => {
        ws.on("open", resolve);
        ws.on("error", reject);
    });

    return { ready, rpc, rpcFire, onEvent, close };
}

// ── Seq counter (per-session, mirrors TermViewModel.inputSeq) ────────────────

let inputSeq = 0;
function resetSeq() { inputSeq = 0; }

// Send via controllerinput RPC with seq (fire-and-forget).
function sendInputFire(client, blockId, text) {
    const inputdata64 = Buffer.from(text, "utf8").toString("base64");
    client.rpcFire("controllerinput", { blockid: blockId, inputdata64, seq: inputSeq++ });
}

// Send via controllerinput RPC with seq, awaiting the response.
async function sendInputAwaited(client, blockId, text) {
    const inputdata64 = Buffer.from(text, "utf8").toString("base64");
    return client.rpc("controllerinput", { blockid: blockId, inputdata64, seq: inputSeq++ });
}

// ── Timing helpers ─────────────────────────────────────────────────────────

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

// ── Wait for pattern in PTY output ──────────────────────────────────────────

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

// ── Echo-latency scenario ────────────────────────────────────────────────────
//
// Sends `echo SENTINEL\r` via controllerinput+seq. Times from RPC dispatch
// until the sentinel appears in PTY output (full input→PTY→echo round-trip).
// Sentinel: zero-padded index + random run tag — no sentinel is a substring
// of another in the same run.

async function measureEchoLatency(client, blockId, count, warmup, label) {
    process.stdout.write(`\n=== ${label} (${count} samples, ${warmup} warmup) ===\n`);
    process.stdout.write("  Waiting for shell prompt...");

    await sendInputAwaited(client, blockId, "\r");
    await waitForPattern(client, "$", 8000).catch(() => {});
    process.stdout.write(" ok\n");

    const runTag = Math.floor(Math.random() * 99999).toString().padStart(5, "0");
    const allSamples = [];

    for (let i = 0; i < count + warmup; i++) {
        const idx = i.toString().padStart(5, "0");
        const sentinel = `BENCH${idx}_r${runTag}_END`;

        const t0 = performance.now();
        sendInputFire(client, blockId, `echo ${sentinel}\r`);
        await waitForPattern(client, sentinel, 5000);
        const dt = performance.now() - t0;

        if (i >= warmup) {
            allSamples.push(dt);
            process.stdout.write(
                `\r  Sample ${i - warmup + 1}/${count} — last: ${dt.toFixed(1)} ms   `
            );
        }

        await sleep(30);
    }

    process.stdout.write("\n");
    const s = stats(allSamples);
    process.stdout.write(
        `  p50: ${s.p50.toFixed(1)} ms  p95: ${s.p95.toFixed(1)} ms  ` +
        `p99: ${s.p99.toFixed(1)} ms  max: ${s.max.toFixed(1)} ms\n`
    );
    return s;
}

// ── Character-stream scenario ────────────────────────────────────────────────
//
// Simulates holding down a key: sends STREAM_LEN 'a' chars one at a time via
// controllerinput+seq at CHAR_DELAY ms/char (default 0 = as fast as possible).
// Bookended by unique alphanumeric sentinels for timing and ordering detection.
// Ctrl+C after each burst clears the typed line.

async function measureStreamThroughput(client, blockId, streamLen, count, warmup, label) {
    process.stdout.write(`\n=== ${label} — char stream (${streamLen}×'a' at ${CHAR_DELAY}ms/char, ${count} bursts) ===\n`);
    process.stdout.write(`  Tip: watch the terminal — you should see a stream of 'a' chars appearing one by one.\n`);

    const allTimes = [];
    const allViolations = [];

    for (let burst = 0; burst < count + warmup; burst++) {
        const tag = `T${burst.toString().padStart(3,"0")}X${Math.floor(Math.random()*99999).toString().padStart(5,"0")}`;
        const startSentinel = `Z${tag}Z`;
        const endSentinel   = `Y${tag}Y`;

        let capturedOutput = "";
        const unsub = client.onEvent((msg) => {
            if (msg.data?.data64) {
                capturedOutput += Buffer.from(msg.data.data64, "base64").toString("utf8");
            }
        });

        for (const ch of startSentinel) {
            sendInputFire(client, blockId, ch);
            if (CHAR_DELAY > 0) await sleep(CHAR_DELAY);
        }

        const t0 = performance.now();
        for (let i = 0; i < streamLen; i++) {
            sendInputFire(client, blockId, "a");
            if (CHAR_DELAY > 0) await sleep(CHAR_DELAY);
        }

        for (const ch of endSentinel) {
            sendInputFire(client, blockId, ch);
            if (CHAR_DELAY > 0) await sleep(CHAR_DELAY);
        }

        await new Promise((resolve, reject) => {
            const timer = setTimeout(() => { unsub(); reject(new Error(`Stream timeout (${endSentinel})`)); }, 15000);
            const check = setInterval(() => {
                if (capturedOutput.includes(endSentinel)) {
                    clearInterval(check); clearTimeout(timer); unsub(); resolve();
                }
            }, 3);
        });
        const dt = performance.now() - t0;

        let violations = 0;
        const si = capturedOutput.indexOf(startSentinel);
        const ei = capturedOutput.indexOf(endSentinel);
        if (si !== -1 && ei > si) {
            const body = capturedOutput.slice(si + startSentinel.length, ei)
                .replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
            const printable = body.split("").filter(c => /[a-zA-Z]/.test(c));
            violations = printable.filter(c => c !== "a").length;
        }

        if (burst >= warmup) {
            allTimes.push(dt);
            allViolations.push(violations);
            const msPerChar = (dt / streamLen).toFixed(1);
            process.stdout.write(
                `\r  Burst ${burst - warmup + 1}/${count} — ` +
                `${dt.toFixed(0)} ms total  ${msPerChar} ms/char  violations: ${violations}/${streamLen}   `
            );
        }

        sendInputFire(client, blockId, "\x03");
        await sleep(200);
    }

    process.stdout.write("\n");
    const s = stats(allTimes);
    const totalViolations = allViolations.reduce((a, b) => a + b, 0);
    const msPerChar = (s.p50 / streamLen).toFixed(1);
    process.stdout.write(
        `  p50: ${s.p50.toFixed(0)} ms total  ${msPerChar} ms/char  ` +
        `p95: ${s.p95.toFixed(0)} ms  ` +
        `order violations: ${totalViolations}/${count * streamLen} chars\n`
    );
    if (totalViolations > 0) {
        process.stdout.write(`  ⚠  ${totalViolations} ordering violations — seq reorder buffer may have regressed\n`);
    } else {
        process.stdout.write(`  ✓  Zero ordering violations\n`);
    }
    return { ...s, violations: allViolations, total_violations: totalViolations };
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
    const auth = findAuthFile(getArg("--ws-url"), getArg("--auth-key"));
    const wsEndpoint = auth.ws_endpoint;
    const authKey = auth.auth_key;

    console.log(`\nAgentMux terminal echo-latency benchmark`);
    console.log(`Instance: ${auth.instance}  WS: ws://${wsEndpoint}/ws`);
    console.log(`Path: controllerinput RPC + seq (same as frontend TermViewModel)\n`);

    const client = openWs(wsEndpoint, authKey);
    await client.ready;

    let blockId;
    try {
        process.stdout.write("Opening terminal pane...");
        resetSeq();
        const paneResp = await client.rpc("pane.open", { view: "term" });
        blockId = paneResp.data?.block_id;
        if (!blockId) throw new Error(`pane.open failed: ${JSON.stringify(paneResp)}`);
        process.stdout.write(` block=${blockId}\n`);

        await client.rpc("eventsub", {
            event: "blockfile",
            scopes: [`block:${blockId}`],
            allscopes: false,
        });

        await sleep(1200);

        const results = {};

        // ── Quiet echo latency ──────────────────────────────────────────────
        results.quiet = await measureEchoLatency(
            client, blockId, COUNT, WARMUP,
            "Quiet terminal — echo latency"
        );

        // ── Busy echo latency ───────────────────────────────────────────────
        if (RUN_BUSY) {
            process.stdout.write("\n  Launching background load (seq 1 50000)...\n");
            sendInputFire(client, blockId, "seq 1 50000 > /dev/null &\r");
            await sleep(200);

            results.busy = await measureEchoLatency(
                client, blockId, COUNT, WARMUP,
                `Busy terminal (concurrent seq 1 50000) — echo latency`
            );

            sendInputFire(client, blockId, "kill %1 2>/dev/null; wait\r");
            await sleep(300);
        }

        // ── Stream throughput ───────────────────────────────────────────────
        if (RUN_STREAM) {
            results.stream_quiet = await measureStreamThroughput(
                client, blockId, STREAM_LEN, COUNT, WARMUP,
                "Quiet terminal"
            );

            if (RUN_BUSY) {
                process.stdout.write("\n  Launching background load for stream test...\n");
                sendInputFire(client, blockId, "seq 1 50000 > /dev/null &\r");
                await sleep(200);

                results.stream_busy = await measureStreamThroughput(
                    client, blockId, STREAM_LEN, COUNT, WARMUP,
                    "Busy terminal"
                );

                sendInputFire(client, blockId, "kill %1 2>/dev/null; wait\r");
                await sleep(300);
            }
        }

        // ── Summary ─────────────────────────────────────────────────────────
        process.stdout.write("\n── Summary ─────────────────────────────────────────────\n");
        process.stdout.write(
            `  quiet: p50=${results.quiet.p50.toFixed(1)} ms  ` +
            `p95=${results.quiet.p95.toFixed(1)} ms  p99=${results.quiet.p99.toFixed(1)} ms\n`
        );
        if (results.busy) {
            process.stdout.write(
                `  busy:  p50=${results.busy.p50.toFixed(1)} ms  ` +
                `p95=${results.busy.p95.toFixed(1)} ms  p99=${results.busy.p99.toFixed(1)} ms\n`
            );
        }
        if (results.stream_quiet) {
            const mspc = (results.stream_quiet.p50 / STREAM_LEN).toFixed(1);
            process.stdout.write(
                `  stream: p50=${results.stream_quiet.p50.toFixed(0)} ms  ${mspc} ms/char  ` +
                `violations=${results.stream_quiet.total_violations}/${COUNT * STREAM_LEN}\n`
            );
        }

        if (OUTPUT_FILE) {
            const payload = {
                instance: auth.instance,
                timestamp: new Date().toISOString(),
                ws_endpoint: wsEndpoint,
                count: COUNT,
                warmup: WARMUP,
                stream_len: STREAM_LEN,
                char_delay: CHAR_DELAY,
                ...results,
            };
            writeFileSync(OUTPUT_FILE, JSON.stringify(payload, null, 2));
            console.log(`\nResults saved to ${resolve(OUTPUT_FILE)}`);
        }

    } finally {
        if (blockId) {
            try { await client.rpc("object.DeleteBlock", { blockId }); } catch { /* ignore */ }
        }
        client.close();
    }
}

main().catch((err) => {
    console.error("\nError:", err.message);
    process.exit(1);
});
