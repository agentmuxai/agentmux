#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// muxspect — live introspection into the CURRENT running AgentMux instance's
// process/turn state. Sibling to muxlog (history) — muxspect answers "what is
// this instance doing right now," not "what happened."
//
// Phase 1 of docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md:
// queries the instance the caller is ALREADY inside (via $AGENTMUX_LOCAL_URL /
// $AGENTMUX_AUTH_KEY, inherited from the environment exactly the way
// agentmux-mcp already does for every other /api/v1/* route — no new IPC, no
// new auth scheme). Cross-instance querying (`muxspect targets` / `-i <other>`)
// is Phase 2, not implemented here.
//
// Deployed by agentmux-srv next to muxlog.mjs (~/.agentmux/shell/muxspect.mjs);
// the bash/zsh/pwsh/fish `muxspect` functions delegate here. Run standalone
// with `node muxspect.mjs ...` (works from any subshell, unlike the shell
// function) — but only from within a pane whose environment already carries
// $AGENTMUX_LOCAL_URL/$AGENTMUX_AUTH_KEY (an agent pane, or any shell pane for
// the URL — the auth key specifically is only injected for agent-CLI-type
// controllers today, see agentmux-srv/src/server/agent_handlers/input.rs).

const USAGE = `muxspect — live process/turn-state introspection for the current AgentMux instance

Usage:
  muxspect                        same as 'muxspect list'
  muxspect list [--json]          summary of every controller-backed block
  muxspect describe <block_id> [--json]
                                  full detail for one block (process status,
                                  controller status, OS process tree)
  muxspect watch <block_id>       poll 'describe' every 2s until Ctrl+C
  muxspect help                   this message

Requires $AGENTMUX_LOCAL_URL and $AGENTMUX_AUTH_KEY in the environment —
already present in any agent pane. Not a general-purpose cross-instance tool
yet: this only ever queries the instance you're already inside.`;

function fail(msg) {
    console.error(`muxspect: ${msg}`);
    process.exit(1);
}

function requireEnv() {
    const url = process.env.AGENTMUX_LOCAL_URL;
    const authKey = process.env.AGENTMUX_AUTH_KEY;
    if (!url || !authKey) {
        fail(
            "$AGENTMUX_LOCAL_URL / $AGENTMUX_AUTH_KEY not set — run this from inside an AgentMux agent pane.\n" +
            "muxspect only queries the instance you're already running in; it can't discover or guess another one."
        );
    }
    return { url, authKey };
}

async function apiGet(url, authKey, urlPath) {
    let resp;
    try {
        resp = await fetch(`${url}${urlPath}`, { headers: { "X-AuthKey": authKey } });
    } catch (e) {
        fail(`could not reach ${url} — instance unreachable (${e.message})`);
    }
    if (!resp.ok) {
        const body = await resp.text().catch(() => "");
        fail(`request failed (${resp.status}): ${body || resp.statusText}`);
    }
    return resp.json();
}

function ageString(lastComputedMs) {
    if (!lastComputedMs) return "unknown";
    const ageMs = Date.now() - lastComputedMs;
    if (ageMs < 0) return "0s"; // clock skew — never print a negative age
    if (ageMs < 1000) return `${ageMs}ms`;
    if (ageMs < 60_000) return `${Math.round(ageMs / 1000)}s`;
    return `${Math.round(ageMs / 60_000)}m`;
}

function renderList(data) {
    const blocks = data.blocks ?? [];
    if (blocks.length === 0) {
        console.log("(no controller-backed blocks in this instance)");
        return;
    }
    const rows = blocks.map((b) => ({
        block_id: b.block_id,
        type: b.controller_type || "?",
        lifecycle: b.lifecycle,
        turn: b.is_agent_pane ? "agent" : "-",
        confidence: b.liveness_confidence,
        age: ageString(b.last_computed_ms),
    }));
    const cols = ["block_id", "type", "lifecycle", "turn", "confidence", "age"];
    const widths = Object.fromEntries(
        cols.map((c) => [c, Math.max(c.length, ...rows.map((r) => String(r[c]).length))])
    );
    const line = (r) => cols.map((c) => String(r[c]).padEnd(widths[c])).join("  ");
    console.log(line(Object.fromEntries(cols.map((c) => [c, c]))));
    for (const r of rows) console.log(line(r));
    console.log(`\n(as of query time — each row's own 'age' shows how stale its computed status is)`);
}

function renderDescribe(data) {
    const ps = data.process_status ?? {};
    const cs = data.controller_status;
    console.log(`block_id:            ${data.block_id}`);
    console.log(`lifecycle:           ${ps.lifecycle ?? "unknown"}  (computed ${ageString(ps.last_computed_ms)} ago)`);
    console.log(`controller_type:     ${ps.controller_type || "(no controller)"}`);
    console.log(`is_agent_pane:       ${ps.is_agent_pane ?? false}`);
    console.log(`liveness_confidence: ${ps.liveness_confidence ?? "none"}`);
    console.log(`tracking_confidence: ${data.tracking_confidence}`);
    if (cs) {
        console.log(`\ncontroller status:`);
        console.log(`  shellprocstatus:   ${cs.shellprocstatus || "(none)"}`);
        console.log(`  shellprocexitcode: ${cs.shellprocexitcode}`);
        console.log(`  turn_active:       ${cs.turn_active ?? false}`);
        console.log(`  spawn_ts_ms:       ${cs.spawn_ts_ms ?? "(never spawned)"}`);
    } else {
        console.log(`\ncontroller status:   (no controller for this block_id)`);
    }
    const processes = data.processes ?? [];
    console.log(`\nprocesses (${processes.length}):`);
    if (processes.length === 0) {
        console.log(`  (none tracked)`);
    } else {
        for (const p of processes) {
            console.log(`  pid=${p.pid}  rss=${Math.round(p.rss_bytes / 1024)}KB  ${p.command || "(unknown command)"}`);
        }
    }
}

async function main() {
    const args = process.argv.slice(2);
    const cmd = args[0] && !args[0].startsWith("-") ? args[0] : "list";
    const json = args.includes("--json");

    if (cmd === "help" || args.includes("--help") || args.includes("-h")) {
        console.log(USAGE);
        return;
    }

    const { url, authKey } = requireEnv();

    if (cmd === "list") {
        const data = await apiGet(url, authKey, "/api/v1/muxspect/list");
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderList(data);
        return;
    }

    if (cmd === "describe") {
        const blockId = args[1];
        if (!blockId) fail("describe requires a block_id — 'muxspect describe <block_id>'");
        const data = await apiGet(url, authKey, `/api/v1/muxspect/describe?block_id=${encodeURIComponent(blockId)}`);
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderDescribe(data);
        return;
    }

    if (cmd === "watch") {
        const blockId = args[1];
        if (!blockId) fail("watch requires a block_id — 'muxspect watch <block_id>'");
        console.log(`watching ${blockId} — Ctrl+C to stop\n`);
        for (;;) {
            const data = await apiGet(url, authKey, `/api/v1/muxspect/describe?block_id=${encodeURIComponent(blockId)}`);
            console.log(`--- ${new Date().toISOString()} ---`);
            renderDescribe(data);
            console.log("");
            await new Promise((r) => setTimeout(r, 2000));
        }
    }

    fail(`unknown command '${cmd}' — 'muxspect help' for usage`);
}

main().catch((e) => fail(e?.message ?? String(e)));
