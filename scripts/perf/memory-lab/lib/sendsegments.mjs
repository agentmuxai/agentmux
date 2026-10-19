// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Where a slow send's time went. A send is a Runtime.evaluate that awaits an RPC in the page. Timed in three places
// (the lab's clock, the page's clock and the page's own stopwatch, all one machine), its time splits into:
//   before: lab started -> JS started in the page (CDP delivery, then the page's main-thread queue)
//   inPage: the RPC's round trip as the page's code saw it (to srv, srv's handler, and the answer back through the page's own message queue)
//   after:  the promise resolved in the page -> the lab got the result (the page's queue, then CDP delivery)
// With srv's own log (when it entered the handler, when it finished) `inPage` splits further, and "after" being
// large would put the delay outside the app's RPC layer altogether.

/** The expression evaluated in the page for one send; `call` is the RpcApi call, as source. Returns { pageStart, pageMs, error }. */
export function timedCall(call) {
    return `(async () => {
  const pageStart = Date.now();
  const t0 = performance.now();
  let error = null;
  try { await ${call}; } catch (e) { error = String(e && e.message ? e.message : e); }
  return { pageStart, pageMs: Math.round((performance.now() - t0) * 10) / 10, error };
})()`;
}

/** One send's three segments, in ms; any that cannot be worked out (no page figures) are null. */
export function segmentsOf({ start, ms, pageStart, pageMs }) {
    if (pageStart == null || pageMs == null) return { before: null, inPage: null, after: null };
    const before = Math.max(0, pageStart - start);
    const after = Math.max(0, start + ms - (pageStart + pageMs));
    return { before: Math.round(before), inPage: Math.round(pageMs), after: Math.round(after) };
}

/** For the sends at or over `slowMs`: how many, and which segment held the most time in each (ties go to the earlier one). */
export function slowSendBreakdown(sends, slowMs = 5000) {
    const slow = sends.filter((s) => s.ms >= slowMs);
    const largest = { before: 0, inPage: 0, after: 0, unknown: 0 };
    for (const s of slow) {
        const seg = segmentsOf(s);
        if (seg.inPage == null) {
            largest.unknown++;
            continue;
        }
        const top = ["before", "inPage", "after"].reduce((best, k) => (seg[k] > seg[best] ? k : best), "before");
        largest[top]++;
    }
    const pick = (k) => slow.map((s) => segmentsOf(s)[k]).filter((v) => v != null).sort((a, b) => a - b);
    const max = (a) => (a.length ? a[a.length - 1] : null);
    return { slowSends: slow.length, largestSegment: largest, maxBeforeMs: max(pick("before")), maxInPageMs: max(pick("inPage")), maxAfterMs: max(pick("after")) };
}
