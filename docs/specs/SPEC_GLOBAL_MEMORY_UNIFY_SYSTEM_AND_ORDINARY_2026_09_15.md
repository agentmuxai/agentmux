# Spec: Global Memory — unify "system" and "ordinary" into one Memory list (postmortem + refactor plan)

**Status:** implemented. §2.2's open question resolved as option 1 (no
visual distinction at all — full uniformity): the user said "proceed" to a
summary that named this as an open call rather than picking a specific
option, so this was a judgment call, not an explicit confirmation — matches
"we don't need separate Section and Memory" taken literally, but flagged
here in case that reading was wrong. — #3232
**Date:** 2026-09-15
**Verified against:** code as of `c00644790` (`frontend/app/view/global-bundle/
global-bundle-manager.tsx`, `global-bundle-model.ts`) plus
`docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` (the spec that
introduced the split this document proposes partially undoing).
**Related:** `SPEC_ARMORY_GLOBAL_MEMORY_DECLUTTER_2026_09_15.md` (the
preceding pane-declutter pass this one continues), the two follow-up PRs to
it (#3227, #3232) that progressively simplified "AgentMux system entry"
wording down to "Memory" before the user asked for the underlying split
itself to go, not just its wording.

## 1. Postmortem — what we built, and why the frontend half was a mistake

### 1.1 What SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md actually asked for

Read in full before writing this document, not assumed. Its real ask (§1)
was narrow and well-justified: give AgentMux-controlled policy (the kind of
content already hand-maintained in `~/.agentmux/agents/CLAUDE.md` with
override wording, invisible to any UI) a place in the Armory that is
**structurally isolated from every generic bundle-editing surface** — so
that an agent with MCP-tool write access to ordinary Global Memory can never
touch, weaken, or delete AgentMux-level policy, even by accident (§2, "No
accidental mutation"). That's a real, still-valid safety property — see
§1.2 below for what of it this document is NOT proposing to remove.

### 1.2 What's staying — the backend isolation was correct and isn't the problem

- `db_bundles.is_system`, the two storage methods
  (`bundle_memory_upsert`/`bundle_memory_upsert_system`) with their mutual
  guards, the two delete methods, the reorder guard, and the two RPC
  commands (`upsertsystemmemory`/`deletesystemmemory`, never wired to any
  MCP tool) all still do real work: they are the only thing standing between
  an agent's own generic bundle-write access and AgentMux's own override
  policy. **None of that is in scope to remove here.**
- The composed-file behavior — system entries always sort first, wrapped in
  the "IMPORTANT... OVERRIDE... MUST follow" preamble — is also unaffected.
  It's real, working prompt-priority machinery or GlobalBundleManager (the
  agent actually reads and follows it), not UI decoration.

### 1.3 What went wrong — the frontend mirrored the backend split 1:1, and that was the mistake

`SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` §3.5 asked the frontend to
mirror the backend's isolation exactly: a separate `editingSystemIdAtom`/
`draftSystemNameAtom`/`draftSystemInstructionsAtom` signal triple (parallel
to the ordinary `editingIdAtom`/`draftNameAtom`/`draftInstructionsAtom`
triple), a whole separate `SystemSectionEditor` component (a near-duplicate
of `SectionEditor`), a separate pinned list region with its own empty-state
and its own "+ Add" affordance, and a distinct badge so "a human editing
ordinary Global Memory never confuses the two."

That reasoning made sense for a **security-isolated write path** — you
don't want an ordinary bundle-editing code path to be even structurally
capable of hitting the system RPC by accident. It did **not** need to
extend to the **read/browse/edit-UI layer**. A human looking at Global
Memory doesn't need two separate list regions, two separate empty-states,
two separate "+" buttons, and two words ("Section" vs "System entry" /
"AgentMux") for what is, from a reader's point of view, the same kind of
thing — a named block of text that gets injected into every agent's startup
file — differing in exactly one respect (does it sort first with override
wording, or not).

The cost of mirroring the backend split into the UI: two editor components
to keep in sync, two independent draft-state machines, a page that (before
this document's fix) took nine visually distinct blocks to say "here's what
gets injected into your agent's file" (`SPEC_ARMORY_GLOBAL_MEMORY_
DECLUTTER_2026_09_15.md` §2), and — concretely, in THIS conversation — a
human operator who had to explicitly ask "what is ordinary non-system
global memory?" to understand a distinction the UI itself couldn't explain
in fewer words than that question took to ask. That's the tell that the
split had leaked further into the user-facing model than the thing it was
actually protecting (write-path isolation) required.

### 1.4 A related, separate observation (flagged, not solved here)

Raised independently in this same conversation: the kind of content that
would naturally go in an *ordinary* Global Memory section — "coding
standards," "style rules" — arguably belongs in **Skills**
(`db_skills`/`SkillManager`, a different, already-shipped AgentMux
primitive for exactly this kind of procedural/triggered guidance) rather
than Global Memory, which is meant for context/instructions that should
always be present, not conditionally triggered. This is a real point but a
**separate** question from the system/ordinary UI split this document is
about — it's about what belongs in Global Memory at all, not about how many
visually distinct regions Global Memory's own pane should have. Not
resolved here; worth its own follow-up if the distinction between "always-
on Memory" and "triggered Skill" content needs sharper guidance or tooling.

## 2. Refactor plan

### 2.1 Goal

One list. One "Memory" vocabulary (already most of the way there after
#3227/#3232 — this closes the last gap). One editor. `is_system` becomes a
per-entry property an entry has, not a reason for it to live in a parallel
universe of components and state.

### 2.2 Frontend changes

**List rendering** (`global-bundle-manager.tsx`): replace the two separate
`<For each={model.systemSectionsAtom()}>` / `<For each={model.
ordinarySectionsAtom()}>` blocks (plus their separate empty-states and
separate "+ Add" affordances) with **one** `<For>` over a single merged,
reactive list built as `[...model.systemSectionsAtom(), ...model.
ordinarySectionsAtom()]` — not `model.sectionsAtom()` directly, since that
memo is ordered by raw `sort_order`/`name` and isn't guaranteed
system-first (only the backend's `bundle_memory_list_global` `ORDER BY
is_system DESC, ...` and the composed-file formatter guarantee that,
independently of this atom's own ordering — see `SPEC_GLOBAL_MEMORY_
SYSTEM_TIER_2026_08_24.md` §3.2/§3.4). Building the merged list explicitly
from the two split atoms keeps the on-screen order visibly matching actual
injection order without depending on an ordering guarantee `sectionsAtom`
doesn't document as making.

Per row, branch internally (not by rendering two different component
trees) on `section.is_system`:
- Which editing-state atom to check (`editingSystemIdAtom` vs
  `editingIdAtom`).
- Which edit/remove handlers to call (`startEditSystem`/`removeSystem` vs
  `startEdit`/`remove`) — still routes to the correct, still-isolated RPC
  under the hood. This refactor changes presentation, not the backend
  write-path guarantee from §1.2.
- Whether to show ↑/↓ reorder controls (hidden for `is_system` rows — the
  backend already silently no-ops a reorder attempt on one, per §3.2's
  guard; the UI simply shouldn't offer a control that does nothing, same
  reasoning the original spec already used for this exact case).

**Add flow:** one "+ Add Memory" button, calling `model.startNew()`
(creates an ordinary entry) — the common case. The dedicated
"create a **new** system entry" UI affordance goes away; existing system
rows remain fully visible, editable, and removable (via the correct
isolated RPC), so nothing already configured breaks or becomes
inaccessible. `GlobalBundleViewModel.startNewSystem()`/`saveSystemEdit()`
stay in the model (still needed for editing existing system rows) but lose
their only UI trigger for the *new-entry* path specifically.

**Editor component:** collapse `SectionEditor`/`SystemSectionEditor` into
one component. The two currently differ only in which model signals/
methods they read and one label string ("Add Memory" vs "Save" already
converged in #3232); a single component taking `isSystem: boolean` (read
from the entry being edited, not user-settable — creating a NEW system
entry is no longer exposed per the paragraph above) can select the right
signal pair/handler pair internally.

**Terminology:** no further changes expected beyond what #3227/#3232
already did — this document is about removing the remaining *structural*
split (two lists, two empty-states, two add-buttons), not further wording.

**Visual distinction — OPEN QUESTION, not decided here:** removing the
separate list/badge means a pinned, override-everything policy entry will
render identically to an ordinary one once merged. `SPEC_GLOBAL_MEMORY_
SYSTEM_TIER_2026_08_24.md` §1/§3.5 treated that distinction as a real
safety/clarity goal ("a human editing ordinary Global Memory never confuses
the two"), not incidental. Options, not chosen here:
1. No visual distinction at all — full uniformity, matching "we don't need
   separate Section and Memory" taken literally.
2. A minimal, non-disruptive signal retained per row (e.g. the existing
   accent border/background this document would otherwise remove, or a
   small icon) — enough that a human scrolling the list can still tell
   "this one overrides everything" without a separate list region.
Needs a decision before/during implementation; flagged rather than guessed.

### 2.3 Backend changes

**None required.** `is_system`, the two storage methods, the two RPC
commands, the reorder guard, and the composed-file split all stay exactly
as `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` shipped them (§1.2
above). This is a frontend-only refactor.

### 2.4 Test impact

- `global-bundle-model.test.ts`'s `formatGlobalBundleBlock` tests are
  unaffected (backend-mirroring logic, unchanged).
- `global-bundle-manager.test.tsx` has no current coverage of the system-
  tier rendering path (only the Claude Code reference-file block is
  covered) — worth adding a test for the merged-list ordering (system rows
  first regardless of `sectionsAtom`'s own order) as part of this refactor,
  since that ordering guarantee becomes purely a frontend responsibility
  once the merge happens (§2.2).

### 2.5 Rollout

Single PR, frontend-only, no migration. Low risk: no RPC/schema change,
existing system rows continue to work exactly as before (still editable/
removable via their real, isolated RPCs) — only the UI's presentation of
the same underlying data changes.
