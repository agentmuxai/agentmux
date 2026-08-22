# Spec: Armory rail — "Global Memory" / "Personal Memory" rename + reposition

**Date:** 2026-08-22
**Author:** Camper
**Status:** Implemented
**Motivated by:** direct request — abstract away, for the user, that the
Armory currently exposes two structurally different backing systems under
confusing names; present a single "Memory" concept split by scope instead.

## Problem

The Armory rail had three memory-adjacent tabs with names that don't
describe what they actually are, discovered while investigating this
request:

| Rail id | Old label | What it actually is |
|---|---|---|
| `memory` | "Memories" | `GlobalBrainManager` — workspace-wide sections (`is_global` Memory bundles) composed into every agent's `CLAUDE.md` at launch |
| `native_memory` | "Native Memory" | Per-agent version history over the harness's own native memory files (currently Claude Code's `MEMORY.md`/topic files) |
| `bundles` | "ABF" | The actual Armory Bundle Format CRUD — an agent's capability stack (instructions, context files, MCP servers, skills, provider/model) |

"Memories" and "ABF" both being memory-shaped-but-different, plus a third
"Native Memory" tab at the opposite end of the rail, made the distinction
illegible to a user who hasn't read the specs behind each one.

## Design

Two tabs renamed to reflect the one thing users actually need to know —
scope (workspace-wide vs. per-agent) — not which backend system answers
it:

- `memory` (`GlobalBrainManager`): **"Memories" → "Global Memory"**
- `native_memory` (native memory history): **"Native Memory" → "Personal Memory"**

Repositioned so both sit together, directly below Accounts:

```
Before: Accounts, Memories, Skills, MCP Servers, ABF, Native Memory
After:  Accounts, Global Memory, Personal Memory, Skills, MCP Servers, ABF
```

**`ABF` is explicitly out of scope** (confirmed directly) — it keeps its
current tab, label, and position. This is a rename + reorder only, not a
data-model unification: `Global Memory` and `Personal Memory` remain two
separate backing systems under the hood (`is_global` Memory bundles vs.
native memory files) — see
`docs/reports/REPORT_ARMORY_STASH_MEMORY_SYNC_STATUS_2026_08_07.md` and
`docs/specs/archive/SPEC_MEMORY_IDENTITY_ARCH_2026_06_19.md` for why that
separation itself stays as-is. Only the section ids stay stable
internally (`memory`, `native_memory`) — they're persisted in
`block.meta["armory:section"]`; renaming the *label* doesn't touch a
user's already-selected tab, renaming the *id* would have stranded it
back to "accounts."

**Write access:** confirmed both Global Memory and Personal Memory are
(and stay) writable by both the human and the agent — this rename doesn't
reintroduce or imply the older "bundles are human-only, native memory is
agent-only" governance split described in the docs above. Neither
component's own UI copy claimed otherwise; nothing needed to change on
that front.

## Files changed

- `frontend/app/view/armory/armory-model.ts` — `ARMORY_SECTION_LABELS`.
- `frontend/app/view/armory/armory-view.tsx` — `RAIL` array order, plus a
  short tooltip on each memory tab clarifying scope (workspace-wide vs.
  per-agent) at a glance.
- `frontend/app/view/armory/armory-view.test.tsx` — updated the two
  existing label/order assertions, added one new label assertion for the
  Personal Memory rename (the prior test suite only ever asserted the
  Global Memory side of a rename, since "Native Memory" had never been
  renamed before).

## Non-goals

- No change to `ABF`.
- No data-model change — `Global Memory` and `Personal Memory` are not
  merged, bridged, or given a new shared table. Purely a rail-level
  rename + reorder.
- No change to either pane's own component (`GlobalBrainManager`,
  `NativeMemoryManager`) beyond what the rename required (none — neither
  had the old label as literal text in its own JSX).
