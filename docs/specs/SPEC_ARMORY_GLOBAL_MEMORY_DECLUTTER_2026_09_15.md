# Spec: Armory Global Memory — declutter to a simple file list

**Status:** implemented — see the PR implementing this spec for the final
shape; §4's "after" diagram and §3 matched the shipped UI in visual
verification. — #3227
**Date:** 2026-09-15
**Verified against:** `2da71583a` (code, not spec prose).
**Related:** `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` (system-tier entries,
kept but re-skinned here), `SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md`
(owns the "applies to" chips this spec removes), PR #3199 (markdown-preview +
resizable pattern this spec reuses a third time), `SPEC_ARMORY_PERSONAL_MEMORY_
CONTENT_VIEW_2026_09_11.md` (PR #3218, the second reuse of that same pattern).
**Editing scope, confirmed directly with the user before writing this:**
inline add/edit/remove/reorder stays — this is a visual declutter, not a
switch to a read-only, click-through file browser.

## 1. The request

Verbatim: "there is too much info on the memory global pane. We want a short
sentence at the top. Then we want a list of files used as global (read by the
agent) the path, and then the file in a preview (preview already working)
then we want the combined preview at the end. We dont need info of who it
applies to, or new section."

## 2. Current state — why it reads as cluttered

`frontend/app/view/global-bundle/global-bundle-manager.tsx` stacks **nine**
visually distinct blocks, each with its own chrome, for one pane:

1. Intro paragraph, 2 sentences (`:133-136`)
2. "Takes effect on next agent restart" banner (`:138`)
3. Error banner, conditional (`:140-142`)
4. Claude Code provider config block — badge + path + caption + markdown
   preview (`:165-208`) — the ONE place that already does "path, then a
   preview"
5. System tier section list — badge + name + Edit/Remove per row, content
   never shown outside Edit mode (`:214-271`)
6. Ordinary section list — name + description + move-up/down + Edit/Remove
   per row, content ALSO never shown outside Edit mode (`:273-345`)
7. Add bar — "+ New section" button + "Promote existing bundle" dropdown
   (`:347-367`)
8. "Applies to" — filename chips (`CLAUDE.md`, `GEMINI.md`, …) + a warning
   chip for providers with no confirmed file yet (`:373-393`)
9. Combined preview — toggle, collapsed by default (`:395-415`)

Two things compound the clutter beyond raw block count: (a) blocks 5 and 6
never show a section's actual content unless you click Edit — you cannot
scan what Global Memory currently says without opening every row — and (b)
each block type has its own bespoke chrome (badge here, caption there,
description line only on ordinary sections, badge only on system/reference),
so nothing about the page reads as one consistent list.

## 3. Design

Collapse blocks 1–2 into one sentence, unify 4–6 into a single, consistently-
rendered list, delete 8 outright, keep 7 (shrunk), keep 9 at the end.

**One sentence, replacing blocks 1–3:**

> Every agent inherits this at launch — takes effect after a restart.

(Wording is a starting point, not final — the point is ONE line carrying
both facts a user actually needs: this is inherited, and edits aren't live
until restart. The error banner (3) stays, since it's conditional and only
appears when there's something to say — it was never part of the "too much
info" complaint on its own.)

**One unified list, replacing blocks 4–6 — "Global Memory files":** every
entry (the Claude Code reference file, system-tier entries, ordinary
sections) renders with the IDENTICAL shape:

- A compact header line: the entry's label — the reference file's real disk
  path in monospace for that one entry, a plain name for system/ordinary
  entries (they have no disk path; inventing one would be misleading) — plus
  a small `AgentMux` tag inline (not a separate badge block) for system-tier
  entries, plus right-aligned compact icon actions (↑ ↓ Edit Remove — hidden
  entirely on the read-only reference entry, which has none of these).
- A markdown-rendered content preview directly below, in the same resizable
  wrapper-div pattern already used twice (PR #3199, PR #3218):
  `<Markdown>` wrapped in a `resize: vertical` sized div, `.content.<x>-
  markdown-content { overflow: auto }` compound-selector override. This is
  what makes content visible WITHOUT clicking Edit — the biggest single
  scanability gain here.
- Edit mode (unchanged behavior) swaps the preview for the existing inline
  `SectionEditor`/`SystemSectionEditor` textarea form in place, for
  system/ordinary entries only.

**Delete block 8 entirely, no replacement** — the user was explicit
("we dont need info of who it applies to"). The provider-filename-mapping
concept it displayed (`SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md`)
isn't being removed from the SYSTEM — just from this view.

**Shrink block 7** — "+ New section" and the promote dropdown stay
functionally as-is, just visually minimal (e.g. a single small "+" row at
the end of the list, not a separate bordered bar).

**Keep block 9 at the very end, unchanged** — still a toggle, collapsed by
default. It's the one block the user asked to KEEP as-is ("then we want the
combined preview at the end"), and leaving it collapsed is consistent with
the rest of this declutter (it's the biggest single content dump on the
page — a full composed file). Flagged as an open call in §6 if the user
wants it always-expanded instead.

## 4. ASCII art — before / after

### Before (today, 9 stacked blocks)

```
┌──────────────────────────────────────────────────────────┐
│ Inherited by every agent at launch — composed into its    │  ← block 1
│ startup file (e.g. CLAUDE.md) in order.                   │    (2 sentences)
├──────────────────────────────────────────────────────────┤
│ Takes effect on next agent restart.                       │  ← block 2
├──────────────────────────────────────────────────────────┤
│ Claude Code provider config — reference only...           │  ← block 4
│ ┌────────────────────────────────────────────────────┐   │
│ │ [Claude Code — shared provider config]  /home/.../  │   │
│ │ Used by default spawned agents.                     │   │
│ │ ┌──────────────────────────────────────────────┐   │   │
│ │ │ (markdown preview)                            │   │   │
│ │ └──────────────────────────────────────────────┘   │   │
│ └────────────────────────────────────────────────────┘   │
├──────────────────────────────────────────────────────────┤
│ [AgentMux] System Policy Name        [Edit] [Remove]      │  ← block 5
│                       (content hidden unless editing)     │
├──────────────────────────────────────────────────────────┤
│ Coding Standards                                          │  ← block 6
│ Style rules for this workspace       [↑][↓][Edit][Remove] │
│                       (content hidden unless editing)      │
├──────────────────────────────────────────────────────────┤
│ [+ New section]   [Promote existing bundle ▾]              │  ← block 7
├──────────────────────────────────────────────────────────┤
│ Applies to: [CLAUDE.md] [GEMINI.md]  not yet applied to:.. │  ← block 8
├──────────────────────────────────────────────────────────┤
│ ▸ Combined preview                                          │  ← block 9
└──────────────────────────────────────────────────────────┘
```

### After (proposed)

```
┌──────────────────────────────────────────────────────────┐
│ Every agent inherits this at launch — takes effect after   │  ← one sentence
│ a restart.                                                  │
├──────────────────────────────────────────────────────────┤
│ /home/user/.agentmux/shared/providers/claude/CLAUDE.md      │  ← file 1 (real
│ ┌──────────────────────────────────────────────────────┐  │    disk path,
│ │ (markdown preview)                                    │  │    read-only,
│ └──────────────────────────────────────────────────────┘  │    no actions)
├──────────────────────────────────────────────────────────┤
│ AgentMux · System Policy Name          [Edit] [Remove]     │  ← file 2 (system
│ ┌──────────────────────────────────────────────────────┐  │    tier — tag
│ │ (markdown preview)                                    │  │    inline, not a
│ └──────────────────────────────────────────────────────┘  │    separate badge)
├──────────────────────────────────────────────────────────┤
│ Coding Standards                  [↑][↓][Edit][Remove]     │  ← file 3
│ ┌──────────────────────────────────────────────────────┐  │    (ordinary
│ │ (markdown preview)                                    │  │    section, no
│ └──────────────────────────────────────────────────────┘  │    description
├──────────────────────────────────────────────────────────┤    line — the
│ [+]                                                         │    preview IS
├──────────────────────────────────────────────────────────┤    the summary)
│ ▸ Combined preview                                          │  ← kept, at
└──────────────────────────────────────────────────────────┘    the end
```

Nine blocks with divergent chrome become: one sentence, one consistently-
shaped list (path/label → preview → minimal actions, repeated per entry),
one small add affordance, one unchanged combined-preview toggle at the
bottom. No block is hand-wavy "info about the system" — every remaining
line is either the actual content or a control that acts on it.

## 5. Non-goals

- **Not a data-model change.** Still `db_bundles` `is_global` rows for
  system/ordinary entries and the same `CLAUDE_CONFIG_DIR` reference file for
  the one real-path entry — purely presentational.
- **Not a switch to click-through/read-only editing.** Confirmed directly
  with the user before writing this spec: inline Edit/Remove/reorder stays.
  A file-browser-style redesign (list → click → separate detail/edit panel,
  mirroring how Personal Memory works) is a different, larger design; revisit
  as its own spec if wanted later.
- **Not touching Personal Memory** (`SPEC_ARMORY_PERSONAL_MEMORY_CONTENT_
  VIEW_2026_09_11.md`, PR #3218) — separate surface, already shipped,
  unaffected by this.
- **Not resolving** whether the provider-filename mapping (today's "applies
  to" chips) should surface somewhere else entirely (e.g. a tooltip, a
  details page) — the user asked for it gone from this view; where else it
  might belong, if anywhere, is undecided and out of scope here.

## 6. Open questions for implementation time

- Exact wording of the one-sentence intro (§3's line is a starting point).
- Whether "Combined preview" should default open now that everything else on
  the page is already visible-by-default (§3 argues for keeping it
  collapsed; flagging as a call worth double-checking, not a settled one).
- Whether the system-tier `AgentMux ·` tag reads clearly enough inline, or
  still needs some visual distinction (the current version uses a filled
  badge specifically so a user editing ordinary Global Memory never confuses
  the two, per `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` §3.5 — inlining
  it shouldn't lose that distinction, just its own dedicated header row).

## 7. Implementation notes

- Reuse the markdown-preview pattern exactly as established twice already
  (PR #3199, PR #3218): `<Markdown>` wrapped in a `resize: vertical` sized
  div (`height`/`min-height`/`max-height`, matching `.global-bundle-machine-
  config-content`'s existing values), `contentClass` + a compound
  `.content.<x>-markdown-content { overflow: auto }` selector to beat
  `markdown.scss`'s base `overflow: scroll` rule.
- `SectionEditor`/`SystemSectionEditor` (global-bundle-manager.tsx) are
  reused unchanged for edit mode — only the READ-mode row changes shape.
- CSS: `.global-bundle-applies-to*` (global-bundle.scss) can be deleted
  outright. `.global-bundle-machine-config*`, the system-section rules, and
  the ordinary-section rules likely collapse into one shared `.global-
  bundle-file*` class family instead of three divergent sets, since all
  three now render identically.
