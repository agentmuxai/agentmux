// Capture a Chromium trace (all processes: browser, renderer main+compositor,
// GPU) via CDP for N seconds, save it, and summarize where time goes per
// process/thread and per top-level event name — for when a JS CPU profile
// shows the renderer main thread idle but the UI is still laggy.
//
//   node scripts/ui-screenshots/trace-capture.mjs <port> <targetIdSubstr> <seconds> <out.json>
import WebSocket from "ws";
import fs from "node:fs";

const [port = "9223", targetSel = "", secs = "10", out = "trace.json"] = process.argv.slice(2);
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && (t.id.includes(targetSel) || t.url.includes(targetSel)));
if (!page) { console.error("no page target matching", targetSel); process.exit(1); }
console.error("attaching to", page.title);

const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 1 << 30 });
await new Promise((r, e) => { ws.once("open", r); ws.once("error", e); });
let id = 0; const pending = new Map(); const chunks = [];
let done; const complete = new Promise((r) => (done = r));
ws.on("message", (m) => {
  const j = JSON.parse(m);
  if (j.id && pending.has(j.id)) { pending.get(j.id)(j); pending.delete(j.id); return; }
  if (j.method === "Tracing.dataCollected") chunks.push(...j.params.value);
  if (j.method === "Tracing.tracingComplete") done();
});
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });

const cats = [
  "devtools.timeline", "disabled-by-default-devtools.timeline", "disabled-by-default-devtools.timeline.frame",
  "blink", "blink.user_timing", "cc", "gpu", "viz", "input", "latencyInfo", "v8.execute", "toplevel", "ipc",
].join(",");
await send("Tracing.start", { categories: cats, transferMode: "ReportEvents", options: "sampling-frequency=1000" });
console.error(`tracing for ${secs}s ...`);
await new Promise((r) => setTimeout(r, Number(secs) * 1000));
await send("Tracing.end");
await complete;
ws.close();
fs.writeFileSync(out, JSON.stringify({ traceEvents: chunks }));
console.error(`wrote ${chunks.length} events -> ${out}`);

// ---- summary ----
const procName = new Map(), threadName = new Map();
for (const e of chunks) {
  if (e.ph === "M" && e.name === "process_name") procName.set(e.pid, e.args.name);
  if (e.ph === "M" && e.name === "thread_name") threadName.set(`${e.pid}:${e.tid}`, e.args.name);
}
const byThread = new Map(), byName = new Map(), longTasks = [];
// Only count complete events ('X') at depth 0-ish: approximate by summing dur of
// events whose name is a known top-level runner, plus all events by name for ranking.
for (const e of chunks) {
  if (e.ph !== "X" || !e.dur) continue;
  const tkey = `${procName.get(e.pid) || e.pid} / ${threadName.get(`${e.pid}:${e.tid}`) || e.tid}`;
  if (e.name === "MessageLoop::RunTask" || e.name === "ThreadControllerImpl::RunTask" || e.name === "RunTask") {
    byThread.set(tkey, (byThread.get(tkey) || 0) + e.dur);
    if (e.dur > 50000) longTasks.push({ tkey, dur: e.dur, ts: e.ts });
  }
  const nkey = `${e.name}  [${tkey}]`;
  byName.set(nkey, (byName.get(nkey) || 0) + e.dur);
}
const top = (m, n) => [...m].sort((a, b) => b[1] - a[1]).slice(0, n);
const ms = (us) => (us / 1000).toFixed(0).padStart(7);
console.log(`\nBUSY TIME PER THREAD (RunTask sum over ${secs}s):`);
for (const [k, v] of top(byThread, 14)) console.log(`  ${ms(v)} ms  ${(100 * v / (Number(secs) * 1e6)).toFixed(0).padStart(3)}%  ${k}`);
console.log(`\nLONG TASKS >50ms: ${longTasks.length}  (max ${ms(Math.max(0, ...longTasks.map((l) => l.dur)))} ms)`);
const ltBy = new Map(); for (const l of longTasks) ltBy.set(l.tkey, (ltBy.get(l.tkey) || 0) + 1);
for (const [k, v] of top(ltBy, 6)) console.log(`  ${String(v).padStart(4)}  ${k}`);
console.log("\nTOP EVENTS BY TOTAL DURATION (inclusive, so nested events double-count):");
for (const [k, v] of top(byName, 30)) console.log(`  ${ms(v)} ms  ${k}`);
