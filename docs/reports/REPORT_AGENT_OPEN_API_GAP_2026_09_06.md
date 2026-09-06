# Report: no App API verb opens an agent — automation cannot start what it can stop

**Date:** 2026-09-06
**Status:** implemented — §5 steps 1, 2 and 4 (HTTP route, `OpenAgent` MCP tool, `muxopen`) shipped in the same PR as this report; cross-channel (step 3), the `muxspect` addressable column, LAN and WAN remain open
**Author:** Camper
**Repo state:** main @ `2c7604f1f` (v0.55.35)
**Probed live** against a running instance (`v0.55.34`, `http://127.0.0.1:55019`)
**Related:** `REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md`
(four-tier audit — did not cover this gap),
`SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22.md`,
`SPEC_FLEET_BROADCAST_CROSS_TIER_TARGETING_2026_08_22.md`
**Blocks:** #2939 / #1400 (container agents) whenever a GUI is unavailable

---

## 1. The question, and the answer

**Asked:** does the agent-facing App API let an agent (or any automation) launch
an agent into a pane?

**Answer: no.** The capability exists inside srv, but nothing an agent can reach
exposes it.

| Surface | Can it launch an agent? | Evidence |
|---|---|---|
| `agent.open` RPC | **Yes** — creates the block, seeds the resume session, registers the controller | `server/app_api/agent_open.rs:148`, `COMMAND_AGENT_OPEN = "agent.open"` (`rpc_types/commands.rs:333`) |
| `/api/v1/*` HTTP (agent-facing, `X-AuthKey`) | **No such route** | Enumerated every `/api/v1/agent*` route: only `identity/*`, `memory/*`, `preset/*` |
| MCP tools (55 exposed) | **No launch verb** | Full list from `agentmux-mcp/src/main.rs`; nearest is `NewTab`, which opens an *empty* tab |
| `pane.open` (`/api/v1/pane/open`) | **No** — opens a *view* into a pane; does not create an agent or register a controller | `server/app_api/pane.rs` |

`agent.open` is reachable only over the WebSocket RPC engine the **frontend**
uses. It has no HTTP mapping and no MCP wrapper, so it is effectively
GUI-only.

### 1.1 How this was established

Not by inspection alone. Against the live instance, using this agent's own
`AGENTMUX_AUTH_KEY` / `AGENTMUX_LOCAL_URL`:

- `GET /api/v1/self?block_id=…` → 200 with real block/tab/window/workspace ids,
  proving the credential and the HTTP surface both work.
- `GET /agentmux/reactive/agents` → `{"error":"unauthorized"}` — the reactive
  routes take a *different* key, so raw curl is not a way around this.
- `FleetList` → `Scouto` (`fb770209-…`) present with `addressable: false`,
  `block_id: null`. Its container `agentmux-fb770209-…` has been **Up 5 days**.
- `SendMessage → Scouto` → **`agent not found: Scouto`**.

The last two together are the whole problem: a container agent can be running
as a container, be visible in the directory, and still be unreachable — with no
verb anywhere that would make it reachable.

---

## 2. Why it matters

### 2.1 The lifecycle is asymmetric

`FleetBulkStop` was deliberately extended to reach **cross-channel** targets
(`SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22.md`, "Implemented (cross-channel
only)"). So today:

> Automation can **stop** an agent in another channel. It cannot **start** an
> agent anywhere — not cross-channel, not even in its own instance.

That asymmetry is the finding. A destructive verb crossed the instance boundary
before the constructive one existed at all.

### 2.2 It blocks container agents specifically

`agent not found` for `Scouto` is thrown at `backend/reactive/handler.rs:385` —
the **registry lookup**, before any controller is consulted. A container agent
with `block_id: null` is not in the delivery registry, so nothing can be
injected into it.

This is the third member of a known failure family, and the first two are fixed:

| | Case | Status |
|---|---|---|
| #2930 | Subprocess agents dropped entirely | Fixed (for *running* agents) |
| #2960 | Persistent agents that have not spawned | Fixed — spawn-on-demand |
| — | **Container/subprocess agents with no block** | **Unfixed** |

#2960's fix is persistent-only: the commit is *"deliver to persistent agents
that have not spawned yet"* and all the "spawns on first message" machinery
lives in `blockcontroller/persistent.rs`. There is no subprocess equivalent.

**Consequence for #2939/#1400:** every headless verification of a container
agent currently requires a human to open a pane first. On a machine where the
GUI is in use by a person, that is not merely inconvenient — the work cannot
proceed at all.

### 2.3 The prior audit missed it

`REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md` audited the four
tiers and listed gaps 3.1–3.4 (broadcast reach, bulk-stop scope, muxspect
Phase B/C, UI automation). It examined the control verbs that **exist** and
never noted that no launch verb exists. Worth recording as a lesson: an audit
scoped to "are the existing verbs correct?" will not surface "a verb is
missing."

---

## 3. Proposal

### 3.1 `agent.open` over the agent-facing HTTP surface

