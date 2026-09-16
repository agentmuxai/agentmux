#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// muxsh — open editor/browser panes from a terminal, no GUI required.
//
// AgentMux's own successor to the `wsh` CLI it inherited from Wave Terminal
// and retired 2026-04-12 (docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md —
// zero real usage of the imported 20-subcommand surface). This is not that
// CLI revived: it's a narrow, purpose-built tool for the one gap the research
// behind it found real (docs/reports/REPORT_WSH_STYLE_CLI_FOR_AGENT_APP_API_2026_09_16.md) —
// nothing on the command line could open a pane, the same way nothing could
// launch an agent before `muxopen` (docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md).
//
// Thin wrapper over `POST /api/v1/pane/open` (the same `app_api::open_pane`
// logic the WebSocket RPC and the `OpenEditor` MCP tool both call — the route
// already accepts `view: "browser"` + `url`, the MCP tool just doesn't expose
// it), authenticated exactly the way muxlog/muxspect/muxopen already are:
// $AGENTMUX_LOCAL_URL + $AGENTMUX_AUTH_KEY inherited from the pane
// environment. No new IPC, no new auth scheme.
//
// Deployed by agentmux-srv next to muxlog.mjs/muxspect.mjs/muxopen.mjs
// (~/.agentmux/shell/muxsh.mjs); the shell `muxsh` functions delegate here.
// From a tool-spawned subshell call the core directly:
//   node ~/.agentmux/shell/muxsh.mjs open <file>
//   node ~/.agentmux/shell/muxsh.mjs web <url>
//
// Exit codes: 0 opened, 1 usage/environment error, 2 the server rejected the
// open or is unreachable.

import { pathToFileURL } from "node:url";

import { agentmuxFetch, readAgentmuxEnv } from "./lib/muxclient.mjs";

const HELP = `muxsh — open editor/browser panes from the terminal (no GUI required)

usage:
  muxsh open <file>              open a file in an editor pane
    --title <t>                  pane/tab title (default: file name)
    --split <right|left|down|up> split direction relative to the calling pane (default: right)
    --collapse-tree               open with the file-tree sidebar collapsed
    --floating                    open in a floating window instead of a docked split
    --no-focus                    open without focusing the new pane

  muxsh web <url>                open a URL in a browser pane
    --title <t>                  pane/tab title
    --split <right|left|down|up> split direction relative to the calling pane (default: right)
    --floating                    open in a floating window instead of a docked split
    --no-focus                    open without focusing the new pane

  muxsh help                     this text

Each call always opens a NEW pane (unlike muxopen, which is idempotent per
agent) — files and URLs aren't identity-scoped the way agents are.

Splitting relative to the calling pane requires $AGENTMUX_BLOCKID (set in
every AgentMux terminal pane) — without it the new pane is inserted at the
tab root instead, regardless of --split.

Requires $AGENTMUX_LOCAL_URL and $AGENTMUX_AUTH_KEY (present in any
AgentMux-opened pane). Opens into the instance this pane belongs to;
cross-instance opening is not implemented here.`;

const EDITOR_ONLY_FLAGS = ["--collapse-tree"];

/** Parse argv (already stripped of node + script path).
 *
 * Returns `{help}` | `{error}` | `{subcommand: "open", file, title, split,
 * collapseTree, floating, focus}` | `{subcommand: "web", url, title,
 * floating, focus}`. Pure — no I/O, no process.exit — same contract as
 * muxopen's parseArgs.
 */
