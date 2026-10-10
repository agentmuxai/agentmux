// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared analysis of native-window samples (from panesampler) against the main window.
// Only panes attached to the main window's right edge at rest are judged: their right edge must keep
// the same offset from the main window's right edge whatever the width. Inner panes move with the
// layout's proportions, so their offsets change legitimately and are not counted.
export function parseSamples(text) {
    return text.trim().split("\n").filter(Boolean).map((l) => {
        const [t, ...ws] = l.split(" ");
        return { t: +t, w: ws.filter(Boolean).map((x) => { const [id, r] = x.split(":"); const [X, Y, W, H] = r.split(",").map(Number); return { id, X, Y, W, H }; }) };
    });
}
const q = (a, p) => (a.length ? a[Math.min(a.length - 1, Math.floor(p * a.length))] : 0);
// The main window is the one whose size matches the page's viewport at the start (not merely the largest:
// another AgentMux window can be bigger).
export function findMain(samples, hint) {
    const first = samples.find((s) => s.w.length);
    const match = first.w.find((w) => Math.abs(w.W - hint.W) <= 1 && Math.abs(w.H - hint.H) <= 40);
    return match ?? first.w.reduce((a, b) => (a.W * a.H >= b.W * b.H ? a : b));
}
export function paneEdgeLag(samples, { from = -Infinity, to = Infinity, hint } = {}) {
    const first = samples.find((s) => s.w.length > 1);
    if (!first) return null;
    const main0 = findMain(samples, hint), mid = main0.id;
    const edge = new Map(first.w.filter((w) => w.id !== mid && w.W > 0 && Math.abs(w.X + w.W - (main0.X + main0.W)) <= 12).map((w) => [w.id, (w.X + w.W) - (main0.X + main0.W)]));
    const dev = []; let lastBad = null, lastMainChange = null, pm = null, mainSizes = new Set();
    for (const s of samples) {
        const m = s.w.find((w) => w.id === mid); if (!m) continue;
        if (pm && (pm.W !== m.W || pm.H !== m.H)) lastMainChange = s.t;
        pm = m;
        if (s.t < from || s.t > to) continue;
        mainSizes.add(`${m.W}x${m.H}`);
        for (const [id, off] of edge) {
            const w = s.w.find((x) => x.id === id);
            if (!w || w.W === 0) continue;
            const d = Math.abs((w.X + w.W) - (m.X + m.W) - off);
            dev.push(d); if (d > 2) lastBad = s.t;
        }
    }
    dev.sort((a, b) => a - b);
    return {
        edgePanes: edge.size, mainSizes: mainSizes.size,
        offSharePct: dev.length ? Math.round((100 * dev.filter((d) => d > 2).length) / dev.length) : 0,
        devPx: { p50: q(dev, 0.5), p90: q(dev, 0.9), p99: q(dev, 0.99), max: dev.at(-1) ?? 0 },
        settleMs: lastBad != null && lastMainChange != null ? Math.round(lastBad - lastMainChange) : 0,
    };
}
// How far the page's layout width trails the native window width (points), sampled at sampler times.
export function pageTrail(samples, pageFrames /* [[epochMs, innerWidth]] */, { from, to, hint }) {
    const mid = findMain(samples, hint).id;
    const tr = []; let j = 0;
    for (const s of samples) {
        if (s.t < from || s.t > to) continue;
        const m = s.w.find((w) => w.id === mid); if (!m) continue;
        while (j + 1 < pageFrames.length && pageFrames[j + 1][0] <= s.t) j++;
        if (pageFrames[j][0] > s.t) continue;
        tr.push(Math.abs(m.W - pageFrames[j][1]));
    }
    tr.sort((a, b) => a - b);
    return { samples: tr.length, offSharePct: tr.length ? Math.round((100 * tr.filter((d) => d > 2).length) / tr.length) : 0, trailPx: { p50: q(tr, 0.5), p90: q(tr, 0.9), p99: q(tr, 0.99), max: tr.at(-1) ?? 0 } };
}
