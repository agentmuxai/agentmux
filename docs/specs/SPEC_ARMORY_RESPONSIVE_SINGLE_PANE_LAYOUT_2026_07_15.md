# SPEC — Armory: eliminate split-screen list+detail layouts, single-pane at every width

**Status:** Draft — spec only, no code written yet (per explicit request).
**Trigger:** user report — Armory panes are frequently used narrow (AgentMux panes are
user-resizable/tileable, not fixed-width dialogs), and the current list+detail split-screen
layouts either stay side-by-side (unusable when narrow) or collapse into a stacked
list-above-detail layout that still shows both at once, cramped. Explicit constraint: no split
screen, and no tree-of-selections either.
**Scope:** Armory only (hamburger → Armory), all 5 tabs. Two of the three affected components
(`AgentPrimitiveModal.scss`, used by the Skills/MCP Servers tabs) are also shared with the
per-agent Agent Setup Modal outside Armory — see §5 for why that's flagged as a separate
decision, not silently included.
**Verify before acting:** all file:line citations checked against `main` @ `b2e78754`
(v0.53.6) on 2026-07-15.

---

## 1. Current state, tab by tab

Checked all 5 `RAIL` entries in `frontend/app/view/armory/armory-view.tsx:16-21`
(Accounts / Memories / Skills / MCP Servers / Bundles) for split-screen usage:

| Tab | Component | Layout | Split-screen? |
|---|---|---|---|
| **Accounts** | `AccountsManager` → `AccountsGallery.tsx` | `.accounts-gallery-grid` — a wrapping tile grid, connect flow opens as an overlay/modal | **No.** Already fine. |
| **Memories** (native "brain") | `GlobalBrainManager` | `.global-brain-sections` — single vertical list; clicking a row expands it **in place** into an edit form (`SectionEditor`), replacing the row's own content | **No.** Already fine — and the best in-house precedent (§3). |
| **Skills** | `SkillManager` → `.agent-primitive-modal` (shared, `AgentPrimitiveModal.scss`) | Fixed `220px` list + `flex: 1` detail, side by side | **Yes — no responsive fallback at any width.** |
| **MCP Servers** | `McpManager` → `.agent-primitive-modal` (same shared component) | Same as Skills | **Yes — same gap.** |
| **Bundles** | `MemoryManager` → `MemoryManagerBody` (`memory-view.scss`) | `.memory-view { display: flex; flex-direction: row }` — `240px` list + `flex: 1` detail | **Yes**, and it *does* have a `@container memory-pane (max-width: 767px)` fallback (`memory-view.scss:277-292`) — but that fallback only changes `flex-direction` to `column`: the list caps to `max-height: 40%` and stays visible **above** the detail, which now has `60%` of a narrow pane to work with. **Still two things visible at once in a thin pane** — this is very likely exactly what triggered the report; it looks like a responsive fix but doesn't remove the split, it just re-orients it. |

**Net: 3 of 5 tabs (Bundles, Skills, MCP Servers) show a list and a detail simultaneously, one
of them even after its "responsive" breakpoint fires.** Accounts and Memories don't have this
problem today and aren't touched by this spec except as the precedent that shaped §3's design.

---

## 2. Why the current "responsive" attempt for Bundles doesn't solve it

