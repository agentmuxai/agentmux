// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What fills the page's slow frames while the agents stream? The long tasks (lib/longtasks.mjs) say the main thread was
// blocked and for how long; they cannot say whether that was script (a message handler, a flush) or the rendering step
// (style, layout, paint), or which script. The Long Animation Frames API (a frame of 50 ms or more, with its scripts and the
// time its rendering step took) can. The probe also times the websocket's handler per message size, since a big message is
// one task that everything behind it waits for, and reads how many panes each frame's stream flush covered.
//
// Report-only: it observes, wraps `onmessage` once to time it (calling through, parsing nothing), and changes no behaviour.

/** The invokerType values a LoAF script entry has, in the order each frame's script time is split by. The last slot is any other. */
export const SCRIPT_TYPES = ["user-callback", "event-listener", "resolve-promise", "reject-promise", "classic-script", "module-script", "other"];

/** Message size bins (UTF-16 units of the websocket frame's text): the upper edges, and what each is called. */
export const SIZE_EDGES = [1024, 10 * 1024, 100 * 1024, 1024 * 1024];
export const SIZE_LABELS = ["<1 KB", "<10 KB", "<100 KB", "<1 MB", ">=1 MB"];

/** Messages this size or bigger have each one's handling time kept (the ten-odd biggest). */
export const BIG_MESSAGE_CHARS = 100 * 1024;

/** The most frames the page keeps; past it frames are counted but not kept, and the record says so. */
export const FRAME_CAP = 5000;
/** The most distinct "longest script" labels interned per record, and the most big messages kept. */
export const KEY_CAP = 300;
export const BIG_CAP = 20;
/** Panes counted per frame by the product's counter: index = panes flushed in the frame, the last slot "that many or more". */
export const PANE_BINS = 33;

/**
 * Installed in the page, after the long-task observer. Once per record: a second install while one is live does nothing;
 * after `DETACH_FRAMES` a new one starts afresh (the lab detaches and installs again to archive a run's record).
 * Returns true, or why it did not start (an older Chromium without the entry type).
 */
