// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Wall vs thread-CPU time (tdur) of top-level events per key thread, plus the top-level JS functions.
// Usage: node trace-cpu.mjs trace.json   (the drag window is found from the ResizeObserver work)
import fs from "node:fs";
const { traceEvents: ev } = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const thr = new Map(); for (const e of ev) if (e.ph === "M" && e.name === "thread_name") thr.set(`${e.pid}:${e.tid}`, e.args.name);
const X = ev.filter((e) => e.ph === "X" && e.dur != null);
const ro = X.filter((e) => e.name === "LocalFrameView::NotifyResizeObservers" && e.dur > 300).map((e) => e.ts).sort((a, b) => a - b);
const t0 = ro[0] - 20000, t1 = ro.at(-1) + 80000, spanMs = (t1 - t0) / 1000;
const win = X.filter((e) => e.ts >= t0 && e.ts <= t1);
const byT = new Map(); for (const e of win) { const k = `${e.pid}:${e.tid}`; (byT.get(k) ?? byT.set(k, []).get(k)).push(e); }
const top = (list) => { list.sort((a, b) => a.ts - b.ts || b.dur - a.dur); const out = []; let end = -1; for (const e of list) { if (e.ts + e.dur <= end) continue; out.push(e); end = e.ts + e.dur; } return out; };
const rows = [];
for (const [k, list] of byT) { const t = top(list.slice()); const wall = t.reduce((s, e) => s + e.dur, 0) / 1000, cpu = t.reduce((s, e) => s + (e.tdur ?? 0), 0) / 1000; rows.push({ name: thr.get(k) ?? k, pid: k.split(":")[0], wall, cpu }); }
rows.sort((a, b) => b.wall - a.wall);
console.log(`drag window ${spanMs.toFixed(0)} ms`);
for (const r of rows.slice(0, 6)) console.log(`  ${r.name.padEnd(24)} wall ${r.wall.toFixed(0).padStart(5)} ms (${(100 * r.wall / spanMs).toFixed(0)}%)  cpu ${r.cpu.toFixed(0).padStart(5)} ms (${(100 * r.cpu / spanMs).toFixed(0)}%)`);
const main = rows.find((r) => /CrRendererMain/.test(r.name)); const mk = [...byT.keys()].find((k) => thr.get(k) === main.name && k.split(":")[0] === main.pid);
const fc = top(byT.get(mk).filter((e) => e.name === "FunctionCall"));
const agg = new Map(); for (const e of fc) { const d = e.args?.data ?? {}; const key = (d.url ?? "").replace(/^.*localhost:5340/, "").replace(/\?.*/, "") + ":" + d.functionName; const a = agg.get(key) ?? { w: 0, c: 0, n: 0 }; a.w += e.dur / 1000; a.c += (e.tdur ?? 0) / 1000; a.n++; agg.set(key, a); }
for (const [k, a] of [...agg.entries()].sort((x, y) => y[1].w - x[1].w).slice(0, 4)) console.log(`  JS ${k.slice(0, 60).padEnd(60)} x${a.n}: wall ${a.w.toFixed(0)} ms, cpu ${a.c.toFixed(0)} ms`);
