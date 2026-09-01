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

import { pathToFileURL } from "node:url";

const USAGE = `muxspect — live process/turn-state introspection for the current AgentMux instance

Usage:
  muxspect                        same as 'muxspect list'
  muxspect list [--json]          summary of every controller-backed block
  muxspect describe <block_id> [--json]
                                  full detail for one block (process status,
                                  controller status, OS process tree)
  muxspect watch <block_id>       poll 'describe' every 2s until Ctrl+C
  muxspect dock <block_id> [--json]
                                  Activity Dock ToolNode status snapshot for
                                  one block — diagnoses stuck 'running'
                                  entries (SPEC_MUXSPECT_DOCK_DIAGNOSIS_
                                  AND_REMEDIATION_2026_08_06.md)
  muxspect dock clear <block_id> <node_id>
                                  force-clear one stuck dock entry, live, in
                                  whatever renderer currently has that block
                                  open — no pane reload needed. muxspect's
                                  only mutating command; see the spec above.
  muxspect conversations [--json]  every agent's most recent activity across
                                  host, cross-channel, LAN, and connected WAN
                                  in one glance — host/cross-channel entries
                                  include a last-message preview; LAN/WAN
                                  are liveness-only for now
                                  (SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_
                                  VISIBILITY_2026_08_21.md Phase A)
  muxspect conversation <agent> [--json]
                                  read the tail transcript of one agent by
                                  name — resolves host first, then
                                  cross-channel (other AgentMux channels on
                                  this same host); does not reach LAN/WAN
  muxspect find <block_id_or_agent> [--json]
                                  which running instance(s), if any, have a
                                  controller/dispatch matching this block id
                                  or agent name — checks this instance first
                                  (ANY controller type), then every other
                                  channel via the shared reactive registry
                                  (agent-registered blocks ONLY on the
                                  cross-channel tier — a non-agent controller,
                                  e.g. a plain shell, in another channel isn't
                                  findable by block_id there). No forwarded
                                  call unless a channel actually matches. A
                                  UUID-shaped query is sent as block_id;
                                  anything else as agent name
                                  (SPEC_MUXSPECT_CROSS_INSTANCE_FIND_2026_08_22.md,
                                  Ext 4 of REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_
                                  INSPECTION_2026_08_22.md)
  muxspect verify-sender <agent_name> [--json]
                                  is an agent named <agent_name> currently
                                  REGISTERED, and via what tier (spawner /
                                  host / cross-channel / lan / wan)?
                                  Registry-liveness only — NOT cryptographic
                                  sender verification; does not replace
                                  checking a JEKT's own TRUST=/SIG= fields
                                  (SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21.md).
                                  Exits non-zero unless the verdict is
                                  'found'.
  muxspect layout [tab_id] [--json]
                                  the PERSISTED layout tree for every tab (or
                                  one), as an indented outline: direction,
                                  size, minimized/magnified flags, block ids —
                                  plus the layout doctor's verdict on each
                                  tree. Answers "what does the layout actually
                                  look like right now" without reading
                                  db_layout out of SQLite by hand. Note it
                                  reports what is PERSISTED, which can lag
                                  what the frontend is currently rendering.
                                  Exits non-zero if any tree has violations.
  muxspect work [state] [--json]  the Muxqueue backlog — the shared work queue
                                  any agent can add to and any agent can claim
                                  (REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md).
                                  Optional state filter: open | claimed | done |
                                  failed | cancelled. Shows who holds what, how
                                  many attempts each item has burned, and the
                                  result/reason recorded on it. Answers "what is
                                  outstanding, and is anything stuck" without
                                  claiming anything yourself — this command is
                                  strictly read-only.
  muxspect help                   this message

Requires $AGENTMUX_LOCAL_URL and $AGENTMUX_AUTH_KEY in the environment.

IMPORTANT (reagent P1 on PR #2380): the bare 'muxspect' shell FUNCTION is
only defined in INTERACTIVE terminal panes (it's sourced from the shell
rcfile), but those panes only get $AGENTMUX_LOCAL_URL, not
$AGENTMUX_AUTH_KEY — so the function loads there but auth fails. Agent-CLI
tool calls (the primary intended use — an agent inspecting its own running
instance) get BOTH env vars, but tool-spawned shells don't source the
rcfile, so the 'muxspect' function isn't defined there either. Until a
Phase 2 fix wires this up properly, the ONLY invocation that reliably works
today is calling the deployed core directly by path (same caveat muxlog
itself documents for tool-spawned subshells):
  node ~/.agentmux/shell/muxspect.mjs list
See docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md for the
full story and the planned fix.`;

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

