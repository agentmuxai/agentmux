# SPEC: Multi-Agent Fleet Control — Select, Broadcast, and Bulk-Act on Many Agents at Once

Status: Research complete, spec drafted — not yet implemented
Date: 2026-08-20

## 0. Origin

Prompted by a direct ask from the repo owner: recent work touched the Swarm
pane, and the natural next question was "can we build tools that let us
control many agents at once?" — with three candidate homes floated (Swarm,
Warden, or somewhere new). This spec is the research + recommendation the
repo owner asked for before any code gets written.

## 1. Where does this belong? Swarm.

**Swarm's own widget description is literally "Multi-agent orchestration"**
(`agentmux-srv/src/config/widgets.json:26`) — but today it is a pure,
read-only telemetry viewer with zero selection state and zero bulk-action
plumbing on either side (frontend or backend). Its charter already promises
what this spec proposes to build; it just hasn't been built yet.

**Warden is not the right home.** Warden's own spec draws the line
explicitly: *"Both touch agent control, but Swarm is workflow (task queues,
lifecycle); Warden is policy (who can do what)"* (`specs/SPEC_WARDEN_WIDGET_2026-05-25.md`,
~line 388). Warden today is a trust/governance/audit surface (Host/LAN/
Internet trust tiers, an injection audit log, a per-agent
`auto_continue_enabled` opt-in registry for the stall-recovery Supervisor
watcher) — every action it exposes is single-target
(`deregisterAgent(agentId)`, `SupervisorNudge(target_agent, action)`).
Warden's spec does describe an unbuilt "kill all" governance action
(SPEC_WARDEN_WIDGET lines ~199-206), which is the one piece of this domain
that plausibly belongs there long-term — see §6 for how this spec avoids
foreclosing that.

**Recommendation: build fleet selection + broadcast + bulk-action directly
into Swarm**, reusing the audit-trail concept Warden already owns for
recording what happened, rather than inventing a third pane or splitting
one coherent feature across two.

## 2. What exists today (nothing to extend, only to reuse)

A full internal audit (see research summary, §8) confirmed every existing
control primitive in this codebase is single-target by construction — a
`String` id field, never `Vec<String>`:

- **Swarm** (`frontend/app/view/swarm/`): tracks `AgentTreeNode`s per
  top-level block via `RpcApi.AgentTrackedBlocksCommand` → backend
  `agent.tracked-blocks`. No selection signal exists anywhere in
  `SwarmViewModel`. Every action (Focus, Retire, per-shell Stop) is a
  single-row button.
- **Agent App API** (`agentmux-srv/src/server/app_api/agent_io.rs`):
  `agent.send`, `agent.stop`, `agent.kill-process`, `agent.kill-tree` all
  take exactly one `block_id`. `agent.list` / `agent.tracked-blocks` are the
  only "return everything" reads.
- **MCP tools** (`agentmux-mcp/src/main.rs`): `SendMessage`, `SupervisorNudge`,
  `Loop`, `Shell*`, `FocusWindow` all take a single target. `DiscoverAgents`
  is the one read-only "enumerate every reachable agent" tool.
- **jekt/muxbus transport** (`agentmux-common/src/jekt_sign.rs`,
  `agentmux-srv/src/server/reactive.rs`): `sign_jekt`/`verify_jekt` and the
  inject pipeline are strictly 1:1 — no multicast envelope exists at any
  layer, and `TRUST`/`TIER` are computed per (sender, message) pair, not
  per broadcast.
- **Frontend**: no `selectedIds`-style plural selection state exists
  anywhere in the app (grepped for the whole naming family — zero hits). No
  shift/ctrl-click range selection on tabs, panes, or agent lists. The one
  in-repo precedent for "check several items, then one action applies to
  the checked set" is `BundleImportPreviewModalPanel`
  (`frontend/app/view/memory/components/BundleImportPreviewModal.tsx`) — a
  `Record<id, boolean>` signal + per-row checkbox + one aggregating submit
  button. This is the pattern to mirror, not extend (nothing pane/agent-
  specific exists to build on).
- **Reusable result-shape precedent**: `importagents`/`exportagents`
  (`agentmux-srv/src/server/agent_handlers/core.rs:549-671, 677-744`)
  already return a clean partial-success/failure envelope —
  `{ imported: Vec<String>, skipped: Vec<String>, failed: Vec<String> }` —
  for looping over many items server-side. This is the shape every new bulk
  RPC in this spec follows.