export const FRAME_PROBE = `(() => {
  if (window.__labFrames) return true;
  if (!(window.PerformanceObserver && PerformanceObserver.supportedEntryTypes && PerformanceObserver.supportedEntryTypes.includes("long-animation-frame"))) return "no long-animation-frame";
  const TYPES = ${JSON.stringify(SCRIPT_TYPES)};
  const EDGES = ${JSON.stringify(SIZE_EDGES)};
  const BIG = ${BIG_MESSAGE_CHARS};
  const F = (window.__labFrames = { n: 0, frames: [], keys: [], truncated: false, sizes: EDGES.map(() => [0, 0, 0, 0]).concat([[0, 0, 0, 0]]), big: [] });
  // The product counts the panes each stream flush covered into this when it is present (stream-scheduler.ts); a build without the counter leaves it at zero.
  window.__labFrameCounter = { frames: 0, hist: new Array(${PANE_BINS}).fill(0) };
  const keyIndex = new Map();
  const intern = (k) => {
    let i = keyIndex.get(k);
    if (i === undefined) {
      if (F.keys.length >= ${KEY_CAP}) return -1;
      i = F.keys.length;
      F.keys.push(k);
      keyIndex.set(k, i);
    }
    return i;
  };
  const shortUrl = (u) => String(u || "").replace(/^.*\\//, "").slice(0, 60);
  // A message's time in the page's handler (the tap in longtasks.mjs and the real handler behind it), by size. Wraps onmessage once per socket and reads the record
  // from the window each call, so a detach and a new install keep working without wrapping again. Parses nothing.
  const wsm = window.globalWS;
  if (wsm && !wsm.__labFramesTapped && typeof wsm.onmessage === "function") {
    wsm.__labFramesTapped = true;
    const orig = wsm.onmessage.bind(wsm);
    wsm.onmessage = function (e) {
      const cur = window.__labFrames;
      if (!cur) return orig(e);
      const len = typeof e.data === "string" ? e.data.length : (e.data && e.data.byteLength) || 0;
      const t0 = performance.now();
      try {
        return orig(e);
      } finally {
        const ms = performance.now() - t0;
        let bin = EDGES.length;
        for (let i = 0; i < EDGES.length; i++) if (len < EDGES[i]) { bin = i; break; }
        const b = cur.sizes[bin];
        b[0]++;
        b[1] += len;
        b[2] += ms;
        if (ms > b[3]) b[3] = ms;
        if (len >= BIG) {
          const big = cur.big;
          big.push([Math.round(performance.timeOrigin + t0), len, Math.round(ms * 10) / 10]);
          if (big.length > ${BIG_CAP}) { big.sort((x, y) => y[2] - x[2]); big.length = ${BIG_CAP}; }
        }
      }
    };
  }
  try {
    window.__labFramesObserver = new PerformanceObserver((list) => {
      const cur = window.__labFrames;
      if (!cur) return;
      for (const e of list.getEntries()) {
        cur.n++;
        if (cur.frames.length >= ${FRAME_CAP}) { cur.truncated = true; continue; }
        const start = e.startTime;
        const end = start + e.duration;
        const rs = e.renderStart > 0 ? e.renderStart : end;
        const byType = [0, 0, 0, 0, 0, 0, 0];
        let forced = 0;
        let top = null;
        let topMs = 0;
        let fTop = null;
        let fTopMs = 0;
        for (const s of e.scripts || []) {
          const ti = TYPES.indexOf(s.invokerType);
          byType[ti < 0 ? TYPES.length - 1 : ti] += s.duration;
          const f = s.forcedStyleAndLayoutDuration || 0;
          forced += f;
          if (s.duration > topMs) { topMs = s.duration; top = s; }
          if (f > fTopMs) { fTopMs = f; fTop = s; }
        }
        const label = (s) => [s.invokerType, s.invoker, s.sourceFunctionName, shortUrl(s.sourceURL) + ":" + (s.sourceCharPosition ?? "")].join("|");
        const key = top ? label(top) : "";
        // at, total, before rendering, rendering, style+layout, blocking, forced style+layout in scripts, script time per invoker type (7), longest script's key and time,
        // and the script that forced the most style+layout: its key and that time
        cur.frames.push([
          Math.round(performance.timeOrigin + start), Math.round(e.duration), Math.round(Math.max(0, rs - start)), Math.round(Math.max(0, end - rs)),
          e.styleAndLayoutStart > 0 ? Math.round(Math.max(0, end - e.styleAndLayoutStart)) : 0, Math.round(e.blockingDuration || 0), Math.round(forced),
          ...byType.map(Math.round), top ? intern(key) : -1, Math.round(topMs), fTop ? intern(label(fTop)) : -1, Math.round(fTopMs),
        ]);
      }
    });
    window.__labFramesObserver.observe({ type: "long-animation-frame" }); // not buffered: what ran before the workload is not the workload
    return true;
  } catch (e) {
    window.__labFrames = null;
    window.__labFrameCounter = null;
    return String(e);
  }
})()`;

/** Reads the page's record back (null when none was installed, e.g. after a reload or on an older Chromium). */
export const READ_FRAMES = `window.__labFrames ? { n: window.__labFrames.n, frames: window.__labFrames.frames, keys: window.__labFrames.keys, truncated: window.__labFrames.truncated, sizes: window.__labFrames.sizes, big: window.__labFrames.big, counter: window.__labFrameCounter ? { frames: window.__labFrameCounter.frames, hist: window.__labFrameCounter.hist } : null } : null`;

/** Stops recording and forgets the page's record; the wrapped onmessage stays (it is not unwrapped) but records nothing. */
export const DETACH_FRAMES = `(window.__labFramesObserver && window.__labFramesObserver.disconnect(), window.__labFramesObserver = null, window.__labFrames = null, window.__labFrameCounter = null, true)`;

// A frame record's fields, by position.
const AT = 0, MS = 1, PRE = 2, RENDER = 3, STYLE = 4, BLOCKING = 5, FORCED = 6, TYPES0 = 7, KEY = 7 + SCRIPT_TYPES.length, TOP_MS = KEY + 1, FKEY = TOP_MS + 1, FTOP_MS = FKEY + 1;

