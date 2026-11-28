#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The pane-minimize click lab: where the time goes between a click on a pane's Minimize button and the frame that shows
// the result, in a window of many agent panes, with and without agents streaming, before and after a change.
//
//   node scripts/perf/minimize-click-lab.mjs --home .dev-home/5302 [--patch <change.patch>] [--agents 12]
//        [--repeat 4] [--merge-repeat 2] [--streaming 4] [--lines 1500] [--out DIR]
//
// Drives a `task dev` build on its own isolated home over CDP (never the machine's own instance), as tab-switch-lab.mjs
// does: one window tab of --agents scripted agents, each given a transcript of --lines lines. A measurement is one real
// mouse click (CDP `Input.dispatchMouseEvent`, so the browser times it as input) on a pane's Minimize button, with
//   - the Event Timing entry of the click (input delay: the click waited behind other work; processing: the handler;
//     presentation: from the handler's end to the next paint),
//   - the first animation frame in which the pane has changed on screen (its element gone for a merge, its height cut
//     for a header chip),
//   - the long tasks and the slowest frame gap in the 3 s after, and
//   - a CPU profile of the same 3 s, reduced to the functions with the most self time.
// Cases: `chip` (Shift + click: the pane becomes a header chip, restored by the next click) and `merge` (a plain click:
// the pane's tabs move into a neighbour, which cannot be restored, so each merge uses a pane of its own), each idle and
// `busy` (--streaming agents are given a turn just before the click, so their panes are mid-stream).
// With --patch (a patch of the frontend that Vite serves from the clone) the lab applies it and takes it off between
// measurements, as tab-switch-lab.mjs does, so before and after are measured in one session on the same transcripts; the
// tree must start with the patch NOT applied and ends as it started.
//
// A lab: it takes the screen of the machine it runs on. The window it drives must be visible (a hidden page paints no
// frames). Not for narko without the operator's yes.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { findInstancePort } from "../dev-seed.mjs";
import { generateConversation } from "./memory-lab/lib/conversation.mjs";
import { connect, installScript } from "./memory-lab/lib/driver.mjs";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const WATCH_MS = 3000;
const SHIFT = 8; // CDP's modifier bit for Shift

/**
 * The in-page probe. `__minLab.arm(blockId, chip)` records, until `read()`: the Event Timing entries of the click, the long
 * tasks, every animation frame's time, and the first frame in which the pane has changed (see `changed`).
 */
export const PROBE = `(() => {
  const lab = (window.__minLab = {});
  lab.arm = (blockId) => {
    lab.stop && lab.stop();
    const sel = '[data-role="pane"][data-blockid="' + blockId + '"]';
    const events = [];
    const longTasks = [];
    const frames = [];
    const heightOf = () => { const el = document.querySelector(sel); return el ? el.getBoundingClientRect().height : null; };
    const before = heightOf();
    const obs = new PerformanceObserver((list) => {
      for (const e of list.getEntries()) {
        if (e.entryType === "event") events.push({ name: e.name, start: e.startTime, procStart: e.processingStart, procEnd: e.processingEnd, dur: e.duration });
        else longTasks.push({ start: e.startTime, ms: e.duration });
      }
    });
    try { obs.observe({ type: "event", durationThreshold: 16 }); } catch {}
    try { obs.observe({ type: "longtask" }); } catch {}
    let downAt = null;
    const onDown = (e) => { if (downAt === null) downAt = e.timeStamp; };
    window.addEventListener("pointerdown", onDown, true);
    let changedAt = null;
    let live = true;
    const tick = (t) => {
      if (!live) return;
      frames.push(t);
      const h = heightOf();
      if (changedAt === null && downAt !== null && (h === null || (before !== null && Math.abs(h - before) > 40))) changedAt = t;
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    lab.stop = () => { live = false; obs.disconnect(); window.removeEventListener("pointerdown", onDown, true); };
    lab.read = () => { lab.stop(); return { events, longTasks, frames, downAt, changedAt, before, after: heightOf() }; };
    return before !== null;
  };
  return true;
})()`;

