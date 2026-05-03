# Swarm Pane Redesign — Active / Retired + Pane-Flip Detail

**Date:** 2026-05-03
**Status:** Spec — pending product confirmation on open questions
**Scope:** `frontend/app/view/swarm/`

## Goal

Replace the current 5-tab swarm UI (Overview / Activity / Instances / History / Search) with a **2-tab** layout (Active / Retired). Each tab's body takes the full pane width (single column, not side-by-side). Clicking an entry flips the pane to a detail face; while flipped, the **top tab bar transforms into a back button** that returns to the list.

## Current state

`swarm-view.tsx` renders 5 tabs in a header toolbar; each tab swaps the body via `Show when={model.tabAtom() === ...}`. The data sources backing each tab are:

| Tab | Source | Type |
|---|---|---|
| Overview | `agentsAtom` | `AgentOverview[]` (id, name, status, session_count, total_tokens, last_active_at) |
| Activity | per-block process tracker | `AgentProcessGroup[]` (block_id → processes) |
| Instances | live subagent registry | `ActiveSubagent[]` (parent_agent, slug, status, last_event_at, event_count) |
| History | session files on disk | `HistorySessionMeta[]` (session_id, slug, modified_at, message_count) |
| Search | full-text over event log | `SearchResult[]` |

Today the swarm pane is widget-launchable via `defwidget@swarm` (CLAUDE.md), hidden by default.

## Target

### Two tabs (replaces five)

```
┌─────────────────────────────────────────────────────────┐
│  [Active]  [Retired]                                    │   ← header tab bar
├─────────────────────────────────────────────────────────┤
│  ▣ AgentA                                  • 2m ago     │
│  ▣ AgentB                                  • 8m ago     │
│  ▸ research-helper · parent AgentA         • 30s ago    │
│  ▸ doc-writer · parent AgentB              • 1m ago     │
└─────────────────────────────────────────────────────────┘
```

**Active tab** rows (full-width list, not split into columns):
1. **Agents** — `AgentOverview` rows where `status ∈ {"active", "idle"}` (i.e. not `"offline"`)
2. **Actively operating subagents** — `ActiveSubagent` rows where `status === "active"`

Visual distinction: agents render with a filled square glyph; subagents with a triangle glyph + "parent: X" subtitle (suggested icons; final pick during impl).

Sort: most-recent-activity-first across the merged list (`max(last_active_at, last_event_at)`).

**Retired tab** rows:
- `ActiveSubagent` rows where `status === "completed"`

