#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// muxsh — AgentMux's own successor to Wave Terminal's `wsh` CLI (which
// AgentMux inherited and fully retired 2026-04-12,
// docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md — zero real usage of the
// imported 20-subcommand surface). Not that CLI revived: a purpose-built
// collection over the Agent App API, scoped per
// docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md — every subcommand
// below maps to a real, verified-unsigned (`$AGENTMUX_AUTH_KEY`-gated, not
// agent-identity-signed) REST route; a few `wsh` commands that looked
// buildable on paper (`pane close`, `secret list`) turned out to need either
// new backend work or a capability a terminal structurally doesn't have —
// see that spec's §5.5/§5.6 for why they're not here.
//
// Grammar: `muxsh <noun> <verb> [args] [flags]`, with `open`/`web`/`view`/
// `edit`/`run` as bare top-level shortcuts (the spec's §2.1 "iconic gesture"
// exception) rather than nested under a noun.
//
// Same architecture and auth as every muxsh-family core: thin Node wrapper,
// $AGENTMUX_LOCAL_URL + $AGENTMUX_AUTH_KEY from the pane env
// (lib/muxclient.mjs), no MCP, no new transport.
//
// Deployed to ~/.agentmux/shell/muxsh.mjs; shell `muxsh` functions delegate
// here. From a tool-spawned subshell call the core directly:
//   node ~/.agentmux/shell/muxsh.mjs open <file>
//
// Exit codes: 0 success, 1 usage/environment error, 2 the server rejected
// the request or is unreachable.

import { pathToFileURL } from "node:url";

import { agentmuxFetch, readAgentmuxEnv } from "./lib/muxclient.mjs";

const HELP = `muxsh — a terminal CLI over the Agent App API (no GUI required)

usage:
  muxsh open <file>              open a file in an editor pane
  muxsh web <url>                open a URL in a browser pane
  muxsh view <path-or-url>       open the right pane type automatically
                                  (URL -> browser, media file -> media, else -> editor)
  muxsh edit <file>               like 'open', but always forces the editor view
    --title <t>                  pane/tab title
    --split <right|left|down|up> split direction relative to the calling pane (default: right)
    --collapse-tree               [open/edit/view-as-editor only] collapse the file-tree sidebar
    --floating                    open in a floating window instead of a docked split
    --no-focus                    open without focusing the new pane

  muxsh pane list                 list tabs/panes in the calling pane's workspace
    --json                       machine-readable output

  muxsh run <cmd...>              run a background shell, print its shell_id
  muxsh run --status <shell_id>   check whether a shell is still running
  muxsh run --stop <shell_id>     stop a running shell

  muxsh config edit                open settings.json in an editor pane
  muxsh config path [data|config|logs|shared]
                                  print the requested AgentMux directory (default: data)

  muxsh agent list                 list agents reachable from this instance
    --json                       machine-readable output
  muxsh agent send <name> <message...>
                                  send a message to a running agent

  muxsh help                      this text

Requires $AGENTMUX_LOCAL_URL and $AGENTMUX_AUTH_KEY (present in any
AgentMux-opened pane). Every command operates on the instance this pane
belongs to; cross-instance operation is not implemented here.

Not here: 'muxsh pane close' and 'muxsh secret list' both looked buildable
on paper but turned out to need either new backend work or an agent
identity a terminal doesn't have — see
docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md §5.5/§5.6.
'muxopen <agent>' (a separate, already-shipped tool) launches an agent into
a pane — not duplicated here as 'muxsh agent open'.`;

const EDITOR_ONLY_FLAGS = ["--collapse-tree"];
const SPLIT_DIRECTIONS = ["right", "left", "down", "up"];
const MEDIA_EXTENSIONS = [
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".ico",
    ".mp4", ".webm", ".mov", ".avi",
    ".mp3", ".wav", ".ogg", ".flac",
    ".pdf",
];
// Not derived from a canonical AgentMux list (open question, SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md §7.4) —
// hand-rolled and may drift from whatever the frontend's own pane-type resolution considers "media".
const URL_PATTERN = /^\w+:\/\//;

const CONFIG_PATH_ENV_VARS = {
    data: "AGENTMUX_DATA_DIR",
    config: "AGENTMUX_CONFIG_DIR",
    logs: "AGENTMUX_LOG_DIR",
    shared: "AGENTMUX_SHARED_DIR",
};

// ── Parsing ──────────────────────────────────────────────────────────────────