**Conclusion: this is new work end-to-end** (new backend batch RPCs, a new
selection/group model, a new frontend toolbar), not a small extension of
an existing hook. Budget accordingly.

## 3. External best practices (full findings in §8)

- **Checkboxes are the correct selection primitive** — but K9s and Lens (the
  two most mature Kubernetes GUI tools) notably ship *without* multi-select,
  relying on filters/scripting instead. Treat that as a documented gap to
  avoid replicating, not a precedent to follow.
- **Saved groups beat ephemeral selection at fleet scale.** Ansible's
  inventory groups (`canary`, `stage1`, `all`) are the load-bearing
  abstraction every safety mechanism (staged rollout, `serial` batching) is
  defined against — not the one-off checkbox state itself.
- **Staged rollout, not just a confirm dialog**, for destructive bulk ops:
  Ansible's `serial: [1, 5, "25%", "100%"]` (canary-then-widen) combined
  with `max_fail_percentage` (auto-abort the whole run if a batch's failure
  rate crosses a threshold) caps blast radius mechanically, not just via a
  human clicking "yes."
- **tmux's `synchronize-panes`** is the direct precedent for "type once,
  send to N agents" — broadcast keystrokes live to every pane in a group.
  Zellij's multi-year-open feature request for the same thing
  (`zellij-org/zellij#302`) is a cautionary tale about this being an
  obvious ask that's easy to deprioritize — don't let that happen here.
- **Confirmation fatigue is real** — reserve blocking confirmations for
  truly irreversible actions (bulk kill); use instant-execute +
  undo-window toast for reversible ones (bulk message/nudge).
- **The single most commonly-cited fleet-ops failure mode across every
  domain surveyed: silent partial failure.** Mature tools always report
  per-target success/failure (a list, not one aggregate toast). This spec
  makes that non-negotiable (§5.4).
- **Never hide scope.** State the exact resolved target count before
  executing ("Stop 14 agents in group `backend`"), never "Stop selected."

## 4. Architecture

```
Human (Swarm pane)                    Agent (MCP tool)
  │ select N agents / pick a group       │ FleetList (discover targets)
  │ or "select all N matching"           │ FleetBroadcast(targets|group, msg)
  ▼                                      ▼
Swarm frontend: selection signal   agentmux-mcp: new tools, same
(Record<blockId, boolean>) +       backend RPCs as the human path
saved-group picker
  │                                      │
  └──────────────┬───────────────────────┘
                  ▼
     agentmux-srv: new batch RPCs (agent_handlers/fleet.rs)
       - fleet.broadcast   { targets: Vec<String> | group_id, message }
       - fleet.bulk-stop   { targets: Vec<String> | group_id, staged?: StagePlan }
       - fleet.group.*     (create/list/update/delete saved groups)
                  │
                  ▼  loops the EXISTING single-target primitives
       agent.send (per target) / agent.stop (per target) / jekt inject (per target)
                  │
                  ▼
       { succeeded: Vec<String>, failed: Vec<{id, error}> }  ← always, never a bool
                  │
                  ├─→ Swarm UI: per-agent result rows (never one aggregate toast)
                  └─→ Warden Audit log: one AuditLogEntry per bulk action invocation
                        (existing AuditLogEntry shape, `agentmux-srv/src/backend/reactive/types.rs`),
                        so Warden's Audit tab sees fleet actions without Warden owning any of the new code
```

No new transport is invented. `fleet.broadcast`/`fleet.bulk-stop` are thin
server-side loops over the existing single-target `agent.send`/`agent.stop`
paths — this deliberately does NOT touch the jekt signing/trust model
(§2's finding that `TRUST`/`TIER` are computed per single message stands;
each fanned-out message is signed and delivered exactly like an individual
`SendMessage` call would be, just looped server-side instead of client-side
N times).

## 5. Design details

### 5.1 Targeting: selection, groups, and "all"