`memory-view.scss:277-292`'s narrow-width fallback is a real, deliberate attempt at
responsiveness (it uses the `@container memory-pane` query the whole app already relies on for
pane-width breakpoints — this isn't an oversight, someone tried). The gap is conceptual, not a
bug: **stacking two panes vertically is still splitting the available space between two
simultaneously-visible surfaces.** In a genuinely thin pane (which is the normal case this
spec is written for, not an edge case), 40% height for a scrollable list leaves room for maybe
2-3 rows before it scrolls, and the remaining 60% squeezes a form with a name field, a
description field, and an `8`-row textarea. Neither half is comfortably usable. The fix isn't a
better split ratio — it's not splitting at all.

---

## 3. Proposed pattern: single-pane, one view at a time, always

**No split, at any width.** Not "split above some breakpoint, stack below it" — never show the
list and the detail simultaneously. This is a deliberate simplification versus trying to tune
a responsive split further, and it directly satisfies the "no split screen" constraint without
a width threshold to get wrong.

### 3.1 Two shapes already exist in this codebase; pick per tab, don't force one pattern everywhere

**Shape A — inline accordion (GlobalBrainManager's existing pattern, unchanged).** Already
proven in Armory today (`global-brain-manager.tsx:93-167`): the list stays single-column, and
selecting a row expands it in place into its edit form, replacing the row's collapsed content.
No navigation, no "back" button — the row *is* the detail. Works well when: content per item is
short (name + one paragraph), so an expanded row doesn't dominate the whole scroll.

**Shape B — single-pane push navigation (new, for Bundles/Skills/MCP).** List view shows only
the list (full width, full height) + the existing "New" button. Selecting an item — or clicking
"New" — **replaces** the list view with a full-width, full-height detail view (the exact same
read-only/edit-form content each of these three already renders today, just no longer squeezed
into a side column). A back affordation at the top of the detail view (`‹ Bundles` /
`‹ Skills` / `‹ MCP Servers`) returns to the list. This is a flat two-state stack (list ⇄
detail), never more than one level — **not** a tree: there's no nested category/folder
structure, no drill-down through groups, just "am I looking at the list, or one item."

**Recommendation:** Shape B for Bundles, Skills, and MCP Servers. Their detail views are full
CRUD forms (name/description/instructions-or-content/bind-row, some with an 8+ row textarea) —
meaningfully bigger than GlobalBrainManager's name+textarea pair, and benefit from the full
pane rather than competing for space inside a scrolling list of siblings. GlobalBrainManager
itself is explicitly **out of scope** — it already does the right thing, don't touch it.

### 3.2 Why not push navigation for everything, replacing Shape A too?

Considered and rejected: `GlobalBrainManager`'s accordion is small, well-tested (existing
coverage), and genuinely the better fit for its content size — converting it to push
navigation would be redundant churn with no user-visible upside and a mechanical diff for its
own sake. "Consistent code shape" isn't a strong enough reason to touch a component that isn't
broken; matching *content shape to interaction shape* is the actual design principle here, not
uniformity for its own sake.

### 3.3 Why not a tree / grouped-category browser?

Explicitly ruled out by the request. Worth stating why it would've been tempting and why it's
wrong here anyway: Skills and MCP Servers could theoretically be grouped (by provider, by
bound/unbound status), and a collapsible tree is a common way to keep a long flat list
manageable in a narrow space. But a tree adds a *third* navigational state (which group is
expanded) on top of list/detail, and for the realistic list sizes here (a handful to a few
dozen skills/MCP servers/bundles per workspace — these are hand-curated primitives, not an
auto-discovered catalog) a plain scrolling list is already fully manageable without grouping.
Grouping is a solution to a list-length problem this feature doesn't have.

---

## 4. Implementation shape

### 4.1 Shared primitive, not three parallel implementations

Skills, MCP Servers, and Bundles all want the identical list↔detail state machine (`view:
"list" | "detail"`, `selectedId`, transitions on select/new/save/cancel/delete/back). Bundles
already has this exact shape today (`MemoryViewModel`'s `selectedIdAtom`/`draftAtom`,
`memory-model.ts`) — Skills/MCP's models (`SkillViewModel`/`McpViewModel`, presumed similarly
shaped given they render through the same `AgentPrimitiveModal` markup) very likely already
have equivalent selection/draft state too, since the *data* flow doesn't change here — only
whether list and detail render simultaneously or as two mutually-exclusive views.

