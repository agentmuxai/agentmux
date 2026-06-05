#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// agent-layout-drift.mjs — Phase 3 layout-reducer drift / overlap verifier.
//
// Drives N expand/collapse cycles on a VIRTUALIZED agent pane via CDP and
// asserts, after every cycle, that the rendered rows are flush and never
// overlap — the user-visible form of INV-1 (positions are a prefix-sum, so
// start[i+1] === end[i] by construction). This is the integration check the
// pure reducer property test (INV-1) can't make: that the SLICE's positions
// match what the browser actually paints.
//
// Models the CDP harness on bench-agent-keystroke.mjs. Spec: §7 of
// SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02. Closes the verification leg of #1235.
//
// ⚠ A FRESH/reloaded pane is all-streaming (sticky-frontier frozen at node 0,
// empty virtualized head). The harness pages in older history (scroll-to-top →
// onLoadOlder) until the virtualized region is non-empty, THEN runs the cycles.

import { WebSocket } from "ws";

// ── CLI ─────────────────────────────────────────────────────────────────────
const args = process.argv.slice(2);
const getArg = (n, d) => { const i = args.indexOf(n); return i !== -1 ? args[i + 1] : d; };
const hasFlag = (n) => args.includes(n);
if (hasFlag("--help")) {
    console.log(`node tools/tests/agent-layout-drift.mjs [options]
  --cdp-port <port>     CEF debug port (default 9223)
  --cycles <n>          Expand/collapse cycles (default 12)
  --toggles <n>         Rows toggled per cycle (default 6)
  --tolerance-px <n>    Max allowed |gap| between consecutive rows (default 1.5)
  --seed-scrolls <n>    Max scroll-to-top passes to page in history (default 30)
  --inter-ms <ms>       Pause after each toggle for reflow (default 140)
  --no-reload           Skip the initial Page.reload`);
    process.exit(0);
}
const CDP_PORT = parseInt(getArg("--cdp-port", "9223"), 10);
const CYCLES = parseInt(getArg("--cycles", "12"), 10);
const TOGGLES = parseInt(getArg("--toggles", "6"), 10);
const TOL = parseFloat(getArg("--tolerance-px", "1.5"));
const SEED_SCROLLS = parseInt(getArg("--seed-scrolls", "30"), 10);
const INTER_MS = parseInt(getArg("--inter-ms", "140"), 10);
const NO_RELOAD = hasFlag("--no-reload");

// ── CDP session (minimal client, from bench-agent-keystroke.mjs) ─────────────
class CdpSession {
    constructor(wsUrl) { this.wsUrl = wsUrl; this.ws = null; this.nextId = 1; this.pending = new Map(); }
    async connect() {
        await new Promise((resolve, reject) => {
            this.ws = new WebSocket(this.wsUrl, { perMessageDeflate: false, maxPayload: 64 * 1024 * 1024 });
            this.ws.on("open", resolve);
            this.ws.on("error", reject);
            this.ws.on("message", (d) => this.onMessage(d));
            this.ws.on("close", () => { for (const { reject } of this.pending.values()) reject(new Error("CDP closed")); this.pending.clear(); });
        });
    }
    onMessage(data) {
        const msg = JSON.parse(data.toString());
        if (msg.id == null) return;
        const p = this.pending.get(msg.id);
        if (!p) return;
        this.pending.delete(msg.id);
        msg.error ? p.reject(new Error(`CDP ${msg.error.code}: ${msg.error.message}`)) : p.resolve(msg.result);
    }
    async send(method, params = {}) {
        const id = this.nextId++;
        return new Promise((resolve, reject) => { this.pending.set(id, { resolve, reject }); this.ws.send(JSON.stringify({ id, method, params })); });
    }
    close() { if (this.ws) { this.ws.close(); this.ws = null; } }
}

async function evaluate(session, expression, awaitPromise = false) {
    const res = await session.send("Runtime.evaluate", { expression, awaitPromise, returnByValue: true });
    if (res.exceptionDetails) throw new Error(`evaluate: ${res.exceptionDetails.text}`);
    return res.result?.value;
}

