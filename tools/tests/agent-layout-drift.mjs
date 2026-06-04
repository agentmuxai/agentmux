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
// Models the CDP harness on bench-agent-keystroke.mjs.
//
// Spec: SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02 §7. Closes the verification
// leg of #1235. Pre-Phase-3 baseline (TanStack): ~5 mismatches / 1 overlap
// after 9 expand/collapse cycles. Target: 0 overlap, max |gap| ≤ tolerance.
//
// ⚠ Prereq: AgentMux running (CDP 127.0.0.1:9223) with an agent pane whose
// history EXCEEDS the streaming buffer, so `.agent-document-virtualizer` holds
// ≥ 2 rows. Fresh panes freeze the sticky-frontier at node 0 (empty virtualized
// head) and do NOT exercise Phase 3 — load older history first.

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
  --inter-ms <ms>       Pause after each toggle for reflow (default 120)
  --no-cleanup          Leave toggles applied (default: restore)
  --help`);
    process.exit(0);
}
const CDP_PORT = parseInt(getArg("--cdp-port", "9223"), 10);
const CYCLES = parseInt(getArg("--cycles", "12"), 10);
const TOGGLES = parseInt(getArg("--toggles", "6"), 10);
const TOL = parseFloat(getArg("--tolerance-px", "1.5"));
const INTER_MS = parseInt(getArg("--inter-ms", "120"), 10);

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

// Pick the workspace page (not a warm-pool window): the main page's URL has no
// windowLabel=window-pool / pool param.
async function findWorkspaceTarget(port) {
    const res = await fetch(`http://127.0.0.1:${port}/json`).catch((e) => { throw new Error(`CDP :${port} unreachable: ${e.message} — is AgentMux running?`); });
    const targets = (await res.json()).filter((t) => t.type === "page");
    const main = targets.find((t) => !/window-pool|pool=1/.test(t.url || "")) || targets[0];
    if (!main) throw new Error(`no page targets on :${port}`);
    return main;
}

// ── DOM probe: rendered-row gaps in the virtualized container ────────────────
// Returns { n, overlaps, maxAbsGap, worst:[{a,b,gap}], err }.
const PROBE = `
(() => {
  const c = document.querySelector('.agent-document-virtualizer');
  if (!c) return { err: 'no .agent-document-virtualizer' };
  const rows = [...c.querySelectorAll('.agent-document-row')];
  if (rows.length < 2) return { err: 'virtualized region has < 2 rows — need a long-history pane', n: rows.length };
  const r = rows.map(el => { const b = el.getBoundingClientRect(); return { id: el.getAttribute('data-node-id'), top: b.top, bottom: b.bottom }; })
               .sort((a,b) => a.top - b.top);
  let overlaps = 0, maxAbsGap = 0; const worst = [];
  for (let i = 1; i < r.length; i++) {
    const gap = r[i].top - r[i-1].bottom;          // 0 == flush; <0 == overlap
    if (gap < 0) overlaps++;
    if (Math.abs(gap) > maxAbsGap) maxAbsGap = Math.abs(gap);
    if (Math.abs(gap) > 1.0) worst.push({ a: r[i-1].id, b: r[i].id, gap: +gap.toFixed(2) });
  }
  worst.sort((x,y) => Math.abs(y.gap) - Math.abs(x.gap));
  return { n: r.length, overlaps, maxAbsGap: +maxAbsGap.toFixed(2), worst: worst.slice(0, 5) };
})()`;

// Toggle expansion on up to `count` virtualized rows by focusing each and
// pressing 'e' (DocumentRow.handleRowKey maps 'e' → onExpand for the
// expandable kinds: tool / agent_message / section). Returns the ids toggled.
async function toggleRows(session, count) {
    const ids = await evaluate(session, `
      (() => {
        const c = document.querySelector('.agent-document-virtualizer');
        if (!c) return [];
        const rows = [...c.querySelectorAll('.agent-document-row[tabindex]')];
        const picked = [];
        for (let i = 0; i < rows.length && picked.length < ${count}; i += Math.max(1, Math.floor(rows.length / ${count}))) {
          rows[i].focus();
          rows[i].dispatchEvent(new KeyboardEvent('keydown', { key: 'e', bubbles: true }));
          picked.push(rows[i].getAttribute('data-node-id'));
        }
        return picked;
      })()`);
    return ids || [];
}

const fmt = (n) => (n == null ? "—" : `${n}`);

async function main() {
    console.log(`agent-layout-drift: CDP 127.0.0.1:${CDP_PORT}, ${CYCLES} cycles × ${TOGGLES} toggles, tol ${TOL}px`);
    const target = await findWorkspaceTarget(CDP_PORT);
    console.log(`  page: ${target.title || "(no title)"}`);
    const session = new CdpSession(target.webSocketDebuggerUrl);
    await session.connect();
    await session.send("Runtime.enable");

    // Fresh reload so we start from a known render (spec §7: "fresh reload → …").
    await session.send("Page.enable").catch(() => {});
    await session.send("Page.reload", { ignoreCache: false }).catch(() => {});
    await new Promise((r) => setTimeout(r, 2500)); // let history + first paint settle

    const base = await evaluate(session, PROBE);
    if (base.err) throw new Error(`${base.err} (n=${fmt(base.n)})`);
    console.log(`  baseline: ${base.n} rows, ${base.overlaps} overlap, maxAbsGap ${base.maxAbsGap}px`);

    let worstOverlaps = base.overlaps, worstGap = base.maxAbsGap, worstDetail = base.worst;
    for (let cy = 1; cy <= CYCLES; cy++) {
        await toggleRows(session, TOGGLES);
        await new Promise((r) => setTimeout(r, INTER_MS));
        const p = await evaluate(session, PROBE);
        if (p.err) { console.log(`  cycle ${cy}: ${p.err}`); continue; }
        if (p.overlaps > worstOverlaps) { worstOverlaps = p.overlaps; worstDetail = p.worst; }
        if (p.maxAbsGap > worstGap) worstGap = p.maxAbsGap;
        console.log(`  cycle ${cy.toString().padStart(2)}: ${p.n} rows, overlaps ${p.overlaps}, maxAbsGap ${p.maxAbsGap}px`);
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
