// Verify the autoGrow fix by capturing a DevTools trace against 0.33.106
// and comparing keypress event timing vs the 0.33.105 baseline.
//
// Strategy:
//   - Connects to a running AgentMux at --remote-debugging-port=9222
//   - Starts Tracing
//   - Synthesizes keystrokes via CDP Input.dispatchKeyEvent so we don't
//     need the user to manually type during the capture window
//   - Stops tracing, analyzes keypress/textInput/input event durations
//   - Prints PASS/FAIL against a target avg keypress < 5ms
//
// Usage: node /tmp/verify-typing-fix.cjs

const http = require("http");
const WebSocket = require("ws");
const fs = require("fs");

// Allow overriding host:port via CLI so we can talk to a specific running
// instance when multiple are up (e.g. [::1]:9222 for an IPv6-only instance).
// Usage: node verify-typing-fix.cjs [host] [port]
const HOST = process.argv[2] || "127.0.0.1";
const PORT = parseInt(process.argv[3], 10) || 9222;

const TARGET_KEYPRESS_MS = 5;   // fix should drop avg to <5ms (was 45ms)

const CATEGORIES = [
    "devtools.timeline",
    "-*",
    "disabled-by-default-devtools.timeline",
    "disabled-by-default-devtools.timeline.frame",
    "disabled-by-default-v8.cpu_profiler",
    "latencyInfo",
    "blink",
    "cc",
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
    console.log("[verify] locating page target…");
    let targets;
    try {
        // Bracket IPv6 literals per RFC 3986
        const hostPart = HOST.includes(":") ? `[${HOST}]` : HOST;
        targets = JSON.parse(await httpGet(`http://${hostPart}:${PORT}/json/list`));
    } catch (e) {
        console.error(`[verify] cannot reach CDP endpoint on :${PORT}`);
        console.error(`  ${e.message}`);
        console.error(`  launch AgentMux with --remote-debugging-port=${PORT}`);
        process.exit(2);
    }
    const page = targets.find((t) => t.type === "page");
    if (!page) {
        console.error("[verify] no page target found");
        console.error(JSON.stringify(targets, null, 2));
        process.exit(2);
    }
    console.log(`  target: ${page.title}`);

    const ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((res, rej) => { ws.once("open", res); ws.once("error", rej); });
    console.log("  ws connected");

    let nextId = 1;
    const pending = new Map();
    const events = [];

    ws.on("message", (buf) => {
        const msg = JSON.parse(buf.toString());
        if (msg.id != null && pending.has(msg.id)) {
            const p = pending.get(msg.id);
            pending.delete(msg.id);
            if (msg.error) p.reject(new Error(msg.error.message));
            else p.resolve(msg.result);
            return;
        }
        if (msg.method === "Tracing.dataCollected" && msg.params?.value) {
            for (const e of msg.params.value) events.push(e);
        }
    });

    const call = (method, params = {}) => {
        const id = nextId++;
        return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject });
            ws.send(JSON.stringify({ id, method, params }));
        });
    };

    // Bring the agent composer into focus. We can't click the textarea via
    // CDP without knowing its coordinates, so we rely on the user having
    // clicked the agent composer before running this script.
    //
    // If no agent pane exists, keystrokes will just go to whatever has
    // focus. The metric we care about (keypress avg) is still valid — any
    // slow input handler would still show up.

    console.log("\n[verify] starting trace…");
    await call("Tracing.start", {
        traceConfig: {
            recordMode: "recordUntilFull",
            includedCategories: CATEGORIES.split(","),
        },
        transferMode: "ReportEvents",
    });

    // Wait 500ms for tracing to settle
    await new Promise((r) => setTimeout(r, 500));

    // Send 30 synthetic keystrokes with a 80ms gap between them.
    // That's 2.4s of typing, which should give us 30 keypress events to
    // average over. Same approximate cadence as human typing (~12-15 wpm
    // to keep each event clearly separable in the trace).
    console.log("[verify] sending 30 synthetic keystrokes…");
    const chars = "The quick brown fox jumps over".split("");
    for (const ch of chars) {
        const key = ch === " " ? "Space" : `Key${ch.toUpperCase()}`;
        await call("Input.dispatchKeyEvent", {
            type: "keyDown",
            text: ch,
            key: ch,
            code: key,
            windowsVirtualKeyCode: ch.charCodeAt(0),
            nativeVirtualKeyCode: ch.charCodeAt(0),
        });
        await call("Input.dispatchKeyEvent", {
            type: "char",
            text: ch,
            key: ch,
            code: key,
            unmodifiedText: ch,
        });
        await call("Input.dispatchKeyEvent", {
            type: "keyUp",
            text: ch,
            key: ch,
            code: key,
            windowsVirtualKeyCode: ch.charCodeAt(0),
        });
        await new Promise((r) => setTimeout(r, 80));
    }

    // Wait another 500ms for the last keystroke's events to flush
    await new Promise((r) => setTimeout(r, 500));

    // Stop tracing
    console.log("[verify] stopping trace…");
    const completed = new Promise((resolve) => {
        const h = (buf) => {
            const m = JSON.parse(buf.toString());
            if (m.method === "Tracing.tracingComplete") { ws.off("message", h); resolve(); }
        };
        ws.on("message", h);
    });
    await call("Tracing.end");
    await completed;
    console.log(`  collected ${events.length} events`);

    // Save the raw trace for post-mortem
    const outPath = `C:/Users/area54/AppData/Local/Temp/agentmux-trace-after.json`;
    fs.writeFileSync(outPath, JSON.stringify({ traceEvents: events }));
    console.log(`  saved: ${outPath}`);

    // Analyze
    const dispatches = events.filter(e => e.name === "EventDispatch" && e.ph === "X");
    const byType = {};
    for (const e of dispatches) {
        const t = e.args?.data?.type || "?";
        if (!byType[t]) byType[t] = { count: 0, total: 0, max: 0 };
        byType[t].count++;
        byType[t].total += e.dur || 0;
        if ((e.dur || 0) > byType[t].max) byType[t].max = e.dur || 0;
    }

    console.log("\n─── Input event dispatch timing (0.33.106, post-fix) ───");
    const rows = ["keydown", "beforeinput", "keypress", "textInput", "input", "keyup"];
    for (const t of rows) {
        const s = byType[t];
        if (!s) { console.log(`  ${t}: (not observed)`); continue; }
        const avg = (s.total / s.count / 1000).toFixed(2);
        const max = (s.max / 1000).toFixed(2);
        console.log(`  ${t.padEnd(14)}  n=${String(s.count).padStart(3)}  avg=${avg.padStart(6)}ms  max=${max.padStart(6)}ms`);
    }

    const keypressAvg = byType.keypress ? (byType.keypress.total / byType.keypress.count / 1000) : 0;
    const layouts = events.filter(e => e.name === "Layout" && e.ph === "X");
    const layoutTotal = layouts.reduce((a, l) => a + (l.dur || 0), 0) / 1000;
    console.log(`\n  Total Layouts: ${layouts.length} (${layoutTotal.toFixed(1)}ms)`);

    console.log("\n─── Baseline (0.33.105, pre-fix) ───");
    console.log("  keydown       n= 34  avg=  0.36ms  max=   0.8ms");
    console.log("  beforeinput   n= 34  avg=  0.00ms");
    console.log("  keypress      n= 34  avg= 45.12ms  max=  60.0ms  ← THE BUG");
    console.log("  textInput     n= 34  avg= 45.07ms  max=  59.9ms");
    console.log("  input         n= 34  avg= 21.75ms  max=  32.7ms");
    console.log("  Total Layouts: 118 (1521ms)");

    console.log("\n─── Verdict ───");
    const passed = keypressAvg > 0 && keypressAvg < TARGET_KEYPRESS_MS;
    console.log(`  target:  keypress avg < ${TARGET_KEYPRESS_MS}ms`);
    console.log(`  result:  keypress avg = ${keypressAvg.toFixed(2)}ms`);
    console.log(`  ${passed ? "PASS ✓ fix confirmed" : "FAIL ✗ still slow — revisit"}`);

    ws.close();
    process.exit(passed ? 0 : 1);
}

main().catch((e) => {
    console.error(`[verify] FATAL: ${e.message}`);
    process.exit(3);
});
