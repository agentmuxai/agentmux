# Spec: Promote Agent Stash from a modal to a top-anchored drawer

**Status:** proposed — nothing in this spec is implemented. Follows PR
#3516 (the backpack icon became a real toggle) and PR #3522 (the toggle's
active state is now an accent-color tint, not a bordered square), both
shipped and both reused here unmodified — this revision only changes what
the toggle opens.

## Revision history

- **2026-09-22, first draft**: proposed a real pane **split** (§0 below,
  preserved as the original ask). Fully researched and written, never
  implemented.
- **2026-09-22, same day, this revision**: changed the mechanism to a
  **drawer** (§0.2) after reconsidering that a split-off sibling pane
  could get dragged, buried, or otherwise lost in a busy multi-pane
  layout — a real, board risk a modal never had and a drawer doesn't
  either. This revision also states a new **general responsive-floor
  rule** (§3.2) for config/reference surfaces broadly (Settings,
  Toolchain, Armory, and Stash), not previously written down anywhere as
  an explicit rule, though two of those four already substantially
  comply. Most of the split-specific design (§3 old, dimension-aware
  orientation, open-or-focus-via-layout-tree-scan, the four split-specific
  open questions) is now moot and removed rather than kept as dead
  weight — a git history diff against the merged first draft is the
  record of what changed, not a "kept for reference" section in the
  current text.

## 0. The ask, verbatim (both rounds)

### 0.1 First round (superseded mechanism, kept for the historical record)

> lets reconsider a bit .. the indicator is still a good start, but the
> greater change is we want stash to be promoted to it's own pane. Split
> along the longer dimension. The new pane is generic with a new type of
> pane tab called \<agent\>'s stash. Build out that gui from the current
> stash overlay. But the overlay is to be decommissioned.

### 0.2 Second round (current design, this revision)

> right, I am reconsidering that. the separate pane may get lost. instead,
> I am considering a split pane solution with flextable controls (switch
> horizontal/vertical) and resize. there is also the drawer approach like
> the shell

> this is good [re: accent-color tint] ... right .. the contents need to
> be responsive and support upto the form factor of a mobile phone. we
> need to enforce that as a general rule for panes, settings, toolchain,
> armory, and the config based systems since they are the ones that a
> user may want open in a smaller state to refer to and tweak. same for
> the stash. The drawer always paints from the top, which matches where
> the icon is. yes, lets update it in light of all of this

Three decisions embedded in the second round, each addressed below:

1. Stash becomes a **drawer** — a resizable region docked to the agent
   pane's own header (top edge), not a modal and not a split-off sibling
   pane. Same non-modal, non-layout-tree-leaf category as the existing
   shell drawer, just anchored to the opposite edge (§3.1).
2. **Responsive floor, stated as a general rule**, not a Stash-specific
   one: panes/settings/toolchain/armory/"config based systems" — anything
   a user might want open in a narrow, glance-and-tweak state — must
   remain usable down to a mobile-phone-width viewport (§3.2).
3. Content is still **AgentStashModal's existing GUI**, ported in, with
   the modal fully decommissioned once the drawer exists (§3.4/§3.5 — the
   part of the first draft that carries over unchanged, since it's about
   the CONTENT, not the host).

## 1. Relationship to existing work

### 1.1 PR #3516 + PR #3522 — the toggle icon (shipped, unmodified by this revision)