/** Reduces what the probe read to the numbers a click is judged by (all in ms; null when not seen). */
export function summarizeClick(raw) {
  const click = (raw.events ?? []).filter((e) => e.name === "click").sort((a, b) => b.dur - a.dur)[0];
  const t0 = raw.downAt ?? click?.start ?? null;
  const gaps = [];
  for (let i = 1; i < (raw.frames ?? []).length; i++) if (raw.downAt == null || raw.frames[i] >= raw.downAt) gaps.push(raw.frames[i] - raw.frames[i - 1]);
  const tasks = (raw.longTasks ?? []).filter((t) => t.start + t.ms >= (t0 ?? 0));
  return {
    seen: !!click,
    inputDelay: click ? Math.round(click.procStart - click.start) : null,
    processing: click ? Math.round(click.procEnd - click.procStart) : null,
    presentation: click ? Math.round(click.start + click.dur - click.procEnd) : null,
    clickMs: click ? Math.round(click.dur) : null,
    changedAfter: raw.changedAt != null && t0 != null ? Math.round(raw.changedAt - t0) : null,
    longTaskCount: tasks.length,
    longTaskMs: Math.round(tasks.reduce((a, t) => a + t.ms, 0)),
    worstFrameMs: gaps.length ? Math.round(Math.max(...gaps)) : null,
  };
}

/** The `n` functions with the most self time (ms) in a cpuprofile of the Chrome DevTools protocol. */
export function topSelf(profile, n = 12) {
  const nodes = new Map((profile?.nodes ?? []).map((x) => [x.id, x]));
  const self = new Map();
  const samples = profile?.samples ?? [];
  const deltas = profile?.timeDeltas ?? [];
  for (let i = 0; i < samples.length; i++) self.set(samples[i], (self.get(samples[i]) ?? 0) + (deltas[i] ?? 0));
  const byFn = new Map();
  for (const [id, us] of self) {
    const f = nodes.get(id)?.callFrame;
    if (!f) continue;
    const name = f.functionName || "(anonymous)";
    if (name === "(idle)" || name === "(program)") continue;
    const key = `${name} ${(f.url || "").split("/").slice(-2).join("/")}:${(f.lineNumber ?? 0) + 1}`;
    byFn.set(key, (byFn.get(key) ?? 0) + us / 1000);
  }
  return [...byFn].sort((a, b) => b[1] - a[1]).slice(0, n).map(([fn, ms]) => ({ fn, ms: Math.round(ms * 10) / 10 }));
}

/** Layout and style reads: the native calls that force the page to compute before they answer. */
const READS = new Set(["getBoundingClientRect", "getPropertyValue", "getComputedStyle", "getClientRects", "checkVisibility", "querySelector", "querySelectorAll", "scrollIntoView", "focus", "getBBox", "elementFromPoint"]);
const NOT_A_CALLER = new Set(["(root)", "(program)", "(idle)", "(garbage collector)"]);
const fnKey = (f) => `${f.functionName || "(anonymous)"} ${(f.url || "").split("/").slice(-2).join("/")}:${(f.lineNumber ?? 0) + 1}`;

/**
 * Where the main thread's time goes by caller, from a cpuprofile of the Chrome DevTools protocol. `topSelf` says which
 * function had the time; this says who was on the stack.
 *   inclusive: per function, the ms in which it was anywhere on the stack (a function counts once per sample, however
 *     often it recurs), the `n` largest. A function the page's own code calls shows here with everything below it.
 *   readers: per layout or style read (getBoundingClientRect and the like), the nearest caller that has a script file, with
 *     the self ms of the read, the `n` largest. This is what attributes a read to the code that asked for it.
 * `windowMs` keeps only the samples in the first windowMs of the profile (the click and what follows at once).
 */
