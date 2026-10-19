// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Where does a slow answer wait? When a send's answer reaches the page's JS seconds after srv wrote it, it can have
// waited in srv's queue for that connection, on the socket, or for the page's main thread. From inside the page's own
// thread the last two look alike. A second connection read by a Web Worker (its own thread, so no main-thread work
// in the way) separates them:
//
// - it replays the page's event subscriptions, so srv sends it the same stream, and counts what arrives per 100 ms;
// - every `pingMs` it makes a cheap RPC (an `eventsub` for an event nobody publishes) and times the answer, on a
//   connection that is receiving that stream.
//
// A worker that keeps up with the stream and gets its answers in milliseconds, while the page's sends take seconds,
// says the page's main thread (or the browser's hand-over to it) holds the answer. A worker that is also slow says
// srv's egress or the socket. Lab only: the second connection is a second subscriber, so the srv does a little more
// work than without it.

/** Runs in the page: records the page's own subscriptions (the messages its websocket control sends), then forces a reconnect so the page re-sends all of them. */
export const CAPTURE_SUBSCRIPTIONS = `(async () => {
  const ws = window.globalWS;
  if (!ws || !ws.baseHostPort) return { error: "no globalWS" };
  window.__labSubs = [];
  const trace = [];
  const t0 = performance.now();
  const stamp = (m) => trace.push(Math.round(performance.now() - t0) + " " + m);
  const origSend = ws.sendMessage.bind(ws);
  const origOpen = ws.onopen.bind(ws);
  const origClose = ws.onclose.bind(ws);
  ws.sendMessage = function (data) {
    try { if (data && data.message && data.message.command === "eventsub") window.__labSubs.push(data); } catch {}
    return origSend(data);
  };
  ws.onopen = function (e) { stamp("onopen"); return origOpen(e); };
  ws.onclose = function (e) { stamp("onclose"); return origClose(e); };
  stamp("reconnect(true) open=" + ws.open + " noReconnect=" + ws.noReconnect + " times=" + ws.reconnectTimes);
  ws.reconnect(true);
  // The socket closes and the page opens another within milliseconds, re-sending every subscription: wait until it has opened again and been quiet for a second.
  // On a window with many agents just created, srv can be slow to finish the close handshake (seconds; up to ~10 s seen): wait up to a minute.
  let reopenedAt = 0;
  for (let i = 0; i < 240; i++) {
    await new Promise((r) => setTimeout(r, 250));
    if (trace.some((l) => l.endsWith("onopen")) && ws.open) {
      if (!reopenedAt) reopenedAt = Date.now();
      if (Date.now() - reopenedAt >= 1000) break;
    }
  }
  ws.sendMessage = origSend;
  ws.onopen = origOpen;
  ws.onclose = origClose;
  return { subs: window.__labSubs.length, open: ws.open, trace: trace.slice(0, 12) };
})()`;

/** The worker's code (a string: it is made into a Blob inside the page). */
export const WORKER_SOURCE = `
let ws = null;
const buckets = Object.create(null); // 100 ms -> [messages, bytes]
const pings = []; // [sent epoch ms, ms to the answer or null]
const pending = new Map();
let seq = 0;
let timer = null;
let opened = null;
let closed = null;
let total = [0, 0];
function send(message) { ws.send(JSON.stringify({ wscommand: "rpc", message })); }
function ping() {
  if (!ws || ws.readyState !== 1) return;
  const reqid = "labprobe-" + (++seq) + "-" + Math.random().toString(16).slice(2, 8);
  pending.set(reqid, pings.length);
  pings.push([Date.now(), null]);
  send({ command: "eventsub", reqid, source: "labprobe", data: { event: "labprobe", scopes: [], allscopes: false } });
}
onmessage = (e) => {
  const m = e.data;
  if (m.cmd === "start") {
    ws = new WebSocket(m.url);
    ws.onopen = () => {
      opened = Date.now();
      for (const s of m.subs) ws.send(JSON.stringify(s));
      timer = setInterval(ping, m.pingMs);
      postMessage({ ready: true, at: opened });
    };
    ws.onclose = () => { closed = Date.now(); };
    ws.onmessage = (ev) => {
      const now = Date.now();
      const data = ev.data;
      const len = typeof data === "string" ? data.length : (data && data.byteLength) || 0;
      const key = now - (now % 100);
      const b = (buckets[key] ??= [0, 0]);
      b[0]++; b[1] += len;
      total[0]++; total[1] += len;
      if (typeof data !== "string") return;
      if (len < 400 && data.indexOf('"type":"ping"') >= 0) { ws.send(JSON.stringify({ type: "pong", stime: now })); return; }
      if (pending.size && len < 2000) {
        for (const [reqid, idx] of pending) {
          if (data.indexOf(reqid) >= 0) { pings[idx][1] = now - pings[idx][0]; pending.delete(reqid); break; }
        }
      }
    };
  } else if (m.cmd === "read") {
    postMessage({ read: true, opened, closed, total, buckets: Object.entries(buckets).map(([k, v]) => [Number(k), v[0], v[1]]), pings, unanswered: pending.size });
  } else if (m.cmd === "stop") {
    clearInterval(timer);
    try { ws.close(); } catch {}
    postMessage({ stopped: true });
  }
};
`;

