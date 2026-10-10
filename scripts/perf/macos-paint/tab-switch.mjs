// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
import { connectPage } from "../../ui-screenshots/lib/cdp-client.mjs";
const label = process.argv[2] ?? "tab-switch", rounds = +(process.argv[3] ?? 4);
const { session: page } = await connectPage(9223, "window_transparent");
const ev = async (e) => { const r = await page.send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 400)); return r.result.value; };
const out = await ev(`(async () => {
  const ta = await import("/frontend/app/store/tab-actions.ts");
  const g = await import("/frontend/app/store/global.ts");
  const ws = g.atoms.workspace(); const ids = [...(ws.pinnedtabids ?? []), ...ws.tabids];
  const tl = document.querySelector(".tile-layout"); let c = tl; while (c && !(c.style && c.style.position === "absolute" && /h-full/.test(c.className))) c = c.parentElement;
  const boxes = [...c.parentElement.children].filter((x) => x.style && x.style.position === "absolute" && /h-full/.test(x.className));
  const visible = (b) => { const cs = getComputedStyle(b); return cs.visibility === "visible" && cs.opacity !== "0"; };
  const loaf = []; const po = new PerformanceObserver((l) => { for (const e of l.getEntries()) loaf.push(e.duration); }); po.observe({ type: "long-animation-frame", buffered: false });
  const rows = [];
  const order = []; for (let r = 0; r < ${rounds}; r++) for (let i = 0; i < ids.length; i++) order.push(i);
  // Visit tabs in a shuffled-but-fixed order so every tab is a destination from different sources.
  for (const i of order) {
    if (visible(boxes[i])) { const j = (i + 1) % ids.length; await ta.setActiveTab(ids[j]); await new Promise((r) => setTimeout(r, 500)); }
    const t0 = performance.now(); let tVis = null, frames = 0;
    const done = new Promise((res) => { const loop = () => { frames++; if (tVis == null && visible(boxes[i])) tVis = performance.now() - t0; if (tVis != null || frames > 60) res(); else requestAnimationFrame(loop); }; requestAnimationFrame(loop); });
    const p = ta.setActiveTab(ids[i]);
    await done; const tRpc = (await p, performance.now() - t0);
    await new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(res)));
    rows.push({ i, toVisibleMs: tVis == null ? null : +tVis.toFixed(1), rpcDoneMs: +tRpc.toFixed(1), framesToVisible: frames });
    await new Promise((r) => setTimeout(r, 450));
  }
  po.disconnect();
  const q = (a, p) => { const s = a.filter((x) => x != null).sort((x, y) => x - y); return s.length ? +s[Math.min(s.length - 1, Math.floor(p * s.length))].toFixed(1) : null; };
  const vis = rows.map((r) => r.toVisibleMs);
  return { slow: rows.filter((r) => r.toVisibleMs == null || r.toVisibleMs > 40), perDest: [0,1,2,3,4].map((i) => rows.filter((r) => r.i === i).map((r) => r.toVisibleMs)), tabs: ids.length, boxes: boxes.length, switches: rows.length, toVisibleMs: { p50: q(vis, .5), p90: q(vis, .9), max: q(vis, 1) }, rpcDoneMs: { p50: q(rows.map((r) => r.rpcDoneMs), .5), max: q(rows.map((r) => r.rpcDoneMs), 1) }, framesToVisible: { p50: q(rows.map((r) => r.framesToVisible), .5), max: q(rows.map((r) => r.framesToVisible), 1) }, longFrames: loaf.length, longMs: Math.round(loaf.reduce((a, b) => a + b, 0)), longMax: Math.round(Math.max(0, ...loaf)) };
})()`);
console.log(label, JSON.stringify(out));
process.exit(0);
