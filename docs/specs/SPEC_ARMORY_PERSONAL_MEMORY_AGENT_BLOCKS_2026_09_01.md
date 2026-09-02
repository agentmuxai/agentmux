# Spec: Armory → Memory → Personal — browse by agent block, not a dropdown

**Date:** 2026-09-01
**Status:** Proposed
**Motivated by:** direct request — *"in the Personal section of armory, list the
agents out as blocks, similar to the my agents in the agent pane. when selecting
one, that's when you can go into the memory."*

## Problem

`NativeMemoryManager` (the Personal tab) currently opens on **two stacked
`<select>` dropdowns** — "Select an agent…" then "Select a file…" — with an
empty body until both are chosen. Three things are wrong with that:

- **No overview.** You cannot see which agents even *have* memories without
  selecting each one in turn. The tab opens on nothing.
- **Inconsistent with the rest of the app.** The Agent pane presents the same
  set of entities as a scannable card grid ("My Agents", `MyAgentsList` /
  `AgentCard`). Personal Memory is the only place agents appear as a dropdown.
- **Two decisions before any content.** Agent *and* file must both be picked
  before anything renders, so the common case ("what did this agent remember?")
  costs two interactions and shows nothing in between.

## Design

Replace the agent `<select>` with a **card grid**, mirroring My Agents' visual
language. Two states in one pane:

```
┌─ Memory ▸ Personal ──────────────────────────────┐
│  [Manoz  5 files]  [AgentY  2 files]  [Lazo  —]  │   ← grid (default)
│  [Nark   1 file ]  [Posa   —       ]             │
└──────────────────────────────────────────────────┘
        │ click a card
        ▼
┌─ Memory ▸ Personal ▸ Manoz ──────── [← All agents] ┐
│  File: [MEMORY.md ▾]                               │
│  <NativeMemoryHistoryPanel …/>                     │
└────────────────────────────────────────────────────┘
```

- **Grid (default).** One card per agent definition, from the existing
  `useAgentDefinitions()` hook the tab already uses.
- **Detail.** Clicking a card replaces the grid with that agent's memory view:
  the existing file `<select>` + `NativeMemoryHistoryPanel`, unchanged, plus a
  back affordance. Drill-in, not a third dropdown.
- **Selection state** lives in a signal, same as today's `selectedAgentId` —
  no new persistence. Deliberately *not* meta-backed: unlike the Armory rail's
  own section, a half-finished drill-in isn't worth restoring across a remount.

### Do NOT reuse `AgentCard` directly

`AgentCard`'s props are `launching`, `disabled`, `installed`, `onLaunch`, plus
Option E's session-zone "+ New" button — it is a *launcher* control, and its
click contract is "start this agent". Wiring it to "browse this agent's
memories" would drag launch semantics, install-state fetching and session-zone
logic into a read-only browser, and would make a future `AgentCard` change
silently alter this tab.

Instead add a small `MemoryAgentCard` in `frontend/app/view/native-memory/`
that **mirrors the visual language** (icon, name, provider hint, same card
metrics from the shared SCSS tokens) and carries only what this tab needs:
`agent`, `fileCount`, `state`, `onSelect`. If the two drift visually later,
that is a real prompt to extract a shared presentational shell — a follow-up,
not a prerequisite.

### Per-card memory count — and the state it must distinguish

The count is what makes the grid worth having (it answers "who has memories?"
at a glance), but it needs one `agent:memory:list` call per agent. So:

- Fetch counts **lazily and concurrently** after the grid mounts, each card
  resolving independently — never blocking the grid's first paint.
- Each card renders one of **four** states, and the distinction matters:

  | State | Card shows | Why it is its own state |
  |---|---|---|
  | loading | skeleton/spinner | count not yet known |
  | `n` files | `5 files` | the useful case |
  | zero files | `—` / "No memories yet" | a real, valid answer |
  | **error** | a small warning affordance, card still clickable | **not** the same as zero |

  Collapsing the last two is the specific trap here.
  `memory_dir_for_agent` returns a hard **HTTP 500**, not an empty list, when
  it cannot resolve — which is exactly what
  `SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md` (#2901) was about:
  every agent with a blank `working_directory` failed that way, and the tab
  had no way to say so. Rendering an error as "no memories" would hide the
  next occurrence of that class of bug behind a plausible-looking empty state.
  A failing card must look *different* from an empty one.

- Reuse the existing stale-response guard: today's `latestRequestId` pattern
  (reagent P2, PR #2678) exists because overlapping fetches can resolve out of
  order. N concurrent per-card fetches make that *more* likely, not less —
  each card owns its own request generation.

### Files

- `frontend/app/view/native-memory/native-memory-manager.tsx` — grid/detail
  split; keep the file `<select>` + `NativeMemoryHistoryPanel` verbatim in the
  detail view, including the `keyed` remount guard that panel's own doc comment
  requires.
- `frontend/app/view/native-memory/MemoryAgentCard.tsx` — new.
- `frontend/app/view/native-memory/native-memory-manager.scss` — grid layout +
  card styles, using existing design tokens (no raw hex — the repo's stylelint
  `color-no-hex` rule is enforced).

### Tests

- Grid renders one card per agent from `useAgentDefinitions()`.
- Clicking a card shows that agent's detail view; back returns to the grid.
- **A card whose count fetch rejects renders the error state, distinctly from
  a card with zero files** — the regression guard for the trap above.
- Counts resolve independently: one rejecting card does not blank the others.
- Switching agents twice quickly cannot let a stale response paint the wrong
  agent's file list (preserves PR #2678's guard through the new structure).

## Non-goals

- **No change to `NativeMemoryHistoryPanel`** or the `agent:memory:*` RPCs.
  This is a navigation/presentation change over the same data.
- **No change to the Global sub-tab**, and no change to the Memory tab's own
  Global/Personal split (`SPEC_ARMORY_MEMORY_TAB_MERGE_2026_08_30.md`).
- **No editing/deleting memories from the grid.** The tab stays read-only +
  history/diff/revert exactly as today; the grid only changes how you *reach*
  an agent.
- **No filter/sort bar** in v1. `AgentPickerFilterBar` exists and could be
  adopted later if the agent count makes the grid unwieldy — premature until
  the grid exists and we know how many agents actually carry memories.
- **No shared presentational extraction between `AgentCard` and
  `MemoryAgentCard`** in v1 — see the reuse note above; revisit if they drift.