/** One record from several (the document before a reload, and the one after): counts add, frames join with their keys re-numbered, the biggest messages stay. */
export function mergeFrameRecords(records) {
    const list = records.filter(Boolean);
    if (!list.length) return null;
    const keys = [];
    const index = new Map();
    const frames = [];
    for (const r of list) {
        const map = (i) => {
            if (i < 0) return -1;
            const k = r.keys[i];
            let j = index.get(k);
            if (j === undefined) {
                if (keys.length >= KEY_CAP) return -1;
                j = keys.length;
                keys.push(k);
                index.set(k, j);
            }
            return j;
        };
        for (const f of r.frames) {
            const copy = f.slice();
            copy[KEY] = map(f[KEY]);
            copy[FKEY] = map(f[FKEY]);
            frames.push(copy);
        }
    }
    const sizes = SIZE_LABELS.map((_, i) => list.reduce((s, r) => {
        const b = r.sizes?.[i] ?? [0, 0, 0, 0];
        return [s[0] + b[0], s[1] + b[1], s[2] + b[2], Math.max(s[3], b[3])];
    }, [0, 0, 0, 0]));
    const counters = list.map((r) => r.counter).filter(Boolean);
    return {
        n: list.reduce((n, r) => n + r.n, 0),
        frames,
        keys,
        truncated: list.some((r) => r.truncated),
        sizes,
        big: list.flatMap((r) => r.big ?? []).sort((a, b) => b[2] - a[2]).slice(0, BIG_CAP),
        counter: counters.length ? { frames: counters.reduce((n, c) => n + c.frames, 0), hist: Array.from({ length: PANE_BINS }, (_, i) => counters.reduce((n, c) => n + (c.hist[i] ?? 0), 0)) } : null,
    };
}

const round1 = (n) => Math.round(n * 10) / 10;

/**
 * Summarises a run's slow frames (a merged record; `windows`: [{ round, start, end }] in epoch ms; `excluded`: the lab's own
 * forced-GC windows, whose frames are the instrument's and are left out and counted). Null when the probe never ran. `available:
 * false` says the Chromium has no Long Animation Frames; `panesPerFrame.available: false` that the build has no flush counter.
 */
export function summariseFrames(raw, windows = [], excluded = []) {
    if (!raw) return null;
    const inExcluded = (f) => excluded.some((w) => f[AT] < w.end && f[AT] + f[MS] > w.start);
    const frames = raw.frames.filter((f) => !inExcluded(f));
    const total = frames.reduce((n, f) => n + f[MS], 0);
    const sum = (i) => frames.reduce((n, f) => n + f[i], 0);
    const scriptByType = Object.fromEntries(SCRIPT_TYPES.map((t, i) => [t, round1(sum(TYPES0 + i))]));
    const scriptMs = SCRIPT_TYPES.reduce((n, t) => n + scriptByType[t], 0);
    // What each frame was mostly made of: its script, its rendering step, or neither (a long wait inside the frame, GC).
    const dominant = { script: 0, rendering: 0, other: 0 };
    for (const f of frames) {
        const s = SCRIPT_TYPES.reduce((n, _, i) => n + f[TYPES0 + i], 0);
        const r = f[RENDER];
        const other = Math.max(0, f[MS] - s - r);
        if (s >= r && s >= other) dominant.script++;
        else if (r >= other) dominant.rendering++;
        else dominant.other++;
    }
    const label = (i) => (i >= 0 ? raw.keys[i] : "(no script)");
    const byKey = new Map();
    for (const f of frames) {
        const k = label(f[KEY]);
        const e = byKey.get(k) ?? { script: k, frames: 0, topScriptMs: 0, maxMs: 0 };
        e.frames++;
        e.topScriptMs += f[TOP_MS];
        e.maxMs = Math.max(e.maxMs, f[TOP_MS]);
        byKey.set(k, e);
    }
    // Which scripts forced the most synchronous style and layout (read layout after writing the DOM), by the one that forced the most in each frame.
    const forcers = new Map();
    for (const f of frames) {
        if (f[FKEY] < 0 || f[FTOP_MS] <= 0) continue;
        const k = label(f[FKEY]);
        const e = forcers.get(k) ?? { script: k, frames: 0, forcedMs: 0, maxMs: 0 };
        e.frames++;
        e.forcedMs += f[FTOP_MS];
        e.maxMs = Math.max(e.maxMs, f[FTOP_MS]);
        forcers.set(k, e);
    }
    const byRound = windows.map((w) => {
        const inside = frames.filter((f) => f[AT] >= w.start && f[AT] < w.end);
        return { round: w.round, count: inside.length, totalMs: inside.reduce((n, f) => n + f[MS], 0), maxMs: inside.reduce((m, f) => Math.max(m, f[MS]), 0) };
    });
    const counted = raw.counter && raw.counter.frames > 0;
    const hist = raw.counter?.hist ?? [];
    const paneFrames = counted ? hist.reduce((n, c) => n + c, 0) : 0;
    return {
        available: true,
        count: raw.n - (raw.frames.length - frames.length),
        excludedFrames: raw.frames.length - frames.length,
        recorded: frames.length,
        // Past the cap the page counts frames but stops keeping them: the figures below then cover only the kept ones.
        truncated: !!raw.truncated || raw.n > raw.frames.length,
        totalMs: total,
        maxMs: frames.reduce((m, f) => Math.max(m, f[MS]), 0),
        over500ms: frames.filter((f) => f[MS] >= 500).length,
        split: { scriptMs: round1(scriptMs), renderingMs: round1(sum(RENDER)), styleLayoutMs: round1(sum(STYLE)), blockingMs: round1(sum(BLOCKING)), forcedStyleLayoutInScriptsMs: round1(sum(FORCED)) },
        scriptByType,
        dominant,
        // The forced time the list names: only the script that forced the most in each frame is kept, so this is part of `split.forcedStyleLayoutInScriptsMs`, not all of it.
        forcedAttributedMs: round1([...forcers.values()].reduce((n, e) => n + e.forcedMs, 0)),
        forcedLayoutScripts: [...forcers.values()].sort((a, b) => b.forcedMs - a.forcedMs).slice(0, 10).map((e) => ({ ...e, forcedMs: Math.round(e.forcedMs) })),
        topScripts: [...byKey.values()].sort((a, b) => b.topScriptMs - a.topScriptMs).slice(0, 10).map((e) => ({ ...e, topScriptMs: Math.round(e.topScriptMs) })),
        longest: [...frames].sort((a, b) => b[MS] - a[MS]).slice(0, 10).map((f) => ({
            at: new Date(f[AT]).toISOString(), ms: f[MS], beforeRenderingMs: f[PRE], renderingMs: f[RENDER], styleLayoutMs: f[STYLE], blockingMs: f[BLOCKING],
            scriptMs: round1(SCRIPT_TYPES.reduce((n, _, i) => n + f[TYPES0 + i], 0)), longestScript: label(f[KEY]), longestScriptMs: f[TOP_MS],
            forcedStyleLayoutMs: f[FORCED], mostForcingScript: f[FKEY] >= 0 && f[FTOP_MS] > 0 ? label(f[FKEY]) : null,
        })),
        byRound,
        messages: {
            bySize: SIZE_LABELS.map((size, i) => {
                const [count, chars, ms, max] = raw.sizes[i] ?? [0, 0, 0, 0];
                return { size, count, chars, totalHandleMs: round1(ms), maxHandleMs: round1(max) };
            }),
            // The biggest messages and how long the handler took on each: a big one is a single task everything behind it waits for.
            biggest: (raw.big ?? []).map(([at, chars, ms]) => ({ at: new Date(at).toISOString(), chars, handleMs: ms })),
        },
        panesPerFrame: counted
            ? {
                  available: true,
                  frames: paneFrames,
                  hist: Object.fromEntries(hist.map((c, i) => [i === PANE_BINS - 1 ? `${i}+` : String(i), c]).filter(([, c]) => c > 0)),
                  meanPanes: paneFrames ? round1(hist.reduce((n, c, i) => n + c * i, 0) / paneFrames) : 0,
                  maxPanes: hist.reduce((m, c, i) => (c > 0 ? i : m), 0),
              }
            : { available: false },
    };
}

