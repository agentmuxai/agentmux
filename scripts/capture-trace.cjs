#!/usr/bin/env node
// Capture a Chromium DevTools Performance trace from a running AgentMux
// instance via Chrome DevTools Protocol (CDP).
//
// Usage:
//   1. Launch AgentMux with the remote debugging port enabled:
//        ~/Desktop/agentmux-cef-0.33.105-x64-portable/agentmux.exe \
//          --remote-debugging-port=9222
//      (or any other free port; default is 9222)
//
//   2. In another terminal, run this script:
//        node scripts/capture-trace.cjs [seconds] [port]
//
//      Defaults: 10 seconds, port 9222. The script:
//        - connects to the CEF page target via CDP
//        - starts Tracing with the same categories DevTools Performance uses
//        - waits `seconds` for the user to type in the agent composer
//        - stops tracing and writes the collected events to
//          ~/Desktop/Trace-<timestamp>-cdp.json (compatible with DevTools)
//
//   3. Analyze the output with scripts/analyze-trace.cjs or re-import into
//      DevTools via Performance panel "Load…".
//
// Notes:
//   - Does NOT inject synthetic keystrokes. The user types manually during
//     the capture window so the trace reflects real-world conditions (same
//     as DevTools' record button).
//   - Leaves the running AgentMux instance alone — read-only attach.
//   - Exits non-zero if the debug endpoint isn't reachable (wrong port,
//     AgentMux not launched with the flag, etc.).

const http = require("http");
const fs = require("fs");
const path = require("path");
const os = require("os");
const WebSocket = require("ws");

const SECONDS = parseInt(process.argv[2], 10) || 10;
const PORT = parseInt(process.argv[3], 10) || 9222;

// These match the default categories used by DevTools' Performance panel
// when you click Record. Keeping this list in sync means the resulting JSON
// can be re-imported into DevTools and viewed as a flame graph.
const CATEGORIES = [
    "devtools.timeline",
    "-*",
    "v8.execute",
    "disabled-by-default-devtools.timeline",
    "disabled-by-default-devtools.timeline.frame",
    "disabled-by-default-devtools.timeline.stack",
    "disabled-by-default-v8.cpu_profiler",
    "disabled-by-default-v8.gc",
    "disabled-by-default-cppgc",
    "disabled-by-default-blink.feature_usage",
    "latencyInfo",
    "loading",
    "blink",
    "blink.user_timing",
    "cc",
    "gpu",
    "benchmark",
    "rail",
    "toplevel",
    "v8",
].join(",");

function httpGet(url) {
    return new Promise((resolve, reject) => {
        http.get(url, (res) => {
            let body = "";
            res.on("data", (c) => (body += c));
            res.on("end", () => resolve(body));
        }).on("error", reject);
    });
}

async function main() {
    console.log(`[capture-trace] AgentMux CDP capture`);
    console.log(`  port: ${PORT}`);
    console.log(`  duration: ${SECONDS}s`);
    console.log();

    // 1. Discover targets
    let targetsRaw;
    try {
        targetsRaw = await httpGet(`http://127.0.0.1:${PORT}/json/list`);
    } catch (e) {
        console.error(`[capture-trace] ERROR: cannot reach http://127.0.0.1:${PORT}/json/list`);
        console.error(`  ${e.message}`);
        console.error(`  did you launch AgentMux with --remote-debugging-port=${PORT} ?`);
        process.exit(1);
    }
    const targets = JSON.parse(targetsRaw);
    const page = targets.find((t) => t.type === "page" && t.webSocketDebuggerUrl);
    if (!page) {
        console.error(`[capture-trace] ERROR: no 'page' target with debugger URL found`);
        console.error(`  got: ${JSON.stringify(targets, null, 2)}`);
        process.exit(1);
    }
    console.log(`  target: ${page.title}  (${page.url})`);

    // 2. Connect to the page's debug socket
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
        ws.once("open", resolve);
        ws.once("error", reject);
    });
    console.log(`  ws connected`);

    // 3. CDP request dispatcher — id-based matching
    let nextId = 1;
    const pending = new Map();
    const events = [];   // collected Tracing.dataCollected

    ws.on("message", (buf) => {
        const msg = JSON.parse(buf.toString());
        if (msg.id != null && pending.has(msg.id)) {
            const { resolve, reject } = pending.get(msg.id);
            pending.delete(msg.id);
            if (msg.error) reject(new Error(msg.error.message));
            else resolve(msg.result);
            return;
        }
        // Stream event: the trace data arrives in many Tracing.dataCollected
        // messages, then a Tracing.tracingComplete at the end.
        if (msg.method === "Tracing.dataCollected" && msg.params && msg.params.value) {
            for (const e of msg.params.value) events.push(e);
        }
    });

    function call(method, params) {
        const id = nextId++;
        const body = JSON.stringify({ id, method, params: params || {} });
        return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject });
            ws.send(body);
        });
    }

    // 4. Start tracing. `returnAsStream: false` means the browser streams
    // Tracing.dataCollected events back instead of buffering them all.
    console.log(`\n[capture-trace] starting trace…`);
    await call("Tracing.start", {
        traceConfig: {
            recordMode: "recordUntilFull",
            includedCategories: CATEGORIES.split(","),
        },
        transferMode: "ReportEvents",
    });

    // 5. Countdown — user types during this window
    console.log(`[capture-trace] type in the agent composer now (${SECONDS}s)…`);
    for (let i = SECONDS; i > 0; i--) {
        process.stdout.write(`\r  ${i}s remaining`);
        await new Promise((r) => setTimeout(r, 1000));
    }
    console.log(`\r  recording complete  `);

    // 6. Stop tracing. Wait for tracingComplete to ensure all
    // dataCollected messages have arrived.
    console.log(`[capture-trace] flushing…`);
    const completed = new Promise((resolve) => {
        const h = (buf) => {
            const msg = JSON.parse(buf.toString());
            if (msg.method === "Tracing.tracingComplete") {
                ws.off("message", h);
                resolve();
            }
        };
        ws.on("message", h);
    });
    await call("Tracing.end");
    await completed;
    console.log(`  received ${events.length} trace events`);

    // 7. Write the collected events to a file in DevTools-compatible format.
    // DevTools' Performance panel understands either a bare `traceEvents`
    // array wrapped in an object, or just the array. Matching the format
    // of what the user captured manually keeps analyze-trace.cjs working
    // without any changes.
    const out = {
        metadata: {
            source: "agentmux-capture-trace.cjs",
            startTime: new Date().toISOString(),
            captureDurationSec: SECONDS,
        },
        traceEvents: events,
    };
    const stamp = new Date().toISOString().replace(/[-:T.Z]/g, "").slice(0, 14);
    const outPath = path.join(os.homedir(), "Desktop", `Trace-${stamp}-cdp.json`);
    fs.writeFileSync(outPath, JSON.stringify(out));
    console.log(`  wrote: ${outPath}`);
    console.log(`  size:  ${(fs.statSync(outPath).size / 1024 / 1024).toFixed(1)} MB`);

    ws.close();
    console.log(`\n[capture-trace] done.`);
    console.log(`  next: node scripts/analyze-trace.cjs (if present)`);
    console.log(`        or re-import in DevTools → Performance → Load…`);
}

main().catch((e) => {
    console.error(`[capture-trace] FATAL: ${e.message}`);
    process.exit(1);
});