Both already-shipped PRs are reused exactly as-is. The backpack icon is a
`ToggleIconButtonDecl`: accent-color tint while open (PR #3522), a second
click closes it — wired via three `AgentModel` callbacks
(`_openAgentStashModal`, `_closeAgentStashModal`, `_isAgentStashOpen`) set
in `agent-view.tsx`'s `onMount`, read inside `agent-model.ts`'s
`endIconButtons()` (`createMemo`-wrapped in `blockframe.tsx`, so the
highlighted state tracks every close path).

What changes in THIS revision: what those three callbacks DO. In the
superseded split-pane design (§0.1), they drove layout-tree splits and
block creation/deletion — real complexity, covered in the first draft and
now removed. In this drawer design, they collapse to almost nothing (§3.3)
— a strict simplification, and one of the concrete benefits of switching
mechanisms, not just a response to the "may get lost" concern.

### 1.2 The actual closest precedent: the shell's own drawer

**File read in full:** `frontend/app/view/agent/components/
ResizableDetailsDrawer.tsx` (94 lines) — wraps `AgentShellSubblock`,
docked below the composer per `SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md`.
This is now the primary precedent (the split-pane precedents from the
first draft — the archived Toolchain/TrustCenter migration, the
Agent-History-as-tab spec — are demoted to "still relevant for how the
CONTENT gets ported," §3.4, since that part of the design is unchanged).

Confirmed properties, load-bearing for this design:

- **Not a layout-tree leaf.** Per the shell's own spec: "the Shell drawer
  has no placement concept at all. It is a normal sibling inside a
  `display:flex; flex-direction:column` container" — plain DOM order, not
  the `LayoutNode`/split-tree system real panes use. Show/hide is a pure
  `<Show when={detailsOpenAtom()}>` toggle; opening/closing never touches
  `LayoutTreeActionType` (no `InsertNode`/`SplitHorizontal`/etc.) — this
  is exactly what eliminates every piece of layout-tree complexity §0.1's
  design needed (open-or-focus scanning, stale-node-after-await races,
  block creation/deletion).
- **Resize is bespoke, one-dimensional, edge-anchored**
  (`ResizableDetailsDrawer.tsx:41-91`): a pointer-drag handle on the
  drawer's FREE edge (opposite the pinned edge). The shell's composer
  hugs the pane BOTTOM, so the drawer's bottom edge is pinned and its top
  edge is free — the handle sits on top, and dragging it UP grows the
  drawer (`delta = dragStartY - ev.clientY`, line 52). Clamped between
  `MIN_HEIGHT=120`/`MAX_HEIGHT=600` (lines 36-39).
- **Persistence: per-pane, via block meta.** Height lives on
  `term:shellheight` (lines 16-18, 60-63) — "remembered per pane across
  drawer close/reopen."
- **Docs:** `SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md` (placement +
  resize-handle design — the direct template for §3.3),
  `SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md`,
  `SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md`.

**Confirmed NOT to exist**, ruling out an alternative worth naming: an
interactive "switch orientation after the fact" control for an existing
pane split (the "flextable controls" idea floated alongside the drawer in
§0.2). Searched the full `LayoutTreeActionType` enum and the codebase for
"swap direction"/"toggle orientation"/"flip split" — zero hits. Resize of
an EXISTING split already works today (the generic `TileLayout`/
`layoutResize.ts` splitter), but flipping a split's orientation after
creation would be net-new layout-tree work (a new action type, flipping
the parent `LayoutNode`'s `flexDirection` via the already-existing
`reverseFlexDirection` helper, plus a new UI affordance — none of which
exists). Between that undone work and the shell's proven, already-shipped
drawer pattern, the drawer is both the lower-risk and the better fit for
"may get lost" — a real UI decision, not just a tie-breaker on
implementation cost.

## 2. What's confirmed vs. assumed (direct code reading, 2026-09-22)

### 2.1 `AgentStashModal`'s six tabs — unchanged from the first draft

This part of the research doesn't depend on the host (pane vs. drawer) —
carried over from the first draft verbatim:

| Tab id | Component | Props needed | Modal coupling |
|---|---|---|---|
| `accounts` | `AgentIdentityLinksPanel` | `agentId` | None. |
| `memory` | `AgentNativeMemoryModal` | `agentId`, `agentName`, `workingDirectory`, `onClose` | Real — see §3.4. |
| `mcp` | `AgentMcpModal` | `agentId` | None. |
| `skills` | `AgentSkillsModal` | `agentId` | None. |
| `startup` | `AgentStartupModal` | `agentId` | None. |
| `registration` | `AgentRegistrationPanel` | `agentId` | None. |

Five of six tab bodies take only `agentId` and have zero modal-specific
coupling. Only the Memory tab needs real adaptation work (§3.4).

### 2.2 The two container queries in `AgentStashModal.scss` — corrected attribution

(This corrects a mis-citation reagentx caught in the first draft's PR
review, kept accurate here rather than re-introducing the error.) Two
SEPARATE queries exist, and only one needs new design:

- **Tab-strip icon-only compression** (`.agent-stash-modal-tab span {
  display:none; }`, lines 100-106) is scoped to `@container agent-stash`
  — a `container-name` `.agent-stash-modal` establishes on **itself**
  (lines 53-54), not on any modal-specific ancestor. Already
  self-contained; ports to the drawer's own root unchanged.
- **A separate `min-width:0` flex rescue** (lines 117-122) targets
  `@container modal-mount`, a name only `ModalLayer.tsx`'s mount node
  establishes. This one needs a replacement container source once hosted
  in a drawer (§3.4) — or reconsideration of whether it's still needed at
  all in the drawer's own flex context.

### 2.3 Settings and Armory already substantially meet the new responsive floor; Toolchain does not

Checked directly (not assumed) before writing §3.2's rule:

- `frontend/app/view/settings/settings.scss` and
  `frontend/app/view/armory/armory-view.scss` both already establish
  their own `container-type: inline-size` + named container (`settings`,
  `armory` respectively — same technique `AgentStashModal.scss` already
  uses for its self-contained query, §2.2), and both already carry
  breakpoints down to **479px** (`settings.scss:561`,
  `armory-view.scss:223`) — genuinely mobile-phone-width territory, not
  just "narrower than default." Settings additionally has a 767px tier
  (`settings.scss:544`) and a wide-screen 1024px cap (`settings.scss:537`).
- **Toolchain has no container query or media query at all** (grep across
  `frontend/app/view/toolchain` — zero hits). It is a real gap relative
  to the rule this spec now states, but auditing/fixing it is not this
  spec's job (§7) — flagged here so the gap is recorded, not silently
  discovered later.
- This means §3.2's rule is mostly **formalizing an already-real,
  partially-established pattern** (two of four cited surfaces already
  comply, down to a concrete, provably-mobile breakpoint) rather than
  inventing a new requirement from nothing — the rule's job is closing
  the Toolchain gap and holding new surfaces (Stash) to the same bar,
  not retrofitting something that doesn't already have precedent.

## 3. Design

### 3.1 Anchoring: top, matching the icon

The shell's drawer is bottom-anchored because its trigger (the composer)
hugs the pane bottom. Stash's trigger (the backpack icon) lives in the
pane's HEADER — so its drawer is top-anchored: pinned to the header's
bottom edge, growing DOWNWARD into the pane body, with the resize handle
on its own bottom (free) edge — the exact mirror image of
`ResizableDetailsDrawer`'s bottom-anchored, top-handle design.

Recommended implementation: **generalize `ResizableDetailsDrawer` with an
`anchor: "top" | "bottom"` prop** (default `"bottom"`, preserving the
shell's existing behavior byte-for-byte) rather than writing a
near-duplicate component. The only logic that flips is which edge the
handle sits on and the sign of the drag delta
(`ResizableDetailsDrawer.tsx:52`: `dragStartY - ev.clientY` for
bottom-anchored/top-handle becomes `ev.clientY - dragStartY` for
top-anchored/bottom-handle) — everything else (clamp bounds, persisted
meta key pattern, pointer capture/cleanup) is anchor-independent and
should stay shared, not forked.

Persisted height: a NEW block-meta key (e.g. `agentStash:drawerheight`),
NOT reusing `term:shellheight` — the two drawers are independent surfaces
on the same pane and must remember their own sizes independently (see
§5's coexistence risk for why this matters more than it sounds).

Like the shell drawer, this is a **plain DOM sibling**, not a layout-tree
leaf — inserted into `agent-view.tsx`'s render tree directly below the
pane header, above the transcript scroll area, gated by a new
`stashOpenAtom` (parallel to, and independent from, the shell's own
`detailsOpenAtom`).

### 3.2 Responsive floor — a general rule, not Stash-specific

**The rule:** any pane, drawer, or modal whose job is reference-and-tweak
config content — this spec names Settings, Toolchain, Armory, and Stash
explicitly, per the request, but the rule is written for the CLASS of
surface, not just these four — must remain usable down to a mobile-phone
viewport width. Concretely, matched to what Settings/Armory already
prove works (§2.3): a `container-type: inline-size` +
`container-name`d wrapper on the surface's own root, with query tiers
that include something at or below **479px**, not just a "narrow
desktop window" breakpoint in the 700-800px range.

**Applied to Stash specifically:** the six ported tabs (§2.1) inherit
this floor. The existing modal's own breakpoints (560px tab-compression,
400px flex rescue — §2.2) were tuned for a **modal's** minimum practical
width, not a phone's; at implementation time, both should be re-examined
against a real ~375-430px target (common phone CSS-width range) rather
than assumed to already satisfy the new floor just because they're
already responsive to SOME degree. The Memory tab in particular (its own
780×520 standalone layout, before the modal's neutralization — §3.4)
needs explicit verification at phone width, not just "no console errors."

**Explicitly out of scope for THIS spec** (§7): auditing or fixing
Settings' or Armory's existing compliance (already good, §2.3), and
retrofitting Toolchain (confirmed non-compliant, §2.3, but a separate,
standalone task — folding an unrelated pane's CSS rewrite into a Stash
migration spec would blur scope for both).

### 3.3 Toggle semantics — now nearly trivial

Compare against the split-pane design's §3.3/§3.4 (removed from this
revision, see git history): no layout-tree scanning, no open-or-focus
dedup, no stale-node-after-await race, no dimension-aware split-direction
math. The three `AgentModel` callbacks PR #3516 already wired become:

- **`_isAgentStashOpen`**: `stashOpenAtom()` — a plain read.
- **`_openAgentStashModal`** (rename to `_openAgentStashDrawer` at
  implementation time): `setStashOpenAtom(true)`.
- **`_closeAgentStashModal`** (rename to `_closeAgentStashDrawer`):
  `setStashOpenAtom(false)`.

No new backend RPCs, no new `LayoutTreeActionType`, no new
`block-registry.ts` entry, no new `ViewModel`/barrel three-file pattern
(§3.1 of the first draft — moot now, since there's no new Block/view-type
at all; the ported content is a plain component, the same category as
`AgentShellSubblock`, not a pane's `viewComponent`).

### 3.4 Content porting — unchanged from the first draft, still the real work

`AgentNativeMemoryModal` (the `memory` tab's body) is still the one tab
needing real adaptation (§2.1):

- Its `onClose` prop drives its own footer "Close" button — dropped
  entirely in the drawer, same reasoning as the first draft (nothing
  meaningful to "close" from inside one of six tabs; the drawer itself
  closes via the toggle, §3.3).
- Its fixed 780×520 standalone sizing is already neutralized once for
  the modal case (`AgentStashModal.scss:135-142`,
  `.agent-stash-modal-panel .agent-memory-modal { width:100%; height:100%;
  max-width:none; max-height:none; ... }`). The drawer version needs the
  same neutralization against its own stylesheet — AND needs it verified
  at the new §3.2 phone-width floor, not just "not visually broken at
  desktop width."
- The tab-strip's self-contained `@container agent-stash` query (§2.2)
  ports unchanged. The separate `min-width:0` rescue
  (`@container modal-mount`, §2.2) needs a new container source scoped to
  the drawer's own root — the same conclusion the first draft reached,
  unaffected by the pane→drawer change.

`AgentStashModal.scss`'s OUTER modal-sizing rule (`.modal-panel:has(...)
{ width: min(780px,100%); height:560px; ... }`) is dropped outright, same
as the first draft — a drawer's height is user-controlled (§3.1), not
shrink-to-fit.

### 3.5 Decommissioning the modal — unchanged from the first draft

Per the ask's "the overlay is to be decommissioned," this migration
removes, not deprecates, matching the first draft's explicit deviation
from this codebase's usual "leave a superseded modal-layer kind declared"
habit:

- `AgentStashModal.tsx` + `.scss` — deleted once the drawer's content has
  absorbed everything (§3.4).
- The `"agent-stash"` `ModalLayerRequest` kind, and its
  `BACKDROP_DISMISSIBLE_KINDS` entry — with it goes the entire class of
  risk that entry's own comments document (an accidental backdrop click
  discarding in-progress draft state, PR #2315). A drawer has no
  backdrop; this is a strict simplification.
- `modal-dispatch.tsx`'s `"agent-stash"` case.

Left alone (unrelated, out of scope): the two already-dead
`AgentIdentityRequest`/`AgentMemoryRequest` kinds predating
`AgentStashModal` itself.

## 4. Testing plan

- **Anchor generalization** (§3.1): if `ResizableDetailsDrawer` gains an
  `anchor` prop, a unit/component test confirming the handle position and
  drag-delta sign both flip correctly for `anchor="top"`, while
  `anchor="bottom"` (the shell's existing behavior) is provably
  unchanged — a regression test for the shell drawer, not just a new test
  for Stash's.
- **Responsive floor** (§3.2): for each of the four named surfaces
  (Settings, Toolchain, Armory, Stash), a check — automated where
  feasible (e.g. a lint/CI rule asserting a `container-name`d wrapper
  exists and at least one query tier is `<= 479px`), manual otherwise —
  that a phone-width viewport doesn't clip or overflow un-scrollably.
  Toolchain's current zero-compliance (§2.3) means this is the first real
  test coverage that surface would get for this property.
- **Toggle round-trip**: open (drawer appears, grows downward from the
  header) → button reads active → close (drawer disappears) → button
  reads inactive — reusing PR #3516's existing toggle-wiring tests as the
  template, retargeted at `stashOpenAtom` instead of `modalLayer`. Given
  §3.3's simplification, this should need noticeably fewer test cases
  than the split-pane design's equivalent section did.
- **Tab content parity**: same as the first draft — five of six tabs need
  no new coverage beyond "still mounts" with only `agentId`; the Memory
  tab needs a regression test confirming the removed `onClose` footer
  button doesn't orphan a Save/Cancel flow, PLUS a new phone-width
  rendering check per §3.2.

Live verification (manual): open Stash from the header icon, confirm the
drawer grows downward from directly under the header (not from the
bottom, not as an overlay), confirm resize works via the bottom handle,
confirm a second toggle click closes it, confirm the old backpack modal
no longer opens anywhere, confirm the drawer's content remains usable at
a real phone-width browser viewport.

## 5. Risks / tradeoffs — stated plainly, not resolved here

- **Two independent drawers can now compete for the same pane's vertical
  space.** The shell drawer (bottom-anchored, up to 600px) and Stash's
  drawer (top-anchored, height TBD) both live in the same pane's flex
  column. Nothing in this spec caps their COMBINED height, so both open
  at max size simultaneously could squeeze the conversation transcript
  toward zero. Not resolved here (§6 open question) — options include a
  combined-height cap, mutual exclusion (opening one closes the other),
  or accepting it as a user-manageable edge case the way overlapping
  browser DevTools panels are.
- **The drawer can't later become "a real independent pane"** (dragged
  out, floated in its own window, split further) the way the first
  draft's split-pane design naturally could — a real capability given up
  in exchange for "can't get lost" and the §3.3 simplification. If a
  future need for Stash-as-independent-pane emerges, this is a
  reconsideration point, not a dead end (the ported content, §3.4,
  isn't drawer-specific — it could still be re-hosted in a pane later
  with the same effort the first draft already scoped).
- **The responsive-floor rule (§3.2) has a real, wider blast radius than
  Stash alone** — Toolchain's confirmed non-compliance (§2.3) is now a
  named, written-down gap. Stating the rule here creates a paper trail
  for that gap without this spec committing to closing it (§7) — worth
  being honest that "the rule now exists" and "every named surface
  follows it" are two different claims, only the first of which this
  spec makes true immediately.

## 6. Open questions

1. **Do the shell drawer and Stash's drawer coexist (both open at once)
   or are they mutually exclusive** (§5)? Neither existing precedent
   answers this — the shell drawer has never had a sibling drawer to
   compete with before.
2. **Is a combined max-height cap needed**, and if so what is it, given
   the shell drawer already independently clamps to 600px and Stash's
   likely wants a comparable range for its own six tabs?
3. **Does fixing Toolchain's responsive gap (§2.3) belong to this spec, a
   follow-up spec, or an untracked task?** Named here as a real,
   discovered gap; not decided here whether it needs its own tracking
   artifact.
4. **Should the §3.2 responsive-floor rule be written down somewhere more
   permanent than this spec** (e.g. a short living design-principles doc
   other specs can cite), so a FIFTH future config surface doesn't have
   to rediscover the rule from this Stash-specific document? Not decided
   here — flagged since "general rule" living inside a feature-specific
   spec is itself a minor inconsistency this spec doesn't fully resolve.

## 7. Out of scope

- Auditing or fixing Settings' or Armory's existing responsive compliance
  (already good, §2.3) — no work needed, named only as evidence the
  §3.2 floor is achievable.
- Retrofitting Toolchain's confirmed zero responsive coverage (§2.3/§5) —
  a real, separate task, not folded into this migration.
- A repo-wide "every pane is a CSS container" convention — this spec
  scopes container-query needs to Stash's own drawer root (§3.4) and the
  §3.2 rule to the four named surface types, not every future view type
  by default.
- Any visual/UX redesign of the six tabs' own content (§3.4) — porting,
  not redesigning.
- Cleaning up the two already-dead `AgentIdentityRequest`/
  `AgentMemoryRequest` modal-layer kinds predating this spec (§3.5).
- Extending `giveFocus()` or any pane-focus-routing fix — moot in this
  revision (no new pane is created), unlike the first draft where it was
  a live concern.