/** The `summary.md` line for a run's slow frames. */
export function framesLine(s) {
    if (!s) return "Page frames: not recorded";
    if (!s.available) return "Page frames: not available (this Chromium has no Long Animation Frames)";
    const share = (n) => (s.totalMs > 0 ? Math.round((n / s.totalMs) * 100) : 0);
    const top = s.topScripts[0];
    return (
        `Page frames over 50 ms: ${s.count}${s.truncated ? " (truncated)" : ""}, ${Math.round(s.totalMs)} ms in all, longest ${s.maxMs} ms; ` +
        `script ${share(s.split.scriptMs)}%, rendering ${share(s.split.renderingMs)}% of that time, ${Math.round(s.split.forcedStyleLayoutInScriptsMs)} ms of it forced style/layout inside scripts` +
        (s.forcedLayoutScripts[0] ? ` (${Math.round(s.forcedAttributedMs)} ms named, most: ${s.forcedLayoutScripts[0].script}, ${s.forcedLayoutScripts[0].forcedMs} ms in ${s.forcedLayoutScripts[0].frames} frames)` : "") + "; " +
        `mostly script ${s.dominant.script}, mostly rendering ${s.dominant.rendering}, other ${s.dominant.other}` +
        (top ? `; most common longest script: ${top.script} (${top.frames} frames)` : "") +
        (s.panesPerFrame.available ? `; panes flushed per frame: mean ${s.panesPerFrame.meanPanes}, max ${s.panesPerFrame.maxPanes}` : "; panes per frame: not available in this build")
    );
}