/** Runs in the page: starts the worker on a second connection that replays the captured subscriptions. */
export function startWorkerScript({ pingMs = 250 } = {}) {
    return `(async () => {
  const ws = window.globalWS;
  if (!ws || !ws.authOpts || !ws.authOpts.authKey) return { error: "no auth key on the page's socket" };
  const url = ws.baseHostPort + "/ws?tabid=labprobe&authkey=" + encodeURIComponent(ws.authOpts.authKey);
  let worker;
  try {
    worker = new Worker(URL.createObjectURL(new Blob([${JSON.stringify(WORKER_SOURCE)}], { type: "text/javascript" })));
  } catch (e) { return { error: "worker not allowed: " + e }; }
  window.__labWsWorker = worker;
  const ready = new Promise((resolve) => {
    worker.onmessage = (e) => { if (e.data && e.data.ready) resolve({ ok: true }); };
    worker.onerror = (e) => resolve({ error: "worker error: " + (e.message || e) });
    setTimeout(() => resolve({ error: "the worker's socket did not open in 10 s" }), 10000);
  });
  worker.postMessage({ cmd: "start", url, subs: window.__labSubs || [], pingMs: ${Number(pingMs)} });
  return await ready;
})()`;
}

/** Runs in the page: asks the worker for what it has recorded. */
export const READ_WORKER = `(async () => {
  const worker = window.__labWsWorker;
  if (!worker) return null;
  return await new Promise((resolve) => {
    worker.onmessage = (e) => { if (e.data && e.data.read) resolve(e.data); };
    worker.postMessage({ cmd: "read" });
    setTimeout(() => resolve(null), 20000);
  });
})()`;

/** Runs in the page: stops the worker and its socket. */
export const STOP_WORKER = `(async () => {
  const worker = window.__labWsWorker;
  if (!worker) return true;
  worker.postMessage({ cmd: "stop" });
  await new Promise((r) => setTimeout(r, 200));
  worker.terminate();
  window.__labWsWorker = null;
  return true;
})()`;

const round1 = (x) => Math.round(x * 10) / 10;

/** Cumulative count of `[key, n]` buckets up to and including time `t` (epoch ms; a bucket is the 100 ms starting at its key). */
function cumulative(buckets) {
    const sorted = [...buckets].sort((a, b) => a[0] - b[0]);
    const times = sorted.map((b) => b[0]);
    const counts = [];
    let sum = 0;
    for (const b of sorted) counts.push((sum += b[1]));
    return { times, counts, total: sum };
}

/** When the stream first held `n` messages: the end of the first bucket whose running count reaches `n`, or null. */
function timeReaching(cum, n) {
    let lo = 0;
    let hi = cum.counts.length - 1;
    if (!cum.counts.length || cum.counts[hi] < n) return null;
    while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (cum.counts[mid] >= n) hi = mid;
        else lo = mid + 1;
    }
    return cum.times[lo] + 100;
}

/**
 * How far behind the worker's stream the page's was, over [from, to]: for each page bucket in the window, the page had
 * received N messages by its end, and the worker reached N earlier; the difference is how long the page was behind.
 * Both streams count from the moment each was recorded, so `pageOffset`/`workerOffset` (messages one side has that the
 * other does not, from before the window) are taken as the totals ratio's difference at the window start and removed.
 * Returns the largest such lag in the window, and null when the two cannot be lined up.
 */