**Recommend:** extract a small shared layout component (e.g. `<PrimitiveListDetail>` or
similar — naming is the implementer's call) that takes `{ view: Accessor<"list" | "detail">,
list: JSX.Element, detail: JSX.Element, backLabel: string, onBack: () => void }` and renders
the mutually-exclusive single-pane layout with the back affordation. `MemoryManagerBody`,
`SkillManager`, and `McpManager` each keep their own existing list-item markup and
read-only/edit-form markup exactly as-is (no changes needed to what's *inside* each pane) —
only the outer "are both visible at once" wrapper changes. This avoids three near-duplicate
implementations of the same back-button-and-visibility logic, which is exactly the kind of
divergence that made `memory-view.scss`'s attempted fix and `AgentPrimitiveModal.scss`'s
complete absence-of-a-fix two different half-solutions to one problem in the first place.

### 4.2 CSS

Both `memory-view.scss` and `AgentPrimitiveModal.scss` currently define the list/detail split
as `display: flex` on a shared row parent with a fixed-width list child. The single-pane
version is simpler, not just different: the parent shows exactly one child
(`.is-active`/`display: none` on the other, same toggle mechanism `armory-view.tsx` already
uses for its own tab panes at `bundle-manager-pane.is-hidden`, or a plain `<Show>`), each child
at `width: 100%; height: 100%`. **This can delete code**, not just add a breakpoint:
`memory-view.scss:277-292`'s stacking fallback becomes entirely unnecessary once there's
nothing to stack — the single-pane behavior is now correct at every width, so the container
query that used to switch between "row" and "column" split goes away.

### 4.3 Back affordation

A single shared small component/class for the "‹ Back to X" control at the top of every detail
view, consistent styling across all three tabs (currently there isn't one anywhere in Armory to
copy, since nothing needs one today). Suggest reusing whatever chevron/back-icon convention
already exists elsewhere in the app's settings/modal chrome, rather than inventing new iconography.

### 4.4 What doesn't change

- Every existing RPC call, model method, and validation rule in `MemoryViewModel` /
  `SkillViewModel` / `McpViewModel` — this is a pure layout change, no data-flow change.
- The read-only-view → edit-form → save/cancel state machine each tab already has internally
  (e.g. Bundles' `selectedAtom`/`draftAtom` two-step) stays exactly as-is; it just now renders
  in a full-width single pane instead of a `flex: 1` side column.
- GlobalBrainManager and AccountsManager — out of scope, already correct (§3.2, §1).

---

## 5. Scope decision the implementer/user should confirm before starting

`AgentPrimitiveModal.scss` (and its `.agent-primitive-modal` markup) is **shared** between
Armory's Skills/MCP Servers tabs and the per-agent **Agent Setup Modal**'s own Skills/MCP tabs
(`AgentSkillsModal.tsx`/`AgentMcpModal.tsx`, outside Armory — CLAUDE.md's "Not widgets" table).
This spec's ask was scoped to "all screens in the Armory," but fixing the shared component
necessarily also changes those non-Armory surfaces, since there's only one
`AgentPrimitiveModal.scss` today.

Two ways to handle this, implementer's call:
1. **Let it ride** — the Agent Setup Modal is also frequently opened at a modest width and
   would very plausibly benefit from the identical fix; sharing the component was presumably
   intentional for exactly this kind of consistency. Simplest, no extra work.
2. **Fork before changing** — duplicate the component so Armory gets the new single-pane
   layout and the Agent Setup Modal keeps its current split, if there's a reason (not identified
   in this investigation) the modal specifically wants the split kept. Extra code, no known
   justification found — recommend against this unless the user has one.

**Recommendation: option 1.** No spec or code found suggesting the Agent Setup Modal's split
is intentional-and-load-bearing; it looks like the same unaddressed gap in a shared component,
not a considered difference. Flagging explicitly rather than silently expanding scope, per this
codebase's own convention (see `SPEC_LIGHT_THEME_DEPTH_AND_MORE_THEMES_2026_07_13.md` for the
same "confirm the scope boundary rather than assume" pattern on a different feature).

`AgentNativeMemoryModal.scss` (referenced in `AgentPrimitiveModal.scss:7`'s own comment as
sharing "the same list+detail shape") is a **different** component (per-agent native-memory
file browser, also outside Armory) — not touched by this spec, not verified against this
investigation. Flagging only so a future pass doesn't assume it's already covered.

---

## 6. Test coverage to add

- Selecting a Bundle/Skill/MCP server from the list shows only the detail view (list not
  present in the DOM, or `display: none`/hidden — whichever mechanism is chosen).
- "New" from the list view opens the detail view in create mode.
- The back affordation returns to the list view with the list's scroll position and content
  intact (no refetch needed — data doesn't change, only which view is shown).
- Save / Cancel / Delete from the detail view all return to the list view afterward, matching
  each tab's current post-save/cancel/delete behavior (verify none of the three currently do
  something list-view-specific on save that a full navigation-back would break — e.g. does
  Bundles currently re-select the saved item and show its read-only detail rather than
  returning to the list? If so, decide whether that behavior is preserved in the new shape or
  intentionally simplified to "always return to list," and note that in the actual PR, since
  this spec inspected the *layout* code, not every save-completion code path in each model).
- No `@container` layout test still asserts the old stacked-at-narrow behavior for Bundles
  (`memory-view.scss:277`'s removed breakpoint) — delete/update any existing test coupled to it.
- Manual: resize an Armory-hosting pane down to a genuinely thin width (a few hundred px) for
  each of the three tabs — list should be fully readable, detail should be fully usable, at no
  point should both be visible or either be cramped.

---

## 7. Suggested PR split

1. **PR A** — shared `<PrimitiveListDetail>`-style layout primitive + Bundles migration (single
   tab, proves the pattern, smallest blast radius — Bundles' own model is already closest to
   this shape).
2. **PR B** — Skills + MCP Servers migration onto the same shared primitive (both share
   `AgentPrimitiveModal.scss` today, so naturally land together). Confirms the §5 scope decision
   before touching the Agent Setup Modal's shared usage.

---

## 8. Sources

- `frontend/app/view/armory/armory-view.tsx`, `armory-view.scss`
- `frontend/app/view/memory/memory-manager.tsx`, `memory-model.ts`, `memory-view.scss`
- `frontend/app/view/skill/skill-manager.tsx`
- `frontend/app/view/mcp/mcp-manager.tsx`
- `frontend/app/view/agent/components/AgentPrimitiveModal.scss`
- `frontend/app/view/brain/global-brain-manager.tsx`, `global-brain.scss`
- `frontend/app/view/accounts/AccountsGallery.tsx`, `accounts-manager.tsx`
- `docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md` (current Armory
  tab set/order this spec assumes)
- GitHub issue #2024 (Armory consolidation tracker — confirmed no open item covers this;
  independent scope)