/** Parse the shared open/web/view/edit flag set (--title/--split/--collapse-tree/--floating/--no-focus).
 * Pure. Returns `{title, split, collapseTree, floating, focus}` or `{error}`. */
function parsePaneOpenFlags(argv, startIndex, subcommand) {
    let title = null;
    let split = null;
    let collapseTree = false;
    let floating = false;
    let focus = true;

    for (let i = startIndex; i < argv.length; i++) {
        const arg = argv[i];
        if (arg === "--title") {
            title = argv[++i];
            if (!title) return { error: "--title requires a value" };
        } else if (arg === "--split") {
            split = argv[++i];
            if (!split) return { error: "--split requires a value" };
            if (!SPLIT_DIRECTIONS.includes(split)) {
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
            return { error: `'${arg}' is only valid with 'muxsh open'/'muxsh edit', not 'muxsh web'` };
        }
    }
    return { title, split, collapseTree, floating, focus };
}

/** Guess the pane.open `view` for `muxsh view <path-or-url>`. Pure. */
export function guessView(target) {
    if (URL_PATTERN.test(target)) return "browser";
    const lower = target.toLowerCase();
    if (MEDIA_EXTENSIONS.some((ext) => lower.endsWith(ext))) return "media";
    return "editor";
}

/** Parse argv (already stripped of node + script path).
 *
 * Returns `{help}` | `{error}` | a command-specific shape discriminated by
 * `.command`. Pure — no I/O, no process.exit.
 */
export function parseArgs(argv) {
    if (argv.length === 0) return { help: true, exitCode: 1 };
    if (argv[0] === "help" || argv[0] === "--help" || argv[0] === "-h") {
        return { help: true, exitCode: 0 };
    }

    const head = argv[0];

    if (head === "open" || head === "web" || head === "edit") {
        const target = argv[1];
        if (!target || target.startsWith("-")) {
            return { error: `'muxsh ${head}' requires a ${head === "web" ? "url" : "file path"} as its first argument` };
        }
        const flags = parsePaneOpenFlags(argv, 2, head === "edit" ? "open" : head);
        if (flags.error) return flags;
        // 'edit' is 'open' forcing the editor view, everywhere else identical.
        const subcommand = head === "edit" ? "open" : head;
        if (subcommand === "open") {
            return { command: "open", subcommand, file: target, ...flags };
        }
        return { command: "web", subcommand, url: target, ...flags };
    }

    if (head === "view") {
        const target = argv[1];
        if (!target || target.startsWith("-")) {
            return { error: "'muxsh view' requires a file path or url as its first argument" };
        }
        const view = guessView(target);
        const flags = parsePaneOpenFlags(argv, 2, view === "browser" ? "web" : "open");
        if (flags.error) return flags;
        if (view === "browser") {
            return { command: "web", subcommand: "web", url: target, ...flags };
        }
        // "media" and "editor" both use the pane.open editor-shaped request
        // (view/file), matching what buildPaneOpenBody already emits for "open".
        return { command: "open", subcommand: "open", file: target, view, ...flags };
    }

    if (head === "pane") {
        if (argv[1] !== "list") {
            return { error: `unknown 'muxsh pane' verb '${argv[1] ?? ""}' (expected 'list')` };
        }
        const json = argv.slice(2).includes("--json");
        const extra = argv.slice(2).filter((a) => a !== "--json");
        if (extra.length > 0) return { error: `unknown argument '${extra[0]}'` };
        return { command: "pane-list", json };
    }

    if (head === "run") {
        if (argv[1] === "--status" || argv[1] === "--stop") {
            const verb = argv[1] === "--status" ? "run-status" : "run-stop";
            const shellId = argv[2];
            if (!shellId) return { error: `'muxsh run ${argv[1]}' requires a shell id` };
            if (argv.length > 3) return { error: `unknown argument '${argv[3]}'` };
            return { command: verb, shellId };
        }
        const cmdParts = argv.slice(1);
        if (cmdParts.length === 0) return { error: "'muxsh run' requires a command" };
        return { command: "run-create", cmd: cmdParts.join(" ") };
    }

    if (head === "config") {
        if (argv[1] === "edit") {
            if (argv.length > 2) return { error: `unknown argument '${argv[2]}'` };
            return { command: "config-edit" };
        }
        if (argv[1] === "path") {
            // Zero further args validly defaults to "data" — checked against
            // CONFIG_PATH_ENV_VARS, not treated as a missing-verb error.
            const kind = argv[2] ?? "data";
            if (argv.length > 3) return { error: `unknown argument '${argv[3]}'` };
            if (!Object.hasOwn(CONFIG_PATH_ENV_VARS, kind)) {
                return { error: `unknown 'muxsh config path' kind '${kind}' (expected data/config/logs/shared)` };
            }
            return { command: "config-path", kind };
        }
        return { error: `unknown 'muxsh config' verb '${argv[1] ?? ""}' (expected 'edit' or 'path')` };
    }

    if (head === "agent") {
        if (argv[1] === "list") {
            const json = argv.slice(2).includes("--json");
            const extra = argv.slice(2).filter((a) => a !== "--json");
            if (extra.length > 0) return { error: `unknown argument '${extra[0]}'` };
            return { command: "agent-list", json };
        }
        if (argv[1] === "send") {
            const name = argv[2];
            const messageParts = argv.slice(3);
            if (!name) return { error: "'muxsh agent send' requires an agent name" };
            if (messageParts.length === 0) return { error: "'muxsh agent send' requires a message" };
            return { command: "agent-send", name, message: messageParts.join(" ") };
        }
        return { error: `unknown 'muxsh agent' verb '${argv[1] ?? ""}' (expected 'list' or 'send')` };
    }

    return { error: `unknown command '${head}'` };
}

// ── pane.open (open / web / view / edit) ────────────────────────────────────

/** Build the /api/v1/pane/open request body for a parsed open/web/view command.
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

    if (parsed.command === "open") {
        body.view = parsed.view ?? "editor";
        body.file = parsed.file;
        if (parsed.collapseTree && body.view === "editor") body.tree_expanded = false;
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

// ── pane list ────────────────────────────────────────────────────────────────

/** Render `GET /api/v1/tabs` response as a human-readable outline. Pure. */
export function renderTabs(tabs) {
    if (!Array.isArray(tabs) || tabs.length === 0) return "no tabs found";
    return tabs
        .map((tab) => {
            const panes = Array.isArray(tab.panes) ? tab.panes : [];
            const header = `tab ${tab.tab_name ?? tab.tab_id}${tab.active ? " (active)" : ""}`;
            const paneLines = panes.map((p) => `  ${p.view ?? "?"}  ${p.block_id}  ${p.title ?? ""}`.trimEnd());
            return [header, ...paneLines].join("\n");
        })
        .join("\n");
}

// ── run ──────────────────────────────────────────────────────────────────────

export function renderShellCreate(body) {
    return `shell_id: ${body.shell_id}`;
}

export function renderShellStatus(body) {
    const lines = [`running: ${body.running}`];
    if (body.exit_code !== undefined && body.exit_code !== null) lines.push(`exit_code: ${body.exit_code}`);
    lines.push(`line_count: ${body.line_count}`);
    return lines.join("\n");
}

export function renderShellStop(body) {
    return `stopped: ${body.stopped}`;
}

/** Build the /api/v1/shell/create request body. Pure — separated out (rather
 * than inlined in main()) specifically so muxsh.contract.test.mjs can assert
 * its field names against docs/specs/app-api-manifest.json, same reason
 * buildRequestBody exists for pane.open. */
export function buildShellCreateBody(parsed, env = {}) {
    return { agent_block_id: env.AGENTMUX_BLOCKID, cmd: parsed.cmd };
}

export function buildShellStatusBody(parsed) {
    return { shell_id: parsed.shellId };
}

export function buildShellStopBody(parsed) {
    return { shell_id: parsed.shellId };
}

// ── agent list / send ────────────────────────────────────────────────────────

/** Render `GET /agentmux/discovery`'s host.agents into a human-readable list. Pure. */
export function renderAgentList(discovery) {
    const agents = discovery?.host?.agents;
    if (!Array.isArray(agents) || agents.length === 0) return "no agents found";
    return agents
        .map((a) => `${a.addressable ? "*" : " "} ${a.name}${a.addressable ? "" : "  (not addressable)"}`)
        .join("\n");
}

export function renderAgentSend(body) {
    return body.success ? "sent" : `not sent: ${body.error ?? "unknown error"}`;
}

/** Build the /agentmux/reactive/inject request body. Pure — see
 * buildShellCreateBody's doc comment for why this is a separate, exported
 * function rather than inlined in main(). */
export function buildAgentSendBody(parsed) {
    return { target_agent: parsed.name, message: parsed.message };
}

// ── main ─────────────────────────────────────────────────────────────────────

function fail(msg, code = 1) {
    process.stderr.write(`muxsh: ${msg}\n`);
    process.exit(code);
}

async function apiCall(url, authKey, path, opts, notFoundHint) {
    let result;
    try {
        result = await agentmuxFetch(url, authKey, path, opts);
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
        const hint = result.status === 404 && notFoundHint
            ? ` (HTTP 404 — this AgentMux instance may predate ${notFoundHint})`
            : "";
        fail(`${body.error ?? `HTTP ${result.status}`}${hint}`, 2);
    }
    return body;
}

async function main() {
    const parsed = parseArgs(process.argv.slice(2));

    if (parsed.help) {
        console.log(HELP);
        process.exit(parsed.exitCode);
    }
    if (parsed.error) fail(`${parsed.error}\n\n${HELP}`);

    if (parsed.command === "config-path") {
        const envVar = CONFIG_PATH_ENV_VARS[parsed.kind];
        const value = process.env[envVar];
        if (!value) fail(`${envVar} is not set in this pane's environment`);
        console.log(value);
        return;
    }

    const { url, authKey } = readAgentmuxEnv();
    if (!url || !authKey) {
        fail(
            "AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY not set — run from a pane " +
            "AgentMux opened (agent or shell), or export them from one.",
        );
    }

    switch (parsed.command) {
        case "open":
        case "web": {
            const body = await apiCall(url, authKey, "/api/v1/pane/open", {
                method: "POST",
                body: buildRequestBody(parsed, process.env),
            }, "/api/v1/pane/open");
            console.log(renderResult(body));
            return;
        }
        case "config-edit": {
            const configDir = process.env.AGENTMUX_CONFIG_DIR;
            if (!configDir) fail("AGENTMUX_CONFIG_DIR is not set in this pane's environment");
            const openParsed = { command: "open", subcommand: "open", file: `${configDir}/settings.json`, title: "settings.json", split: null, collapseTree: false, floating: false, focus: true };
            const body = await apiCall(url, authKey, "/api/v1/pane/open", {
                method: "POST",
                body: buildRequestBody(openParsed, process.env),
            }, "/api/v1/pane/open");
            console.log(renderResult(body));
            return;
        }
        case "pane-list": {
            const blockId = process.env.AGENTMUX_BLOCKID ?? "";
            const body = await apiCall(url, authKey, `/api/v1/tabs?block_id=${encodeURIComponent(blockId)}`, {}, "/api/v1/tabs");
            if (parsed.json) {
                console.log(JSON.stringify(body, null, 2));
            } else {
                console.log(renderTabs(body.tabs ?? body));
            }
            return;
        }
        case "run-create": {
            if (!process.env.AGENTMUX_BLOCKID) fail("AGENTMUX_BLOCKID is not set in this pane's environment — 'muxsh run' needs it to attribute the shell");
            const body = await apiCall(url, authKey, "/api/v1/shell/create", {
                method: "POST",
                body: buildShellCreateBody(parsed, process.env),
            }, "/api/v1/shell/create");
            console.log(renderShellCreate(body));
            return;
        }
        case "run-status": {
            const body = await apiCall(url, authKey, "/api/v1/shell/status", {
                method: "POST",
                body: buildShellStatusBody(parsed),
            }, "/api/v1/shell/status");
            console.log(renderShellStatus(body));
            return;
        }
        case "run-stop": {
            const body = await apiCall(url, authKey, "/api/v1/shell/stop", {
                method: "POST",
                body: buildShellStopBody(parsed),
            }, "/api/v1/shell/stop");
            console.log(renderShellStop(body));
            return;
        }
        case "agent-list": {
            const body = await apiCall(url, authKey, "/agentmux/discovery", {}, "/agentmux/discovery");
            if (parsed.json) {
                console.log(JSON.stringify(body, null, 2));
            } else {
                console.log(renderAgentList(body));
            }
            return;
        }
        case "agent-send": {
            const body = await apiCall(url, authKey, "/agentmux/reactive/inject", {
                method: "POST",
                body: buildAgentSendBody(parsed),
            }, "/agentmux/reactive/inject");
            console.log(renderAgentSend(body));
            return;
        }
        default:
            fail(`internal error: unhandled command '${parsed.command}'`);
    }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    main().catch((e) => fail(e.message ?? String(e), 2));
}