- Ephemeral selection: a checkbox per top-level agent card in Swarm,
  backed by `Record<blockId, boolean>` (mirrors
  `BundleImportPreviewModalPanel`'s pattern for UI consistency — see §2).
  "Select all visible" and a separate, explicitly-labeled "select all N
  matching" (if Swarm ever gains filtering) are two distinct actions per
  §3's cross-page ambiguity finding — never conflate them.
- Saved groups: a new small table, `db_agent_groups (id, name, agent_ids,
  created_at)`. Created via "Save selection as group" from the current
  checkbox state; reused later as a one-click target set. This is the
  Ansible-inventory-group lesson from §3 — the durable abstraction bulk
  actions should target, not raw ephemeral selection alone.
- Target resolution for both the RPC layer and the MCP tools accepts
  *either* an explicit `targets: Vec<String>` (agent ids or block ids) *or*
  a `group_id` — resolved server-side to a concrete list before any action
  runs, and that resolved list (with its count) is always shown/returned
  before execution.

### 5.2 Broadcast (non-destructive)

`fleet.broadcast { targets_or_group, message }` loops `agent.send` (or the
jekt inject path directly) once per resolved target, server-side, and
returns `{ succeeded: Vec<String>, failed: Vec<{id, error}> }`. Per §3:
non-destructive, so no blocking confirmation — execute immediately, surface
an undo-*window* toast only if a genuine undo is possible (it generally
isn't for "a message was already read by an agent," so in practice this is
"immediate execute + a clear per-agent result list," not a literal undo).

### 5.3 Bulk stop / kill (destructive)

`fleet.bulk-stop { targets_or_group, staged: Option<StagePlan> }` where
`StagePlan { batch_sizes: Vec<usize>, max_fail_percentage: u8 }` — directly
modeled on Ansible's `serial` + `max_fail_percentage` (§3). Default (no
`StagePlan`): a single batch, all targets, but ALWAYS behind a blocking
confirmation modal that lists the exact resolved target set and count
(never "stop selected"). Optional staged mode for large N: canary batch
first, auto-abort the remaining batches if the failure rate in any batch
exceeds `max_fail_percentage`.

### 5.4 Feedback — no aggregate toast, ever

Every bulk action, destructive or not, renders a per-target result list in
Swarm after completion (id, succeeded/failed, error message on failure) —
this is the single most load-bearing UX requirement in this spec, directly
answering §3's "silent partial failure is the #1 real-world fleet-ops
failure mode" finding. A single "Done" or "12/14 succeeded" toast is
explicitly insufficient on its own; it may summarize, but the detail list
must be reachable in the same view without navigating away.

### 5.5 MCP tools (agent-initiated fleet control)

- `FleetList {}` — thin wrapper combining `DiscoverAgents` + `agent.list`
  for an agent to see what's reachable before deciding whom to target.
- `FleetBroadcast { targets_or_group, message }` — calls `fleet.broadcast`.
- `FleetBulkStop { targets_or_group, staged? }` — calls `fleet.bulk-stop`.
  Given the destructive nature, this tool's schema should require an
  explicit `targets` list or a pre-existing named `group_id` — no
  "stop everything" implicit default, ever, from an agent-initiated call.

New tools, not modified existing ones (`SendMessage` keeps its current
single-`to` schema) — avoids any ambiguity about whether existing
single-target call sites suddenly need to handle array inputs.

## 6. Warden tie-in (without Warden owning any of this)

Every bulk action writes one `AuditLogEntry` (existing shape,
`agentmux-srv/src/backend/reactive/types.rs:284-312`) to the same audit log
Warden's Audit tab already reads — so a fleet broadcast or bulk-stop is
visible there automatically, with zero new code on Warden's side. When
Warden's own spec'd-but-unbuilt "kill all" governance action
(SPEC_WARDEN_WIDGET lines ~199-206) eventually gets built, it should call
`fleet.bulk-stop` with a group resolving to "every agent" rather than
inventing its own bulk-stop mechanism — noted here explicitly so that work,
whenever it happens, doesn't duplicate this spec's primitive.

## 7. Explicitly out of scope (this spec)

- Cross-instance / LAN / WAN fleet control (targeting agents on a different
  AgentMux instance) — everything here is host-local, consistent with every
  existing control primitive being host-local-first per §2.
- tmux-style *live* keystroke synchronization (typing into N terminals
  simultaneously) — `fleet.broadcast` sends one discrete message per
  target, not a live input mirror. Worth a future spec if there's demand
  (§3 flags this as the direct tmux precedent), but it's a materially
  different mechanism (continuous vs. one-shot) and shouldn't block this
  spec's more common "broadcast an instruction" use case.
- Warden's "kill all" / capability-set / quota governance model — untouched,
  see §6 for the intended future relationship.
- A generic cross-app multi-select primitive (tabs, panes, windows) beyond
  what Swarm's fleet-control UI needs — no evidence of demand elsewhere
  today (§2).

## 8. Full research findings (appendix)

