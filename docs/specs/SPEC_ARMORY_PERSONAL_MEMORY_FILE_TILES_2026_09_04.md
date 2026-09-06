# Spec: Armory → Memory → Personal — file tiles, not a dropdown

**Date:** 2026-09-04
**Status:** Proposed
**Motivated by:** direct request — *"we want the grid layout extended to more
screens. The individual file view should be tiles, not a dropdown."*

## Problem

[`SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md`](./SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md)
replaced the **agent** `<select>` with a card grid and explicitly left the
**file** `<select>` in place ("Non-goals: the file picker stays a dropdown for
now"). That half-migration is the thing this spec finishes.

The remaining dropdown has the same three problems the agent one had:

- **No overview.** The `<option>` list carries a bare filename and nothing
  else, so "which of these is worth opening?" can only be answered by opening
  each in turn — even though `agent:memory:list` already returns `is_index`,
  `metadata_type`, `size_bytes` and `modified_at` for every file. The data was
  fetched and then thrown away at render.
- **Inconsistent within its own flow.** The drill-down now reads
  grid → *dropdown* → list. The file step is the only one that isn't a
  scannable surface, sitting between two that are.
- **The index file is invisible.** `MEMORY.md` (the file loaded into every
  session) sorts first only by the accident of uppercase preceding lowercase in
  ASCII. Rename the index and it lands mid-list with nothing marking it.

## Design

Promote the file picker to its own **screen** in the drill-down, rendered as a
tile grid with the same metrics as the agent grid above it:

```
agents grid  ──click──▶  FILE TILES  ──click──▶  version history
     ▲                        │                        │
     └──── "← All agents" ────┘                        │
                              ▲──── "← All files" ─────┘
```

```
┌─ Memory ▸ Personal ─────────────────────────────────────┐
│ ← All agents  ·  clare-06219                            │
├─────────────────────────────────────────────────────────┤
│ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐      │
│ │ ▤ MEMORY.md  │ │ ▤ feedback_… │ │ ▤ project_…  │      │
│ │ [INDEX]      │ │ [FEEDBACK]   │ │ [PROJECT]    │      │
│ │ 439 B · 2d   │ │ 1.2 KB · 3d  │ │ 2.1 KB · 3d  │      │
│ └──────────────┘ └──────────────┘ └──────────────┘      │
└─────────────────────────────────────────────────────────┘
```

### Back walks one level, not all the way out

The back button is the only exit from a detail screen, so it steps back exactly
one level — `← All files` from the history screen, `← All agents` from the file
grid. Making the file grid a screen you could only leave by returning to the
agent grid would make it cheaper to *avoid* the new screen than to use it.

The header doubles as the breadcrumb: agent name always, `· <filename>` only
once a file is open.

### Index file pinned first

Sort is `is_index` first, then `localeCompare` on the filename. The index is
the file the agent loads every session; its position is worth stating rather
than leaving to collation (see the third bullet under Problem). It also carries
an `index` badge and a left rule.

### Do NOT reuse `MemoryAgentCard`

Same reasoning that spec gave for not reusing `AgentCard`: the two tiles carry
different payloads (`MemoryCountState` + provider logo vs. file metadata), and
sharing one component would let a change to either grid silently alter the
other. They share a visual language via SCSS, not a component.

### Loading is not empty, and empty is not an error

The file grid keeps the same three-state care `MemoryAgentCard` takes with its
per-agent count — an unresolved list renders `Loading files…`, a resolved empty
one renders "This agent hasn't remembered anything yet.", and a failed
`agent:memory:list` keeps surfacing its real error text. Collapsing the first
or third into the second is exactly how #2901 (blank `working_directory` → hard
HTTP 500) stayed invisible behind a plausible empty state.

### Narrow panes

Both grids collapse to a single column under `@container armory (max-width:
359px)`. Panes go down to 128px (`MinNodeSizePx`), where the existing
`minmax(200px, 1fr)` track overflows its own container — a pre-existing bug in
the agent grid that this change fixes for both.

## Files

| File | Change |
|---|---|
| `frontend/app/view/native-memory/MemoryFileCard.tsx` | **new** — the tile + its label helpers |
| `frontend/app/view/native-memory/native-memory-manager.tsx` | detail view split into two screens; `sortedFiles` memo; one-level-back button + breadcrumb |
| `frontend/app/view/native-memory/native-memory-manager.scss` | file grid + tile styles, breadcrumb, narrow-pane column collapse; dead `.native-memory-manager-field` (the `<select>`'s styles) removed |
| `frontend/app/view/native-memory/native-memory-manager.test.tsx` | 6 combobox-driven tests migrated to tiles; 10 new |

## Tests

Beyond the migrated six, the new cases are: one tile per file with no combobox
left in the DOM; a tile rendering `metadata_type` + size + age; the index
pinned first *from a last-alphabetical name* (so passing can only be the
`is_index` rule); Enter-key activation; back stepping one level at a time;
the filename appearing in the header only on the history screen; the empty
state; and the label helpers' edge cases (0 bytes, missing timestamp — a
missing `modified_at` must not render as 1970).

## Non-goals

- **The version history stays a list.** It is already a scannable surface with
  per-row selection for diffing; tiles would cost that without adding anything.
- **No filter/sort bar on the file grid.** The agent grid got one at 30+ cards
  ([`SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md`](./SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md));
  a single agent's memory directory is nowhere near that. Add it when a real
  directory makes it necessary, not before.
- **No file content preview on the tile.** `agent:memory:list` returns metadata
  only; previewing would need a new RPC per tile.