export function callersOf(profile, { n = 12, windowMs = Infinity } = {}) {
  const nodes = new Map((profile?.nodes ?? []).map((x) => [x.id, x]));
  const parent = new Map();
  for (const x of nodes.values()) for (const c of x.children ?? []) parent.set(c, x.id);
  const inclusive = new Map();
  const readers = new Map();
  const samples = profile?.samples ?? [];
  const deltas = profile?.timeDeltas ?? [];
  let at = 0;
  for (let i = 0; i < samples.length; i++) {
    const us = deltas[i] ?? 0;
    at += us;
    if (at / 1000 > windowMs) break;
    const stack = [];
    for (let id = samples[i]; id !== undefined; id = parent.get(id)) {
      const f = nodes.get(id)?.callFrame;
      if (f) stack.push(f);
    }
    if (!stack.length || NOT_A_CALLER.has(stack[0].functionName)) continue;
    const seen = new Set();
    for (const f of stack) {
      if (NOT_A_CALLER.has(f.functionName)) continue;
      const k = fnKey(f);
      if (seen.has(k)) continue;
      seen.add(k);
      inclusive.set(k, (inclusive.get(k) ?? 0) + us / 1000);
    }
    if (READS.has(stack[0].functionName)) {
      const caller = stack.slice(1).find((f) => f.url);
      const k = `${stack[0].functionName} <- ${caller ? fnKey(caller) : "(no script caller)"}`;
      readers.set(k, (readers.get(k) ?? 0) + us / 1000);
    }
  }
  const top = (m) => [...m].sort((a, b) => b[1] - a[1]).slice(0, n);
  return {
    inclusive: top(inclusive).map(([fn, ms]) => ({ fn, ms: Math.round(ms * 10) / 10 })),
    readers: top(readers).map(([read, ms]) => ({ read, ms: Math.round(ms * 10) / 10 })),
  };
}

const median = (xs) => {
  const v = xs.filter((x) => x != null).sort((a, b) => a - b);
  return v.length ? v[Math.floor(v.length / 2)] : null;
};

/** One line per variant and case: the median over the valid runs. */
export function tableOf(results) {
  const rows = [];
  for (const [variant, byCase] of Object.entries(results)) {
    for (const [name, all] of Object.entries(byCase)) {
      const runs = all.filter((r) => r.click.seen);
      const per = (key) => median(runs.map((r) => r.click[key]));
      rows.push({
        variant,
        case: name,
        runs: `${runs.length}/${all.length}`,
        inputDelay: per("inputDelay"),
        processing: per("processing"),
        presentation: per("presentation"),
        clickMs: per("clickMs"),
        changedAfter: per("changedAfter"),
        longTaskMs: per("longTaskMs"),
        worstFrameMs: per("worstFrameMs"),
      });
    }
  }
  return rows;
}

// ---- driving ----

function parseArgs(argv) {
  const get = (k, d) => {
    const i = argv.indexOf(k);
    return i >= 0 ? argv[i + 1] : d;
  };
  return {
    home: resolve(get("--home", "")),
    patch: get("--patch", "") ? resolve(get("--patch", "")) : "",
    out: resolve(get("--out", `minimize-click-lab-${new Date().toISOString().replace(/[:.]/g, "-")}`)),
    agents: Number(get("--agents", 12)),
    repeat: Number(get("--repeat", 4)),
    mergeRepeat: Number(get("--merge-repeat", 2)),
    streaming: Number(get("--streaming", 4)),
    lines: Number(get("--lines", 1500)),
    restMs: Number(get("--rest-ms", 0)),
    rounds: Number(get("--rounds", 1)),
    // Maximize (the pane's Magnify button; restored by a second click, which is not counted) and Close (a pane of its own
    // each, as for a merge). 0 leaves them out.
    maxRepeat: Number(get("--max-repeat", 0)),
    closeRepeat: Number(get("--close-repeat", 0)),
  };
}

const git = (args) => execFileSync("git", args, { encoding: "utf8" });

async function pageReady(d) {
  for (let waited = 0; !(await d.eval(`typeof TabRpcClient === "object" && !!document.querySelector(".tab[data-tab-id]")`).catch(() => false)); waited += 1000) {
    if (waited > 180000) throw new Error("the page had no RPC client and tabs after 3 minutes");
    await sleep(1000);
  }
}

