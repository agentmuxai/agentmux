# Spec: find/filter and sort for Armory → Memory → Personal

**Date:** 2026-09-02
**Status:** Proposed
**Motivated by:** direct request — *"we want find and filter features on the
Personal memories inside armory, similar to the my agents in agent pane."*

## Background

`SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md` (#2917) replaced the
Personal Memory tab's two `<select>` dropdowns with a card grid mirroring the
Agent pane's "My Agents", but explicitly deferred filtering as a non-goal:

> **No filter/sort bar** in v1. `AgentPickerFilterBar` exists and could be
> adopted later if the agent count makes the grid unwieldy — premature until
> the grid exists and we know how many agents actually carry memories.

The grid now exists. On this machine it already renders **33 agent cards**
(confirmed live during #2917's own testing) — well past "unwieldy," and this
is the trigger to build it.

## What "similar to My Agents" means here — and where it must diverge

`AgentPickerFilterBar.tsx` (`frontend/app/view/agent/components/`) is the
reference: a fixed text-filter input (magnifying-glass icon, clear button,
Escape-to-clear) plus a sort `<select>` at the far right, both purely
presentational — `AgentPicker.tsx` owns the signals and does the actual
filtering/sorting in a `createMemo` over `MyAgentsList`'s rows.

**Reuse the interaction pattern, not the component.** Same reasoning
`MemoryAgentCard` already applied to `AgentCard`
(`SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md` §"Do NOT reuse
`AgentCard` directly"): `AgentPickerFilterBar`'s `AgentSortOption` type
(`"recent" | "name" | "type"`) is launch-oriented — "recently launched" reads
`RecentSessionRow.started_at`, data this grid doesn't have and shouldn't
fetch just to power a sort control. A new `MemoryAgentFilterBar` component,
visually identical (same icon/input/clear-button/sort-select DOM shape and
SCSS classes renamed to `memory-agent-filter-*`), with its own sort enum.

### Sort options

| Value | Meaning | Notes |
|---|---|---|
| `name` (default) | Alphabetical by display name | Matches today's de-facto order closely enough that switching to this as default is not a visible regression for most users — `agents()` is already roughly registration order, not alphabetical, so this is a genuine (minor) improvement, not a no-op. |
| `count` | Most files first | See ordering rule below for `loading`/`error` states. |
| `provider` | Grouped by provider, then name within each group | Cheap — `AgentDefinition.provider` is already loaded with the list; no extra fetch. |

Explicitly **not** offering `AgentPickerFilterBar`'s `"recent"` (most
recently launched) — this tab has no launch-recency data and fetching it
solely to power a sort control would be exactly the kind of scope creep
`SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md` deferred. A
*"most recently modified memory"* sort is a natural future option once file
mtimes are available per-card (see Non-goals) — noted, not built now.

**Ordering rule for `count` sort, since counts resolve asynchronously and
can error:** resolved counts sort by file count descending; `loading` and
`error` cards sort **after every resolved count**, in that relative order
(loading before error), each group alphabetical by name internally. Rationale:
a sort control's job is to surface the most useful cards first — an agent
whose count isn't known yet (or failed) is never the "most files" answer, so
burying it at the bottom under a numeric sort is correct, not a bug. This
mirrors why the four-state design (`SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md`)
keeps `error` and `count: 0` visually distinct even here: an error-sorted-last
card is still labeled "Couldn't read memories" on the card itself, never
silently conflated with a genuinely empty one.

### Filter

Case-insensitive substring match against the agent's display name
(`agent.name || agent.slug || agent.id`, matching `MemoryAgentCard`'s own
existing name-resolution fallback chain), narrowing the grid client-side.

**Unlike `AgentPickerFilterBar`, this is pure client-side filtering over an
already-fully-loaded list** — `useAgentDefinitions()` returns the complete
set up front (no pagination, no backend `identity_id`-style RPC filter to
mirror). No debounce needed; `agents()` is typically well under a few hundred
rows.

### Optional: "only agents with memories" toggle

A checkbox/toggle narrowing to cards whose resolved count is `> 0` — genuinely
useful here in a way it isn't for My Agents (every agent in that list is
inherently "yours to launch"; here, most agents on a given machine will
plausibly have **zero** memories, based on this session's own live test where
the vast majority of the 33 rendered cards read "No memories yet"). Included
as an explicit **v1 feature**, not deferred, given that live evidence — but
scoped narrowly: a single checkbox, no combination with the sort options
beyond straightforward AND-filtering, and it only ever hides `count: 0`
cards — never `loading` or `error` ones, so a slow/failing count fetch can't
make a card vanish out from under a user mid-load.

## Design

```
┌─ Memory ▸ Personal ────────────────────────────────────┐
│  🔍 Filter agents...              ☐ Has memories  Sort ▾│  ← MemoryAgentFilterBar
│  [Manoz  5 files]  [AgentY  2 files]  [Lazo  —]         │  ← grid, filtered/sorted
│  [Nark   1 file ]  [Posa   —       ]                    │
└──────────────────────────────────────────────────────────┘
```

- `MemoryAgentFilterBar` (new, `frontend/app/view/native-memory/`): text
  input + "Has memories" checkbox + sort `<select>`. Purely presentational,
  same as its Agent-pane counterpart — no data access of its own.
- `NativeMemoryManager.tsx` owns three new signals (`nameFilter`,
  `onlyWithMemories`, `sortBy`) and a `createMemo` producing the filtered +
  sorted card list from `agents()` and `counts()` (both already in scope
  today). The existing per-card count-fetch effect (keyed on the agent-ID
  *set*, per #2917's own ReAgent-fixed design) is untouched — filtering/
  sorting is a pure view over its output, never a reason to refetch.
- Bar renders only in the grid view, not the per-agent detail view (matches
  `AgentPickerFilterBar`'s scope: narrows the list you pick FROM, not
  anything past that point).
- Empty-result state (filter matches nothing): a distinct message
  ("No agents match \"{query}\"") — not the existing "No agents defined yet."
  string, which specifically means zero agents exist at all. Collapsing the
  two would misreport a working filter as a broken/empty app state.

## Persistence

Mirror `AgentPickerFilterBar`'s precedent exactly:  sort choice persisted to
`localStorage` (key `nativeMemory:sortBy`, analogous to
`agentPicker:sortBy`), survives remount/reopen. The "has memories" toggle and
the text filter are **not** persisted — same convention as
`AgentPicker.tsx`'s own `filterQuery` (session-only signal, cleared on
remount): a stale hidden-by-filter grid on next open is more surprising than
a cleared filter.

## Tests

- Filtering narrows the grid to matching names only; case-insensitive.
- Clearing the filter (button and Escape) restores the full grid.
- Each sort mode produces the documented order, including the `loading`/
  `error`-sorts-last rule for `count` sort.
- "Has memories" hides `count: 0` cards but never `loading` or `error` ones.
- Sort choice persists across a remount (localStorage); filter text and the
  toggle do not.
- Filtering to zero matches renders the "No agents match" message, distinct
  from the zero-agents-total empty state.
- The filter bar does not render in the per-agent detail view.

## Non-goals

- **No backend/RPC changes.** Filtering and sorting are purely client-side
  over data already fetched (`agents()`, `counts()`).
- **No "most recently modified memory" sort.** Would require fetching a max
  file-mtime per agent — a new data dependency the count-fetch effect
  doesn't have today. Worth a future spec if requested; not bundled here to
  keep this change scoped to what was asked.
- **No changes to `MemoryAgentCard.tsx`'s own rendering** — filtering/sorting
  operates on the list fed into the grid's `<For>`, not the card component.
- **No shared component extraction** between `AgentPickerFilterBar` and the
  new `MemoryAgentFilterBar` in v1 — same reuse-vs-mirror reasoning as
  `MemoryAgentCard` vs. `AgentCard`; revisit if they drift apart.
