// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Move the main window in small steps (set_window_position, host UI task) while sampling
// every native window of the process; report how far the pane windows drift from the main window.
import { spawn } from "node:child_process";
import { connectPage } from "../../ui-screenshots/lib/cdp-client.mjs";
import os from "node:os";
const pid = process.argv[2], label = process.argv[3] ?? "move", steps = 80, stepPx = +(process.argv[4] ?? 3);
const { session: page } = await connectPage(9223, "window_transparent");
const ev = async (e) => { const r = await page.send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 300)); return r.result.value; };
const creds = await ev(`({ port: window.__AGENTMUX_IPC_PORT__, token: window.__AGENTMUX_IPC_TOKEN__ })`);
const ipc = async (cmd, args) => (await (await fetch(`http://127.0.0.1:${creds.port}/ipc`, { method: "POST", headers: { "Content-Type": "application/json", Authorization: `Bearer ${creds.token}` }, body: JSON.stringify({ cmd, args }) })).json());
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const r0 = (await ipc("get_window_rect", { label: "main" })).data;
await ev(`(() => { const fp = (window.__fp = { frames: [], run: true }); const loop = (t) => { if (!fp.run) return; fp.frames.push(t); requestAnimationFrame(loop); }; requestAnimationFrame(loop); return true; })()`);
const samp = spawn("./panesampler", [pid, "3.6"], { stdio: ["ignore", "pipe", "pipe"] }); let out = ""; samp.stdout.on("data", (d) => (out += d));
await sleep(600);
const ps = await ev("performance.now()");
for (let i = 0; i < steps; i++) { const a = performance.now(); const dx = i < steps / 2 ? (i + 1) * stepPx : (steps - i - 1) * stepPx; await ipc("set_window_position", { label: "main", x: r0.x + dx, y: r0.y }); const w = 16 - (performance.now() - a); if (w > 0) await sleep(w); }
const pe = await ev("performance.now()");
await ipc("set_window_position", { label: "main", x: r0.x, y: r0.y });
await new Promise((r) => samp.on("close", r));
const fr = (await ev("(() => { window.__fp.run = false; return window.__fp.frames; })()")).filter((t) => t >= ps && t <= pe + 100);
const gaps = fr.slice(1).map((t, i) => t - fr[i]).sort((a, b) => a - b); const q = (a, p) => +a[Math.min(a.length - 1, Math.floor(p * a.length))].toFixed(1);
const samples = out.trim().split("\n").map((l) => { const [t, ...ws] = l.split(" "); return { t: +t, w: ws.filter(Boolean).map((x) => { const [id, r] = x.split(":"); const [X, Y, W, H] = r.split(",").map(Number); return { id, X, Y, W, H }; }) }; });
const big = (s) => s.w.reduce((a, b) => (a.W * a.H >= b.W * b.H ? a : b));
const first = samples.find((s) => s.w.length > 1); const mid = big(first).id;
const rest = new Map(first.w.filter((w) => w.id !== mid).map((w) => [w.id, { dx: w.X - big(first).X, dy: w.Y - big(first).Y }]));
const dev = []; let mainPositions = new Set(); let lastBad = 0, lastMove = 0, pm = null;
for (const s of samples) { const m = s.w.find((w) => w.id === mid); if (!m) continue; mainPositions.add(m.X); if (pm && pm.X !== m.X) lastMove = s.t; pm = m;
  for (const [id, r0p] of rest) { const w = s.w.find((x) => x.id === id); if (!w || w.W === 0) continue; const d = Math.max(Math.abs((w.X - m.X) - r0p.dx), Math.abs((w.Y - m.Y) - r0p.dy)); dev.push(d); if (d > 1) lastBad = s.t; } }
dev.sort((a, b) => a - b);
console.log(`${label}: page frames ${fr.length} gap p50/p90/max ${q(gaps, .5)}/${q(gaps, .9)}/${q(gaps, 1)} ms | main window distinct x positions seen ${mainPositions.size}/${steps} | panes offset from main (px) p50=${q(dev, .5)} p90=${q(dev, .9)} p99=${q(dev, .99)} max=${dev.at(-1)} | share >1px: ${(100 * dev.filter((d) => d > 1).length / dev.length).toFixed(0)}% | last drift ${(lastBad - lastMove).toFixed(0)} ms after the last main move | load ${os.loadavg()[0].toFixed(1)}`);
process.exit(0);