export function pageLagIn(pageBuckets, workerBuckets, from, to) {
    // pageBuckets: [key, messages, bytes, biggest, processingMs] (the page tap's shape); workerBuckets: [key, messages, bytes].
    const page = cumulative((pageBuckets ?? []).map((b) => [b[0], b[1]]));
    const worker = cumulative((workerBuckets ?? []).map((b) => [b[0], b[1]]));
    if (!page.total || !worker.total) return null;
    // The page's count at `from` and the worker's: the streams may differ by messages only one of them gets, so line them up at the window start by counts per elapsed time.
    const upTo = (cum, t) => {
        let n = 0;
        for (let i = 0; i < cum.times.length && cum.times[i] + 100 <= t; i++) n = cum.counts[i];
        return n;
    };
    const base = upTo(page, from) - upTo(worker, from); // messages the page has counted that the worker has not (or minus)
    let max = 0;
    let any = false;
    for (let i = 0; i < page.times.length; i++) {
        const end = page.times[i] + 100;
        if (end < from || end > to) continue;
        const want = page.counts[i] - base;
        if (want <= 0) continue;
        const tw = timeReaching(worker, want);
        if (tw == null) continue;
        any = true;
        max = Math.max(max, Math.max(0, end - tw));
    }
    return any ? round1(max) : null;
}

/** The worker's cheap RPCs, in the window: how many were answered, and the slowest answer. */
export function pingsIn(pings, from, to) {
    const inWindow = (pings ?? []).filter(([at]) => at >= from && at <= to);
    const answered = inWindow.filter(([, ms]) => ms != null);
    return { sent: inWindow.length, answered: answered.length, maxMs: answered.length ? Math.max(...answered.map(([, ms]) => ms)) : null };
}

/**
 * The probe's result for a run: both streams' totals (they should be close, or the lag figures mean little), the worker's
 * RPC latency over the run, and for each slow send (window: the call made -> the answer reaching the page's JS) the page's
 * lag behind the worker's stream and the slowest worker RPC.
 *
 * @param worker what `READ_WORKER` returned; @param pageWs the page tap's per-100 ms buckets
 * @param slowSends `[{ round, agent, from, to }]` in epoch ms
 */
export function summariseWorkerProbe(worker, pageWs, slowSends = []) {
    if (!worker) return null;
    const pageTotal = (pageWs ?? []).reduce((n, b) => n + b[1], 0);
    const answered = (worker.pings ?? []).filter(([, ms]) => ms != null).map(([, ms]) => ms).sort((a, b) => a - b);
    const q = (p) => (answered.length ? answered[Math.min(answered.length - 1, Math.floor(answered.length * p))] : null);
    return {
        streams: { pageMessages: pageTotal, workerMessages: worker.total?.[0] ?? 0, workerMB: round1((worker.total?.[1] ?? 0) / 1048576) },
        workerRpc: { sent: worker.pings?.length ?? 0, unanswered: worker.unanswered ?? 0, medianMs: q(0.5), p95Ms: q(0.95), maxMs: answered.length ? answered[answered.length - 1] : null },
        slowSends: slowSends.map((s) => ({ round: s.round, agent: s.agent, pageLagBehindWorkerMs: pageLagIn(pageWs, worker.buckets, s.from, s.to), workerRpcInWindow: pingsIn(worker.pings, s.from, s.to) })),
    };
}

/** One line for the run's summary. */
export function workerProbeLine(p) {
    if (!p) return "Worker probe: no result.";
    const r = p.workerRpc;
    const lags = p.slowSends.map((s) => s.pageLagBehindWorkerMs).filter((x) => x != null);
    const pingMax = p.slowSends.map((s) => s.workerRpcInWindow.maxMs).filter((x) => x != null);
    return `Worker probe (a second socket read off the main thread): ${p.streams.workerMessages} messages against the page's ${p.streams.pageMessages}; its own RPC took a median ${r.medianMs ?? "n/a"} ms, 95th percentile ${r.p95Ms ?? "n/a"} ms, worst ${r.maxMs ?? "n/a"} ms (${r.unanswered} unanswered). ${p.slowSends.length ? `In the ${p.slowSends.length} slow send(s): the page's stream was up to ${lags.length ? Math.max(...lags) : "n/a"} ms behind the worker's, and the worker's slowest RPC was ${pingMax.length ? Math.max(...pingMax) : "n/a"} ms.` : "No slow sends to set against it."}`;
}
