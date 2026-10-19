// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Is the window's main thread blocked while the agents stream? A slow page command in the lab
// can be the page (its thread busy, so a command waits to run or its answer waits to be read) or
// srv (a handler waiting). The page's long tasks (the Long Tasks API: any main-thread task of 50 ms
// or more) are the direct evidence for the first: install the observer at the start of the workload,
// read it at the end, and set it beside each round's window and each send's time.

/** Installed in the page. Keeps every long task (start as epoch ms, duration) and the ten longest with what Chromium says about them. */
export const LONG_TASK_OBSERVER = `(() => {
  if (window.__labLong) return true;
  const L = (window.__labLong = { n: 0, tasks: [], top: [] });
  try {
    window.__labLongObserver = new PerformanceObserver((list) => {
      for (const e of list.getEntries()) {
        const at = Math.round(performance.timeOrigin + e.startTime);
        const ms = Math.round(e.duration);
        L.n++;
        if (L.tasks.length < 20000) L.tasks.push([at, ms]);
        const a = e.attribution && e.attribution[0];
        L.top.push({ at, ms, name: e.name, container: a ? [a.containerType, a.containerName, a.containerSrc, a.containerId].filter(Boolean).join(" ") : "" });
        L.top.sort((x, y) => y.ms - x.ms);
        if (L.top.length > 10) L.top.length = 10;
      }
    });
    window.__labLongObserver.observe({ type: "longtask" }); // not buffered: what ran before the workload (startup, the layout) is not the workload
    return true;
  } catch (e) {
    return String(e);
  }
})()`;

/** Reads the observer's record back (null when it was never installed, e.g. after a reload). */
export const READ_LONG_TASKS = `window.__labLong ? { n: window.__labLong.n, tasks: window.__labLong.tasks, top: window.__labLong.top } : null`;

/** Stops the observer and forgets the page's record, so the next install starts a fresh one and the old one collects nothing more. */
export const DETACH_LONG_TASKS = `(window.__labLongObserver && window.__labLongObserver.disconnect(), window.__labLongObserver = null, window.__labLong = null, true)`;

/** One record from several (the document before a reload, and the one after): counts add, tasks join, the ten longest stay. */
export function mergeLongTaskRecords(records) {
    const list = records.filter(Boolean);
    if (!list.length) return null;
    return {
        n: list.reduce((n, r) => n + r.n, 0),
        tasks: list.flatMap((r) => r.tasks),
        top: list.flatMap((r) => r.top).sort((a, b) => b.ms - a.ms).slice(0, 10),
    };
}

/**
 * Summarises the long tasks of a run: totals, the longest with its attribution, and per round
 * (`windows`: [{ round, start, end }] in epoch ms) how many there were, how long they took
 * in all, and the longest, so growth with the streaming panes shows. Tasks that overlap an
 * `excluded` window (the lab's own forced-GC memory samples, whose pauses are the instrument's,
 * not the workload's) are left out and counted in `excludedTasks`.
 */
export function summariseLongTasks(raw, windows = [], excluded = []) {
    if (!raw) return null;
    const all = raw.tasks.map(([at, ms]) => ({ at, ms }));
    const inExcluded = (t) => excluded.some((w) => t.at < w.end && t.at + t.ms > w.start);
    const tasks = all.filter((t) => !inExcluded(t));
    const total = tasks.reduce((n, t) => n + t.ms, 0);
    const byRound = windows.map((w) => {
        const inside = tasks.filter((t) => t.at >= w.start && t.at < w.end);
        return { round: w.round, count: inside.length, totalMs: inside.reduce((n, t) => n + t.ms, 0), maxMs: inside.reduce((m, t) => Math.max(m, t.ms), 0) };
    });
    return {
        count: raw.n - (all.length - tasks.length),
        excludedTasks: all.length - tasks.length,
        recorded: tasks.length,
        // Past the cap the page counts tasks but stops keeping their times: totals, per-round and overlap figures then cover only the recorded ones.
        truncated: raw.n > all.length,
        totalMs: total,
        maxMs: tasks.reduce((m, t) => Math.max(m, t.ms), 0),
        over1s: tasks.filter((t) => t.ms >= 1000).length,
        longest: raw.top.filter((t) => !inExcluded(t)).map((t) => ({ ...t, at: new Date(t.at).toISOString() })),
        byRound,
    };
}

/** Which of the sends (each `{ start, ms }`, start in epoch ms) overlap a page long task of 500 ms or more, and how many are slow (over 5 s) in all. */
export function slowSendsAgainstLongTasks(sends, raw, slowMs = 5000) {
    const tasks = (raw?.tasks ?? []).map(([at, ms]) => ({ at, ms })).filter((t) => t.ms >= 500);
    const slow = sends.filter((s) => s.ms >= slowMs);
    const overlapping = slow.filter((s) => tasks.some((t) => t.at < s.start + s.ms && t.at + t.ms > s.start));
    return { slowSends: slow.length, overlappingLongTask: overlapping.length };
}
