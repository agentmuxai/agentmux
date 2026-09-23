// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Attach to a CDP page target and capture a V8 CPU profile of the renderer
// main thread for N seconds. Writes a .cpuprofile (loadable in DevTools
// Performance/JS Profiler) and prints a self-time summary.
//
//   node scripts/ui-screenshots/cpu-profile.mjs <port> <targetIdSubstr> <seconds> <out.cpuprofile>
import WebSocket from "ws";
import fs from "node:fs";

const [port = "9223", targetSel = "", secs = "10", out = "profile.cpuprofile"] = process.argv.slice(2);
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && (t.id.includes(targetSel) || t.url.includes(targetSel)));
if (!page) { console.error("no page target matching", targetSel); process.exit(1); }
console.error("attaching to", page.title, page.url);

const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false });
await new Promise((r, e) => { ws.once("open", r); ws.once("error", e); });
let id = 0; const pending = new Map();
ws.on("message", (m) => { const j = JSON.parse(m); if (j.id && pending.has(j.id)) { pending.get(j.id)(j); pending.delete(j.id); } });
const send = (method, params = {}) => new Promise((res, rej) => { const i = ++id; pending.set(i, (j) => (j.error ? rej(new Error(`${method}: ${j.error.message}`)) : res(j))); ws.send(JSON.stringify({ id: i, method, params })); });
// A dropped CDP connection (target reload/close) sends no responses; reject
// everything still waiting instead of hanging forever.
const failAll = (why) => { for (const f of pending.values()) f({ error: { message: why } }); pending.clear(); };
ws.on("close", () => failAll("CDP socket closed"));
ws.on("error", (e) => failAll(`CDP socket error: ${e.message}`));

await send("Profiler.enable");
await send("Profiler.setSamplingInterval", { interval: 500 }); // µs
await send("Profiler.start");
console.error(`profiling for ${secs}s ...`);
await new Promise((r) => setTimeout(r, Number(secs) * 1000));
const { result } = await send("Profiler.stop");
ws.close();
const profile = result.profile;
fs.writeFileSync(out, JSON.stringify(profile));

// --- summary: self time per function, and per URL ---
const nodes = new Map(profile.nodes.map((n) => [n.id, n]));
const selfUs = new Map();
const dt = profile.timeDeltas;
for (let i = 0; i < profile.samples.length; i++) selfUs.set(profile.samples[i], (selfUs.get(profile.samples[i]) || 0) + (dt[i] || 0));
const total = [...selfUs.values()].reduce((a, b) => a + b, 0);
const byFn = new Map(), byUrl = new Map();
for (const [nid, us] of selfUs) {
  const cf = nodes.get(nid).callFrame;
  const url = (cf.url || "").replace(/^.*\/(node_modules|app)\//, "$1/").slice(0, 70);
  const key = `${cf.functionName || "(anon)"}  ${url}:${cf.lineNumber}`;
  byFn.set(key, (byFn.get(key) || 0) + us);
  byUrl.set(url || "(native/idle)", (byUrl.get(url || "(native/idle)") || 0) + us);
}
const top = (m, n) => [...m].sort((a, b) => b[1] - a[1]).slice(0, n);
// Busy = every sample except the `(idle)` node; the full sampled duration stays
// the denominator. (Counting `(idle)` as busy made an idle capture read ~100%.)
let idleUs = 0;
for (const [nid, us] of selfUs) if (nodes.get(nid).callFrame.functionName === "(idle)") idleUs += us;
console.log(`total sampled: ${(total / 1000).toFixed(0)} ms over ${secs}s  -> main-thread busy ≈ ${(100 * (total - idleUs) / Math.max(1, total)).toFixed(0)}% (idle ${(idleUs / 1000).toFixed(0)} ms)\n`);
console.log("TOP SELF-TIME BY FILE:");
for (const [k, v] of top(byUrl, 12)) console.log(`  ${(v / 1000).toFixed(0).padStart(6)} ms  ${(100 * v / total).toFixed(1).padStart(5)}%  ${k}`);
console.log("\nTOP SELF-TIME BY FUNCTION:");
for (const [k, v] of top(byFn, 25)) console.log(`  ${(v / 1000).toFixed(0).padStart(6)} ms  ${(100 * v / total).toFixed(1).padStart(5)}%  ${k}`);