<details>
<summary>Internal: Swarm pane audit</summary>

Swarm (`view: "swarm"`, `defwidget@swarm`) is a live monitoring dashboard,
not a control surface, today. Per top-level agent block a user can:
expand/collapse (`toggleAgentCollapsed`, `swarm-model.ts:1127`), Focus
(`swarm-view.tsx:27-68`), copy id, expand a subagent/workflow row's
activity feed (`toggleDispatchExpanded`, `swarm-model.ts:1316`), "Retire" a
finished row (client-local only, `retireRow`, `swarm-model.ts:1148`), and
Stop one background shell (`swarm-view.tsx:431-437`, the only real
backend-mutating action, and it's per-shell not per-agent).

Data model: `AgentTreeNode` (`swarm-model.ts:169-195`) per tracked block
(`SwarmViewModel.trackedBlockIdsAtom`, `RpcApi.AgentTrackedBlocksCommand` →
backend `agent.TrackedBlocks`), fanning into four independently-polled
buckets: `agentToolRows` (Task-tool subagents), `workflowRows` (Workflow
dispatches, collapsed to one row regardless of member count),
`shellRows`, `cronRows`. Backend: `agentmux-srv/src/backend/subagent_watcher/`,
`shell_node.rs`, `server/cron.rs`; `rpc_types/agent.rs:235,261` /
`commands.rs:287-290` explicitly document Swarm as the consumer. No
`swarm.*` RPC namespace exists.

No selection state anywhere in `SwarmViewModel` — no `selectedIds`, no
"select all," every action is a per-row button. No bulk RPCs exist
backend-side.

</details>

<details>
<summary>Internal: Warden pane audit</summary>

`specs/SPEC_WARDEN_WIDGET_2026-05-25.md` frames Warden as governance/trust:
"monitor and control every AgentMux instance reachable from this host
across three trust layers: Host / LAN / Internet," explicitly distinguished
from Swarm ("Swarm is workflow... Warden is policy"). Most of the original
vision (`governance.json`, capability sets, kill-all, quotas) is unbuilt —
only a "Phase 1 shell" exists.

Implemented today: **Host tab** — lists `ReactiveHandler`-registered agents,
per-row "deregister" (soft-kill, removes jekt routing, doesn't kill the
PTY) (`frontend/app/view/warden-host/warden-host-manager.tsx:31-64,89-100`).
**LAN/Internet tabs** — largely stubs. **Audit tab** — reads the jekt
injection audit ring buffer (`GET /agentmux/reactive/audit`,
`agentmux-srv/src/backend/reactive/handler.rs`, `AuditLogEntry` in
`agentmux-srv/src/backend/reactive/types.rs:284-312`). **Supervisor tab** —
per-agent `auto_continue_enabled` opt-in toggle list + a filtered audit
feed of nudge decisions (`warden-supervisor-manager.tsx:29-98`) — a
continuation-watcher opt-in registry, not enforcement.

`SupervisorNudge` (MCP tool) is Warden's one live automation primitive:
single `target_agent: string`, one `action` (`nudge`/`decline`),
`agentmux-mcp/src/main.rs:147-157`, enforced via
`record_supervisor_decision` (`handler.rs:725,1037`). Strictly
single-agent, stall-recovery-flavored.

No multi-select UI, no batch endpoint, no bulk/kill-all actually
implemented (spec'd, SPEC_WARDEN_WIDGET lines ~199-206, unbuilt).

</details>

<details>
<summary>Internal: full control-primitive inventory</summary>

**Agent App API** (`agentmux-srv/src/server/app_api/agent_io.rs` +
`rpc_types/agent.rs`): `agent.open`, `agent.send`, `agent.stop`,
`agent.status`, `agent.output`, `agent.process-list`, `agent.kill-process`,
`agent.kill-tree` — every write/action verb takes exactly one `block_id`.
`agent.tracked-blocks` and `agent.list` are the two "return everything"
reads (unfiltered fan-out, no target list accepted). Reusable partial-
result-shape precedent: `importagents`/`exportagents`
(`agent_handlers/core.rs:549-671, 677-744`) — `{imported, skipped, failed}:
Vec<String>` each, though neither has a frontend caller today (RPC-only).

**MCP tools** (`agentmux-mcp/src/main.rs`): `SendMessage` (`to: string`,
`main.rs:111-122`, POSTs `/agentmux/reactive/inject` with singular
`target_agent`, `server/reactive.rs:1143`), `SupervisorNudge`
(`target_agent: string`), `Loop`/`LoopStop`/`LoopList` (per-agent-process-
scoped, no cross-agent visibility), `DiscoverAgents` (no args — the one
"enumerate every reachable agent" tool, GET `/agentmux/discovery`),
`GetAgentTranscript` (`agent: string`), `Shell`/`ShellInput`/`ShellStatus`/
`ShellStop` (single `shell_id`), `FocusWindow` (single `window_id`). No
multi-target primitive anywhere.

**Frontend multi-select**: no `selectedIds`-style plural selection state
exists anywhere (grepped, zero hits). No shift/ctrl-click range selection
on tab bar, pane grid, or agent list. List-with-detail views (MCP/Skill
managers, `AgentPicker.tsx`) use single-`selectedIdAtom` master-detail, not
multi-select. `WardenSupervisorManager` renders a table with a per-row
checkbox, but each checkbox independently fires its own RPC — no "check N,
then batch-apply" affordance. The one true precedent:
`BundleImportPreviewModalPanel`
(`frontend/app/view/memory/components/BundleImportPreviewModal.tsx`) —
`Record<id, boolean>`-style per-item selection signals (lines 40, 43, 46),
checkbox rows (lines 142-149, 176-186, 204-211, 258-263), one aggregating
submit button (line 309).

**jekt/muxbus transport**: strictly 1:1 at every layer. `InjectRequest`
carries a single `target_agent: String` (`reactive.rs:1143`, used
throughout `handle_reactive_inject`, `reactive.rs:385+`). `sign_jekt`/
`verify_jekt`/`sign_lan_jekt`/`verify_lan_jekt`
(`agentmux-common/src/jekt_sign.rs:68,78,245,267`) all single-target — no
multi-recipient envelope. Delivery-tier resolution
(`agent_registry::lookup`/`lookup_all_shared`, `reactive.rs:452,571`) fans
out over delivery *paths* for one recipient, not over multiple *distinct*
recipients. CLAUDE.md's own jekt marker format (`TRUST=`/`TIER=`/
`ESCALATE=`) is entirely phrased per single (sender, message) pair.

</details>

<details>
<summary>External: multi-agent orchestration & fleet-ops best practices</summary>

**Orchestration patterns**: Magentic-One uses a single Orchestrator
delegating to specialized agents with a shared ledger as collective memory,
redirecting on error (Microsoft Learn / AutoGen docs) — hub-and-spoke, not
peer-to-peer. LangGraph treats human approval as a first-class primitive
(`interrupt_before`/`interrupt_after` pause a node, expose full state,
resume with an explicit decision, rewind-to-checkpoint to branch) — a
strong model for a per-agent pause/approval primitive a bulk action could
invoke across a selection. OpenAI's Swarm/Agents SDK uses lightweight
handoffs, not centralized bulk control.

**Fleet UX**: checkboxes are the industry-default selection model, but K9s
and Lens (mature K8s GUIs) notably ship without GUI multi-select, relying
on filters/scripting — a documented gap, not a precedent to copy. Ansible's
inventory groups (canary/stage1/stage2/all) are the load-bearing
abstraction every safety mechanism is defined against, more than ephemeral
selection itself. Staged rollout: `serial: [1, 5, "25%", "100%"]` +
`max_fail_percentage` caps blast radius mechanically. `--check --diff`
dry-run previews without applying. tmux's `synchronize-panes` is the direct
precedent for "type once, send to N" — mirrors keystrokes live to every
pane in a group; Zellij's multi-year-open equivalent feature request
(`zellij-org/zellij#302`) is a cautionary tale about deprioritization. PM2
uses a simple `<name|namespace|id|'all'>` targeting grammar plus a live
per-process dashboard as a reasonable minimal bar.

**Failure modes**: confirmation fatigue is real and self-defeating —
reserve blocking confirms for irreversible actions, use undo-window toasts
for reversible ones. NN/g's proximity rule: never place a destructive bulk
action adjacent to a benign one; layer redundant signals (color + icon +
label) since users act on autopilot during repetitive fleet-ops sessions.
Hidden scope is the top accidental-broadcast cause — always state the
concrete target count before executing. Silent partial failure is the #1
fleet-specific pitfall across every domain surveyed — mature UIs report
per-target success/failure explicitly, never one aggregate toast.
Cross-page "select all" ambiguity — "select all N matching" must be a
distinct, separately-confirmed step from "select all on this page."

</details>