async function findWorkspaceTarget(port) {
    const res = await fetch(`http://127.0.0.1:${port}/json`).catch((e) => { throw new Error(`CDP :${port} unreachable: ${e.message} — is AgentMux running?`); });
    const targets = (await res.json()).filter((t) => t.type === "page");
    const main = targets.find((t) => !/window-pool|pool=1/.test(t.url || "")) || targets[0];
    if (!main) throw new Error(`no page targets on :${port}`);
    return main;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── DOM probe ────────────────────────────────────────────────────────────────
// { n (virtualized rows), streaming, panes, overlaps, maxAbsGap, worst, err }
const PROBE = `
(() => {
  const c = document.querySelector('.agent-document-virtualizer');
  const panes = document.querySelectorAll('.agent-document').length;
  const streaming = document.querySelectorAll('.agent-document-streaming-buffer .agent-document-row').length;
  const base = { panes, streaming };
  if (!c) return { ...base, err: 'no .agent-document-virtualizer' };
  const rows = [...c.querySelectorAll('.agent-document-row')];
  if (rows.length < 2) return { ...base, n: rows.length, err: 'virtualized region < 2 rows' };
  const r = rows.map(el => { const b = el.getBoundingClientRect(); return { id: el.getAttribute('data-node-id'), top: b.top, bottom: b.bottom }; })
               .sort((a,b) => a.top - b.top);
  let overlaps = 0, maxAbsGap = 0; const worst = [];
  for (let i = 1; i < r.length; i++) {
    const gap = r[i].top - r[i-1].bottom;
    if (gap < 0) overlaps++;
    if (Math.abs(gap) > maxAbsGap) maxAbsGap = Math.abs(gap);
    if (Math.abs(gap) > 1.0) worst.push({ a: r[i-1].id, b: r[i].id, gap: +gap.toFixed(2) });
  }
  worst.sort((x,y) => Math.abs(y.gap) - Math.abs(x.gap));
  return { ...base, n: r.length, overlaps, maxAbsGap: +maxAbsGap.toFixed(2), worst: worst.slice(0, 5) };
})()`;

// Scroll the agent document to the top to trigger onLoadOlder, paging older
// history into the virtualized region. Repeat until it stops growing or we
// have a healthy number of virtualized rows.
async function seedHistory(session) {
    let prev = -1, stable = 0;
    for (let i = 0; i < SEED_SCROLLS; i++) {
        const s = await evaluate(session, `
          (() => {
            const doc = document.querySelector('.agent-document');
            if (!doc) return { v: -1 };
            doc.scrollTop = 0;
            doc.dispatchEvent(new Event('scroll'));
            const c = document.querySelector('.agent-document-virtualizer');
            return { v: c ? c.querySelectorAll('.agent-document-row').length : 0 };
          })()`);
        if (s.v < 0) return -1;
        if (s.v >= 8) return s.v;                 // plenty to exercise overlap
        if (s.v === prev) { if (++stable >= 3) return s.v; } else stable = 0;
        prev = s.v;
        await sleep(450);                          // let onLoadOlder fetch + prepend + restore
    }
    return prev;
}

const fmt = (n) => (n == null ? "—" : `${n}`);

async function toggleRows(session, count) {
    return evaluate(session, `
      (() => {
        const c = document.querySelector('.agent-document-virtualizer');
        if (!c) return 0;
        const rows = [...c.querySelectorAll('.agent-document-row[tabindex]')];
        let n = 0;
        for (let i = 0; i < rows.length && n < ${count}; i += Math.max(1, Math.floor(rows.length / ${count}))) {
          rows[i].focus();
          rows[i].dispatchEvent(new KeyboardEvent('keydown', { key: 'e', bubbles: true }));
          n++;
        }
        return n;
      })()`);
}

async function main() {
    console.log(`agent-layout-drift: CDP 127.0.0.1:${CDP_PORT}, ${CYCLES} cycles × ${TOGGLES} toggles, tol ${TOL}px`);
    const target = await findWorkspaceTarget(CDP_PORT);
    console.log(`  page: ${target.title || "(no title)"}`);
    const session = new CdpSession(target.webSocketDebuggerUrl);
    await session.connect();
    await session.send("Runtime.enable");
    await session.send("Page.enable").catch(() => {});

    if (!NO_RELOAD) {
        await session.send("Page.reload", { ignoreCache: false }).catch(() => {});
        await sleep(3000);
    }

    let p = await evaluate(session, PROBE);
    console.log(`  post-reload: panes=${fmt(p.panes)} streaming=${fmt(p.streaming)} virtualized=${fmt(p.n)}${p.err ? " (" + p.err + ")" : ""}`);

    if (p.err || (p.n ?? 0) < 2) {
        console.log(`  paging in older history (≤${SEED_SCROLLS} scroll passes)...`);
        const seeded = await seedHistory(session);
        await sleep(300);
        p = await evaluate(session, PROBE);
        console.log(`  after seeding: virtualized=${fmt(p.n)} (seedHistory→${fmt(seeded)})`);
    }

    if (p.err || (p.n ?? 0) < 2) {
        session.close();
        console.error(`\n✗ INCONCLUSIVE: couldn't populate the virtualized region (panes=${fmt(p.panes)} streaming=${fmt(p.streaming)} virtualized=${fmt(p.n)}). This pane has no virtualizable history — open/seed a long-history agent pane.`);
        process.exit(3);
    }

    console.log(`  baseline: ${p.n} virtualized rows, ${p.overlaps} overlap, maxAbsGap ${p.maxAbsGap}px`);
    let worstOverlaps = p.overlaps, worstGap = p.maxAbsGap, worstDetail = p.worst;

    for (let cy = 1; cy <= CYCLES; cy++) {
        await toggleRows(session, TOGGLES);
        await sleep(INTER_MS);
        const q = await evaluate(session, PROBE);
        if (q.err) { console.log(`  cycle ${cy}: ${q.err} (re-seeding)`); await seedHistory(session); continue; }
        if (q.overlaps > worstOverlaps) { worstOverlaps = q.overlaps; worstDetail = q.worst; }
        if (q.maxAbsGap > worstGap) worstGap = q.maxAbsGap;
        console.log(`  cycle ${cy.toString().padStart(2)}: ${q.n} rows, overlaps ${q.overlaps}, maxAbsGap ${q.maxAbsGap}px`);
    }

    session.close();
    console.log("─".repeat(60));
    console.log(`worst over ${CYCLES} cycles: overlaps=${worstOverlaps}, maxAbsGap=${worstGap}px (tolerance ${TOL})`);
    if (worstDetail?.length) console.log(`  worst gaps: ${JSON.stringify(worstDetail)}`);
    const pass = worstOverlaps === 0 && worstGap <= TOL;
    console.log(pass ? "\n✓ PASS — 0 overlap, rows flush within tolerance" : "\n✗ FAIL — overlap or drift exceeded tolerance");
    process.exit(pass ? 0 : 1);
}

main().catch((err) => { console.error(`\nagent-layout-drift: ${err.message}`); process.exit(2); });
