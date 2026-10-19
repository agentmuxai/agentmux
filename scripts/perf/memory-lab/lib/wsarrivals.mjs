// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the page's websocket received while a slow send waited (see the tap in lib/longtasks.mjs). `ws` is a list of
// 100 ms buckets, [start in epoch ms, messages, bytes, biggest message in bytes, the page's processing time in ms (absent in older records)], for the buckets with any traffic.

const BUCKET_MS = 100;

/** Totals over a window [from, to): messages, bytes, the biggest single message, and the longest stretch with nothing arriving. */
export function arrivalsBetween(ws, from, to) {
    const inside = (ws ?? []).filter(([t]) => t + BUCKET_MS > from && t < to).sort((a, b) => a[0] - b[0]);
    let messages = 0;
    let bytes = 0;
    let biggest = 0;
    let processingMs = 0;
    for (const [, n, b, max, proc] of inside) {
        messages += n;
        bytes += b;
        biggest = Math.max(biggest, max);
        processingMs += proc ?? 0;
    }
    // Silence: the gaps between consecutive arrivals, and from the window's start / to its end.
    let longestSilenceMs = 0;
    let prevEnd = from;
    for (const [t] of inside) {
        longestSilenceMs = Math.max(longestSilenceMs, Math.max(from, t) - prevEnd);
        prevEnd = Math.max(prevEnd, t + BUCKET_MS);
    }
    longestSilenceMs = Math.max(longestSilenceMs, to - prevEnd);
    // processingMs: the page's own time handling these messages; as a share of the window it says whether the page keeps up with what arrives.
    const windowMs = Math.max(1, to - from);
    return { messages, bytes, biggestMessageBytes: biggest, longestSilenceMs: Math.max(0, Math.round(longestSilenceMs)), processingMs, processingShare: Math.round((processingMs / windowMs) * 1000) / 1000 };
}

/** For each send at or over `slowMs` that has page figures: what arrived during its time in the page. */
export function arrivalsDuringSlowSends(ws, sends, slowMs = 5000) {
    return sends
        .filter((s) => s.ms >= slowMs && s.pageStart != null && s.pageMs != null)
        .map((s) => ({ round: s.round, agent: s.agent, inPageMs: Math.round(s.pageMs), ...arrivalsBetween(ws, s.pageStart, s.pageStart + s.pageMs) }));
}

/** The whole run's websocket traffic in one line of numbers. */
export function arrivalTotals(ws) {
    let messages = 0;
    let bytes = 0;
    let biggest = 0;
    let processingMs = 0;
    for (const [, n, b, max, proc] of ws ?? []) {
        messages += n;
        bytes += b;
        biggest = Math.max(biggest, max);
        processingMs += proc ?? 0;
    }
    return { messages, bytes, biggestMessageBytes: biggest, processingMs };
}