async function lineCount(d, blockId) {
  const r = await d.eval(`RpcApi.BlockfileLineCountCommand(TabRpcClient, { block_id: ${JSON.stringify(blockId)}, filename: "output" })`);
  return Number(r?.count ?? 0);
}

async function fillTranscripts(d, lines) {
  for (let round = 1; round <= 40; round++) {
    const counts = await Promise.all(d.agents.map((a) => lineCount(d, a.blockId)));
    console.log(`  transcripts: ${counts.join(" ")} lines`);
    if (Math.min(...counts) >= lines) return counts;
    await d.send(d.agents.map((a) => a.blockId), { staggerMs: 300 });
    let last = counts.join();
    for (let still = 0; still < 4; ) {
      await sleep(5000);
      const now = (await Promise.all(d.agents.map((a) => lineCount(d, a.blockId)))).join();
      still = now === last ? still + 1 : 0;
      last = now;
    }
  }
  throw new Error("the transcripts did not reach the size asked for in 40 rounds");
}

/** One measurement: a click on the Minimize (or Restore) button of the pane `blockId`. */
async function clickOnce(d, blockId, chip, restMs = 0, control = ".block-frame-minimize") {
  if ((await d.eval("document.visibilityState")) !== "visible") throw new Error("the lab window is hidden (minimized or covered): it paints no frames, so nothing can be measured");
  await d.eval(PROBE);
  const button = `[data-role="pane"][data-blockid="${blockId}"] ${control}`;
  const where = await d.eval(`(() => { const el = document.querySelector(${JSON.stringify(button)}); if (!el) return null; const r = el.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
  if (!where) return { skipped: `the pane has no ${control} button` };
  if (!(await d.eval(`window.__minLab.arm(${JSON.stringify(blockId)})`))) return { skipped: "the pane is not on screen" };
  await d.session.send("Profiler.enable");
  await d.session.send("Profiler.setSamplingInterval", { interval: 250 });
  await d.session.send("Profiler.start");
  const modifiers = chip ? SHIFT : 0;
  await d.session.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: where.x, y: where.y });
  // A person rests the pointer on the button before the click; a pane's drag picture starts after 250 ms of rest.
  if (restMs > 0) await sleep(restMs);
  await d.session.send("Input.dispatchMouseEvent", { type: "mousePressed", x: where.x, y: where.y, button: "left", clickCount: 1, modifiers });
  await d.session.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: where.x, y: where.y, button: "left", clickCount: 1, modifiers });
  await sleep(WATCH_MS);
  const { profile } = await d.session.send("Profiler.stop");
  const raw = await d.eval(`window.__minLab.read()`);
  return { click: summarizeClick(raw), top: topSelf(profile), callers: callersOf(profile, { n: 15 }), callers1s: callersOf(profile, { n: 15, windowMs: 1000 }) };
}

/** Gives `n` agents a turn and lets their panes start streaming. */
async function startStreaming(d, panes, n) {
  await d.send(panes.slice(0, n).map((a) => a.blockId));
  await sleep(1500);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  mkdirSync(args.out, { recursive: true });
  const port = findInstancePort(args.home);
  if (!port) throw new Error(`no running dev build is using ${args.home}`);
  // --rounds N alternates before, after, before, after ... (N times each), so a drift in the machine does not read as a gain.
  const variants = args.patch ? Array.from({ length: args.rounds }, (_, r) => ["before", "after"].map((v) => ({ v, key: args.rounds > 1 ? `${v}-${r + 1}` : v }))).flat() : [{ v: "before", key: "before" }];
  if (args.patch && !existsSync(args.patch)) throw new Error("--patch must name the change as a patch of the tree (git diff base..change -- frontend)");
  const merges = 2 * (args.mergeRepeat + args.closeRepeat) * variants.length;
  if (merges > args.agents - 2) throw new Error(`${merges} merges need ${merges + 2} panes: raise --agents or lower --merge-repeat`);
  const script = join(args.home, "min-lab-conversation.json");
  writeFileSync(script, JSON.stringify(generateConversation({ turns: 40, seed: 1, realistic: true })));
  installScript(args.home, script);
  const d = await connect(port, args.home, join(args.home, "lab-work"));
  await pageReady(d);
  await d.setupAccount();
  await pageReady(d);
  await d.build([args.agents]);
  console.log(`dev build on port ${port}, ${d.agents.length} agents`);
  await fillTranscripts(d, args.lines);
  await d.eval(`location.reload()`).catch(() => {});
  await sleep(4000);
  await d.reconnect();
  await pageReady(d);
  await sleep(30000);

  let applied = false;
  const setVariant = async (variant) => {
    const want = variant === "after";
    if (want !== applied) {
      git(want ? ["apply", args.patch] : ["apply", "-R", args.patch]);
      applied = want;
      await sleep(6000); // Vite picks the files up
      await d.eval(`location.reload()`).catch(() => {});
      await sleep(4000);
      await d.reconnect();
      await pageReady(d);
      await sleep(20000);
    }
  };

  const panes = [...d.agents];
  let next = 0; // the next pane a merge may use
  const results = {};
  try {
    for (const { v: variant, key } of variants) {
      await setVariant(variant);
      results[key] = {};
      for (const [busy, label] of [[false, "idle"], [true, "busy"]]) {
        // chip: minimize and restore the same pane, so it is repeatable
        const chipPane = panes[panes.length - 1];
        for (let i = 0; i < args.repeat; i++) {
          if (busy) await startStreaming(d, panes, args.streaming);
          const down = await clickOnce(d, chipPane.blockId, true, args.restMs);
          if (!down.skipped) (results[key][`chip-${label}`] ??= []).push(down);
          else console.log(`  chip-${label}: ${down.skipped}`);
          await sleep(1500);
          await clickOnce(d, chipPane.blockId, false); // restore, not counted
          await sleep(1500);
        }
        for (let i = 0; i < args.mergeRepeat; i++) {
          if (busy) await startStreaming(d, panes, args.streaming);
          const pane = panes[next++];
          const r = await clickOnce(d, pane.blockId, false, args.restMs);
          if (!r.skipped) (results[key][`merge-${label}`] ??= []).push(r);
          else console.log(`  merge-${label}: ${r.skipped}`);
          await sleep(2000);
        }
        // maximize: the same pane each time (the last), restored by a second click, which is not counted
        for (let i = 0; i < args.maxRepeat; i++) {
          if (busy) await startStreaming(d, panes, args.streaming);
          const up = await clickOnce(d, chipPane.blockId, false, args.restMs, ".block-frame-magnify");
          if (!up.skipped) (results[key][`maximize-${label}`] ??= []).push(up);
          else console.log(`  maximize-${label}: ${up.skipped}`);
          await sleep(1500);
          await clickOnce(d, chipPane.blockId, false, 0, ".block-frame-magnify");
          await sleep(1500);
        }
        // close: a pane of its own each time, which cannot be brought back
        for (let i = 0; i < args.closeRepeat; i++) {
          if (busy) await startStreaming(d, panes, args.streaming);
          const pane = panes[next++];
          const r = await clickOnce(d, pane.blockId, false, args.restMs, ".block-frame-default-close");
          if (!r.skipped) (results[key][`close-${label}`] ??= []).push(r);
          else console.log(`  close-${label}: ${r.skipped}`);
          await sleep(2000);
        }
        console.log(`${key} ${label} done`);
        writeFileSync(join(args.out, "minimize-click-lab.json"), JSON.stringify({ args, port, results }, null, 1));
      }
    }
    const table = tableOf(results);
    writeFileSync(join(args.out, "minimize-click-lab.json"), JSON.stringify({ args, port, results, table }, null, 1));
    console.table(table);
    for (const [variant, byCase] of Object.entries(results)) {
      for (const [name, runs] of Object.entries(byCase)) console.log(`${variant} ${name}: top self time of the first run`, JSON.stringify(runs[0]?.top?.slice(0, 6)));
    }
  } finally {
    await setVariant("before").catch(() => {}); // the tree ends as it started: the patch not applied
    d.session.close();
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}