export function parseArgs(argv) {
    if (argv.length === 0) return { help: true, exitCode: 1 };
    if (argv[0] === "help" || argv[0] === "--help" || argv[0] === "-h") {
        return { help: true, exitCode: 0 };
    }

    const subcommand = argv[0];
    if (subcommand !== "open" && subcommand !== "web") {
        return { error: `unknown subcommand '${subcommand}' (expected 'open' or 'web')` };
    }

    const target = argv[1];
    if (!target || target.startsWith("-")) {
        return {
            error: `'muxsh ${subcommand}' requires a ${subcommand === "open" ? "file path" : "url"} as its first argument`,
        };
    }

    let title = null;
    let split = null;
    let collapseTree = false;
    let floating = false;
    let focus = true;

    for (let i = 2; i < argv.length; i++) {
        const arg = argv[i];
        if (arg === "--title") {
            title = argv[++i];
            if (!title) return { error: "--title requires a value" };
        } else if (arg === "--split") {
            split = argv[++i];
            if (!split) return { error: "--split requires a value" };
            if (!["right", "left", "down", "up"].includes(split)) {
                return { error: `--split must be one of right/left/down/up, got '${split}'` };
            }
        } else if (arg === "--collapse-tree") {
            collapseTree = true;
        } else if (arg === "--floating") {
            floating = true;
        } else if (arg === "--no-focus") {
            focus = false;
        } else {
            return { error: `unknown argument '${arg}'` };
        }

        if (subcommand === "web" && EDITOR_ONLY_FLAGS.includes(arg)) {
            return { error: `'${arg}' is only valid with 'muxsh open', not 'muxsh web'` };
        }
    }

    if (subcommand === "open") {
        return { subcommand, file: target, title, split, collapseTree, floating, focus };
    }
    return { subcommand, url: target, title, split, floating, focus };
}

/** Build the /api/v1/pane/open request body for a parsed `open`/`web` call.
 *
 * `env` carries `AGENTMUX_BLOCKID`/`AGENTMUX_TABID` — injected into every
 * terminal pane for exactly this purpose (`blockcontroller/shell/lifecycle.rs`).
 * Without a `split_reference_block_id`, the server's `resolve_placement`
 * (`server/app_api/pane.rs`) always falls back to plain "insert" regardless
 * of `split_direction` — so `split_direction` is only meaningful, and is
 * therefore only sent, when a block id is actually known. Mirrors exactly
 * how the `OpenEditor` MCP tool derives this from `AGENTMUX_BLOCKID`
 * (`agentmux-mcp/src/main.rs`). Ignored when floating (`floating` panes are
 * documented as ignoring `split_direction`/`split_reference_block_id`
 * entirely — `rpc_types/block.rs`), so they're omitted rather than sent and
 * silently discarded. Pure, for testability.
 */
export function buildRequestBody(parsed, env = {}) {
    const body = { focus: parsed.focus, floating: parsed.floating };
    if (parsed.title) body.title = parsed.title;

    const blockId = env.AGENTMUX_BLOCKID;
    const tabId = env.AGENTMUX_TABID;
    if (tabId) body.tab_id = tabId;
    if (blockId && !parsed.floating) {
        body.split_direction = parsed.split ?? "right";
        body.split_reference_block_id = blockId;
    }

    if (parsed.subcommand === "open") {
        body.view = "editor";
        body.file = parsed.file;
        if (parsed.collapseTree) body.tree_expanded = false;
    } else {
        body.view = "browser";
        body.url = parsed.url;
    }
    return body;
}

/** Render the success line(s) for a /api/v1/pane/open response body.
 * Pure — returns the text rather than printing, for testability. */
export function renderResult(body) {
    return [`opened: ${body.view} pane`, `  block ${body.block_id}`, `  tab   ${body.tab_id}`].join("\n");
}

function fail(msg, code = 1) {
    process.stderr.write(`muxsh: ${msg}\n`);
    process.exit(code);
}

async function main() {
    const parsed = parseArgs(process.argv.slice(2));
    if (parsed.help) {
        console.log(HELP);
        process.exit(parsed.exitCode);
    }
    if (parsed.error) fail(`${parsed.error}\n\n${HELP}`);

    const { url, authKey } = readAgentmuxEnv();
    if (!url || !authKey) {
        fail(
            "AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY not set — run from a pane " +
            "AgentMux opened (agent or shell), or export them from one.",
        );
    }

    let result;
    try {
        result = await agentmuxFetch(url, authKey, "/api/v1/pane/open", {
            method: "POST",
            body: buildRequestBody(parsed, process.env),
        });
    } catch (e) {
        fail(`cannot reach ${url}: ${e.message ?? e}`, 2);
    }

    let body;
    try {
        body = await result.resp.json();
    } catch {
        body = {};
    }

    if (!result.ok) {
        const hint = result.status === 404
            ? " (HTTP 404 — this AgentMux instance may predate /api/v1/pane/open)"
            : "";
        fail(`${body.error ?? `HTTP ${result.status}`}${hint}`, 2);
    }

    console.log(renderResult(body));
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    main().catch((e) => fail(e.message ?? String(e), 2));
}
