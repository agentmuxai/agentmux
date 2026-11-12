#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The window-tab switch lab (docs/reports/REPORT_TAB_SWITCH_CATCH_UP_2026_11_11.md §4): what a person sees on
// the agent panes of a window tab in the frames after the switch to it, before and after a change.
//
//   node scripts/perf/tab-switch-lab.mjs --home .dev-home/5302 --patch <change.patch> [--repeat 2] [--out DIR]
//        [--cases A,B,C] [--lines 3000] [--hidden-s 45] [--settle-s 30]
//
// Drives a `task dev` build on its own isolated home, over CDP (never the machine's own instance): a first window
// tab with one scripted agent and a second with three, each given a transcript of --lines lines by playing the fake
// Claude's turns. The change under test is a patch of the frontend that Vite serves from the clone: the lab applies it
// and takes it off between measurements (the tree must start with the patch NOT applied; it ends as it started), and
// reloads the page after each swap, so before and after are measured in one session, on the same transcripts.
//
// A measurement is the switch to the second tab as a call in the page (the tab's own click handler), with a probe
// that samples every animation frame for 3.5 s: each pane's scroller geometry, whether the transcript is covered,
// and the screen position of the rows in view.
//   A  first show since the page loaded, three finished panes (the operator's 17:00:18 switch)
//   B  show after the tab was hidden --hidden-s seconds while one agent kept working
//   C  as B with all three agents working
// Also taken in A, before the tab is ever shown: the page's heap after a collection and its DOM node count.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { findInstancePort } from "../dev-seed.mjs";
import { Screencast, encode, findFfmpeg } from "../ui-screenshots/lib/recorder.mjs";
import { generateConversation } from "./memory-lab/lib/conversation.mjs";
import { connect, installScript } from "./memory-lab/lib/driver.mjs";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const PROBE_MS = 3500;

/** The in-page probe: `__tabLab.run(blockIds, tabSelector, ms)` clicks the tab and samples every frame. */
export const PROBE = `(() => {
  const lab = (window.__tabLab = {});
  lab.run = (blockIds, tabSelector, ms) =>
    new Promise((resolve) => {
      const frames = blockIds.map(() => []);
      const longTasks = [];
      const logs = [];
      const observer = new PerformanceObserver((list) => {
        for (const e of list.getEntries()) longTasks.push({ at: Math.round(e.startTime - origin), ms: Math.round(e.duration) });
      });
      try { observer.observe({ type: "longtask" }); } catch {}
      const info = console.info;
      console.info = function (...a) {
        try {
          const s = a.map(String).join(" ");
          if (/pane-cover|agent-open-rest|prewarm/.test(s)) logs.push({ at: Math.round(performance.now() - origin), s: s.slice(0, 160) });
        } catch {}
        return info.apply(this, a);
      };
      let origin = performance.now();
      const sample = (i) => {
        const block = document.querySelector('[data-blockid="' + blockIds[i] + '"]');
        const el = block && block.querySelector(".agent-document");
        if (!el) return null;
        const r = el.getBoundingClientRect();
        const hit = document.elementFromPoint(r.left + r.width / 2, r.top + Math.min(r.height / 2, 200));
        const rows = [];
        for (const row of el.querySelectorAll("[data-node-id]")) {
          const b = row.getBoundingClientRect();
          if (b.bottom < r.top || b.top > r.bottom) continue;
          rows.push([row.getAttribute("data-node-id"), Math.round((b.top - r.top) * 10) / 10]);
          if (rows.length >= 16) break;
        }
        return {
          vis: !!(hit && block.contains(hit)),
          cov: el.hasAttribute("data-show-cover"),
          fol: el.getAttribute("data-follow-state"),
          st: Math.round(el.scrollTop * 10) / 10,
          sh: el.scrollHeight,
          ch: el.clientHeight,
          rows,
        };
      };
      const tick = () => {
        const t = Math.round((performance.now() - origin) * 10) / 10;
        for (let i = 0; i < blockIds.length; i++) {
          const s = sample(i);
          frames[i].push(s ? { t, ...s } : null);
        }
        if (t < ms) requestAnimationFrame(tick);
        else {
          observer.disconnect();
          console.info = info;
          resolve({ frames, longTasks, logs });
        }
      };
      requestAnimationFrame(() => {
        origin = performance.now();
        document.querySelector(tabSelector).click();
        tick();
      });
    });
  return true;
})()`;

