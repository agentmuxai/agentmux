// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Per-thread busy time + top events in a Chromium trace (CDP Tracing JSON).
// Usage: node trace-threads.mjs trace.json [auto | t0_us t1_us]
import fs from "node:fs";
const { traceEvents: ev } = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const proc = new Map(), thr = new Map();
for (const e of ev) { if (e.ph === "M" && e.name === "process_name") proc.set(e.pid, e.args.name); if (e.ph === "M" && e.name === "thread_name") thr.set(`${e.pid}:${e.tid}`, e.args.name); }
const X = ev.filter((e) => e.ph === "X" && e.dur != null);
let tmin = Infinity, tmax = -Infinity; for (const e of X) { if (e.ts < tmin) tmin = e.ts; if (e.ts + e.dur > tmax) tmax = e.ts + e.dur; }
let t0 = +process.argv[3] || tmin, t1 = +process.argv[4] || tmax;
if (process.argv[3] === "auto") {
  // the drag = first..last ResizeObserver notification that did real work
  const ro = X.filter((e) => e.name === "LocalFrameView::NotifyResizeObservers" && e.dur > 300).map((e) => e.ts).sort((a, b) => a - b);
  if (ro.length) { t0 = ro[0] - 20000; t1 = ro[ro.length - 1] + 80000; }
}
const win = X.filter((e) => e.ts >= t0 && e.ts <= t1);
const spanMs = (t1 - t0) / 1000;
// busy = union of top-level (non-nested) X events per thread
const byThread = new Map();
for (const e of win) { const k = `${e.pid}:${e.tid}`; (byThread.get(k) ?? byThread.set(k, []).get(k)).push(e); }
const rows = [];
for (const [k, list] of byThread) {
  list.sort((a, b) => a.ts - b.ts || b.dur - a.dur);
  let busy = 0, end = -1;
  for (const e of list) { const s = e.ts, f = e.ts + e.dur; if (f <= end) continue; busy += f - Math.max(s, end); end = f; }
  rows.push({ proc: proc.get(+k.split(":")[0]) ?? k.split(":")[0], name: thr.get(k) ?? k.split(":")[1], busyMs: busy / 1000, n: list.length });
}
rows.sort((a, b) => b.busyMs - a.busyMs);
console.log(`window ${spanMs.toFixed(0)} ms`);
console.log("--- busiest threads (union of top-level events)");
for (const r of rows.slice(0, 12)) console.log(`${r.busyMs.toFixed(0).padStart(6)} ms (${(100 * r.busyMs / spanMs).toFixed(0).padStart(3)}%)  ${r.proc} / ${r.name}  [${r.n} events]`);
// renderer main thread event totals (self-agnostic, by name)
const mains = [...byThread.entries()].filter(([k]) => /CrRendererMain/.test(thr.get(k) ?? "")).map(([k, l]) => ({ k, l, busy: rows.find((r) => r.n === l.length)?.busyMs ?? 0 }));
const tot = (names, list) => names.map((n) => { const m = list.filter((e) => e.name === n); return `${n}: ${(m.reduce((s, e) => s + e.dur, 0) / 1000).toFixed(0)}ms/${m.length}`; });
const NAMES = ["LocalFrameView::UpdateStyleAndLayout", "UpdateLayoutTree", "Layout", "Blink.ForcedStyleAndLayout.UpdateTime", "LocalFrameView::NotifyResizeObservers", "PrePaint", "Paint", "Layerize", "Commit", "WebGLRenderingContextBase::DrawingBufferClientRestoreFramebufferBinding", "FunctionCall", "RunTask"];
for (const [k, list] of byThread) {
  const nm = thr.get(k) ?? "";
  if (!/CrRendererMain/.test(nm)) continue;
  const busy = rows.find((r) => r.name === nm && r.n === list.length);
  if (!busy || busy.busyMs < 150) continue;
  console.log(`--- renderer main ${proc.get(+k.split(":")[0])} (${busy.busyMs.toFixed(0)} ms busy)`);
  console.log("   " + tot(NAMES, list).join("\n   "));
}
for (const want of [/CrBrowserMain/, /VizCompositorThread/, /CrGpuMain/, /Compositor$/]) {
  for (const [k, list] of byThread) {
    const nm = thr.get(k) ?? ""; if (!want.test(nm)) continue;
    const r = rows.find((x) => x.name === nm && x.n === list.length); if (!r || r.busyMs < 50) continue;
    const agg = new Map(); for (const e of list) agg.set(e.name, (agg.get(e.name) ?? 0) + e.dur);
    const top = [...agg.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6).map(([n, d]) => `${n} ${(d / 1000).toFixed(0)}ms`).join("; ");
    console.log(`--- ${proc.get(+k.split(":")[0])} / ${nm}: ${r.busyMs.toFixed(0)} ms busy. top: ${top}`);
  }
}
