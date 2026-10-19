// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How long it takes to open an agent whose conversation is already in the store (SPEC_AGENT_OPEN_FIRST_SCREEN_THEN_REST,
// P0.5). The pane already writes one `[agent-open]` line per open (frontend/app/view/agent/open-trace.ts: the phases since the
// pane mounted, in ms) and an `[agent-open-rest]` line when the older history has finished loading. The lab reads those two
// lines in the page, and adds what the page cannot know about itself: when the open was asked for (before the block even
// existed) and how long the main thread was blocked while it ran.

/**
 * Installed in the page: keeps the `[agent-open...]` lines the app logs, with the time (epoch ms) each was logged. Wraps `console.info`
 * and `console.log`; `READ_OPENS` puts the wrapper back if something in the page has replaced it since (and counts how often).
 */
export const OPEN_TAP = `(() => {
  const L = (window.__labOpens ??= { lines: [], rewraps: 0 });
  window.__labOpenTapInstall = () => {
    for (const method of ["info", "log"]) {
      if (console[method].__labOpenTap) continue;
      const orig = console[method];
      const wrapper = function (...args) {
        const first = args[0];
        if (typeof first === "string" && first.startsWith("[agent-open") && L.lines.length < 1000) L.lines.push([Date.now(), first]);
        return orig.apply(this, args);
      };
      wrapper.__labOpenTap = true;
      console[method] = wrapper;
    }
  };
  window.__labOpenTapInstall();
  return true;
})()`;

/** Checks the tap is still in place (re-installing it if not) and returns its record: { lines: [[epoch ms, line], ...], rewraps }, or null before it was ever installed. */
export const READ_OPENS = `(() => {
  const L = window.__labOpens;
  if (!L) return null;
  if (!console.info.__labOpenTap || !console.log.__labOpenTap) {
    L.rewraps++;
    window.__labOpenTapInstall();
  }
  return L;
})()`;

/** The `key=value` pairs of one of the app's lines (numbers as numbers, quoted values unquoted), with which line it is. */
export function parseOpenLine(line) {
    const kind = line.startsWith("[agent-open-rest]") ? "rest" : line.startsWith("[agent-open]") ? "open" : null;
    if (!kind) return null;
    const fields = {};
    for (const m of line.matchAll(/(\w+)=("(?:[^"\\]|\\.)*"|\S+)/g)) {
        let v = m[2];
        if (v.startsWith('"')) {
            try {
                v = JSON.parse(v);
            } catch {
                v = v.slice(1, -1);
            }
        } else if (/^-?\d+(\.\d+)?$/.test(v)) v = Number(v);
        fields[m[1]] = v;
    }
    return { kind, fields };
}

/** The first `[agent-open]` / `[agent-open-rest]` line of the tap's records for a block (the lines carry the block's first 8 characters). */
export function findOpenLines(records, blockId) {
    const prefix = String(blockId).slice(0, 8);
    let open = null;
    let rest = null;
    for (const [at, line] of records?.lines ?? []) {
        const p = parseOpenLine(line);
        if (!p || p.fields.block !== prefix) continue;
        if (p.kind === "open" && !open) open = { at, ...p.fields };
        else if (p.kind === "rest" && !rest) rest = { at, ...p.fields };
    }
    return { open, rest };
}

const round1 = (x) => Math.round(x * 10) / 10;

/**
 * One open, in ms from the moment it was asked for (`rpcStart`, epoch ms). The app's phases count from the pane's mount, and
 * the line is logged when the page has been quiet for a while (`at`), so the mount is `at - total`.
 *
 * - `firstScreenMs`: the first of the history on screen (the `first_screen` phase; the `painted` one on a build without it)
 * - `paintedMs`, `revealedMs`, `quietMs`: the later phases
 * - `restMs`: when the older history had finished loading, from the `[agent-open-rest]` line (null when it never came)
 */
