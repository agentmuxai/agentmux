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
    // After the call is made (the RPC registers itself in TabRpcClient.openRpcs at once), the newest open request is this one:
    // the tap in lib/longtasks.mjs stamps the moment its answer reaches the page's JS (respArrival).
    return `(async () => {
  const pageStart = Date.now();
  const t0 = performance.now();
  let error = null;
  let watch = null;
  try {
    const pending = ${call};
    try {
      const open = [...(window.TabRpcClient?.openRpcs?.values?.() ?? [])].filter((v) => v.startTs >= pageStart - 5);
      const mine = open[open.length - 1];
      if (mine) watch = window.__labWatch = { reqId: mine.reqId, arrival: null, eventAgeMs: null };
    } catch {}
    await pending;
  } catch (e) { error = String(e && e.message ? e.message : e); }
  const pageMs = Math.round((performance.now() - t0) * 10) / 10;
  const w = window.__labWatch === watch ? watch : null;
  window.__labWatch = null;
  return { pageStart, pageMs, respArrival: w?.arrival ?? null, eventAgeMs: w?.eventAgeMs ?? null, error };
})()`;
}

/**
 * One send's segments, in ms; any that cannot be worked out (no page figures) are null. `inPage` splits further when the
 * page saw the answer arrive: `toArrival` (the call made -> the answer reached the page's JS) and `afterArrival` (the answer
 * reached JS -> the call's promise resolved). A large `toArrival` is an answer that was late reaching the page's handler (the
 * browser's queue or the connection); a large `afterArrival` is the RPC layer in the page taking its time once it had it.
 */
export function segmentsOf({ start, ms, pageStart, pageMs, respArrival }) {
    if (pageStart == null || pageMs == null) return { before: null, inPage: null, after: null, toArrival: null, afterArrival: null };
    const before = Math.max(0, pageStart - start);
    const after = Math.max(0, start + ms - (pageStart + pageMs));
    const split = respArrival == null ? { toArrival: null, afterArrival: null } : { toArrival: Math.max(0, Math.round(respArrival - pageStart)), afterArrival: Math.max(0, Math.round(pageStart + pageMs - respArrival)) };
    return { before: Math.round(before), inPage: Math.round(pageMs), after: Math.round(after), ...split };
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
    // Within the page: did the answer reach JS late, or did the page take its time once it had it?
    const split = { toArrival: 0, afterArrival: 0, unknown: 0 };
    for (const s of slow) {
        const seg = segmentsOf(s);
        if (seg.toArrival == null) split.unknown++;
        else if (seg.toArrival >= seg.afterArrival) split.toArrival++;
        else split.afterArrival++;
    }
    return { slowSends: slow.length, largestSegment: largest, inPageSplit: split, maxBeforeMs: max(pick("before")), maxInPageMs: max(pick("inPage")), maxAfterMs: max(pick("after")), maxToArrivalMs: max(pick("toArrival")), maxAfterArrivalMs: max(pick("afterArrival")) };
}
