# Report: Live dispatch cards for Agent/Task/Workflow tool calls in the transcript

**Date:** 2026-08-19
**Status:** Implemented, tested, not yet visually/interactively verified (see Verification below)

## Context

This is a continuation of prior swarm-refinement iterations. Earlier work
(`SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md`) already grouped
agent/subagent activity by originating dispatch in the **Swarm panel** and
**Activity Dock**. The gap this work closes is the **main transcript pane**:
when an agent calls the `Agent`/`Task` tool (spawns a subagent) or a
`Workflow` tool (multi-step orchestration), the transcript rendered it
through the same generic pipeline as `Bash`/`Read`/etc. — a one-line
collapsed summary expanding to a raw `JSON.stringify` dump, with no
reference to the live dispatch state (name, status, member count) the Swarm
panel already tracks for the same call. `"Workflow"` also wasn't a
recognized tool type at all — it fell into the generic `"Other"` bucket.

Goal: replace the raw dump with a compact card showing the dispatch's live
status, reusing data the Swarm panel already fetches.

## Why ordinal matching, not an exact ID link

There is no `tool_use_id` (the transcript's own per-call id) anywhere in the
backend's `AgentDispatch`/`SubAgent` records — confirmed via code read of
`agentmux-srv/src/backend/subagent_watcher/types.rs` and a repo-wide grep.
Building an exact link would require new Rust plumbing of uncertain
feasibility (Claude Code's internal subagent id scheme is undocumented and
unverifiable from this repo — no sample subagent JSONL files were available
locally to inspect). Given that cost/risk, the chosen approach is
frontend-only: within one pane, zip its `Agent`/`Task`/`Workflow` tool-call
nodes (transcript order) against its dispatches (spawn order, reconstructed
client-side) by position, falling back to the existing generic rendering
whenever the match isn't confident. The matcher is all-or-nothing per pane,
not best-effort per node — a wrong card is worse than no card.

A correction surfaced during design: `AgentDispatch` carries no spawn
timestamp, only `last_event_at`, and both `ListDispatches`/`ListActive`
sort by `last_event_at` descending (most-recently-active first) — **not**
spawn order. Spawn order is reconstructed from `SubAgent.spawned_at`
instead (grouped by `dispatch_id`, min per group).

## What changed

**New files:**
- `frontend/app/view/agent/activity/dispatch-source.ts` — module-singleton
  fetch of `ListDispatches`, mirroring the existing `subagent-source.ts`
  pattern. Subscribes to `subagent:spawned/completed/named` +
  `dispatch:updated` (the last one needed for Workflow member-count/status
  updates, not needed by `subagent-source.ts`).
- `frontend/app/view/agent/activity/dispatch-correlation.ts` —
  `correlateDispatchesForBlock(blockId, documentNodes, subagents, dispatches)`,
  the pure ordinal-matching algorithm with an all-or-nothing confidence gate.
  Pure function (subagents/dispatches passed in, not read from atoms
  internally) for testability.
- `frontend/app/view/agent/activity/dispatch-correlation.test.ts` — 6 unit
  tests: exact match, order-independent-of-input-order (defends the
  `last_event_at` vs spawn-order correction above), count mismatch fallback,
  unorderable-dispatch fallback, cross-block isolation, no-dispatch-nodes.
- `frontend/app/view/agent/components/tool-renderers/DispatchCard.tsx` —
  the new renderer: compact card (name, running/done, member count for
  Workflow), falls back to `CompactResult` when no match. Registered at
  priority 10 via `byKind("Agent","Task","Workflow")`. Click opens the
  Swarm pane (`createBlock({ meta: { view: "swarm" } })`, the existing
  convention already used by the process badge and the "Open Swarm"
  command) — no deep-link to the specific row exists in the app, so this
  is a known, accepted limitation, not a bug.

**Modified files:**
- `types.ts` — added `"Workflow"` to `ToolNode["tool"]` union + `TOOL_ICONS`.
- `stream-parser.ts` — `extractToolDetail` and `normalizeToolName`'s
  `knownTools` both learned `"Workflow"`.
- `components/tool-renderers/registry.ts` — widened `ToolRenderer` to
  `(node, ctx?) => JSX.Element` with a new `ToolRenderContext { dispatchMatch?
  }`. Backward compatible under TS's function-assignability rules — every
  existing renderer kept compiling unchanged.
- `components/ToolOverlayLog.tsx` — added a `renderWorkflow` builtin
  (priority 0, defense-in-depth under `DispatchCard`'s priority 10),
  `renderToolResultBody` now threads `ctx` through, registered the new
  `DispatchCard` side-effect import.
- `components/CompactResult.tsx` — added a `"Workflow"` case to `summarize`
  (the fallback path `DispatchCard` itself uses when no match is found).
- `components/ToolBlock.tsx`, `ToolBlockOverlay.tsx`,
  `virtualization/DocumentRow.tsx`, `virtualization/AgentDocumentVirtualList.tsx`,
  `components/AgentDocumentView.tsx` — threaded a `dispatchMatch`
  (singular, resolved per-node) / `dispatchMatches` (the full map, an
  `Accessor`) prop down this chain, computed once via `createMemo` in
  `AgentDocumentView.tsx` where `blockId` and the node array are both
  already in scope. No SolidJS Context was introduced — this mirrors the
  existing `blockId` prop-drilling convention already used in this tree.
  Deliberately did **not** stamp this onto `ToolNode` in `stream-parser.ts`
  — that file's live-streaming/replay/dedup logic is flagged fragile by its
  own comments (several past PR regressions cited), so this stays entirely
  in the render layer.
- `components/ToolBlock.tsx`'s `resultPill()` — `Agent`/`Task`/`Workflow`
  now prefer the live `dispatchMatch` status (`running` / `N/M done` /
  `done`) over the old static `"done"`, and deliberately bypass the
  running/no-result early-return gate that every other tool's pill respects
  — the whole point is showing progress *while* the subagent/workflow is
  still running, before the parent's own `tool_use` resolves.
- `styles/_document-nodes.scss` — new `.agent-dispatch-card` styles, a
  `pill-agent-running` pill variant, and `data-tool="workflow"` color rules
  (reusing the existing `--term-bright-magenta` agent color).

## Review history — five rounds, two failed heuristics, one final policy

ReAgent and Codex both independently flagged, across five review rounds,
that same-kind parallel calls (e.g. two Agent-tool spawns issued in one
turn) could be silently mismatched by the ordinal matcher. Two attempted
fixes both turned out to be unsound on closer scrutiny:

1. A shared-`slug` "same concurrent batch" check — wrong, because that
   precedent describes multiple MEMBERS of one Task/Workflow invocation
   sharing a batch codename, not two separate solo calls (each gets its
   own dispatch_id and plausibly its own distinct slug).
2. A `ToolNode.timestamp`-gap threshold — also wrong, because that
   timestamp is the frontend's own receive-time, and a single turn can
   take many seconds to stream when the model generates a long
   description between two tool_use blocks, so a genuine same-turn pair
   can still land seconds apart at any threshold.

The actually-reliable signal (which assistant turn/message each tool_use
block came from) exists in the raw Anthropic `message.id`, one layer above
the parser, but isn't threaded through to `ToolNode` today — doing so means
touching `stream-parser.ts`, a component this codebase's own comments flag
as fragile with a cited regression history. Given two heuristics had
already failed review, expanding into that component under review pressure
was judged a worse trade than accepting a coverage loss honestly.

**Final policy:** the matcher now unconditionally bails whenever a pane has
more than one dispatch-kind tool node of the same category (`solo` —
covers `Agent`/`Task` — or `workflow`) needing relative ordering. This is
provably correct (no ordering claim is ever made among same-category
calls) at the cost of only matching when a pane has at most one live
solo-kind call and at most one live Workflow-kind call at correlation
time. Threading the real `message.id` through the streaming parser would
close this gap without the coverage cost — a legitimate follow-up, not
required to ship this safely.

## Known, accepted limitations

- **Same-category cap.** See "Review history" above — a pane with two or
  more still-live dispatch-kind tool nodes of the same category never
  gets cards for either, by design.
- **Ordinal matching, not exact.** Falls back to `CompactResult` whenever:
  a dispatch's member data has aged out of `ListActive` (no `spawned_at`
  to order by); or older transcript history not yet paginated into view.
  This was a deliberate, discussed tradeoff (see "Why ordinal matching"
  above), not an oversight.
- **No deep-link to a specific Swarm row.** Clicking a card opens/focuses
  the Swarm pane generally; no "scroll to this exact dispatch" primitive
  exists anywhere in the app today.
- **Agent History tab:** verified (not assumed) that `AgentHistoryView.tsx`
  passes a synthetic `blockId` (`"<realBlockId>:history"`) into the same
  `AgentDocumentView` chain, which never matches any real
  `parent_block_id` — so it safely always falls back to `CompactResult`
  there, with no separate wiring needed.

## Verification

- `tsc --noEmit`: clean (282 files under `frontend/app/view/agent` checked
  via `--listFiles`).
- `vitest run frontend/app/view/agent`: 1228/1228 tests pass across 96
  files, including the 6 new `dispatch-correlation.test.ts` cases.
- `vitest run frontend/app/view/swarm`: 74/74 tests pass (no regression
  from reusing `AgentDispatch`/`ActiveSubagent` types).
- `task build:frontend`: production Vite build succeeds cleanly (only
  pre-existing chunk-size warnings, unrelated to this change).

**Not done:** interactive/visual verification (triggering a real
`Agent`/`Workflow` call in a live pane and watching the card render,
clicking through to Swarm, confirming the live pill updates while a
subagent runs). This machine runs many shared, already-live AgentMux
instances (other agents' work) that shouldn't be disrupted, and this is a
native Rust+CEF desktop app rather than something screenshot-able via
browser tooling. The type/test/build coverage above is solid, but the
actual on-screen appearance and click behavior are unverified by a human or
a visual check.

## Files touched

```
New:
  frontend/app/view/agent/activity/dispatch-source.ts
  frontend/app/view/agent/activity/dispatch-source.test.ts
  frontend/app/view/agent/activity/dispatch-correlation.ts
  frontend/app/view/agent/activity/dispatch-correlation.test.ts
  frontend/app/view/agent/components/tool-renderers/DispatchCard.tsx

Modified:
  frontend/app/view/agent/types.ts
  frontend/app/view/agent/stream-parser.ts
  frontend/app/view/agent/components/tool-renderers/registry.ts
  frontend/app/view/agent/components/ToolOverlayLog.tsx
  frontend/app/view/agent/components/CompactResult.tsx
  frontend/app/view/agent/components/ToolBlock.tsx
  frontend/app/view/agent/components/ToolBlockOverlay.tsx
  frontend/app/view/agent/components/AgentDocumentView.tsx
  frontend/app/view/agent/virtualization/DocumentRow.tsx
  frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx
  frontend/app/view/agent/styles/_document-nodes.scss
```

Plan file: `C:\Users\asafe\.claude\plans\fluttering-strolling-sutherland.md`
