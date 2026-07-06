# SPEC: Swarm Live Feed — UI

**Date:** 2026-07-05
**Status:** Draft
**Owner:** camper
**Companion spec:** `SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05.md`

## Goal

Upgrade the swarm pane into a real-time feed of everything each agent is
doing:

```
▸ Camper                                   [working]
    "verifying release pipeline artifacts"          ← haiku summary, live
    ▸ workflows/deep-research (wf_d1f80d9f)  63/84 agents done
        ▸ verify:vicky-ward-quote            [active]
        ▾ search:journalists                 [completed]
            │ Searching for Vanity Fair 2003 profile…
            │ ⚙ WebSearch {"query": "..."}
            │ ✓ 12 results (2.1 KB)
    ▾ Explore: map frontend UI               [completed]
        │ …live stream as simple text…
```

- Row 1 per agent: name + status chip (exists today in the swarm pane).
- Row 2: **haiku summary** — live one-liner of what the agent is working on.
- Children: **subagents and workflows, collapsed by default**. Workflow
  nodes group their member agents and show `done/total` progress.
- Expanding a subagent shows its **live event stream as simple text**.
- The whole tree is **virtualized** — large swarms (10 agents × workflows ×
  16 concurrent members × hundreds of stream lines) must scroll smoothly.

## What exists (verified 2026-07-05)

- `view/swarm/{swarm-view.tsx,swarm-model.ts}` — 2-level tree (AgentRow →
  SubagentRow), always expanded, no inline stream. Subscribes to
  `subagent:spawned/completed`, `agent:process-added/exited`, scoped
  `ControllerStatus`. Clicking a subagent opens the separate subagent pane.
- `view/subagent/` — separate pane streaming one subagent's events.
- **Agent-pane virtualization** (`view/agent/virtualization/`):
  `AgentDocumentVirtualList` — custom virtualizer (absolute positioning,
  per-row ResizeObserver measurement, scroll anchoring in `anchor.ts`,
  height estimation in `renderers.ts`, layout math in the
  `agent-pane-layout` store) plus a non-virtualized trailing streaming
  buffer. This is the infra to reuse. `@tanstack/solid-virtual` is
  installed but unused — do not adopt it.
- Realtime: `waveEventSubscribe` in `store/wps.ts`; canonical usage pattern
  in `swarm-model.ts` (subscribe in ctor, unsubs array, `dispose()`).
- Design system: tokens in `frontend/app/theme.scss`; stylelint forbids raw
  hex, `!important`, and non-token z-index.

## Design

### 1. Row model — flatten, then virtualize

The tree is state + a flattening selector; the DOM is one virtualized flat
list. Avoids nested scrolling and makes row count the only perf variable.

```ts
type SwarmRow =
  | { kind: "agent";    id: string; node: AgentTreeNode }
  | { kind: "summary";  id: string; agentId: string; text: string }
  | { kind: "workflow"; id: string; wf: WorkflowNode; depth: 1 }
  | { kind: "subagent"; id: string; sa: SubagentNode; depth: 1 | 2 }
  | { kind: "stream";   id: string; line: StreamLine; depth: 2 | 3 };
```

`swarm-model.ts` additions:
- `workflowsByAgent: Map<agentId, WorkflowNode[]>` — from
  `subagent.ListWorkflows` backfill + `workflow:updated` events.
- `subagentsByParent: Map<workflowId | agentId, SubagentNode[]>` — existing
  subagent atoms re-keyed; a subagent with `workflowId` parents to its
  workflow node, else to its agent.
- `summaries: Map<agentId, string>` — from `agent:summary` events, seeded
  from block meta `term:activity`.
- `expanded: Set<rowId>` — collapse state. Defaults: workflows and
  subagents collapsed; agent sections expanded (summary row always shows).
- `streams: Map<subagentId, StreamLine[]>` — ring buffer, cap 500 lines
  (older lines drop; full history remains in the subagent pane).
- `flattenTree(): SwarmRow[]` — memo; only expanded nodes contribute rows.

### 2. Row rendering

- **agent** — name + status chip (existing `AgentStatusChip`), chevron.
- **summary** — caption size, secondary color, italic; empty until first
  `agent:summary` arrives.
- **workflow** — `workflows/<name-or-id>` + `done/total agents` counter +
  advisory status dot; chevron.
- **subagent** — slug + status (existing SubagentRow visuals), chevron;
  click still opens the full subagent pane (unchanged).
- **stream** — monospace caption, single line, CSS-truncated with title
  tooltip:
  - `text` → first 200 chars
  - `tool_use` → `⚙ <name> <input_summary>`
  - `tool_result` → `✓ <preview>` / `✗ <preview>` when is_error
  - `progress` → `… <output>`

Stream backfill on expand: `subagent.GetHistory(agentId, 200)`, then live
append from `subagent:activity`.

### 3. Virtualization — reuse the agent pane's infra

We already built and hardened a virtualizer for the agent document; the
swarm feed is a strictly simpler instance of the same problem (append-heavy
list, bottom pinning, mixed row heights).

- Extract the core (positioning, ResizeObserver measurement, scroll
  anchoring) into a shared module under `frontend/app/element/` — or
  parametrize `AgentDocumentVirtualList` over a row renderer — so the agent
  doc and swarm tree consume one implementation. The streaming-buffer
  partition is disabled for swarm: stream rows are single-line and cheap.
- Height estimates by kind (agent 28, summary 20, workflow 24, subagent 24,
  stream 18 px) feed the existing estimator.
- Follow-output: reuse the anchor pattern — pinned to bottom of an expanded
  stream while at bottom; no viewport jump otherwise.
- `@tanstack/solid-virtual`: candidate for dep removal in a separate
  cleanup PR.

### 4. Styling

Tokens only: `--space-*`, `--text-caption` for summary/stream rows,
`--color-*`/`--*-color` from theme.scss (no raw hex), `--z-*` for any
overlay, no `!important`. Indentation via `--space-*` multiples of depth.

## Phases / PRs

1. **PR-3 (frontend):** swarm tree rework — flatten + collapse/expand +
   summary row + workflow nodes + inline streams, using the extracted
   virtualizer.
2. **PR-4 (polish):** follow-output pinning, ring-buffer tuning, perf probe
   on a synthetic 10-agent × 3-workflow swarm (reuse
   `virtualization/perf-probe.ts`).

Each PR reviewed by reagent, merged only on approval.

## Open questions

1. Ring buffer 500 lines/subagent — enough?
2. Should collapse state persist across pane reloads (block meta) or reset?
   v1: reset.
3. Virtualizer extraction vs parametrization — decide in PR-3 after reading
   `AgentDocumentVirtualList` closely; extraction preferred if the diff to
   the agent pane stays mechanical.
