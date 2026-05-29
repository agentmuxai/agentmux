#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// bench-agent-keystroke.mjs — Agent composer keystroke-latency benchmark.
//
// Counterpart to bench-term-echo.mjs. Measures the per-keystroke cost of
// the agent pane's textarea handler + RAF-coalesced scroll. Drives
// synthetic keystrokes via the Chrome DevTools Protocol (CDP) and reads
// the `agent-keystroke:start` / `agent-keystroke:scheduled` perf-mark
// pair AgentFooter.tsx emits per keystroke.
//
// What it measures (per SPEC_INPUT_RESPONSIVENESS §7.1):
//   - agent-keystroke    — onInput handler synchronous cost (target P95 < 5 ms)
//   - agent-input-raf-cb — coalesced scroll callback cost     (target P95 < 5 ms)
//   - End-to-end keystroke → RAF callback complete            (target P95 < 50 ms)
//
// Prerequisites:
//   1. AgentMux dev or portable build running locally.
//   2. CDP debug port reachable on 127.0.0.1:9223 (dev) or :9222 (release).
//      `task dev` enables this by default.
//   3. An agent pane open and visible (the bench finds it via
//      `document.querySelector('.agent-input')`). Use `pane.open` from
//      the App API or open one manually before running.
//   4. The textarea must accept input. The bench does NOT submit — it
//      types into the textarea and clears it at the end, never firing
//      an agent invocation (zero token cost).
//
// CDP-driven, NOT App-API-driven. We don't have an `agent.synthetic_input`
// RPC and don't want to add one just for benching. Input.dispatchKeyEvent
// goes through the full DOM keydown → input → handler pipeline, which is
// what the spec's "end-to-end" measurement calls for.
//
// Spec: docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md §7.2
// Plan: docs/specs/PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md §Item 3

import { WebSocket } from "ws";
import { writeFileSync } from "fs";

// ── CLI parsing ─────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
function getArg(name, fallback = undefined) {
    const i = args.indexOf(name);
    return i !== -1 ? args[i + 1] : fallback;
}
function hasFlag(name) { return args.includes(name); }

if (hasFlag("--help")) {
    console.log(`
node tools/tests/bench-agent-keystroke.mjs [options]

  --cdp-port <port>       CEF debug port (default: 9223 dev, fallback 9222)
  --count <n>             Keystrokes to sample (default: 200)
  --warmup <n>            Warmup keystrokes to discard (default: 10)
  --inter-key-ms <ms>     Pause between keystrokes (default: 50)
  --p95-threshold-ms <n>  Exit non-zero if end-to-end P95 exceeds this (default: 50)
  --output-file <path>    Save raw JSON results
  --no-cleanup            Leave the typed text in the textarea (default: clear at end)
  --help                  Show this message

Prereq: AgentMux must be running with an agent pane visible
(.agent-input must exist in the DOM).
`);
    process.exit(0);
}

const CDP_PORT = parseInt(getArg("--cdp-port", "9223"), 10);
const COUNT = parseInt(getArg("--count", "200"), 10);
const WARMUP = parseInt(getArg("--warmup", "10"), 10);
const INTER_KEY_MS = parseInt(getArg("--inter-key-ms", "50"), 10);
const P95_THRESHOLD_MS = parseInt(getArg("--p95-threshold-ms", "50"), 10);
const OUTPUT_FILE = getArg("--output-file");
const NO_CLEANUP = hasFlag("--no-cleanup");

// ── CDP target discovery ────────────────────────────────────────────────────

async function fetchTargets(port) {
    const res = await fetch(`http://127.0.0.1:${port}/json`).catch((err) => {
        throw new Error(`Could not reach CDP on port ${port}: ${err.message}. Is AgentMux running?`);
    });
    if (!res.ok) throw new Error(`CDP /json returned HTTP ${res.status}`);
    return res.json();
}

async function findMainPageTarget(port) {
    const targets = await fetchTargets(port);
    // CEF exposes one page per webview; we want the main renderer.
    const pageTargets = targets.filter((t) => t.type === "page");
    if (pageTargets.length === 0) {
        throw new Error(`No 'page' targets on CDP :${port}. Targets: ${JSON.stringify(targets, null, 2)}`);
    }
    // If multiple, prefer one with a webSocketDebuggerUrl AND a title
    // suggesting the main shell. Heuristic: take the first.
    return pageTargets[0];
}

// ── CDP session (minimal protocol client) ───────────────────────────────────

class CdpSession {
    constructor(wsUrl) {
        this.wsUrl = wsUrl;
        this.ws = null;
        this.nextId = 1;
        this.pending = new Map(); // id -> { resolve, reject }
    }

    async connect() {
        await new Promise((resolve, reject) => {
            this.ws = new WebSocket(this.wsUrl, { perMessageDeflate: false, maxPayload: 64 * 1024 * 1024 });
            this.ws.on("open", resolve);
            this.ws.on("error", reject);
            this.ws.on("message", (data) => this.onMessage(data));
            this.ws.on("close", () => {
                for (const { reject } of this.pending.values()) reject(new Error("CDP socket closed"));
                this.pending.clear();
            });
        });
    }