/**
 * What one pane did in the frames after the switch. `frames` are the probe's samples (null when the pane is not
 * mounted). A frame in which the transcript is covered paints nothing of it, so motion is judged between painted frames.
 */
export function summarizePane(frames) {
  const f = frames.filter(Boolean);
  const first = f.findIndex((x) => x.vis);
  if (first < 0) return { shown: false };
  const out = { shown: true, startedVisible: first === 0 && f[0].t < 5, shownAtMs: f[first].t, coverMs: 0, jumpFrames: 0, bigGapFrames: 0, firstBigGapAtMs: null, movedFrames: 0, movedPx: 0, settledAtMs: null, followGapFrames: 0, maxFollowGapPx: 0, heightSteps: 0, heightStepPx: 0, blankFrames: 0 };
  let coverStart = null;
  let coverEnd = null;
  let prev = null;
  for (const x of f.slice(first)) {
    if (x.cov) {
      coverStart ??= x.t;
      coverEnd = x.t;
      out.blankFrames++;
      prev = null;
      continue;
    }
    const gap = x.sh - x.ch - x.st;
    if (x.fol === "following" && gap > 2) {
      out.followGapFrames++;
      out.maxFollowGapPx = Math.max(out.maxFollowGapPx, Math.round(gap));
      // painted while following and more than half a screen short of the bottom: the pane shows older content for the frame
      if (gap > x.ch / 2) {
        out.bigGapFrames++;
        out.firstBigGapAtMs ??= x.t;
      }
    }
    if (prev) {
      const before = new Map(prev.rows);
      let worst = 0;
      for (const [id, top] of x.rows) {
        const was = before.get(id);
        if (was !== undefined) worst = Math.max(worst, Math.abs(top - was));
      }
      // no row in common with the last painted frame: the view was replaced, not moved
      if (prev.rows.length && x.rows.length && !x.rows.some(([id]) => before.has(id))) out.jumpFrames++;
      if (worst > 2) {
        out.movedFrames++;
        out.movedPx += worst;
        out.settledAtMs = x.t;
      }
      const dh = Math.abs(x.sh - prev.sh);
      if (dh > 2) {
        out.heightSteps++;
        out.heightStepPx += dh;
      }
    }
    prev = x;
  }
  if (coverStart != null) out.coverMs = Math.round(coverEnd - coverStart);
  out.movedPx = Math.round(out.movedPx);
  out.heightStepPx = Math.round(out.heightStepPx);
  return out;
}

/** Long tasks of 50 ms or more in the window, and their sum. */
export function summarizeLongTasks(tasks) {
  return { count: tasks.length, totalMs: tasks.reduce((a, t) => a + t.ms, 0), worstMs: tasks.reduce((a, t) => Math.max(a, t.ms), 0) };
}

const median = (xs) => (xs.length ? [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)] : null);