// Ext 5 of docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md:
// this session hit `muxspect conversations` 404ing because the running srv
// predated that route, with nothing saying so — every response now carries
// an `x-agentmux-srv-version` header (server/mod.rs's version_header
// middleware) stamping the instance's own version. Printed to STDERR (not
// stdout) on every call so it never pollutes piped/parsed `--json` output,
// and folded into the error message specifically for a 404 — the one status
// a version mismatch is most likely to explain.
export function logSrvVersion(resp) {
    const v = resp.headers.get("x-agentmux-srv-version");
    if (v) console.error(`[srv v${v}]`);
    return v;
}

async function apiGet(url, authKey, urlPath) {
    let resp;
    try {
        resp = await fetch(`${url}${urlPath}`, { headers: { "X-AuthKey": authKey } });
    } catch (e) {
        fail(`could not reach ${url} — instance unreachable (${e.message})`);
    }
    const srvVersion = logSrvVersion(resp);
    if (!resp.ok) {
        const body = await resp.text().catch(() => "");
        const versionHint =
            resp.status === 404 && srvVersion
                ? ` (this instance is running srv v${srvVersion} — a 404 here may mean it predates this command; check you're not talking to a stale build)`
                : "";
        fail(`request failed (${resp.status}): ${body || resp.statusText}${versionHint}`);
    }
    return resp.json();
}

