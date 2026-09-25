#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Per-switch cost of window-tab switches, from a host log.
//
// Usage: node scripts/tab-switch-report.mjs <agentmux-host-*.log> [--since HH:MM] [--window-ms 3000]
//
// For each `workspace.SetActiveTab` it prints the RPC time, when the reveal
// gate lifted (ms after the RPC line), and the long tasks in the window
// after the reveal: work the gate had already declared settled, which is
// what reads as a glitch. Method and baseline:
// docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_SMOOTHNESS_2026_09_24.md §2.
// Compare runs with `window:keepinactivetabslaidout` off and on.

import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
const file = args.find((a) => !a.startsWith("--"));
if (!file) {
  console.error("usage: node scripts/tab-switch-report.mjs <host log> [--since HH:MM] [--window-ms 3000]");
  process.exit(2);
}
const flag = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};
const since = flag("--since", "00:00");
const windowMs = Number(flag("--window-ms", "3000"));

const events = [];
for (const line of readFileSync(file, "utf8").split("\n")) {
  if (!line.includes("SetActiveTab") && !line.includes("tab-reveal whole-tab") && !line.includes("[perf] long-task")) {
    continue;
  }
  let o;
  try {
    o = JSON.parse(line);
  } catch {
    continue;
  }
  const msg = o?.fields?.message;
  const t = Date.parse(o?.timestamp);
  if (typeof msg !== "string" || Number.isNaN(t)) continue;
  events.push({ t, msg, hhmm: o.timestamp.slice(11, 16) });
}
events.sort((a, b) => a.t - b.t);

const rows = [];
for (let i = 0; i < events.length; i++) {
  const e = events[i];
  if (!e.msg.includes("workspace.SetActiveTab") || e.hhmm < since) continue;
  const rpc = e.msg.split("|")[1]?.trim() ?? "?";
  let reveal = null;
  const after = [];
  for (const f of events.slice(i + 1)) {
    const dt = f.t - e.t;
    if (dt > windowMs + 1000 || f.msg.includes("workspace.SetActiveTab")) break;
    if (reveal == null && f.msg.includes("tab-reveal")) {
      reveal = dt;
    } else if (reveal != null && f.msg.includes("long-task") && dt - reveal <= windowMs) {
      const ms = Number(/long-task ([\d.]+)ms/.exec(f.msg)?.[1]);
      if (!Number.isNaN(ms)) after.push(ms);
    }
  }
  rows.push({
    at: new Date(e.t).toISOString().slice(11, 19),
    rpc,
    reveal,
    n: after.length,
    total: Math.round(after.reduce((a, b) => a + b, 0)),
  });
}

console.log("time(UTC)  rpc     reveal  long tasks after reveal");
for (const r of rows) {
  console.log(
    `${r.at}   ${r.rpc.padEnd(6)}  ${r.reveal == null ? "  -  " : `${r.reveal}ms`.padEnd(6)}  ${r.n} (${r.total} ms)`
  );
}
const withReveal = rows.filter((r) => r.reveal != null);
if (withReveal.length) {
  // Even counts average the two middle samples (Codex P2 on #3686).
  const median = (xs) => {
    const s = [...xs].sort((a, b) => a - b);
    const mid = Math.floor(s.length / 2);
    return s.length % 2 ? s[mid] : Math.round((s[mid - 1] + s[mid]) / 2);
  };
  console.log(
    // Only switches whose reveal was seen: the rest have no "after reveal"
    // window, and their zeros would understate the cost (ReAgent P2 on #3686).
    `\n${rows.length} switches, ${withReveal.length} with a reveal · median reveal ${median(withReveal.map((r) => r.reveal))} ms · median after-reveal long tasks ${median(withReveal.map((r) => r.total))} ms · worst ${Math.max(...withReveal.map((r) => r.total))} ms`
  );
}