/** One line per case and variant: the median over the repeats of what the three panes did together. */
export function tableOf(results) {
  const rows = [];
  for (const [variant, byCase] of Object.entries(results)) {
    for (const [name, all] of Object.entries(byCase)) {
      const runs = all.filter((r) => r.valid !== false);
      const per = (key, pick = Math.max) => median(runs.map((r) => pick(...r.panes.filter((p) => p.shown).map((p) => p[key] ?? 0))));
      rows.push({
        variant,
        case: name,
        runs: runs.length,
        coverMs: per("coverMs"),
        jumpFrames: median(runs.map((r) => r.panes.reduce((a, p) => a + (p.jumpFrames ?? 0), 0))),
        bigGapFrames: median(runs.map((r) => r.panes.reduce((a, p) => a + (p.bigGapFrames ?? 0), 0))),
        movedFrames: median(runs.map((r) => r.panes.reduce((a, p) => a + (p.movedFrames ?? 0), 0))),
        movedPx: median(runs.map((r) => r.panes.reduce((a, p) => a + (p.movedPx ?? 0), 0))),
        settledAtMs: per("settledAtMs"),
        heightSteps: median(runs.map((r) => r.panes.reduce((a, p) => a + (p.heightSteps ?? 0), 0))),
        longTaskMs: median(runs.map((r) => r.longTasks.totalMs)),
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
    patch: resolve(get("--patch", "")),
    out: resolve(get("--out", `tab-switch-lab-${new Date().toISOString().replace(/[:.]/g, "-")}`)),
    repeat: Number(get("--repeat", 2)),
    cases: get("--cases", "A,B,C").split(","),
    lines: Number(get("--lines", 3000)),
    hiddenS: Number(get("--hidden-s", 45)),
    settleS: Number(get("--settle-s", 30)),
    record: !argv.includes("--no-record"),
  };
}

const git = (args) => execFileSync("git", args, { encoding: "utf8" });

/** The page has loaded far enough to call srv: a dev page reloads slowly while Vite transforms its modules. */
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

/** Plays turns on every agent until each transcript has `lines` lines (or the turns run out). */
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

async function reload(d, settleS) {
  await d.eval("location.reload()").catch(() => {});
  await sleep(4000);
  await d.reconnect();
  await pageReady(d);
  await d.eval(`window.__agentmuxOneWay && window.__agentmuxOneWay.enable()`);
  await sleep(settleS * 1000);
}

const tabSelector = (id) => `.tab[data-tab-id="${id}"]`;

/** Takes one measurement: the state set up by `prep`, the switch, 3.5 s of frames. */
async function switchTo(d, tab2, panes) {
  if ((await d.eval("document.visibilityState")) !== "visible") throw new Error("the lab window is hidden (minimized or covered): it paints no frames, so nothing can be measured");
  await d.eval(PROBE);
  await d.eval(`window.__agentmuxOneWay && window.__agentmuxOneWay.reset()`);
  const raw = await d.eval(`window.__tabLab.run(${JSON.stringify(panes.map((a) => a.blockId))}, ${JSON.stringify(tabSelector(tab2))}, ${PROBE_MS})`);
  const oneWay = await d.eval(`(window.__agentmuxOneWay ? window.__agentmuxOneWay.snapshot() : []).map((s) => ({ pane: s.pane, down: s.down, overshoot: s.overshoot, tight: s.tight, blankFrames: s.blankFrames, byCause: s.byCause }))`);
  return {
    panes: raw.frames.map(summarizePane),
    rawFrames: raw.frames.map((fr) => fr.map((x) => x && { ...x, rows: x.rows.slice(0, 6) })),
    longTasks: { ...summarizeLongTasks(raw.longTasks), tasks: raw.longTasks },
    logs: raw.logs,
    oneWay,
  };
}

async function runCase(d, name, args, tab1, tab2, panes) {
  await reload(d, name === "A" ? args.settleS : 12);
  if ((await d.eval(`document.querySelector(".tab.active")?.getAttribute("data-tab-id")`)) !== tab1) {
    await d.eval(`document.querySelector(${JSON.stringify(tabSelector(tab1))}).click()`);
    await sleep(4000);
  }
  let heap = null;
  if (name === "A") {
    await d.session.send("HeapProfiler.collectGarbage").catch(() => {});
    const usage = await d.session.send("Runtime.getHeapUsage").catch(() => null);
    heap = { usedMB: usage ? Math.round(usage.usedSize / 1048576) : null, nodes: await d.eval("document.getElementsByTagName('*').length") };
  } else {
    // warm the tab (it has been shown once), go back, and let the agents work while it is hidden
    await d.eval(`document.querySelector(${JSON.stringify(tabSelector(tab2))}).click()`);
    await sleep(5000);
    await d.eval(`document.querySelector(${JSON.stringify(tabSelector(tab1))}).click()`);
    await sleep(3000);
    const working = name === "B" ? panes.slice(0, 1) : panes;
    const until = Date.now() + args.hiddenS * 1000;
    while (Date.now() < until) {
      await d.send(working.map((a) => a.blockId));
      await sleep(Math.min(15000, Math.max(0, until - Date.now())));
    }
    // a last turn right before the switch, so they are still working when it happens
    await d.send(working.map((a) => a.blockId));
    await sleep(1500);
  }
  const m = await switchTo(d, tab2, panes);
  await d.eval(`document.querySelector(${JSON.stringify(tabSelector(tab1))}).click()`);
  await sleep(1500);
  return { ...m, heap, valid: m.panes.every((p) => p.shown && !p.startedVisible) };
}

async function record(d, args, variant, tab1, tab2, panes) {
  await reload(d, args.settleS);
  const shot = new Screencast(d.session);
  await shot.start();
  await sleep(500);
  await switchTo(d, tab2, panes);
  const { frames, end } = await shot.stop();
  const files = encode({ frames, end, out: join(args.out, `switch-${variant}`), formats: ["mp4"], width: 1000, fps: 30, ffmpeg: findFfmpeg() });
  await d.eval(`document.querySelector(${JSON.stringify(tabSelector(tab1))}).click()`);
  return files;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!existsSync(args.patch)) throw new Error("--patch must name the change as a patch of the tree (git diff base..change -- frontend)");
  mkdirSync(args.out, { recursive: true });
  const port = findInstancePort(args.home);
  if (!port) throw new Error(`no running dev build is using ${args.home}`);
  const script = join(args.home, "tab-lab-conversation.json");
  writeFileSync(script, JSON.stringify(generateConversation({ turns: 40, seed: 1, realistic: true })));
  installScript(args.home, script);
  const d = await connect(port, args.home, join(args.home, "lab-work"));
  await pageReady(d);
  // The logo menu's "New Tab" is a synthetic click away from changing: make the tab through the workspace service, as the menu does.
  d.newTab = async () => {
    const id = await d.eval(`MOS.callBackendService("workspace", "CreateTab", [globalAtoms.workspace().oid, "lab", true, false])`);
    for (let waited = 0; (await d.eval(`document.querySelector(".tab.active")?.getAttribute("data-tab-id")`)) !== id; waited += 500) {
      if (waited > 15000) throw new Error("the new tab did not become active");
      await sleep(500);
    }
    await sleep(2000);
    return id;
  };
  const statePath = join(args.home, "tab-lab-state.json");
  let tabs;
  if (existsSync(statePath)) {
    const s = JSON.parse(readFileSync(statePath, "utf8"));
    d.agents.push(...s.agents);
    tabs = s.tabs;
  } else {
    await d.setupAccount();
    await pageReady(d);
    await d.build([1, 3]);
    const ids = await d.eval(`[...document.querySelectorAll('.tab[data-tab-id]')].map((t) => t.getAttribute('data-tab-id'))`);
    tabs = ids.slice(0, 2);
    writeFileSync(statePath, JSON.stringify({ agents: d.agents, tabs }));
  }
  console.log(`dev build on port ${port}, tabs ${tabs.join(" ")}, agents ${d.agents.map((a) => a.name).join(" ")}`);
  const counts = await fillTranscripts(d, args.lines);
  const [tab1, tab2] = tabs;
  const panes = d.agents.filter((a) => a.tabId === tab2);
  if (panes.length !== 3) throw new Error(`expected 3 agents in the second tab, found ${panes.length}`);

  let applied = false;
  const setVariant = async (variant) => {
    const want = variant === "after";
    if (want !== applied) {
      git(want ? ["apply", args.patch] : ["apply", "-R", args.patch]);
      applied = want;
      await sleep(6000); // Vite picks the files up
    }
  };
  const results = { before: {}, after: {} };
  const heaps = { before: [], after: [] };
  try {
    for (const name of args.cases) {
      for (let rep = 1; rep <= args.repeat; rep++) {
        for (const variant of ["before", "after"]) {
          await setVariant(variant);
          console.log(`case ${name} run ${rep}: ${variant}`);
          let r = await runCase(d, name, args, tab1, tab2, panes);
          if (!r.valid) {
            console.log("  the second tab was already displayed at the click: measuring again");
            (results[variant][name] ??= []).push(r);
            r = await runCase(d, name, args, tab1, tab2, panes);
          }
          (results[variant][name] ??= []).push(r);
          if (r.heap && r.valid) heaps[variant].push(r.heap);
          writeFileSync(join(args.out, "tab-switch-lab.json"), JSON.stringify({ args, port, counts, heaps, results }, null, 1));
        }
      }
    }
    const videos = {};
    if (args.record) {
      for (const variant of ["before", "after"]) {
        await setVariant(variant);
        videos[variant] = await record(d, args, variant, tab1, tab2, panes);
      }
    }
    const table = tableOf(results);
    writeFileSync(join(args.out, "tab-switch-lab.json"), JSON.stringify({ args, port, counts, heaps, results, table, videos }, null, 1));
    console.table(table);
    console.log("heap before the second tab is shown:", JSON.stringify(heaps));
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