// `dock clear` is muxspect's first mutating call — see
// docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §2.
// Same shape as apiGet (same auth header, same fail()-on-non-2xx contract,
// including on the handler's 404 "no such node" response — from the CLI's
// perspective, "the clear didn't happen" is a failure worth a non-zero exit).
async function apiPost(url, authKey, urlPath, body) {
    let resp;
    try {
        resp = await fetch(`${url}${urlPath}`, {
            method: "POST",
            headers: { "X-AuthKey": authKey, "Content-Type": "application/json" },
            body: JSON.stringify(body),
        });
    } catch (e) {
        fail(`could not reach ${url} — instance unreachable (${e.message})`);
    }
    const srvVersion = logSrvVersion(resp);
    if (!resp.ok) {
        const respBody = await resp.text().catch(() => "");
        const versionHint =
            resp.status === 404 && srvVersion
                ? ` (this instance is running srv v${srvVersion} — a 404 here may mean it predates this command; check you're not talking to a stale build)`
                : "";
        fail(`request failed (${resp.status}): ${respBody || resp.statusText}${versionHint}`);
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
    // "pane_type" reflects static pane classification, NOT live turn
    // activity — that's what `lifecycle` is for (`lifecycle_from` maps
    // turn_active=true straight to Lifecycle::Running; see
    // agentmux-srv/src/broker/process.rs). A column literally named "turn"
    // showing pane-type instead of turn state was reviewed as misleading
    // (codex P2 on PR #2380) — `list`'s ProcessStatus rows don't carry
    // BlockControllerRuntimeStatus::turn_active at all (that needs
    // `describe`), so this renames rather than fetching it at extra cost.
    //
    // Uses the server-computed `is_agent` field, NOT the raw
    // `is_agent_pane` flag — subprocess/persistent/acp controllers can
    // report `is_agent_pane: false` even though those controller types are
    // always agents; reimplementing that classification rule in JS instead
    // of reusing ProcessStatus::is_agent() mislabeled exactly those three
    // types as "term" (codex P2 on PR #2380, second round).
    // "last_error" — the last thing that happened to this block was an
    // unrecovered spawn/execution error (muxspect_handlers.rs's
    // last_error_frame). Distinct from lifecycle/confidence (liveness): a
    // block can be `lifecycle: unknown` for perfectly healthy reasons
    // (idle, never opened) — this column is what disambiguates "idle"
    // from "wedged, fix available" at a glance, which is the whole point
    // (docs/reports/REPORT_MUXSPECT_SPAWN_REFUSAL_DIAGNOSIS_EXTENSION_2026_08_03.md §3.2).
    const rows = blocks.map((b) => ({
        block_id: b.block_id,
        type: b.controller_type || "?",
        lifecycle: b.lifecycle,
        pane_type: b.is_agent ? "agent" : "term",
        confidence: b.liveness_confidence,
        age: ageString(b.last_computed_ms),
        last_error: b.last_error ? `yes (${ageString(b.last_error.written_ms)})` : "-",
    }));
    const cols = ["block_id", "type", "lifecycle", "pane_type", "confidence", "age", "last_error"];
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
    console.log(`is_agent:            ${data.is_agent ?? false}  (raw is_agent_pane: ${ps.is_agent_pane ?? false})`);
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
    // The actual "why" for a block with no live controller/process — see
    // muxspect_handlers.rs's last_error_frame. Printed last, since it's
    // usually the answer someone's here for precisely when everything
    // above it is empty.
    if (data.last_error) {
        console.log(`\nlast_error:`);
        console.log(`  message: ${data.last_error.message}`);
        console.log(`  source:  ${data.last_error.source}`);
        console.log(`  age:     ${ageString(data.last_error.written_ms)}`);
    } else {
        console.log(`\nlast_error:           (none)`);
    }
}

// Like ageString, but for a duration already expressed in ms (the `dock`
// endpoint returns `age_ms` directly, not a timestamp to diff against
// Date.now() — using ageString here would silently compute a bogus value).
function msToAge(ms) {
    if (ms < 1000) return `${ms}ms`;
    if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
    return `${Math.round(ms / 60_000)}m`;
}

/**
 * `verify-sender` tier 0 — zero round-trip, checked before any network
 * call. A `task dev` instance's own `AGENTMUX_CHANNEL` (e.g.
 * "dev-agenta-background-task-dashboard-intelligence-6c345e93dbc777e1")
 * and `AGENTMUX_RUNTIME_MODE` ("dev:agenta-background-task-dashboard-
 * intelligence") encode which dev-build/worktree slug this instance was
 * launched from.
 *
 * IMPORTANT — this is a NAMING-CONVENTION HEURISTIC, not an attested
 * identity: `AGENTMUX_CHANNEL`/`AGENTMUX_RUNTIME_MODE` are derived from
 * whatever branch/slug name was used to create the `task dev` instance,
 * not from any launcher-signed or otherwise authenticated "spawned by"
 * record. A dev branch coincidentally or deliberately named e.g.
 * `agenta-unrelated-work` would make a claimed sender named `AgentA`
 * match here even if AgentA never spawned this instance (codex review on
 * PR #2702, P1). Treat a `tier: "spawner"` result as a coordination hint
 * for the common case (a task-dev instance checking the agent whose
 * worktree it's obviously running from), NOT as proof — same caveat as
 * every other tier this command reports (see [`checkSpawnerTier`]'s
 * caller for why this command has no `trust`/`verified` field at all). A
 * real fix would need the launcher to inject an authenticated
 * `AGENTMUX_SPAWNED_BY` value at instance-creation time rather than this
 * inferring it from a name string; out of scope here — see
 * docs/specs/SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21.md's non-goals.
 *
 * Returns `null` (fall through to the network tiers) when neither env var
 * is set, or set but doesn't match `name`. Exported (pure) for
 * muxspect.test.mjs.
 */
export function checkSpawnerTier(name, env) {
    const needle = name.toLowerCase();
    const channel = env.AGENTMUX_CHANNEL || "";
    const runtimeMode = env.AGENTMUX_RUNTIME_MODE || "";
    const channelMatch = channel.toLowerCase().match(/^dev-([a-z0-9]+)-/);
    const runtimeMatch = runtimeMode.toLowerCase().match(/^dev:([a-z0-9]+)-/);
    const spawner = channelMatch?.[1] ?? runtimeMatch?.[1];
    if (!spawner || spawner !== needle) return null;
    return { name, status: "found", tier: "spawner", channel: channel || undefined };
}

// `verify-sender` reports REGISTRY LIVENESS only — does an agent named X
// currently exist in the discovery data. It performs NO cryptographic
// check and is NOT the JEKT protocol's own sender-authentication
// mechanism, which is per-message, automatic, and already computes a real
// `TRUST=`/`SIG=` value on every delivered jekt (HMAC-SHA256 host tier /
// Ed25519 LAN tier / pinned Ed25519 reagent WAN — see this repo's root
// CLAUDE.md, "Is a jekt's sender identity actually verified?"). A
// `status: "found"` result here means "an agent by this name is
// currently registered" — it does NOT mean a specific JEKT claiming
// FROM=X was actually sent by X. Deliberately no `trust`/`verified` field
// in the output for exactly this reason (reagentx-workflow review on PR
// #2702, P1 — an earlier version reused `host-verified`/`network-claimed`,
// which collided with that real, stronger, cryptographic guarantee).
function renderVerifySender(data) {
    console.log(`sender: ${data.name}`);
    console.log(`status: ${data.status}`);
    if (data.tier) console.log(`tier:   ${data.tier}`);
    if (data.last_seen_ms !== undefined && data.last_seen_ms !== null) {
        console.log(`last_seen: ${ageString(data.last_seen_ms)} ago`);
    }
    if (data.channel) console.log(`channel: ${data.channel}`);
    if (data.local_url) console.log(`local_url: ${data.local_url}`);
    if (data.status === "not_found") {
        console.log(`\nno agent named '${data.name}' found on any tier (spawner, host, cross-channel, lan, wan).`);
    } else if (data.status === "stale") {
        console.log(`\nfound but heartbeat is stale.`);
    }
    console.log(`\nNote: this is a registry-liveness check, not cryptographic sender verification.`);
    console.log(`Check the JEKT's own TRUST=/SIG= fields for that (see CLAUDE.md's JEKT section).`);
}

// `conversations` — see SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_
// 2026_08_21.md Phase A. host/cross-channel rows carry a preview + turn
// state; lan/wan rows are liveness-only (no remote-read protocol yet for
// those tiers) — rendered with a "(remote — use 'conversation <agent>' once
// supported)" placeholder rather than blank cells, so the gap is obvious
// rather than looking like missing data.
function renderConversations(data) {
    const agents = data.agents ?? [];
    if (agents.length === 0) {
        console.log("(no agents found on any tier)");
        return;
    }
    const rows = agents.map((a) => ({
        name: a.name,
        tier: a.tier,
        turn: a.turn_active === true ? "active" : a.turn_active === false ? "idle" : "?",
        activity: a.last_activity_ms ? `${ageString(a.last_activity_ms)} ago` : "-",
        preview: a.remote_fetch_required
            ? "(remote — use 'conversation <agent>' once supported)"
            : (a.last_message_preview ?? "(no output yet)"),
    }));
    const cols = ["name", "tier", "turn", "activity"];
    const widths = Object.fromEntries(
        cols.map((c) => [c, Math.max(c.length, ...rows.map((r) => String(r[c]).length))])
    );
    const line = (r) => cols.map((c) => String(r[c]).padEnd(widths[c])).join("  ");
    console.log(line(Object.fromEntries(cols.map((c) => [c, c]))));
    for (const r of rows) {
        console.log(line(r));
        console.log(`  ${r.preview}`);
    }
}

// `conversation <agent>` — reuses the same `/agentmux/reactive/transcript`
// response `GetAgentTranscript` returns, just rendered for a human. `tier`
// is present as of Phase A (host | cross-channel); older srv builds
// predating this change won't send it — printed only when present.
function renderConversation(data) {
    console.log(`agent:       ${data.agent}`);
    if (data.tier) console.log(`tier:        ${data.tier}`);
    if (data.channel) console.log(`channel:     ${data.channel}`);
    console.log(`turn_active: ${data.turn_active ?? false}`);
    const lines = data.lines ?? [];
    console.log(`\n--- transcript tail (${lines.length}${data.truncated ? ", truncated" : ""}) ---`);
    for (const l of lines) console.log(l);
}

// `find <block_id_or_agent>` — Ext 4 of REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_
// INSPECTION_2026_08_22.md / SPEC_MUXSPECT_CROSS_INSTANCE_FIND_2026_08_22.md.
// `results` is empty when nothing matched anywhere — a legitimate "not
// found on this host, in any known channel" answer, not an error.
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// `found: false` (cross-channel only) means the shared registry pointed at
// a channel, but the forwarded describe's own lifecycle came back "unknown"
// — the remote ProcessBroker has no controller for this block_id at all,
// i.e. the registry entry was fresh enough to survive the staleness filter
// srv-side but the block itself is already gone. Shown, not hidden — but
// not counted toward "genuinely found" for the summary line or exit code.
function renderFind(data) {
    const { block_id, agent } = data.query ?? {};
    console.log(`query: ${block_id ? `block_id=${block_id}` : `agent=${agent}`}`);
    const results = data.results ?? [];
    const live = results.filter((r) => r.found !== false);
    if (results.length === 0) {
        console.log("\nnot found on this host, in any known channel.");
        return;
    }
    if (live.length === 0) {
        console.log(`\nnot found — ${results.length} registry match${results.length === 1 ? "" : "es"} pointed at a channel, but the block is already gone there.`);
    } else {
        console.log(`\nfound in ${live.length} place${live.length === 1 ? "" : "s"}:`);
    }
    for (const r of results) {
        const staleNote = r.found === false ? " — gone (registry match, but remote lifecycle is 'unknown')" : "";
        console.log(`\n--- ${r.tier}${r.channel ? ` (${r.channel})` : ""}${staleNote} ---`);
        console.log(`  block_id: ${r.block_id}`);
        if (r.agent_id) console.log(`  agent_id: ${r.agent_id}`);
        if (r.tier === "host" && r.process_status) {
            console.log(`  lifecycle: ${r.process_status.lifecycle ?? "unknown"}`);
            console.log(`  controller_type: ${r.process_status.controller_type ?? "(none)"}`);
        }
        if (r.tier === "cross-channel") {
            if (r.describe) {
                const ps = r.describe.process_status ?? {};
                console.log(`  lifecycle: ${ps.lifecycle ?? "unknown"}`);
                console.log(`  controller_type: ${ps.controller_type ?? "(none)"}`);
            } else {
                console.log(`  (matched in the shared registry, but the forwarded describe call didn't respond in time — channel may be busy or gone)`);
            }
        }
    }
}

function renderDock(data) {
    console.log(`block_id: ${data.block_id}`);
    const nodes = data.nodes ?? [];
    if (nodes.length === 0) {
        console.log("(no tracked dock nodes for this block)");
        return;
    }
    const rows = nodes.map((n) => ({
        node_id: n.node_id,
        tool: n.tool_name,
        status: n.status,
        age: msToAge(n.age_ms),
        stuck: n.stuck ? "STUCK?" : "",
        bg: n.run_in_background ? "bg" : "",
    }));
    const cols = ["node_id", "tool", "status", "age", "stuck", "bg"];
    const widths = Object.fromEntries(
        cols.map((c) => [c, Math.max(c.length, ...rows.map((r) => String(r[c]).length))])
    );
    const line = (r) => cols.map((c) => String(r[c]).padEnd(widths[c])).join("  ");
    console.log(line(Object.fromEntries(cols.map((c) => [c, c]))));
    for (const r of rows) console.log(line(r));
    if (rows.some((r) => r.stuck)) {
        console.log(`\nSTUCK? nodes: 'running' past the promotion threshold with nothing srv-side backing this block.`);
        console.log(`Clear one with: muxspect dock clear ${data.block_id} <node_id>`);
    }
    // issue #2518: a 'bg' row's status is the RAW ToolNode status, terminal
    // ("success") within ~a second — it can NEVER show STUCK? here even if
    // the real dock row has been showing 'running' for hours, because the
    // srv has no visibility into whether this node's <task-notification>
    // ever arrived (that reclassification happens entirely client-side).
    // A long-`age` 'bg' row with status=success is worth checking by hand
    // in the actual UI even though this table reads clean.
    if (rows.some((r) => r.bg)) {
        console.log(`\nbg rows: backgrounded launches — STUCK? never applies to these (server can't see the dock's own <task-notification> tracking). Check the live UI by hand if one looks old.`);
    }
}

/**
 * Render `muxspect work` — the Muxqueue backlog.
 *
 * Exported (pure apart from console output) for muxspect.test.mjs, same as
 * renderLayout below, so the failure paths are tested rather than asserted.
 */
export function renderWork(data, stateFilter) {
    // Same 200-with-{error} shape the layout handler uses, and the same reason
    // for handling it explicitly: falling through to "queue is empty" would
    // report success for exactly the store failure this command exists to
    // surface (reagent P1 on PR #2856, which this renderer is modelled on).
    if (data.error) {
        console.error(`work: ${data.error}`);
        return;
    }
    const items = data.items ?? [];
    if (!items.length) {
        // Distinguish "nothing at all" from "nothing MATCHING", so a filtered
        // query never reads as an empty queue.
        console.log(stateFilter ? `no ${stateFilter} items` : "queue is empty");
        return;
    }

    const now = Date.now();
    console.log(`${items.length} item(s)${stateFilter ? ` (state=${stateFilter})` : ""}\n`);
    for (const it of items) {
        const holder = it.claimed_by ? ` held-by=${it.claimed_by}` : "";
        const attempts = `${it.attempts ?? 0}/${it.max_attempts ?? 0}`;
        // A claimed row whose lease is already in the past is the interesting
        // pathology: nobody is working it, and it stays invisible to `list`
        // until the next claim reaps it. Call that out rather than making the
        // reader compare timestamps by eye.
        const expired =
            it.state === "claimed" && it.claim_expires && it.claim_expires <= now
                ? "  LEASE EXPIRED (returns to the pool on the next claim)"
                : "";
        console.log(`${it.id}  ${it.state}${holder}  attempts=${attempts}${expired}`);
        console.log(`  ${it.title}`);
        if (it.kind) console.log(`  kind=${it.kind}`);
        if (it.target_agent) console.log(`  target-agent=${it.target_agent}`);
        if (it.target_group) console.log(`  target-group=${it.target_group}`);
        if (it.not_before && it.not_before > now) {
            console.log(`  deferred until ${new Date(it.not_before).toISOString()}`);
        }
        // The completion trace for a done item, the reason for a
        // failed/released one — the field WorkComplete calls the only record
        // that the work happened.
        if (it.result) console.log(`  -> ${it.result}`);
        console.log("");
    }

    const stuck = items.filter(
        (it) => it.state === "claimed" && it.claim_expires && it.claim_expires <= now,
    ).length;
    if (stuck) {
        console.log(
            `${stuck} item(s) hold an EXPIRED lease — their claimant is gone. ` +
                `They return to the pool the next time any agent calls WorkClaim.`,
        );
    }
}

/**
 * Render `muxspect layout` — the persisted pane tree per tab as an indented
 * outline, plus the layout doctor's verdict.
 *
 * Exported (pure apart from console output) for muxspect.test.mjs, so the
 * failure paths are testable rather than asserted.
 */
export function renderLayout(data) {
    // A whole-request failure (the store couldn't be read at all) comes back
    // as 200 + {error} with NO `layouts` key — see handle_muxspect_layout for
    // why it isn't a 4xx. Without this branch it fell through to "no layouts
    // found" and exit 0, reporting success for exactly the on-disk failure
    // this command exists to surface (reagent P1 on PR #2856).
    if (data.error) {
        console.error(`layout: ${data.error}`);
        return;
    }
    const layouts = data.layouts ?? [];
    if (!layouts.length) {
        console.log("no layouts found");
        return;
    }
    for (const l of layouts) {
        if (l.error) {
            console.log(`tab ${l.tab_name ?? l.tab_id}  \u2716 ${l.error}`);
            continue;
        }
        const verdict = l.healthy ? "healthy" : `${l.violations.length} violation(s)`;
        console.log(
            `tab ${l.tab_name ?? l.tab_id}  (${l.leaf_count} pane(s), ` +
                `${l.minimized_leaf_count} minimized)  ${verdict}`
        );
        for (const v of l.violations ?? []) console.log(`  ! ${v}`);
        for (const n of l.nodes ?? []) {
            const indent = "  ".repeat(n.depth + 1);
            const flags = [];
            if (n.locked) flags.push("minimized");
            // Only meaningful on a branch — a leaf's own locked flag already
            // says it, and repeating it there would be noise.
            if (n.kind === "branch" && n.effectively_minimized) flags.push("all-minimized");
            if (n.id === l.magnified_node_id) flags.push("magnified");
            const tail = flags.length ? `  [${flags.join(", ")}]` : "";
            const label =
                n.kind === "leaf"
                    ? `leaf ${n.block_id ? n.block_id.slice(0, 8) : "(no block)"}`
                    : `${n.flex_direction} (${n.child_count})`;
            console.log(`${indent}${label}  size=${n.size}${tail}`);
        }
        console.log("");
    }
}

/**
 * Split argv into `{ cmd, sub, blockId, nodeId, json, help }`,
 * flags-and-positionals separated regardless of ordering. Flags can
 * legally appear before OR after the positional args (both 'muxspect
 * describe --json <id>' and 'muxspect --json describe <id>' are natural to
 * type) — filtering them out first, then reading command/args
 * positionally, is what makes both orders work; indexing into the raw,
 * flag-interspersed argv (the original implementation) silently ran
 * 'list' instead of 'describe' — or picked up a flag string as the
 * block_id — depending on where the flag landed (reagent P2 on PR #2380).
 *
 * `dock clear <block_id> <node_id>` doesn't fit the flat `{cmd, blockId}`
 * shape the other subcommands use (three positionals after the flag
 * filter, not two) — `sub` disambiguates `dock <block_id>` (read) from
 * `dock clear <block_id> <node_id>` (write) so `main()` doesn't have to
 * re-parse positional[1] itself.
 *
 * Exported (pure, no I/O) for muxspect.test.mjs.
 */
export function parseArgs(argv) {
    const json = argv.includes("--json");
    const help = argv.includes("--help") || argv.includes("-h");
    const positional = argv.filter((a) => !a.startsWith("-"));
    const cmd = positional[0] ?? "list";
    let sub, blockId, nodeId;
    if (cmd === "dock" && positional[1] === "clear") {
        sub = "clear";
        blockId = positional[2];
        nodeId = positional[3];
    } else {
        blockId = positional[1];
    }
    return { cmd, sub, blockId, nodeId, json, help };
}

async function main() {
    const { cmd, sub, blockId, nodeId, json, help } = parseArgs(process.argv.slice(2));

    if (cmd === "help" || help) {
        console.log(USAGE);
        return;
    }

    // Handled before the unconditional requireEnv() below — tier 0
    // (checkSpawnerTier) is a pure env-var check that shouldn't need
    // $AGENTMUX_LOCAL_URL/$AGENTMUX_AUTH_KEY at all; only the fallback to
    // the srv-backed tiers (1-4) needs them.
    if (cmd === "verify-sender") {
        const senderName = blockId;
        if (!senderName) fail("verify-sender requires an agent name — 'muxspect verify-sender <agent_name>'");
        const spawnerVerdict = checkSpawnerTier(senderName, process.env);
        const data = spawnerVerdict ?? (await (async () => {
            const { url, authKey } = requireEnv();
            return apiGet(url, authKey, `/api/v1/muxspect/verify-sender?name=${encodeURIComponent(senderName)}`);
        })());
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderVerifySender(data);
        if (data.status !== "found") process.exitCode = 1;
        return;
    }

    const { url, authKey } = requireEnv();

    if (cmd === "list") {
        const data = await apiGet(url, authKey, "/api/v1/muxspect/list");
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderList(data);
        return;
    }

    if (cmd === "work") {
        // The sole positional arg is an optional state filter. Validated here
        // rather than passed through blindly: a typo like `muxspect work opne`
        // would otherwise return an empty list that reads exactly like "the
        // queue is empty", which is the wrong answer to a different question.
        const stateFilter = blockId ?? "";
        const VALID = ["open", "claimed", "done", "failed", "cancelled"];
        if (stateFilter && !VALID.includes(stateFilter)) {
            fail(`unknown state '${stateFilter}' — expected one of: ${VALID.join(", ")}`);
        }
        const data = await apiGet(
            url,
            authKey,
            `/agentmux/work?state=${encodeURIComponent(stateFilter)}&limit=200`,
        );
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderWork(data, stateFilter);
        return;
    }

    if (cmd === "conversations") {
        const data = await apiGet(url, authKey, "/api/v1/muxspect/conversations");
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderConversations(data);
        return;
    }

    if (cmd === "conversation") {
        const agentName = blockId;
        if (!agentName) fail("conversation requires an agent name — 'muxspect conversation <agent>'");
        const data = await apiGet(url, authKey, `/agentmux/reactive/transcript?agent=${encodeURIComponent(agentName)}`);
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderConversation(data);
        return;
    }

    if (cmd === "find") {
        const query = blockId; // parseArgs puts the sole positional arg here
        if (!query) fail("find requires a block_id or agent name — 'muxspect find <block_id_or_agent>'");
        const isBlockId = UUID_RE.test(query);
        const params = isBlockId ? `block_id=${encodeURIComponent(query)}` : `agent=${encodeURIComponent(query)}`;
        const data = await apiGet(url, authKey, `/api/v1/muxspect/find?${params}`);
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderFind(data);
        // A cross-channel result with found:false (registry match, but the
        // remote lifecycle reads "unknown" — the block is already gone
        // there) doesn't count as a genuine find for exit-code purposes.
        const genuinelyFound = (data.results ?? []).some((r) => r.found !== false);
        if (!genuinelyFound) process.exitCode = 1;
        return;
    }

    if (cmd === "dock" && sub === "clear") {
        if (!blockId || !nodeId) fail("dock clear requires a block_id and node_id — 'muxspect dock clear <block_id> <node_id>'");
        const data = await apiPost(url, authKey, "/api/v1/muxspect/dock/clear", { block_id: blockId, node_id: nodeId });
        if (json) console.log(JSON.stringify(data, null, 2));
        else console.log(`cleared: node ${nodeId} in block ${blockId}`);
        return;
    }

    if (cmd === "dock") {
        if (!blockId) fail("dock requires a block_id — 'muxspect dock <block_id>'");
        const data = await apiGet(url, authKey, `/api/v1/muxspect/dock?block_id=${encodeURIComponent(blockId)}`);
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderDock(data);
        return;
    }

    if (cmd === "layout") {
        // `blockId` is muxspect's generic first positional; for this command
        // it is an optional tab id.
        const qs = blockId ? `?tab_id=${encodeURIComponent(blockId)}` : "";
        const data = await apiGet(url, authKey, `/api/v1/muxspect/layout${qs}`);
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderLayout(data);
        // A tree with violations is a real finding — exit non-zero so a
        // scripted caller notices without parsing stdout. `data.error` is the
        // whole-request failure (no `layouts` key at all); without it the
        // command exited 0 on a store-read failure.
        if (data.error || (data.layouts ?? []).some((l) => l.violations?.length || l.error)) {
            process.exitCode = 1;
        }
        return;
    }

    if (cmd === "describe") {
        if (!blockId) fail("describe requires a block_id — 'muxspect describe <block_id>'");
        const data = await apiGet(url, authKey, `/api/v1/muxspect/describe?block_id=${encodeURIComponent(blockId)}`);
        if (json) console.log(JSON.stringify(data, null, 2));
        else renderDescribe(data);
        return;
    }

    if (cmd === "watch") {
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

// Only run when executed directly (`node muxspect.mjs ...`) — importing this
// module (muxspect.test.mjs imports `parseArgs`) must not trigger a live
// network call / `process.exit`.
if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    main().catch((e) => fail(e?.message ?? String(e)));
}
