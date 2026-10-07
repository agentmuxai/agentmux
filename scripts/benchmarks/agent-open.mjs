#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agent-open latency benchmark: opens agents from My Agents in a running
// AgentMux instance and checks each open's `[agent-open]` trace against a
// budget. Turns the measurement behind
// docs/reports/REPORT_AGENT_OPEN_STALL_RCA_2026_10_05.md into a repeatable
// check, so a regression in open latency shows up as a failing run instead of
// as a user reporting a 17 s open (SPEC_AGENT_OPEN_LATENCY_2026_09_27.md §5).
//
// Usage:
//   node scripts/benchmarks/agent-open.mjs --agents Maka,Lzop,Parko \
//       [--port N] [--reveal-budget 1500] [--read-budget 1000] \
//       [--paint-budget MS] [--timeout 30000] [--json]
//
// Talks to the instance's CDP remote-debugging port with Node's native
// WebSocket (no dependency), like scripts/ui-screenshots/capture.mjs. Dev
// builds run that port by default; a release instance only with
// AGENTMUX_CDP_PORT set. Use a dev build or a throwaway instance: each open
// starts the agent there, which takes it over from any other instance where
// it's running.
//
// Per agent: open a new agent picker (the Agent widget), click the agent in
// My Agents, wait for its `[agent-open] … source=my-agents` line, then read
// the phases from it (ms since the click):
//   read     history_read − history_start (the transcript read)
//   painted  first rows painted
//   revealed the loading cover lifted (bounded at 1.5 s, PANE_REVEAL_BOUND_MS)
// An agent that's already open in the instance isn't opened again: the picker
// shows its "already open" prompt instead, which the run dismisses and reports
// as skipped. A click that shows no prompt and logs no trace within --timeout
// is a stalled open, and fails the run.
//
// Exit code: 0 when every open is within budget, 1 when any isn't, failed or
// stalled, 2 on a setup error (no CDP target, no picker, an agent not in My
// Agents, or no agent measured).

import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULTS = { port: Number(process.env.AGENTMUX_CDP_PORT) || 9222, revealBudget: 1500, readBudget: 1000, paintBudget: null, timeout: 30000 };
// The Agent widget's icon opens a new agent picker pane.
const PICKER_SELECTOR = ".fa-sparkles";
const ENTRY_SELECTOR = '[data-testid="agent-my-agents-entry"]';
// Shown instead of an open when the agent is already open (MyAgentsList).
const OPEN_PROMPT_SELECTOR = '[data-testid="agent-fork-prompt"]';

function parseArgs(argv) {
    const a = { ...DEFAULTS, agents: [], json: false };
    for (let i = 0; i < argv.length; i++) {
        const k = argv[i];
        const v = () => argv[++i];
        if (k === "--agents") a.agents = v().split(",").map((s) => s.trim()).filter(Boolean);
        else if (k === "--port") a.port = Number(v());
        else if (k === "--reveal-budget") a.revealBudget = Number(v());
        else if (k === "--read-budget") a.readBudget = Number(v());
        else if (k === "--paint-budget") a.paintBudget = Number(v());
        else if (k === "--timeout") a.timeout = Number(v());
        else if (k === "--json") a.json = true;
        else throw new Error(`unknown argument: ${k}`);
    }
    if (a.agents.length === 0) throw new Error("--agents Name1,Name2,… is required");
    return a;
}

/** `key=value` pairs of an `[agent-open]` console line. */
export function parseTrace(line) {
    const body = line.slice(line.indexOf("[agent-open]") + "[agent-open]".length).replace(/"/g, "");
    return Object.fromEntries([...body.matchAll(/(\w+)=(\S+)/g)].map((m) => [m[1], m[2]]));
}

/** The phases the budget applies to, or null when the trace lacks them. */
export function phasesOf(t) {
    const num = (k) => (t[k] === undefined ? undefined : Number(t[k]));
    if (num("history_read") === undefined || num("history_start") === undefined) return null;
    return {
        read: num("history_read") - num("history_start"),
        lines: num("history_lines"),
        painted: num("painted"),
        revealed: num("revealed"),
        total: num("total"),
        outcome: t.outcome,
    };
}

/** Outcomes of an open that completed (open-trace.ts `OpenOutcome`). */
const COMPLETED_OUTCOMES = new Set(["quiet", "unsettled"]);

export function overBudget(p, args) {
    const misses = [];
    if (p.read > args.readBudget) misses.push(`read ${p.read} > ${args.readBudget}`);
    if (p.revealed !== undefined && p.revealed > args.revealBudget) misses.push(`revealed ${p.revealed} > ${args.revealBudget}`);
    if (args.paintBudget != null && p.painted !== undefined && p.painted > args.paintBudget) misses.push(`painted ${p.painted} > ${args.paintBudget}`);
    return misses;
}

/**
 * Why a traced open fails, or [] when it passes. An open that didn't complete
 * (outcome failed, timeout, closed, superseded), has no transcript read, or
 * never revealed is a failure in its own right, whatever its timings.
 */
export function assess(trace, args) {
    const misses = [];
    if (!COMPLETED_OUTCOMES.has(trace.outcome)) misses.push(`outcome=${trace.outcome ?? "?"}`);
    const p = phasesOf(trace);
    if (!p) return [...misses, "no history read in the trace"];
    if (p.revealed === undefined) misses.push("never revealed");
    return [...misses, ...overBudget(p, args)];
}

/**
 * The run's exit code. An agent missing from My Agents is a setup error (the
 * run covered fewer agents than asked), as is a run that measured nothing; a
 * stalled open or a traced open with misses fails it.
 */
export function exitCode(results) {
    if (results.some((r) => r.status === "not in My Agents")) return 2;
    if (results.some((r) => r.misses?.length)) return 1;
    return results.some((r) => r.status === "traced") ? 0 : 2;
}

function percentile(xs, q) {
    if (xs.length === 0) return undefined;
    const s = [...xs].sort((x, y) => x - y);
    return s[Math.min(s.length - 1, Math.floor(q * s.length))];
}

async function connect(port) {
    const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    const page = targets.find((t) => t.type === "page" && /AgentMux/.test(t.title)) || targets.find((t) => t.type === "page");
    if (!page) throw new Error(`no page target on CDP port ${port}`);
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    let seq = 0;
    const pending = new Map();
    const lines = [];
    ws.onmessage = (ev) => {
        const m = JSON.parse(ev.data);
        if (m.id && pending.has(m.id)) {
            pending.get(m.id)(m);
            pending.delete(m.id);
        } else if (m.method === "Runtime.consoleAPICalled") {
            const text = m.params.args.map((x) => x.value ?? x.description ?? "").join(" ");
            if (text.includes("[agent-open]")) lines.push(text);
        }
    };
    await new Promise((r, j) => {
        ws.onopen = r;
        ws.onerror = () => j(new Error(`CDP websocket on port ${port} failed`));
    });
    const send = (method, params = {}) =>
        new Promise((r) => {
            const id = ++seq;
            pending.set(id, r);
            ws.send(JSON.stringify({ id, method, params }));
        });
    const evaluate = async (expression) => (await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })).result?.result?.value;
    await send("Runtime.enable");
    return { ws, lines, evaluate };
}

