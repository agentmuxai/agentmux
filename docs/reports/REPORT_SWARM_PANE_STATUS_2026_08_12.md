# REPORT — Swarm Pane Status & Pending Specs

**Date:** 2026-08-12
**Status:** Audit only — no code changed. Written to establish a baseline before refining Swarm pane behavior.
**Scope:** `frontend/app/view/swarm/{swarm-model.ts,swarm-view.tsx,swarm.tsx}` and every `docs/specs/`, `docs/retro/`, `docs/reports/` file touching "swarm."
**Trigger:** Local `agentmux` clone was 21 commits behind `origin/main`; fast-forwarded to `2ee8f9f36` before this audit. Cross-referenced each swarm spec's own "Status:" line against actual current source, since several were stale.

---

## 1. Current implementation snapshot

The Swarm pane renders a two-level, collapsible tree bucketed by dispatch kind (Agent Tool / Workflow / Shell / Cron), built on the `AgentDispatch` + `SubAgent` model from `SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17` (`ListDispatches`, `dispatch:updated`, `buildDispatchBuckets` in `swarm-model.ts`). Recent commits actually on `main`:

- `af64ed43d` — 60s auto-retire countdown on finished rows (#2440)
- `ce4c60e62` — stop rendering a phantom "Agent" row for a block that doesn't exist (#2438)
- `678c74279` — drop redundant agent-id tag on solo feeds, colorize ANSI in activity feed (#2454)
- `44638297c` — Swarm copy menu / agent-pane paste menu context-menu fix (#2457)
- `f3790992f` — dead-code + docs cleanup sweep (#2407)

## 2. Specs — resolved (code matches, doc status may be stale)

| Spec | Verdict |
|---|---|
| `SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06` | **Done.** `AUTO_RETIRE_DELAY_MS`, `_countdownState`, `useTick(1000)`-driven "disappearing in Ns" — landed as #2440. Doc still says "PR pending"; stale. |
| `RETRO_SWARM_PHANTOM_ROWS_AND_STALE_TRACKING_2026_08_06` | **Done.** `hasRenderableBlock()` implements the retro's proposed fix; shipped as #2438. Server-side cleanup (the retro's second proposed step) not independently verified. |
| `SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17` | **Done.** Doc says "Proposed — no implementation," but `AgentDispatch`/`buildDispatchBuckets`/`dispatch:updated` are all live in `swarm-model.ts`. Doc status is stale. |
| `SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12` | **Superseded/shipped** by `SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20` Phase A (#2234) — `subagent:abandoned` handler and `"abandoned"` status are wired in. |

## 3. Specs — superseded (no action needed, historical only)

- `SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05` — its PR-1 scope (`workflow_id`/`ListWorkflows`) was fully replaced by the `AgentDispatch` model. PR-2 scope (periodic `agent:summary` haiku pushes) was never built and isn't referenced anywhere else either — if still wanted, it needs a fresh spec, not a resurrection of this one.
- `SPEC_SWARM_LIVE_FEED_UI_2026_07_05` — proposed a flattened, virtualized single-row-list with ring-buffered stream lines; superseded in direction by the bucketed-tree + concatenated-activity-feed approach that shipped instead.
- `SPEC_SWARM_TREE_REDESIGN_2026_06_19` — proposed a simple always-expanded 2-level tree; superseded by the collapsible 4-bucket tree that actually shipped.
- `swarm-redesign-active-retired-2026-05-03` — proposed a 5-tab/3D-flip layout; open questions (Search survival, offline agents, cross-session Retired) were never resolved and the design was abandoned in favor of the tree-based chain above.

## 4. Genuinely outstanding — candidates for the "refine swarm pane" pass

1. **Attached-task / background-process chip not surfaced in Swarm pane.** `SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02` §6 item 4 — confirmed still not started, no `attachedTask`/muxspect references in `swarm-model.ts` or `swarm-view.tsx`. This is the same "what's this row actually doing right now" gap flagged back in `REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16` items 3–4, so it's been sitting open across two spec generations.
2. **`run_in_background` flag not modeled as first-class in Swarm.** Shell/Cron buckets exist, but the specific background-flag surfacing from the consolidation report's item 2 is only partially done.
3. **Live haiku-summary pushes for in-progress dispatches (`agent:summary`)** — proposed in `SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05` PR-2, never built, never re-specced. Worth deciding explicitly whether this is still wanted before scoping new work, since it's the one piece of that old spec with no living replacement.
4. **Backend-side stale-tracking cleanup** from `RETRO_SWARM_PHANTOM_ROWS_AND_STALE_TRACKING_2026_08_06`'s second proposed step — the frontend symptom is fixed (#2438), but whether the underlying server-side leak was also addressed hasn't been independently verified.

## 5. Recommendation

Items 1–2 in §4 (attached-task/background-process visibility) are the most concrete, already-specced gap and the natural next target — they're referenced by two independent specs six weeks apart, meaning the need has recurred rather than being a one-off ask. Item 3 needs a product decision (still wanted or not) before any implementation spec is worth writing. Item 4 is a quick verification task, not a design task.
