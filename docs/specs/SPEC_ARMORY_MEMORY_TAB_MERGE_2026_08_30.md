# Spec: Armory rail — merge "Global Memory" + "Personal Memory" into one "Memory" tab

**Date:** 2026-08-30
**Status:** Proposed
**Motivated by:** direct request — collapse the two adjacent memory-scoped
rail tabs into a single "Memory" tab, with Global and Personal as sections
inside it, using the brain icon for the combined tab.

## Background

`docs/specs/SPEC_ARMORY_MEMORY_GLOBAL_PERSONAL_RENAME_2026_08_22.md` split
one "Memory" concept into two adjacent rail tabs — "Global Memory" (`memory`
id, `GlobalBrainManager`) and "Personal Memory" (`native_memory` id,
`NativeMemoryManager`) — to make each tab's scope legible at a glance. That
spec explicitly kept them as two separate rail entries; this spec undoes
that specific choice (one tab, not two) while keeping everything else from
08-22 intact: the two backing systems stay separate under the hood, and
`ABF` is unaffected.

Current rail (`frontend/app/view/armory/armory-view.tsx` `RAIL`):

```
Accounts, Global Memory, Personal Memory, Skills, MCP Servers, ABF
```

## Design

### Rail

Collapse the two memory entries into one:

```
Accounts, Memory, Skills, MCP Servers, ABF
```

- New single rail entry: `{ id: "memory", label: "Memory", icon: "brain" }`
  — reuses the `memory` id and the `brain` icon `Global Memory` already
  has today, so no icon or id churn for the common case.
- `native_memory` is removed as a **rail** entry (no separate button in
  either `.bundle-manager-tab-bar` or `.bundle-manager-rail`).

### Inside the Memory pane

The Memory pane content gets its own small sub-nav — two segments, "Global"
and "Personal" — rendered above the existing manager component, same
pattern as the outer rail but scoped to this one pane:

```
┌─ Memory ──────────────────────────────┐
│  [ Global ] [ Personal ]              │  ← new sub-nav, this pane only
│  ────────────────────────────────────  │
│  <GlobalBrainManager /> or             │
│  <NativeMemoryManager />               │
└────────────────────────────────────────┘
```

Both `GlobalBrainManager` and `NativeMemoryManager` are reused as-is —
this is a wrapping/nesting change, not a rewrite of either component.

### Persistence

- Outer tab selection stays on `block.meta["armory:section"]`, same key,
  now with `native_memory` no longer a selectable value going forward.
- New key `block.meta["armory:memory:subsection"]`, values `"global" |
  "personal"`, default `"global"` — same `useBlockAtom` +
  `RpcApi.SetMetaCommand` pattern `sectionAtom`/`zoomAtom` already use in
  `armory-model.ts`, so the sub-tab also survives a block remount.

### Backward compatibility for existing persisted meta

Users with `armory:section: "native_memory"` already saved (from before
this change) must not silently land on `"accounts"` the way an unrecognized
value does today (`isArmorySection` in `armory-model.ts`) — that would read
as the tab vanishing. Instead, normalize at read time:

- `armory:section === "native_memory"` → treated as `"memory"` **and**
  seeds `armory:memory:subsection` to `"personal"` (so a user who had
  Personal Memory open lands on Memory → Personal, not Memory → Global).
- This normalization is read-only / in-memory (derived in `sectionAtom`'s
  `createMemo`, same place the existing fallback-to-`"accounts"` logic
  lives) — it does not need to rewrite the stored meta value.
- `ArmorySection`'s type keeps `"native_memory"` as a recognized-but-
  non-rail value purely so `isArmorySection` still accepts old persisted
  data instead of rejecting it into the `"accounts"` fallback; `RAIL` and
  `ARMORY_SECTION_LABELS`'s user-facing entries drop it.

### Labels

`ARMORY_SECTION_LABELS` (`armory-model.ts`):

- `memory: "Global Memory"` → `memory: "Memory"`
- `native_memory: "Personal Memory"` entry removed from the *rail-facing*
  label map (kept only internally for the normalization above, not
  rendered anywhere).

`viewName` (pane title) shows `"Memory"` regardless of which sub-tab is
active — same level of granularity `ABF` already gets (no sub-scope in the
pane title), keeping this consistent rather than introducing a new
"Memory — Personal" pattern nowhere else in Armory uses.

## Files to change

- `frontend/app/view/armory/armory-model.ts` — `ArmorySection` type,
  `ARMORY_SECTION_LABELS`, `isArmorySection`/`sectionAtom` normalization
  for legacy `native_memory` meta, new `memorySubsectionAtom`.
- `frontend/app/view/armory/armory-view.tsx` — `RAIL` array (one entry
  instead of two), Memory pane now renders a sub-nav + conditionally
  `GlobalBrainManager`/`NativeMemoryManager` instead of each having its own
  top-level `.bundle-manager-pane`.
- `frontend/app/view/armory/armory-view.scss` — small sub-nav style (can
  likely reuse `.bundle-manager-tab-bar`/`-rail` button styles rather than
  inventing new ones).
- `frontend/app/view/armory/armory-view.test.tsx` — update rail-order and
  label assertions (08-22's tests asserted exactly the two-tab shape this
  spec removes); add coverage for the sub-nav and the legacy
  `native_memory` → Memory/Personal normalization.

## Non-goals

- **No data-model change.** `GlobalBrainManager` (`is_global` Memory
  bundles) and `NativeMemoryManager` (native memory files) remain two
  separate backing systems — same non-goal 08-22 already established.
  This spec only changes how they're grouped in the rail.
- No change to `ABF`.
- No change to either manager component's internals beyond being mounted
  under the new sub-nav instead of directly under the outer rail.
- No change to write access (both sub-sections stay writable by human and
  agent, as 08-22 confirmed).