Sort: most-recent-completion first (use `last_event_at` as proxy if there's no explicit completed_at).

### Pane-flip detail view

Clicking any row flips the entire pane (CSS 3D rotateY) to show a detail face for that entry. **While flipped, the header replaces the tab bar with a back button** that flips back to the list:

```
┌─────────────────────────────────────────────────────────┐
│  ← research-helper                                      │   ← header in detail mode
├─────────────────────────────────────────────────────────┤
│                                                         │
│  parent agent:   AgentA                                 │
│  session:        s_8f3c…                                │
│  model:          claude-opus-4-7                        │
│  events:         42                                     │
│  last event:     30s ago                                │
│                                                         │
│  [ Open subagent pane ]                                 │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

Front face: the active or retired list (whichever tab was selected pre-flip).
Back face: detail content driven by the selected entry.

The header is **part of the flipping container**, not a fixed element above it: the tabs are on the front face of the header, the back button is on the back face. So a single `selectedEntry` signal flips the entire pane (header + body) atomically.

**Detail content by entry type:**

| Entry type | Detail face contents |
|---|---|
| Agent | identity (name, status, model), session count, total tokens, last-active-at; primary action: "Jump to pane" (find existing block; or prompt to launch if not open) |
| Active subagent | parent agent, slug, session id, model, event count, last event time; primary action: "Open subagent pane" (existing `openSubagentPane`) |
| Retired subagent | same as active subagent + completion time; primary action: "View conversation" — embeds the existing `SwarmConversationViewer` inline on the detail face |

### Flip animation

- Container `.swarm-flip` has `position: relative`, `perspective: 1200px`, `transform-style: preserve-3d`
- Both `.swarm-face--front` and `.swarm-face--back` are absolutely positioned, full-size, with `backface-visibility: hidden`
- Front: default orientation; back: `transform: rotateY(180deg)`
- Flipping the parent applies `transform: rotateY(180deg)` for 250ms ease-in-out
- A single `selectedEntry: Accessor<Entry | null>` drives the flip: non-null → flipped, null → list

## Data wiring

No new backend RPC needed for v1 — all required data is already in `SwarmViewModel`:

- `agentsAtom: AgentOverview[]` (already loaded)
- `subagents` source — verify whether `SwarmViewModel` exposes `ActiveSubagent[]` directly or whether it lives in the Instances tab's local state. If local, lift it into the model as `subagentsAtom`.
- `loadHistory()` already populates `historyAtom: HistorySessionMeta[]` when needed for the retired-detail "View conversation" action

Two new derived accessors on `SwarmViewModel`:

```ts
activeEntries: Accessor<Entry[]>   // agents (status != offline) + active subagents, sorted by recency
retiredEntries: Accessor<Entry[]>  // subagents where status === "completed", sorted by recency
selectedEntry: Accessor<Entry | null>  // drives the flip
setSelectedEntry: Setter<Entry | null>
```

Where `Entry` is a tagged union:
```ts
type Entry =
    | { kind: "agent"; data: AgentOverview }
    | { kind: "subagent"; data: ActiveSubagent };
```

## What changes / gets removed

- `SwarmTab` type narrowed to `"active" | "retired"` (was 5-way)
- `tabAtom` / `setTab` retained — still drives which list is rendered on the front face
- Header `.swarm-tabs` + `.swarm-tab` styles retained — just two buttons instead of five
- Five sub-view functions removed: `SwarmOverview`, `SwarmActivity`, `SwarmInstances`, `SwarmHistory`, `SwarmSearch` (some pieces — `SwarmConversationViewer`, `SwarmInstanceRow` — get reused on detail face)
- `SwarmSearch` / search-result code path (full removal, see open question 1)
- `SwarmActivity` / per-block-process-tracker rendering (see open question 2)

What stays:
- `SwarmViewModel.refreshActivity()` / `refreshInstances()` / `loadHistory()` — repurposed as data refreshers feeding the new derived accessors
- `SwarmConversationViewer` — embedded on retired-subagent detail face

## Open questions (need product call before implementing)

1. **Search**: is the full-text Search tab dropped entirely, or does it move to a global control (e.g. Ctrl+F over the swarm content)?
2. **Process-tracker Activity data**: does the concept of "per-block process tracker grouped by agent" survive at all, or is the new model strictly agent + subagent centric? If it survives, where does it surface?
3. **Offline agents**: spec assumes Retired = exited subagents only. Are offline agents shown anywhere, or filtered out entirely?
4. **History coverage**: does Retired include only subagents that exited *this session*, or persisted history from prior sessions too? Does clicking a retired subagent ever load from disk via `historyAtom`?
5. **Empty states**: text for empty Active column ("No active agents — launch one from the widget bar"?) and empty Retired column ("No completed subagents yet"?)

## Implementation order

1. Lift subagent state to `SwarmViewModel` (if not already there) so it's first-class alongside `agentsAtom`.
2. Add the two derived accessors + `selectedEntry` signal.
3. Narrow `SwarmTab` to `"active" | "retired"`; replace the 5 tab buttons with 2; rebuild the body to render either active or retired list (full-width, single column) based on the active tab.
4. Build `SwarmEntryRow` (used by both tabs; visual variant by entry kind).
5. Build `SwarmDetailView` with the entry-kind switch.
6. Wrap **header + body together** in `.swarm-flip` 3D-transform container. The front face has the tab bar + list; the back face has `← {entry name}` + detail. Drive flip from `selectedEntry()`.
7. Delete dead tab code + tab/old-tab SCSS.
8. Smoke (in `task dev`):
   - select-back-select cycle on each entry kind
   - rapid clicks (no animation interleave)
   - swarm pane still launches from widget bar
   - per-pane zoom (just-added) still works on both faces
   - flipped state survives a hot-reload cleanly

## Out of scope (v1)

- Server-side schema changes
- New backend RPCs
- Search (pending question 1)
- Multi-select / bulk actions on entries
- Drag-and-drop or reorder
- Keyboard navigation of the list (arrow keys, enter to flip) — nice-to-have for v1.1
