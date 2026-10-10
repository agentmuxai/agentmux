// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Runs panesampler + resize-frames together, then reports how far each native pane window's
// right/left edges trail where they belong relative to the main window.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
const pid = process.argv[2], label = process.argv[3] ?? "pane-lag";
const log = process.env.HOST_LOG ?? "";
const text = () => (log ? fs.readFileSync(log, "utf8") : "");
const count = (pat) => (text().match(pat) ?? []).length;
const before = { objc: count(/ObjC task NSApp window/g), tasks: count(/SetPaneBoundsViewsTask|CEF bounds \(macos\)/g), lines: text().split("\n").length };
const samp = spawn("./panesampler", [pid, process.env.SAMPLE_SECS ?? "4.2"], { stdio: ["ignore", "pipe", "pipe"] });
let out = ""; samp.stdout.on("data", (d) => (out += d));
await new Promise((r) => setTimeout(r, 600));
const fp = spawnSync("node", ["resize-frames.mjs", "--label", label, "--runs", "1", "--brief", ...(process.env.FP_ARGS ?? "").split(" ").filter(Boolean)], { encoding: "utf8" });
console.log(fp.stdout.trim());
await new Promise((r) => samp.on("close", r));
const after = { objc: count(/ObjC task NSApp window/g), tasks: count(/SetPaneBoundsViewsTask|CEF bounds \(macos\)/g), lines: text().split("\n").length };
if (log) console.log(`host log during run: +${after.lines - before.lines} lines, +${after.tasks - before.tasks} pane-bounds log lines, +${after.objc - before.objc} "ObjC task NSApp window" lines`);
const samples = out.trim().split("\n").map((l) => { const [t, ...ws] = l.split(" "); return { t: +t, w: ws.filter(Boolean).map((x) => { const [id, r] = x.split(":"); const [X, Y, W, H] = r.split(",").map(Number); return { id, X, Y, W, H }; }) }; });
const main0 = (s) => s.w.reduce((a, b) => (a.W * a.H >= b.W * b.H ? a : b));
const first = samples.find((s) => s.w.length > 1);
const mid = main0(first).id;
const rest = new Map(first.w.filter((w) => w.id !== mid).map((w) => [w.id, { r: (w.X + w.W) - (main0(first).X + main0(first).W), l: w.X - main0(first).X }]));
const lag = [], changes = new Map(); let lastBad = 0, lastMainChange = 0, prevMain = null;
for (const s of samples) {
    const main = s.w.find((w) => w.id === mid); if (!main) continue;
    if (prevMain && (prevMain.W !== main.W || prevMain.X !== main.X)) lastMainChange = s.t; prevMain = main;
    for (const w of s.w) {
        if (w.id === mid || !rest.has(w.id)) continue;
        const r0 = rest.get(w.id);
        const dr = Math.abs((w.X + w.W) - (main.X + main.W) - r0.r);
        lag.push(dr); if (dr > 2) lastBad = s.t;
        const key = `${w.X},${w.W}`; const c = changes.get(w.id) ?? new Set(); c.add(key); changes.set(w.id, c);
    }
}
lag.sort((a, b) => a - b);
const q = (p) => lag[Math.min(lag.length - 1, Math.floor(p * lag.length))];
if (process.env.DUMP) { let n = 0; for (const s of samples) { const main = s.w.find((w) => w.id === mid); for (const w of s.w) { if (w.id === mid || !rest.has(w.id)) continue; const dr = Math.abs((w.X + w.W) - (main.X + main.W) - rest.get(w.id).r); if (dr > 300 && n++ < 14) console.log(`t=${(s.t % 100000).toFixed(0)} pane ${w.id} x=${w.X} y=${w.Y} w=${w.W} h=${w.H} | main x=${main.X} w=${main.W} dev=${dr}`); } } }
{
  // blink-outs: runs where a pane window is reported 0x0 while the main window is mid-drag
  const runs = []; const open = new Map(); let t0 = null, tN = null;
  for (const s of samples) {
    const main = s.w.find((w) => w.id === mid); if (!main) continue;
    t0 ??= s.t; tN = s.t;
    for (const id of rest.keys()) {
      const w = s.w.find((x) => x.id === id);
      const gone = !w || w.W === 0 || w.H === 0;
      if (gone && !open.has(id)) open.set(id, s.t);
      if (!gone && open.has(id)) { runs.push({ id, ms: s.t - open.get(id), at: open.get(id) }); open.delete(id); }
    }
  }
  const total = runs.reduce((a, r) => a + r.ms, 0);
  const durs = runs.map((r) => r.ms).sort((a, b) => a - b);
  { let first = null, last = 0, pm = null; for (const s of samples) { const m = s.w.find((w) => w.id === mid); if (!m) continue; if (pm && pm.W !== m.W) { first ??= s.t; last = s.t; } pm = m; }
  console.log('drag spans', (last - first).toFixed(0), 'ms; blink-outs start at (ms after first size change):', runs.map((r) => (r.at - first).toFixed(0)).join(', ')); }
console.log(`blink-outs (pane window 0x0 or absent): ${runs.length} in ${(tN - t0).toFixed(0)} ms across ${rest.size} panes; total ${total.toFixed(0)} ms; run length p50=${(durs[Math.floor(durs.length/2)] ?? 0).toFixed(0)} ms max=${(durs.at(-1) ?? 0).toFixed(0)} ms`);
}
const mainWidths = new Set(samples.map((s) => s.w.find((w) => w.id === mid)?.W)).size;
console.log(`samples=${samples.length} (~${(samples.length / 4.2).toFixed(0)}/s)  main window widths seen=${mainWidths}`);
console.log(`pane right-edge deviation from rest offset (px): p50=${q(0.5)} p90=${q(0.9)} p99=${q(0.99)} max=${lag.at(-1)}  | share of samples off by >2px: ${(100 * lag.filter((x) => x > 2).length / lag.length).toFixed(0)}%`);
console.log(`pane windows distinct (x,w) states seen: ${[...changes.values()].map((c) => c.size).join(", ")}  | last off-position sample is ${(lastBad - lastMainChange).toFixed(0)} ms after the main window's last size change`);
process.exit(0);