Add `POST /api/v1/agent/open`, mapping to the same `agent_open` implementation
the RPC command already calls — **not** a parallel implementation. `agent.open`
carries real concurrency logic (the per-definition `AGENT_OPEN_LOCKS` guarding
the check-live → seed-resume → create-block → register-controller sequence,
added for the TOCTOU reagent flagged on #2059); a second code path would have to
re-derive all of it.

Request shape should mirror the RPC command's existing data type rather than
inventing one, so the two cannot drift.

### 3.2 An MCP verb

Expose it as an MCP tool (working name `OpenAgent`) alongside `FleetList` /
`SendMessage`. `FleetList` already returns exactly the identifiers such a call
needs (`definition_id`, `block_id`, `addressable`, and per-target `channel` /
`local_url` for cross-channel entries), so the discovery half is done.

Minimum arguments: the target agent (by name or definition id), and an optional
tab/workspace placement. Everything else should default to what the UI flow
would do.

### 3.3 Cross-tier reach

The requirement is "openable on any instance, across any channel or the tiered
networks." The tiers already have precedent to follow, and they are **not**
equally ready:

| Tier | Precedent | Assessment |
|---|---|---|
| **Same instance** | — | Straightforward: call the existing impl. |
| **Cross-channel** (same host, different channel) | `FleetBulkStop` (implemented), `FleetBroadcast` | Follow the same resolution path. `FleetList.host.cross_channel[]` already yields each target's own `local_url`, so this is a forward to a sibling srv. |
| **LAN** | `FleetBroadcast` cross-tier targeting; LAN peer visible in `FleetList.lan[]` with `auth_key` | Feasible, but see §4.1 — this is where opening stops resembling stopping. |
| **WAN** | `wan.subscribed_agents`; muxbus | Deliberately deferred. `FleetBulkStop` deferred LAN/WAN too, for reasons that apply *more* strongly to a constructive verb. |

**Recommendation: ship same-instance + cross-channel first; treat LAN as a
second phase; do not do WAN in this work.** That mirrors the sequencing
`FleetBulkStop` already chose, and each tier is independently useful.

### 3.4 Auxiliary tooling

- **`muxopen`** — a shell entry point beside `muxlog`/`muxspect`, for opening an
  agent from a terminal without the GUI. `muxspect` already answers "what is
  running"; this answers "make it run". Same core-invocation caveat applies
  (`node ~/.agentmux/shell/muxopen.mjs` from a tool-spawned shell — see
  `docs/MUXSPECT.md` and the known bare-function gap).
- **`muxspect` cross-reference** — teach it to show `addressable` alongside
  lifecycle, so "container up, agent unreachable" is visible in one place rather
  than requiring `FleetList` + `docker ps` to be correlated by hand, as was
  necessary to find this.
- **A close/stop counterpart** already exists (`FleetBulkStop`, `/agentmux/agent/stop`);
  no new work, but the pairing should be documented so the lifecycle reads as
  symmetric.

---

## 4. Risks and open questions

### 4.1 Opening is not the mirror image of stopping

`FleetBulkStop`'s spec calls stopping "destructive" and scopes it carefully.
Opening is destructive in a different, less obvious way: it **spawns a process,
consumes provider tokens, and can bind credentials**. A remote open is closer to
"execute code on that machine" than "stop something already running". The
`ENFORCE_AGENT_BINDING` work (audit §4.2 — *"built, verified ~90%, never turned
on"*) is directly relevant and should be settled before LAN/WAN, not after.

### 4.2 Concurrency across instances is already unsolved

`agent_open.rs`'s own doc: the `AGENT_OPEN_LOCKS` guard *"only serializes calls
handled by THIS process — it can't see a genuinely different AgentMux
instance/channel racing the same registry entry."* A cross-channel open verb
makes that pre-existing race **reachable on purpose** rather than by accident.
This needs an explicit decision — accept it, or add a cross-instance guard — and
must not be discovered later.

### 4.3 Unverified

- That a *running* container agent is reachable via `SendMessage` (i.e. that
  #2930 holds for containers). None of the four currently-addressable agents are
  containers, and neither live container maps to an addressable agent, so this
  was **not** confirmed — only the unspawned case was.
- Whether opening an agent whose container predates #2933 triggers the
  mount-drift recreate path (#2939 workstream 4, still unobserved in a real app).

### 4.4 Deliberately not proposed

- **Driving the GUI** (`UIClick`/`UIQuery`) to open a pane. It works, but it
  mutates what a human sitting at the machine sees, which is precisely the
  constraint that motivated this report.
- **A second `agent.open` implementation** on the HTTP path. See §3.1.

---

## 5. Suggested order

1. `POST /api/v1/agent/open` → existing impl, same instance only.
2. MCP `OpenAgent` verb over it. **At this point the container work in #2939/#1400
   is unblocked headlessly** — everything after is reach, not capability.
3. Cross-channel reach, following `FleetBulkStop`'s resolution path.
4. `muxopen` + the `muxspect` addressable column.
5. LAN — only after §4.1 is settled.
6. WAN — out of scope here.

---

## 6. Appendix: probe transcript

```
$ curl -H "X-AuthKey: $AGENTMUX_AUTH_KEY" "$AGENTMUX_LOCAL_URL/api/v1/self?block_id=$AGENTMUX_BLOCKID"
{"block_id":"a7759b9c-…","tab_id":"24c454e9-…","window_id":"22e115ce-…",
 "workspace_id":"f01e97cb-…","workspace_name":"Starter workspace"}

$ curl "$AGENTMUX_LOCAL_URL/agentmux/reactive/agents"
{"error":"unauthorized"}

FleetList → Scouto: { addressable: false, block_id: null,
                      definition_id: "fb770209-2029-4100-b01e-16fa89904cac" }

$ docker ps -a --filter name=agentmux-
agentmux-fb770209-2029-4100-b01e-16fa89904cac   Up 5 days   ghcr.io/agentmuxai/agent-claude:latest

SendMessage(to: "Scouto") → Message delivery failed: agent not found: Scouto

$ docker inspect agentmux-fb770209-… --format '{{range .Config.Env}}{{println .}}{{end}}'
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
CLAUDE_CONFIG_DIR=/home/agent/.claude
      # no AGENTMUX_* env at all; no bashwrap on PATH; no /workspace mount
      # (this container predates #2933)
```