async function openAgent(cdp, name, timeout) {
    const opened = await cdp.evaluate(`(() => { const w = document.querySelector(${JSON.stringify(PICKER_SELECTOR)}); if (!w) return false; w.click(); return true; })()`);
    if (!opened) throw new Error(`no Agent widget (${PICKER_SELECTOR}) to open a picker with`);
    // The picker lists agents once it has loaded.
    const listed = Date.now();
    let clicked = "not found";
    while (Date.now() - listed < 5000) {
        clicked = await cdp.evaluate(`(() => {
            const hit = [...document.querySelectorAll(${JSON.stringify(ENTRY_SELECTOR)})].reverse()
                .find((e) => e.offsetParent && (e.innerText || "").trim().split("\\n")[0].trim() === ${JSON.stringify(name)});
            if (!hit) return "not found";
            hit.click();
            return "clicked";
        })()`);
        if (clicked === "clicked") break;
        await new Promise((r) => setTimeout(r, 200));
    }
    if (clicked !== "clicked") return { name, status: "not in My Agents" };
    const seen = cdp.lines.length;
    const started = Date.now();
    while (Date.now() - started < timeout) {
        const line = cdp.lines.slice(seen).find((l) => l.includes(`agent="${name}"`) && l.includes("source=my-agents"));
        if (line) {
            const trace = parseTrace(line);
            return { name, status: "traced", outcome: trace.outcome, trace, phases: phasesOf(trace) };
        }
        // Already open: dismiss the prompt (its ✕) and leave the picker as it was.
        const alreadyOpen = await cdp.evaluate(`(() => {
            const p = [...document.querySelectorAll(${JSON.stringify(OPEN_PROMPT_SELECTOR)})].find((e) => e.offsetParent && (e.innerText || "").includes(${JSON.stringify(name)}));
            if (!p) return false;
            p.querySelector('[aria-label="Cancel"]')?.click();
            return true;
        })()`);
        if (alreadyOpen) return { name, status: "skipped (already open)" };
        await new Promise((r) => setTimeout(r, 100));
    }
    return { name, status: "stalled", misses: [`no trace within ${timeout} ms`] };
}

async function main() {
    const args = parseArgs(process.argv.slice(2));
    const cdp = await connect(args.port);
    const results = [];
    for (const name of args.agents) {
        results.push(await openAgent(cdp, name, args.timeout));
        await new Promise((r) => setTimeout(r, 1500)); // let the pane settle before the next open
    }
    cdp.ws.close();

    // Every open that produced a trace is assessed; a stalled one already
    // carries its miss.
    const measured = results.filter((r) => r.status === "traced");
    for (const r of measured) r.misses = assess(r.trace, args);
    if (args.json) {
        console.log(JSON.stringify({ budgets: { read: args.readBudget, revealed: args.revealBudget, painted: args.paintBudget }, results }, null, 1));
    } else {
        console.log(["agent".padEnd(16), "read", "lines", "painted", "revealed", "result"].join("\t"));
        for (const r of results) {
            if (r.status !== "traced") {
                const verdict = r.misses?.length ? `FAIL: ${r.status}, ${r.misses.join("; ")}` : r.status;
                console.log(`${r.name.padEnd(16)}\t-\t-\t-\t-\t${verdict}`);
                continue;
            }
            const p = r.phases ?? {};
            const cell = (v) => (v === undefined ? "-" : v);
            const verdict = r.misses.length ? `FAIL: ${r.misses.join("; ")}` : `ok (${r.outcome})`;
            console.log([r.name.padEnd(16), cell(p.read), cell(p.lines), cell(p.painted), cell(p.revealed), verdict].join("\t"));
        }
        const col = (k) => measured.map((r) => r.phases?.[k]).filter((x) => Number.isFinite(x));
        for (const k of ["read", "painted", "revealed"]) {
            const xs = col(k);
            if (xs.length) console.log(`${k}: p50 ${percentile(xs, 0.5)} ms, p95 ${percentile(xs, 0.95)} ms, max ${Math.max(...xs)} ms`);
        }
    }
    process.exit(exitCode(results));
}

// Run only when executed directly, not when the tests import the helpers.
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
    main().catch((e) => {
        console.error(`agent-open: ${e.message}`);
        process.exit(2);
    });
}
