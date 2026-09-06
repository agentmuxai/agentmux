#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// muxopen — launch an agent into a pane from a terminal, no GUI required.
//
// The constructive sibling of the stop verbs: muxspect answers "what is
// running", FleetBulkStop stops things, and until this existed nothing on the
// command line could START an agent — automation could stop cross-channel but
// not launch anywhere (docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md).
//
// Thin wrapper over `POST /api/v1/agent/open` (the same `agent.open` impl the
// UI uses, including its TOCTOU serialization), authenticated exactly the way
// muxspect and agentmux-mcp already are: $AGENTMUX_LOCAL_URL +
// $AGENTMUX_AUTH_KEY inherited from the pane environment. No new IPC, no new
// auth scheme.
//
// Deployed by agentmux-srv next to muxlog.mjs/muxspect.mjs
// (~/.agentmux/shell/muxopen.mjs); the shell `muxopen` functions delegate
// here. From a tool-spawned subshell call the core directly:
//   node ~/.agentmux/shell/muxopen.mjs <agent>
//
// Exit codes: 0 opened (or already open — idempotent, `created:false`),
// 1 usage/environment error, 2 the server rejected the open or is unreachable.

import { pathToFileURL } from "node:url";

const HELP = `muxopen — launch an agent into a pane (no GUI required)

usage:
  muxopen <agent>              open by name (case-insensitive) or definition id
  muxopen <agent> --tab <id>   target a specific tab (default: active tab)
  muxopen <agent> --no-focus   open without focusing the new pane
  muxopen help                 this text

Idempotent: an agent already open in the target tab returns its existing
pane ("already open") instead of opening a second one. On success the agent
becomes addressable for agent-to-agent messages.

Requires $AGENTMUX_LOCAL_URL and $AGENTMUX_AUTH_KEY (present in any
AgentMux-opened pane). This opens the agent in the instance this pane
belongs to; cross-instance opening is not implemented here.`;

/** Parse argv (already stripped of node + script path).
 *
 * Returns `{help}` | `{error}` | `{agent, tabId, focus}`. Pure — no I/O, no
 * process.exit — so the flag handling is unit-testable (same contract as
 * muxspect's parseArgs, whose raw-index parser bug is why these are tested).
 */
export function parseArgs(argv) {
    if (argv.length === 0) return { help: true, exitCode: 1 };
    if (argv[0] === "help" || argv[0] === "--help" || argv[0] === "-h") {
        return { help: true, exitCode: 0 };
    }
    const agent = argv[0];
    if (agent.startsWith("-")) {
        return { error: `first argument must be an agent name/id, got '${agent}'` };
    }
    let tabId = null;
    let focus = true;
    for (let i = 1; i < argv.length; i++) {
        if (argv[i] === "--tab") {
            tabId = argv[++i];
            if (!tabId) return { error: "--tab requires a tab id" };
        } else if (argv[i] === "--no-focus") {
            focus = false;
        } else {
            return { error: `unknown argument '${argv[i]}'` };
        }
    }
    return { agent, tabId, focus };
}

/** Render the success line(s) for an /api/v1/agent/open response body.
 * Pure — returns the text rather than printing, for testability. */
export function renderResult(body) {
    const verb = body.created ? "opened" : "already open";
    const lines = [
        `${verb}: ${body.agent_id} (${body.provider}, ${body.controller_type})`,
        `  block ${body.block_id}`,
        `  tab   ${body.tab_id}`,
    ];
    if (!body.created) lines.push(`  status ${body.status}`);
    return lines.join("\n");
}

function fail(msg, code = 1) {
    process.stderr.write(`muxopen: ${msg}\n`);
    process.exit(code);
}

async function main() {
    const parsed = parseArgs(process.argv.slice(2));
    if (parsed.help) {
        console.log(HELP);
        process.exit(parsed.exitCode);
    }
    if (parsed.error) fail(`${parsed.error}\n\n${HELP}`);

    const url = process.env.AGENTMUX_LOCAL_URL;
    const authKey = process.env.AGENTMUX_AUTH_KEY;
    if (!url || !authKey) {
        fail(
            "AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY not set — run from a pane " +
            "AgentMux opened (agent or shell), or export them from one.",
        );
    }

    let resp;
    try {
        resp = await fetch(`${url.replace(/\/$/, "")}/api/v1/agent/open`, {
            method: "POST",
            headers: { "X-AuthKey": authKey, "Content-Type": "application/json" },
            body: JSON.stringify({ agent_id: parsed.agent, tab_id: parsed.tabId, focus: parsed.focus }),
        });
    } catch (e) {
        fail(`cannot reach ${url}: ${e.message ?? e}`, 2);
    }

    let body;
    try {
        body = await resp.json();
    } catch {
        body = {};
    }

    if (!resp.ok) {
        // Surface the impl's own vocabulary (AGENT_NOT_FOUND / CLI_NOT_AVAILABLE
        // / …) verbatim — it names the remedy better than a paraphrase would.
        // A 404 here most likely means the running srv predates this route.
        const hint = resp.status === 404
            ? " (HTTP 404 — this AgentMux instance may predate /api/v1/agent/open)"
            : "";
        fail(`${body.error ?? `HTTP ${resp.status}`}${hint}`, 2);
    }

    console.log(renderResult(body));
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    main().catch((e) => fail(e.message ?? String(e), 2));
}