    onMessage(data) {
        const msg = JSON.parse(data.toString());
        if (msg.id == null) return; // ignore events
        const pending = this.pending.get(msg.id);
        if (!pending) return;
        this.pending.delete(msg.id);
        if (msg.error) pending.reject(new Error(`CDP ${msg.error.code}: ${msg.error.message}`));
        else pending.resolve(msg.result);
    }

    async send(method, params = {}) {
        const id = this.nextId++;
        return new Promise((resolve, reject) => {
            this.pending.set(id, { resolve, reject });
            this.ws.send(JSON.stringify({ id, method, params }));
        });
    }

    async close() {
        if (this.ws) {
            this.ws.close();
            this.ws = null;
        }
    }
}

// ── Bench setup ─────────────────────────────────────────────────────────────

async function evaluate(session, expression, awaitPromise = false) {
    const res = await session.send("Runtime.evaluate", {
        expression,
        awaitPromise,
        returnByValue: true,
    });
    if (res.exceptionDetails) {
        throw new Error(`Runtime.evaluate failed: ${res.exceptionDetails.text} — ${JSON.stringify(res.exceptionDetails)}`);
    }
    return res.result?.value;
}

async function verifyAgentPaneOpen(session) {
    const found = await evaluate(session, `
        (() => {
            const el = document.querySelector('.agent-input');
            return el !== null && el.tagName === 'TEXTAREA';
        })()
    `);
    if (!found) {
        throw new Error("No .agent-input textarea found in the DOM. Open an agent pane before running the bench.");
    }
}

async function focusAgentInput(session) {
    await evaluate(session, `
        (() => {
            const el = document.querySelector('.agent-input');
            el.focus();
            return true;
        })()
    `);
}

// Clear any existing perf marks so we don't mix this run with prior data.
async function clearPerfMarks(session) {
    await evaluate(session, `
        performance.clearMarks();
        performance.clearMeasures();
        true
    `);
}

// Dispatch one synthetic keystroke. Uses Input.dispatchKeyEvent (real
// DOM-level event, going through the renderer's input pipeline) rather
// than Runtime.evaluate fiddling with .value directly.
async function dispatchKey(session, key, code) {
    // keydown → char → keyup (Input.dispatchKeyEvent supports all three;
    // we send rawKeyDown + char + keyUp which is the canonical sequence
    // for a printable character).
    await session.send("Input.dispatchKeyEvent", {
        type: "rawKeyDown",
        key,
        code,
        text: key,
        unmodifiedText: key,
        windowsVirtualKeyCode: key.charCodeAt(0),
        nativeVirtualKeyCode: key.charCodeAt(0),
    });
    await session.send("Input.dispatchKeyEvent", {
        type: "char",
        key,
        code,
        text: key,
        unmodifiedText: key,
    });
    await session.send("Input.dispatchKeyEvent", {
        type: "keyUp",
        key,
        code,
        windowsVirtualKeyCode: key.charCodeAt(0),
        nativeVirtualKeyCode: key.charCodeAt(0),
    });
}

// Read all `agent-keystroke` and `agent-input-raf-cb` measure entries since
// the last clear. Returns { keystroke: number[], rafCb: number[] } arrays
// of durations in ms.
async function readMeasures(session) {
    return evaluate(session, `
        (() => {
            const entries = performance.getEntriesByType('measure');
            return {
                keystroke: entries.filter(e => e.name.startsWith('agent-keystroke:')).map(e => e.duration),
                rafCb: entries.filter(e => e.name.startsWith('agent-input-raf-cb:')).map(e => e.duration),
                submit: entries.filter(e => e.name.startsWith('agent-submit:')).map(e => e.duration),
            };
        })()
    `);
}

async function clearTextarea(session) {
    await evaluate(session, `
        (() => {
            const el = document.querySelector('.agent-input');
            el.value = '';
            el.dispatchEvent(new InputEvent('input', { bubbles: true }));
            return true;
        })()
    `);
}

// ── Statistics ──────────────────────────────────────────────────────────────

function pct(arr, p) {
    if (arr.length === 0) return null;
    const sorted = [...arr].sort((a, b) => a - b);
    const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * p / 100));
    return sorted[idx];
}

function stats(arr) {
    if (arr.length === 0) return { n: 0, p50: null, p95: null, p99: null, max: null, mean: null };
    const sum = arr.reduce((a, b) => a + b, 0);
    return {
        n: arr.length,
        p50: pct(arr, 50),
        p95: pct(arr, 95),
        p99: pct(arr, 99),
        max: Math.max(...arr),
        mean: sum / arr.length,
    };
}

function fmt(n) { return n == null ? "—" : `${n.toFixed(2)} ms`; }