export function describeOpen({ agent, kind, rpcStart, rpcMs, open, rest }) {
    if (!open) return { agent, kind, rpcMs, outcome: "no-line" };
    const mountedAt = open.at - open.total;
    const from = (mark) => (typeof mark === "number" ? round1(mountedAt - rpcStart + mark) : null);
    const out = {
        agent,
        kind,
        rpcMs,
        outcome: open.outcome,
        source: open.source,
        mountAfterMs: round1(mountedAt - rpcStart),
        firstScreenMs: from(open.first_screen ?? open.painted),
        firstScreenPhase: open.first_screen != null ? "first_screen" : open.painted != null ? "painted" : null,
        paintedMs: from(open.painted),
        revealedMs: from(open.revealed),
        quietMs: round1(open.at - rpcStart),
        historyLines: open.history_lines ?? null,
        historyReadMs: typeof open.history_read === "number" && typeof open.history_start === "number" ? round1(open.history_read - open.history_start) : null,
        phases: Object.fromEntries(Object.entries(open).filter(([k, v]) => typeof v === "number" && !["at", "total"].includes(k))),
        rest: rest ? { outcome: rest.outcome, pages: rest.pages, lines: rest.lines, bytes: rest.bytes, ms: rest.ms, doneAfterMs: round1(rest.at - rpcStart) } : null,
        // The span the open's visible part took on the main thread, for `longestTaskIn`.
        span: { start: rpcStart, firstScreenEnd: open.first_screen != null || open.painted != null ? rpcStart + (from(open.first_screen ?? open.painted) ?? 0) : null, end: open.at },
    };
    return out;
}

/** The longest main-thread task (ms) that overlapped [from, to], from the long-task record's `[start, ms]` pairs; 0 when none did. */
export function longestTaskIn(tasks, from, to) {
    let longest = 0;
    for (const [at, ms] of tasks ?? []) if (at < to && at + ms > from && ms > longest) longest = ms;
    return longest;
}

/** Adds to each open the longest long task during its wait for the first screen and during the whole open (the page's tasks of 50 ms or more). */
export function withLongTasks(opens, tasks) {
    return opens.map((o) => {
        if (!o.span) return o;
        const { start, firstScreenEnd, end } = o.span;
        const { span, ...rest } = o;
        return { ...rest, longestTaskToFirstScreenMs: firstScreenEnd == null ? null : longestTaskIn(tasks, start, firstScreenEnd), longestTaskDuringOpenMs: longestTaskIn(tasks, start, end) };
    });
}

const median = (v) => {
    const s = [...v].sort((a, b) => a - b);
    return s.length ? (s.length % 2 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2) : null;
};
const nums = (list, key) => list.map((o) => o[key]).filter((x) => typeof x === "number");

/** Per kind ("regular" / "heavy"): how many opens, the median and the worst first screen, and the worst long task during an open. */
export function summariseOpens(opens) {
    const out = {};
    for (const kind of [...new Set(opens.map((o) => o.kind))]) {
        const list = opens.filter((o) => o.kind === kind);
        const first = nums(list, "firstScreenMs");
        out[kind] = {
            opens: list.length,
            noLine: list.filter((o) => o.outcome === "no-line").length,
            unsettled: list.filter((o) => o.outcome !== "quiet" && o.outcome !== "no-line").length,
            firstScreenMedianMs: first.length ? round1(median(first)) : null,
            firstScreenMaxMs: first.length ? Math.max(...first) : null,
            paintedMedianMs: nums(list, "paintedMs").length ? round1(median(nums(list, "paintedMs"))) : null,
            revealedMedianMs: nums(list, "revealedMs").length ? round1(median(nums(list, "revealedMs"))) : null,
            restMedianMs: list.some((o) => o.rest) ? round1(median(list.filter((o) => o.rest).map((o) => o.rest.doneAfterMs))) : null,
            longestTaskMs: Math.max(0, ...nums(list, "longestTaskDuringOpenMs")),
        };
    }
    return out;
}

/** One line for the run's summary. */
export function opensLine(summary) {
    const parts = Object.entries(summary).map(([kind, s]) => {
        const first = s.firstScreenMedianMs == null ? "no first screen seen" : `first screen median ${s.firstScreenMedianMs} ms, worst ${s.firstScreenMaxMs} ms`;
        const bad = s.noLine || s.unsettled ? ` (${[s.noLine && `${s.noLine} without a line`, s.unsettled && `${s.unsettled} not settled`].filter(Boolean).join(", ")})` : "";
        return `${kind}: ${s.opens} open(s), ${first}, longest main-thread task ${s.longestTaskMs} ms${bad}`;
    });
    return `Opening an agent again after its pane was closed, from the request to the first screen: ${parts.join("; ")}.`;
}