// ── Main ────────────────────────────────────────────────────────────────────

const PRINTABLE_CHARS = "abcdefghijklmnopqrstuvwxyz".split("");

async function main() {
    console.log(`bench-agent-keystroke: connecting to CDP on 127.0.0.1:${CDP_PORT}`);
    const target = await findMainPageTarget(CDP_PORT);
    console.log(`  page target: ${target.url || "(no url)"} — ${target.title || "(no title)"}`);

    const session = new CdpSession(target.webSocketDebuggerUrl);
    await session.connect();
    await session.send("Runtime.enable");
    await session.send("Input.enable").catch(() => {});

    await verifyAgentPaneOpen(session);
    console.log("  ✓ .agent-input found");

    await focusAgentInput(session);
    console.log("  ✓ focused");

    // Warmup
    console.log(`  warming up (${WARMUP} keystrokes)...`);
    for (let i = 0; i < WARMUP; i++) {
        const c = PRINTABLE_CHARS[i % PRINTABLE_CHARS.length];
        await dispatchKey(session, c, `Key${c.toUpperCase()}`);
        await new Promise((r) => setTimeout(r, INTER_KEY_MS));
    }

    // Clear marks so warmup doesn't pollute results
    await clearPerfMarks(session);

    // Measured run
    console.log(`  running ${COUNT} keystrokes (${INTER_KEY_MS} ms apart)...`);
    const startWall = Date.now();
    for (let i = 0; i < COUNT; i++) {
        const c = PRINTABLE_CHARS[i % PRINTABLE_CHARS.length];
        await dispatchKey(session, c, `Key${c.toUpperCase()}`);
        await new Promise((r) => setTimeout(r, INTER_KEY_MS));
    }
    const elapsedWall = Date.now() - startWall;
    console.log(`  ✓ ${COUNT} keystrokes dispatched in ${elapsedWall} ms`);

    // Let any in-flight RAF callbacks complete
    await new Promise((r) => setTimeout(r, 100));

    const measures = await readMeasures(session);
    const keystrokeStats = stats(measures.keystroke);
    const rafCbStats = stats(measures.rafCb);
    const submitStats = stats(measures.submit);

    if (!NO_CLEANUP) {
        await clearTextarea(session);
    }

    await session.close();

    // ── Report ──────────────────────────────────────────────────────────────

    console.log("");
    console.log("─".repeat(70));
    console.log("agent-keystroke (onInput handler synchronous cost)");
    console.log(`  n=${keystrokeStats.n}  P50=${fmt(keystrokeStats.p50)}  P95=${fmt(keystrokeStats.p95)}  P99=${fmt(keystrokeStats.p99)}  max=${fmt(keystrokeStats.max)}  mean=${fmt(keystrokeStats.mean)}`);
    console.log("");
    console.log("agent-input-raf-cb (RAF callback / coalesced scroll cost)");
    console.log(`  n=${rafCbStats.n}  P50=${fmt(rafCbStats.p50)}  P95=${fmt(rafCbStats.p95)}  P99=${fmt(rafCbStats.p99)}  max=${fmt(rafCbStats.max)}  mean=${fmt(rafCbStats.mean)}`);
    if (submitStats.n > 0) {
        console.log("");
        console.log("agent-submit (submit handler cost — should be 0 in a no-submit run)");
        console.log(`  n=${submitStats.n}  P50=${fmt(submitStats.p50)}  P95=${fmt(submitStats.p95)}  max=${fmt(submitStats.max)}`);
    }
    console.log("─".repeat(70));

    if (OUTPUT_FILE) {
        writeFileSync(OUTPUT_FILE, JSON.stringify({
            timestamp: new Date().toISOString(),
            count: COUNT,
            warmup: WARMUP,
            inter_key_ms: INTER_KEY_MS,
            wall_elapsed_ms: elapsedWall,
            keystroke: { stats: keystrokeStats, samples: measures.keystroke },
            raf_cb: { stats: rafCbStats, samples: measures.rafCb },
            submit: { stats: submitStats, samples: measures.submit },
        }, null, 2));
        console.log(`  results saved to ${OUTPUT_FILE}`);
    }

    // Threshold check — spec §2 target: keystroke P95 ≤ 50 ms.
    if (keystrokeStats.p95 != null && keystrokeStats.p95 > P95_THRESHOLD_MS) {
        console.error(`\n✗ FAIL: keystroke P95 ${fmt(keystrokeStats.p95)} exceeds threshold ${P95_THRESHOLD_MS} ms`);
        process.exit(1);
    }

    if (keystrokeStats.n === 0) {
        console.error("\n✗ FAIL: zero agent-keystroke measures captured. Are perf marks deployed? See PR #1146.");
        process.exit(1);
    }

    console.log("\n✓ PASS");
}

main().catch((err) => {
    console.error(`\nbench-agent-keystroke: ${err.message}`);
    process.exit(2);
});
