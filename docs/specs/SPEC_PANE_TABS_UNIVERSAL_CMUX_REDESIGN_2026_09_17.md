# SPEC: Universal Pane Tabs — Every Pane Header Becomes a Tab Strip (cmux-Style Redesign)

**Date:** 2026-09-17
**Status:** active — design finalized, all open questions resolved (§7),
implementation underway on a dedicated branch. See
`docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md` for the
task-level breakdown. Not merged to main until tested locally and explicitly
approved by the repo owner — this redesign is too far-reaching for the usual
merge-on-approval default. — PRs touching this work, newest first: #3309
**Companion (direct predecessor — this generalizes it, does not replace it):**
`SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` (status: implemented) and its
polish cluster: `SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`,
`SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md`,
`SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md`,
`SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md`,
`SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`,
`SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md`,
`SPEC_TERM_PANE_TAB_STRIP_TRAILING_BLUR_2026_09_07.md`.
**Must not contradict (unresolved, in-flight architecture — read before touching drag or
render state):** `SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` (Draft — 6
diagnosed defects behind 4 rounds of dead-tab incidents; do not add a new layer of drag
state on top of this),
`SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md` (§3.1/3.2's global
epoch/transaction model was explicitly **rejected** after 4 review passes — do not
re-propose it here).
**Also related:** `SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md` (the *outer*,
workspace-level tab bar — a different layer, see §2),
`SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` (pane→workspace-tab-bar drag, prior art for a
future pane-tab tear-off),
`SPEC_WIDGET_CONTEXT_MENU_OPEN_ACTIONS_2026_06_24.md` +
`SPEC_WIDGET_CONTEXT_MENU_OPEN_ACTIONS_PHASE_2_2026_09_16.md` (this session's own
"Open in New Window / Floating Pane / New Tab" widget-menu work — reused, not
replaced, by this redesign; see §4.7).
**External reference:** [cmux](https://cmux.com) (manaflow-ai/cmux) — an open-source
native terminal built specifically for running multiple AI coding agents in parallel,
whose shipped architecture (`Workspace → Pane → Surface(=tab) → Panel(=content)`) is
functionally the same pattern this spec proposes generalizing AgentMux toward. Cited
throughout §3.
**Scope:** Frontend primarily. Backend schema support for this feature **already
shipped** (see §2.3) — no `LayoutNodeData` schema change is required. Backend work is
limited to a new generic "add block to an existing pane's stack" reducer path (§4.2)
and normal RPC plumbing; the difficulty of this redesign lives almost entirely in the
frontend (chrome hoisting, tab-strip UI, focus model, interaction design).

---

## 1. Motivation

Today, opening a widget from the widget bar (left-click) always creates a brand-new
split in the current tab's layout tree — every widget you open grows the split-pane
tree by one leaf. There is exactly one narrow exception: agent panes (forks) and
terminal panes (shell tabs) can already hold multiple blocks as an in-pane tab strip,
via a mechanism (`blockStack`) that was deliberately scoped to **same-view-type-only**
stacking (an agent pane's tabs are always more agent forks; a terminal pane's tabs are
always more shells) — see `SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` §1's own
framing: *"a 'tab' IS a fork."*

The result for every other widget type (browser, editor, sysinfo, swarm, armory,
media, drone, help, warden) is layout sprawl: a user who wants a browser pane next to
their existing terminal-and-agent workspace has no choice but to keep splitting,
because there is no way to add a widget as a tab of an *existing* pane unless that
pane already happens to be an agent or terminal pane.

**Even where the tab strip already exists (agent, term), it is a SECOND row, not a
replacement for the header.** Today's chrome for a hoisted pane is two stacked rows:
the full `BlockFrame_Header` (icon, title, minimize/magnify/close) on top, and
`PaneTabStrip` (the tab pills) below it — see §2.4 for exact citations. This spec's
core mandate, stated plainly because it changes the shape of the redesign
significantly from "just extend tabs to more widget types": **collapse those two
rows into one.** The tab strip does not sit below the header — it **becomes** the
header, at the very top of the pane, the way cmux's Surface tab strip is the *only*
chrome element above a pane's content (no separate title bar above it at all). This
applies retroactively to agent/term (whose two-row chrome is restructured, not kept)
and prospectively to every other widget type as it rolls out (§5).

---

## 2. Current architecture (grounding — read before the design section)

### 2.1 Terminology — coined/tightened for this spec

The single most important correction this spec makes to its own earlier drafts,
made explicit here because it is easy to get wrong and, per the discussion that
produced it, ripples into many files (§4.11): **a Pane is always a generic
container. A widget instance is never itself "a Pane" — it is a Pane Tab, one
member of the Pane's tab list.** A Pane holding exactly one widget is not a
degenerate case or a different kind of thing; it is a Pane whose Pane-Tab list
happens to have length 1. This distinction matters because today's code (and this
spec's own §4.1/§4.2 in an earlier pass) sometimes talks as if "pane" and "widget"
are interchangeable — they are not, and the redesign's whole point depends on
keeping them separate.

| Term (this spec) | What it is | Existing code-level name(s) |
|---|---|---|
| **Pane** | A generic container — one leaf position in the split tree. Holds an ORDERED LIST of ≥1 Pane Tabs, exactly one of which is active/visible at a time. Owns pane-level chrome: the tab strip itself, and pane-level controls (minimize, magnify/restore, close-the-whole-pane) — see §4.1/§4.6. | `LayoutNode` (leaf) — `agentmux-common/src/layout_types.rs:71-91`, frontend mirror `frontend/layout/lib/types.ts:185`. Colloquially "pane" in existing docs/code, but historically also used loosely to mean a widget instance — this spec stops that usage. |
| **Pane Tab** | One widget instance living inside a Pane's tab list — a `Block`, viewed from inside a Pane. **A widget IS a Pane Tab; it is never itself "a Pane."** Switching the active Pane Tab does not change which Pane you're in. | `Block` (`agentmux-srv/src/backend/obj.rs:480-495`) as referenced by a `LayoutNodeData.block_stack` entry (`agentmux-common/src/layout_types.rs:32-56`). "Widget" (`defwidget@X` in `widgets.json`) is the template used to CREATE the `Block` that becomes a Pane Tab; the template itself is never persisted. |
| **Window Tab** | The OUTER, browser-style strip at the very top of the window. One Window Tab owns an entire split-pane tree (a whole arrangement of Panes, each with its own Pane Tabs) — one `LayoutModel`. A completely different layer from a Pane; out of scope for this spec, unchanged. | Existing code calls this object `Tab` (`agentmux-srv/src/backend/obj.rs:408-419`, `frontend/app/tab/tabbar.tsx`). This spec adopts **"Window Tab"** as the plain-language, UI-facing term to keep it unambiguously distinct from "Pane Tab" in prose and in any user-facing copy/settings this redesign introduces — it is NOT a proposal to rename the `Tab` struct/RPC/table itself (that is a much larger, unrelated migration; see §7). |

### 2.2 Disambiguation: three different things could all plausibly be called "a tab"

`SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` already had to resolve part of
this naming collision when it shipped (Window Tab vs. the in-pane strip); this spec
sharpens it further with the Pane/Pane-Tab split above:

- **Window Tabs** (`frontend/app/tab/tabbar.tsx`) — the top-of-window browser-style
  strip. Each one owns a whole split-pane tree. Out of scope for this spec; unchanged.
- **Panes** (this spec) — the generic containers that make up a Window Tab's split
  tree. Always containers; never conflated with the widget(s) inside them.
- **Pane Tabs** (this spec's subject) — the widget instances living inside one
  Pane's tab list, switched via the strip rendered as that Pane's header (§4.1).
  This is the layer being generalized.

Every mention of "tab" in this document is qualified as one of the three above
whenever ambiguity is possible; an unqualified "tab" defaults to meaning **Pane
Tab**, this spec's primary subject.

### 2.3 The schema already supports this — no backend migration needed

`LayoutNodeData` (`agentmux-common/src/layout_types.rs:32-56`, shared by the wire
protocol and the srv object store) already has:

```rust
pub struct LayoutNodeData {
    pub block_id: String,           // kept in sync with active_block_id when stacked
    pub block_stack: Vec<String>,   // ordered member block ids — ALREADY plural
    pub active_block_id: String,
    pub extra: serde_json::Map<String, serde_json::Value>,  // forward-compat
}
```

This shipped with the agent/terminal feature and is back-compat tested: a
pre-`blockStack` leaf (`{"blockId":"blk-1"}`) round-trips byte-identical, so the
overwhelming majority of today's single-block leaves are unaffected on disk. A
stacked leaf already persists `blockStack`/`activeBlockId` correctly. **This spec
requires zero changes to this struct.** The gap is entirely in which *code paths*
populate `block_stack` (today: only agent-fork and terminal-shell-tab creation) and
which UI renders a strip for it (today: only `PaneTabStrip` consumers, which are
agent/term/editor — see §2.4).

### 2.4 What already exists and what's still per-view-type-gated

- **`frontend/layout/lib/layoutStack.ts`** — the mutation primitives this spec builds
  on directly: `pushBlockOntoStack(model, nodeId, blockId)`, `setActiveBlockInStack`,
  `closeBlockInStack`. Deliberately bypass `treeReducer` (a stack mutation doesn't
  change the tree's shape). **View-agnostic already** — nothing here assumes a
  particular widget type.
- **`frontend/app/tab/pane-leaf-chrome.tsx`** — the per-leaf router. Its fallback
  swap-one-at-a-time render path is **already fully view-agnostic**: it works for
  any `blockId` via `scopedNodeModel` overriding `NodeModel.blockId`, which flows into
  the ordinary `BlockFrame` header, which already shows the correct icon/title for
  whichever tab is active — with zero widget-type-specific code. What's gated:
  - `HOISTS_OWN_CHROME = new Set(["agent", "term"])` — only these two get a
    **stable** header+strip that survives a tab switch without remounting; every
    other view type gets a plain passthrough where the *entire* block (header
    included) remounts on every switch.
  - `KEEP_ALIVE_TYPES = new Set(["term"])` — only terminal keeps every stack member
    mounted simultaneously (visibility-toggled); everything else (including agent)
    swaps one mounted block at a time.
- **Today's chrome for a hoisted pane is TWO stacked rows, confirmed by direct
  reading of both implementations** — `AgentPaneChrome`
  (`frontend/app/view/agent/agent-view.tsx:350-895`) and `TermPaneChrome`
  (`frontend/app/view/term/term.tsx:442-751`) have the identical structure: a
  `headerElem` built from the real `BlockFrame_Header` component (full icon/
  title/minimize/magnify/close chrome, `agent-view.tsx:427-435`) is rendered FIRST
  (`agent-view.tsx:840`, `term.tsx:699`), and only THEN, in a separate wrapping div
  below it (`.agent-pane-stack-content`, `agent-view.tsx:848`), does
  `PaneTabStrip` render the tab pills (`agent-view.tsx:867`, `term.tsx:717`). A
  progress-bar slot sits between them for agent panes. This is the exact "tabs
  below the header, not replacing it" state this spec's §1 commits to collapsing
  into one row — not a detail to preserve, a structure to remove.
- **`frontend/app/element/PaneTabStrip.tsx`** — the generic, `<T>`-typed tab-strip
  component. No drag-reorder, no tear-off, no pin (`SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md`
  §7 explicitly deferred these as v1 non-goals). Current consumers: agent
  (real `blockStack`), terminal (real `blockStack`), **and** the editor's file-tab
  strip — which is visually similar but is **not** `blockStack`-backed at all; it's a
  files-within-one-block feature with its own state store
  (`editor-pane-state-store.ts`). This spec's editor rollout (§5, Phase 1) needs to
  either migrate the editor onto real `blockStack` semantics or explicitly keep its
  existing files-tabs mechanism as a parallel, non-generalized case — a decision
  this spec defers to the implementer with a recommendation (migrate it — see §7).
- **Plain left-click "add widget" today** (`action-widgets-config.ts:220-222`
  `handleWidgetSelect` → `block-layout-actions.ts:68-88` `createBlock`) —
  unconditionally dispatches `LayoutTreeActionType.InsertNode`, i.e. always a new
  split (a new Pane). **This spec keeps that behavior** — see §4.2 for why — but
  reframes what it means: today's code and mental model treat this as "open the
  widget"; under this spec's terminology (§2.1) it is precisely "create a new Pane
  containing exactly one Pane Tab." No code change at this call site; the change is
  entirely in what a Pane's OWN tab strip can do once it exists (§4.5).

---

## 3. What cmux (and the broader landscape) teach, and what this spec commits to as a result

cmux's shipped hierarchy — `Workspace → Pane (split region) → Surface (tab within a
pane) → Panel (content)` — is, structurally, the same target this spec proposes:
"pane" is the split-tree unit, "surface"/pane-tab is the switchable content unit
inside it. Findings taken directly from cmux's own source, docs, and issue tracker
(a genuinely good primary source — several of the following are documented
regressions cmux shipped, got real user feedback on, and fixed):

1. **Never auto-reorder tabs a user navigates by position or shortcut.** cmux shipped
   auto-surfacing "most recently notified" workspace tabs to the top, got HN
   backlash ("the user can't rely on a stable mapping between shortcut and
   workspace"), and added a setting to disable it (cmux issue #205). **Commitment:**
   pane-tab order in this redesign is never silently reordered by the system; a
   needs-attention state is communicated via a badge (§4.6), never by moving the tab.
2. **Give drag operations visible feedback before mouseup.** cmux was criticized on
   HN for exactly this gap ("you never really know what it's going to do on
   mouseup"). **Commitment:** any future drag affordance this spec's later phases add
   (§5, Phase 3+) must show a drop-zone/ghost preview during the drag, not just on
   release — but see §4.7 for why drag itself is scoped conservatively here.
3. **One authoritative "active pane" resolver — never let a visual indicator drift
   from it during interaction.** cmux's own PR #12612 exists because dragging left a
   sidebar selection indicator pointing at a different target than the pane actually
   receiving keystrokes; the fix was routing every focus-dependent UI element through
   one resolver instead of trusting window-manager-level focus signals as a fallback.
   **Commitment:** §4.6 defines a single two-level focus model (focused pane + active
   tab within it) that every visual indicator must read from — no independent
   drag-time indicator state.
4. **"Add as a tab to an existing pane" must be uniform across every content type
   from day one — do not ship it for some widget types and not others, even
   temporarily.** cmux's own backlog has an open issue (#2770) asking for exactly
   this because terminal surfaces support tab-in-pane fully while
   markdown/browser surfaces always force a new pane — a confusing asymmetry users
   can't predict. AgentMux already has a version of this exact asymmetry today
   (agent/term have it, nine other widget types don't) and this spec's own phased
   rollout (§5) necessarily has a *temporary* version of it too — addressed head-on
   in §5's phasing rationale rather than glossed over.
5. **Offer a keyboard-accessible equivalent for every drag-based action.** cmux pairs
   its mouse drag-to-reorder with dedicated shortcuts (⌥⌘⇧[ / ⌥⌘⇧] reorder within a
   pane; ⌥⌘⇧+arrows move to an adjacent pane). **Commitment:** §4.8.
6. **Tab bar sizing is a genuine, still-unsolved tradeoff, not something to bikeshed
   into a single "correct" answer.** cmux ships fixed-width tabs with no
   full-pane-width option for a lone tab (an open, contested feature request —
   issue #5091). VS Code offers `tabSizing: shrink` (cap shrink instead of unbounded)
   *and* `wrapTabs` (wrap instead of scroll) as competing, user-selectable defaults —
   both shipped, neither declared universally correct. **Commitment:** §4.9 picks one
   default (shrink-with-floor + horizontal scroll on overflow) but treats it as a
   setting, not a hard architectural constraint, consistent with how every mature
   implementation of this pattern has actually handled it.
7. **A contested, adjacent design question exists even inside cmux's own community**
   (GitHub Discussion #1404): should it be "pane contains tabs" (cmux's shipped
   model, and this spec's proposal) or "tab contains panes" (Zellij's model — tabs
   are the outer container, each tab holds a whole pane layout)? **This spec commits
   to pane-contains-tabs**, because it is the minimal-diff extension of AgentMux's
   already-shipped `blockStack` primitive and its already-decided split-tree
   architecture (workspace Tab → LayoutModel → split tree of panes) — flipping to
   Zellij's inverse model would mean redefining what a workspace-level Tab *is*, an
   unrelated and much larger change this spec explicitly does not propose.

Additional, non-cmux-specific lessons folded into §4: VS Code's tab decoration
badges (git status, diagnostics) as prior art for a future agent-needs-attention
badge (§4.6); Warp's Focused/Summary view toggle as an alternative to always
rendering every pane's full strip when space is tight (noted as a future idea, not
adopted here — see §7); JetBrains' dual affordance (drag OR explicit shortcut) for
tear-off, reinforcing point 5 above; the W3C ARIA Authoring Practices Guide's Tabs
Pattern as the concrete accessibility contract (§4.8).

---

## 4. Design

### 4.1 One unified row: the tab strip IS the header, for every pane, even with exactly one tab

**This is the central structural change this spec proposes**, more significant than
just widening which widget types get a tab strip at all (that's §5's job). The
existing two-row chrome (§2.4 — `BlockFrame_Header` on top, `PaneTabStrip` below it,
in `AgentPaneChrome`/`TermPaneChrome`) collapses into ONE row, rendered at the very
top of the pane, matching cmux's model exactly: a Surface tab strip is a pane's
*entire* header — there is no separate title bar sitting above it.

**What the unified row contains, left to right** (a new component, working name
`PaneHeaderTabStrip`, replacing both `BlockFrame_Header` and the standalone
`PaneTabStrip` usage inside every hoisted-chrome pane):

- **Leading pane icon** (optional, small) — only shown when it adds information the
  active tab's own icon doesn't already carry; likely omitted in the common case
  since each tab already shows its own widget-type icon. Left as an implementation
  call, not mandated.
- **The tab strip itself** — one pill per `block_stack` member (or the lone block,
  for an unstacked pane), each with its own icon + title + per-tab close button
  (`×`), exactly as `PaneTabStrip` renders today. **This absorbs the identity role
  `BlockFrame_Header`'s title/icon used to carry** — for a single-tab pane, that one
  pill IS the pane's visible identity, so the redesign does not lose any information
  today's header shows, it relocates it onto the tab.
- **"+"** (§4.5) — trailing the last tab, opens the widget picker.
- **A compact, pane-level control cluster** at the strip's far trailing edge —
  minimize, magnify/restore, and close-PANE (distinct from any tab's own close-TAB
  button — closing the pane when it holds N>1 tabs is a deliberate, separate action,
  not "close whatever tab happens to be active"). This is where
  `BlockFrame_Header`'s `OptMinimizeButton`/`OptMagnifyButton`/close-decl controls
  (`blockframe.tsx:168-251`) move to, unchanged in behavior, just relocated into the
  same row as the tabs instead of a row of their own. Right-click on this cluster
  (or on the strip's empty trailing space) still opens the existing native pane
  context menu (`buildPaneContextMenu`, currently reached via
  `handleHeaderContextMenu`, `blockframe.tsx:77`) — unchanged, just re-anchored.

A pane holding exactly one block therefore looks close to today's plain header at a
glance (one pill instead of a plain title, but same overall row height and control
cluster) — but it is now built from the *same* component every pane uses, so gaining
a second tab is a pure state change (another pill appears) rather than a component
swap or a second row appearing underneath. This directly removes the "tabs live in a
second row below the header" structure §1 and §2.4 identify as the thing to fix, for
agent/term as much as for every newly-onboarded widget type.

> **Superseded 2026-09-24:** the `pane:tabstrip` setting described in the rest
> of this paragraph has been removed — a Pane header always shows its tabs.
> See `SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md` §8.

Exposed as a setting (`pane:tabstrip = always | multi-only`, default `always`) purely
for the case of hiding the tab-pill-as-title distinction when a pane has exactly one
tab and a user prefers the older plain-title look — but note this setting, if
`multi-only`, controls the row's *content* (one plain title vs. one tab pill), not
whether there are one or two rows; the two-row structure itself is not coming back
under any setting. Mirrors VS Code's own `workbench.editor.showTabs` precedent
(default on, user-overridable) for the general shape of offering this as a setting
rather than a hard constraint.

### 4.2 Left-click "add widget" from the widget bar is UNCHANGED — it creates a new Pane

**Correction from an earlier pass of this spec, worth stating explicitly since it
reverses what this section originally proposed:** selecting a widget directly from
the widget bar continues to call `handleWidgetSelect` → `createBlock`
(`block-layout-actions.ts:68-88`), which continues to dispatch `InsertNode` — a new
Pane, unchanged, containing exactly one Pane Tab (§2.1's terminology; §2.4's last
bullet). **This is not a regression to avoid fixing — it is the correct behavior.**
The widget bar has no notion of "which Pane" a click should target (there may be
several visible, or none), so the only well-defined default for a global "open this
widget" action is "give it its own new Pane" — exactly what happens today. Nothing
about this call site changes.

What DOES change is that "the currently focused pane" is no longer the only
thing standing between a user and a multi-Pane-Tab Pane: **every Pane already has
its own "+" button once §4.1/§4.5 ship**, and THAT is the deliberate, unambiguous
mechanism for "add this widget as another Pane Tab in THIS SPECIFIC Pane" — it
inherently knows its target (the Pane it's rendered inside), which a global
widget-bar click structurally cannot. §4.5 is accordingly the primary vehicle for
populating a Pane with heterogeneous Pane Tabs, not a left-click default change.

The minimize/magnify/close-Pane controls (§4.1's trailing control cluster) are
unaffected by any of this — a freshly `InsertNode`-created single-Pane-Tab Pane
gets the same controls a multi-Pane-Tab Pane does, since (§2.1) they were always
Pane-level, never Pane-Tab-level, controls.

**Backend note:** no new backend schema is needed (§2.3), but a generic reducer path
for "push an arbitrary newly-created block onto an EXISTING pane's stack" still
needs to exist for non-agent/non-term widget types, to serve §4.5's "+" picker —
today only agent-fork and terminal-shell-tab creation populate `block_stack` at all,
and both do so only within their own view type. This is new server-side work (a
`Command` variant, or an extension of the existing `pane.open` RPC to accept a
`stack_onto_node_id`/similar parameter), scoped per-widget-type as each type rolls
out (§5).

### 4.3 Hoisted chrome becomes the default, not an opt-in allowlist

`HOISTS_OWN_CHROME` inverts from an allowlist (`{"agent", "term"}`) to covering every
widget type that has completed rollout (§5) — i.e. the goal state has no allowlist at
all; every pane's chrome is the unified `PaneHeaderTabStrip` (§4.1), stable across a
tab switch, and only the *inner content* remounts (or, per `KEEP_ALIVE_TYPES`, stays
mounted and visibility-toggled). Concretely, this retires `AgentPaneChrome`'s and
`TermPaneChrome`'s bespoke `headerElem`-plus-`PaneTabStrip` construction (§2.4) in
favor of both consuming the same shared `PaneHeaderTabStrip`, and every other widget
type's `ViewModel` gains the same render contract `pane-leaf-chrome.tsx` already
defines for agent/term today — real engineering work, not a config flip, which is
why §5 phases it per widget type rather than flipping it globally on day one.

`KEEP_ALIVE_TYPES` (currently `{"term"}` only) is left as a per-type opt-in, not
defaulted to universal — keeping every stack member's block mounted simultaneously
has a real memory/CPU cost (this is exactly why it's opt-in for terminal specifically:
a live PTY session is expensive to tear down and recreate on every switch, unlike
most other widget types). Each widget type's rollout phase (§5) makes its own call on
whether it needs `KEEP_ALIVE_TYPES` membership, with a bias toward NOT adding it
unless a specific widget type demonstrably needs session continuity across switches
(agent is the other clear candidate, given an agent fork's own conversation state).

### 4.4 Heterogeneous stacks are allowed, from each widget type's first day of rollout

Nothing in `layoutStack.ts` or the schema (§2.3) prevents a pane's stack from mixing
widget types — a pane could hold a terminal tab, a browser tab, and a sysinfo tab
side by side. This spec explicitly allows it (no view-type constraint is added), and
explicitly rejects doing a same-type-only rollout followed by a later "now allow
mixing" phase — per §3 point 4 (cmux's own issue #2770 as the cautionary tale: ship
the uniform, unconstrained version from a widget type's first day in the tab-strip
system, don't create a second asymmetry axis (mixing) on top of the necessary first
one (which widget types have rolled out at all, §5).

### 4.5 The "+" on a Pane's own strip — the actual mechanism for adding a heterogeneous Pane Tab

Per §4.2, this — not the widget bar's plain left-click — is the primary, deliberate
way a user adds another Pane Tab to a Pane that already exists. `PaneTabStrip`
already has an `onAdd` prop, but today it's wired per-view-type to narrow,
type-specific behavior (agent forks a new agent; term opens a new shell). This spec
generalizes it: every Pane's "+" opens the same **widget picker** the widget bar's
right-click menu already builds (`buildPaneWidgetMenuItems`,
`action-widgets-config.ts:180-216` — already takes an `onSelect(blockdef)` callback
of exactly the right shape), filtered to widget types that have completed §5's
rollout. Selecting a widget from the picker:

```ts
const blockId = await ObjectService.CreateBlock(blockDef, rtOpts);
markBlockRecentlyCreated(blockId);
pushBlockOntoStack(layoutModel, thisPanesNodeId, blockId);
```

using `pushBlockOntoStack` (`frontend/layout/lib/layoutStack.ts:61`) against **the
specific Pane whose own "+" was clicked** — always unambiguous, since a "+" only
ever exists inside one Pane's strip. This is the whole mechanism §4.2 was missing a
target for at the widget-bar level; a Pane's own strip has that target for free.

### 4.6 Two-level focus model, one authoritative resolver

This redesign introduces a genuine new distinction that didn't matter as much before:
**which pane is focused** (existing `NodeModel.isFocused`, split-tree-level) versus
**which tab is active within that pane** (`LayoutNodeData.active_block_id`,
already-existing field). Per §3 point 3 (cmux's PR #12612 lesson), this spec requires
a single resolver function that both the pane-border focus indicator and the
tab-strip's active-tab highlight read from — never two independently-derived pieces
of state that could drift apart during a switch or a drag. Visual treatment: a
focused pane gets its existing border/highlight treatment (unchanged); the active
tab within any pane (focused or not) gets a distinct highlight (underline or filled
background, TBD in implementation — a visual-design pass, not an architectural
decision this spec needs to make). A future needs-attention badge (e.g. an agent
awaiting input, a terminal bell) is a visually distinct third state, hookable onto
the same tab-strip component later — not designed in full here (flagged in §7), but
the tab-strip's data model should carry an optional badge/status field from the
start so this isn't a breaking addition later.

### 4.7 Interaction with the existing widget-bar right-click menu and floating-pane/new-window machinery

This session's own `SPEC_WIDGET_CONTEXT_MENU_OPEN_ACTIONS_PHASE_2_2026_09_16.md`
work ("Open in New Window" / "Open in Floating Pane" / "Open in New Tab") is
**reused as-is, not replaced**:
- "Open in New Tab" already means "open at the workspace level" (a new outer Tab) —
  unambiguous, unaffected by this spec.
- "Open in New Window" / "Open in Floating Pane" both already create the block
  standalone (no pane/stack involvement) — unaffected.
- The plain left-click default changes (§4.2), and two new right-click entries
  ("Split Right"/"Split Down") are added, but nothing in the existing three-action
  menu needs to change.

### 4.8 Drag-and-drop: deliberately conservative scope for this spec

`SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` is an unresolved Draft
diagnosing six real architectural defects behind four rounds of dead-tab/dead-pane
incidents (unreliable `dragend`-based teardown, overloaded boolean drag state, an
overlay z-layered above pane headers, a last-writer-wins block-component registry,
blind un-versioned layout persistence, and drag state scattered across ~14 stores in
3 layers). **This spec does not add a new layer of drag state on top of that
diagnosed mess.** Phase 3 (§5) ships only a same-pane tab reorder — a local array
mutation within one leaf's `block_stack`, with no cross-pane or cross-window drag
target, no interaction with `crossTabDrag`/`CrossWindowDragMonitor`, and therefore no
new exposure to the six defects above. Cross-pane drag (move a tab from one pane's
strip to another's) and drag-to-tear-off-into-a-new-pane/window are explicitly
deferred to a later phase, gated on the drag-session refactor actually landing (§5,
Phase 4) — this spec's data model doesn't block that later work (a tab is just a
`block_stack` member; moving one is `closeBlockInStack` + `pushBlockOntoStack`
against the target pane, whenever the drag surface to trigger that safely exists).

Every drag-based action this spec DOES ship (Phase 3's reorder) has a
keyboard-accessible equivalent per §3 point 5 — see §4.9.

### 4.9 Keyboard navigation and accessibility

Adopts the W3C ARIA Authoring Practices Guide's Tabs Pattern: the strip is
`role="tablist"`, each tab `role="tab"` with `aria-controls` pointing at its content,
content is `role="tabpanel"` with `aria-labelledby` back to its tab. Roving
`tabindex` (only the active tab is `tabindex="0"`) so `Tab` key focus enters/exits
the strip cleanly and Left/Right arrow cycles tabs without hijacking browser Tab
semantics.

Activation model: **manual**, not automatic — arrow keys move focus within the
tablist without switching the visible tab; `Enter`/`Space` commits the switch. This
is the ARIA guide's recommended choice specifically for cases where activation has a
non-trivial cost (per the guide: prefer manual when switching triggers expensive
work) — directly applicable here, since switching a terminal or agent tab can mean
resuming a live session, not just toggling CSS visibility.

**Resolved shortcuts** (§7 open question 4 — checked directly against the existing
registry, `frontend/app/store/keymodel.ts`, to avoid collisions rather than guess).
The existing registry already has a clean, consistent convention worth extending
rather than inventing a new one: `Cmd:`-prefixed chords (translated per-platform by
`keyutil.ts`) are Window-Tab/workspace-level actions (`Cmd:t` new Window Tab,
`Cmd:[`/`Cmd:]` prev/next Window Tab, `Cmd:1`-`Cmd:9` Window Tab by index); literal
`Ctrl:`-prefixed chords (not translated — always Ctrl, even on macOS) are Pane-level
actions (`Ctrl:[`/`Ctrl:]` cycle Pane focus, `Ctrl:Shift:Digit1`-`9` Pane-by-index,
`Ctrl:Shift:Arrow*` Pane focus by direction). Pane Tab actions extend the `Ctrl:`
family one level deeper, adding a modifier per action type:

| Action | Chord | Collision check |
|---|---|---|
| Next/previous Pane Tab in focused Pane | `Ctrl:Shift:]` / `Ctrl:Shift:[` | Free — `Ctrl:]`/`Ctrl:[` (Pane cycle) and `Ctrl:Shift:Digit1-9` are taken; `Ctrl:Shift:]`/`[` are not |
| New Pane Tab in focused Pane (opens §4.5 picker) | `Ctrl:Shift:T` | Free — `Cmd:t` (new Window Tab) is taken, `Ctrl:Shift:T` is not |
| Reorder active Pane Tab left/right within its strip (§4.8's Phase 3 keyboard equivalent) | `Ctrl:Alt:[` / `Ctrl:Alt:]` | Free |
| Close active Pane Tab in focused Pane | **Reuses existing `Cmd:w`** — no new chord | Not a new binding: `Cmd:w`'s handler (`keymodel.ts:88-91`) is redefined to close the ACTIVE Pane Tab, falling through to today's "close the whole pane" behavior only when it's the last remaining Pane Tab — the same semantic `Cmd+W` already has in every browser/editor that has both tabs and windows |

`docs/specs/SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md` §9.1's own collision notes
(`Ctrl:K` taken, `Ctrl:P` is the command palette, `Ctrl:R` collides with browser
reload) were also checked — none of the above touch those.

### 4.10 Visual design of the tab strip itself

- **Overflow:** shrink-with-a-floor (do not let a tab shrink below a legible label
  width — commonly cited around 80px) then horizontal scroll once the floor is hit.
  Rejected: cmux's shipped fixed-width-no-shrink model (an open complaint in their
  own tracker, issue #5091) and unbounded shrink (illegible tabs). This is a default,
  not a hard constraint — expose as a setting later if real usage shows a need for
  VS Code's alternative `wrapTabs` behavior (§3 point 6).
- **Close button:** on-hover per-tab, matching the existing workspace tab bar's
  established pattern (`frontend/app/tab/tab.tsx`) for visual consistency between the
  two "tab" layers despite them being structurally different (§2.2).
- **Status/badge hook:** the tab-strip's per-tab data model includes an optional
  status field from day one (§4.6) even though no widget type populates it yet in
  this spec's scope — prevents a breaking schema change when a future spec wires up
  agent-needs-attention/terminal-bell badges (prior art: Warp's pane badge overlay,
  VS Code's decoration badges).
- **"+" button:** trailing the last tab, opens the §4.5 picker.

### 4.11 Where the Pane / Pane-Tab distinction (§2.1) ripples through the codebase

This is a vocabulary and mental-model correction, not just a UI change, and per the
discussion that produced it, it touches more files than §4.1-4.10 individually cite.
Listed here as one place to audit against, rather than scattered as asides:

- **`NodeModel`** (`frontend/layout/lib/types.ts:325`) — already structurally
  correct (a leaf-level `NodeModel` plus `blockId`/`activeBlockId`/`blockStack`), but
  its own doc comments and every place it's described in prose should read as "the
  Pane's model," not "the block's model" — several existing comments in
  `layoutStack.ts`/`pane-leaf-chrome.tsx` already drifted into "the block" language
  for what is properly "the Pane."
- **`BlockFrame`/`BlockFrame_Header`** (`frontend/app/block/blockframe.tsx`) — under
  this spec's terminology, what this file renders is closer to "a Pane Tab's
  content frame," not "a block's frame" (the noun "Block" stays as the backend's
  durable-object name, §2.1 — this is about the frontend-facing component name and
  its doc comments, not the schema). Not renamed by this spec (real rename work,
  large blast radius, no functional need) — flagged as a future naming-audit
  candidate, not mandated.
- **`widgets.json` / `CLAUDE.md`'s Widgets table** — the existing table (and this
  repo's own house documentation) describes each `defwidget@X` entry as producing
  "a widget" that appears "in a pane." Under this spec, the precise statement is "a
  widget's `blockdef` creates a Pane Tab, which lives inside a Pane" — worth a
  documentation pass alongside implementation so a future contributor reading
  `CLAUDE.md` doesn't reinherit the old, looser Pane/widget conflation this spec is
  explicitly correcting.
- **This spec's own earlier draft** — §4.2's original version (before this
  correction) itself conflated "the currently focused pane" with "where a plain
  widget-bar click should land," which is exactly the kind of mistake precise
  Pane/Pane-Tab vocabulary is meant to prevent going forward. Left visible in this
  document's history (not scrubbed) as a concrete example of the failure mode.
- **Every `ViewModel` onboarded in §5's phased rollout** — each one's own doc
  comments/README-equivalent should describe its content as "this widget type's
  Pane Tab content," establishing the vocabulary at the point of authorship rather
  than needing a later cleanup pass.

---

## 5. Phased rollout

Rollout is per-widget-type, not a single flag flip — chrome hoisting (§4.3) is real
per-`ViewModel` engineering work, and §3 point 4's "uniform from day one" commitment
applies WITHIN a widget type's rollout (heterogeneous stacking, generic "+", split
alternatives — all present the moment a type ships), not ACROSS the whole widget
catalog at once. The temporary asymmetry between rolled-out and not-yet-rolled-out
widget types during this rollout is accepted explicitly, not glossed over — see the
close-out criterion at the end of this section.

- **Phase 0 (groundwork, no user-visible change):** land the generic
  `addWidgetToFocusedPane` primitive (§4.2) and the generic "+" picker plumbing
  (§4.5) behind the existing `HOISTS_OWN_CHROME` allowlist, so agent/term (already
  hoisted) can validate the new code paths without any other widget type being
  affected yet.
- **Phase 1 (first universal-tab-strip widget types):** always-on tab strip (§4.1,
  default `always`) ships for agent, term (already hoisted — no new chrome work), plus
  the two next-highest-usage types with the most tractable chrome-hoisting lift:
  **browser** and **editor**. Editor's existing files-tab mechanism (§2.4) is migrated
  onto real `blockStack` semantics as part of this phase rather than left as a
  second, parallel tab system.
- **Phase 2:** remaining widget types in usage order: **sysinfo, swarm, armory**.
- **Phase 3:** same-pane tab reorder (drag + keyboard, §4.8/§4.9) ships once Phase 1
  types have had real usage to validate the interaction model on.
- **Phase 4 (remaining low-traffic types + drag maturity):** **media, drone, help,
  warden** complete the rollout; cross-pane drag / tear-off-into-new-pane (§4.8)
  ships only after `SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` lands.
- **Close-out criterion:** the rollout is "done" (asymmetry fully closed) when every
  widget type in `widgets.json` supports being added as a tab to an existing pane via
  the same generic mechanism — tracked as a literal checklist against the widget
  table in `CLAUDE.md`, not left open-ended.

---

## 6. Non-goals

- Changing the workspace-level tab bar (`tabbar.tsx`) in any way — this spec is
  entirely about the inner, per-pane layer (§2.2).
- Any change to floating-pane or new-window mechanics — reused unchanged (§4.7).
- A global transactional/epoch-based render model — already proposed and rejected in
  `SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md` §3.1/3.2 after four review
  passes; not re-litigated here.
- Resolving `SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md`'s six diagnosed
  defects — a separate, already-tracked effort this spec deliberately stays clear of
  (§4.8).
- Switching AgentMux's outer split-tree model from "panes contain tabs" to Zellij's
  inverse "tabs contain panes" — considered and rejected (§3 point 7).
- Full visual-design polish (exact colors, spacing, animation timing) — left to
  implementation/design review, consistent with how the predecessor spec's own
  polish cluster (`SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`,
  `SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md`, etc.) was handled as separate,
  focused follow-ups rather than baked into the original architecture spec.

---

## 7. Open questions — RESOLVED (2026-09-17, before implementation started)

All five were open pending sign-off; the repo owner delegated the decision back to
the implementer ("resolve any open questions before implementing... use best
judgement"). Resolved as follows, each with the reasoning that makes it a
defensible default rather than a coin-flip:

1. **Default for `pane:tabstrip`** (§4.1) — *superseded 2026-09-24: the
   setting was removed and `always` is now the only behavior
   (`SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md` §8).*
   Original resolution: **`always`.** Matches the literal
   original ask verbatim ("the pane header... will be turned into a row of tabs")
   and cmux's own shipped default (every pane always has a strip, regardless of
   tab count). `multi-only` remains available as a user setting (§4.1), just not
   the default.
2. **`Block`/`BlockFrame`/widgets.json naming audit scope** (§4.11) —
   **doc-comments-and-prose only, no component renames, in this pass.** A real
   `BlockFrame` → Pane-Tab-flavored rename is a large, blast-radius-heavy, purely
   cosmetic change with zero user-facing benefit on its own; bundling it into an
   already-large functional redesign multiplies review risk for no functional
   gain. Tracked as a legitimate, separate future cleanup, not folded in here.
3. **Editor files-tabs migration onto `blockStack`** — **yes, migrate, as part of
   Phase 1** (confirmed, not just recommended). Keeping the editor's bespoke
   files-tab mechanism (`editor-pane-state-store.ts`) running in parallel with the
   new generic Pane Tab strip would be exactly the kind of "some widget types work
   one way, others work a different way" asymmetry §3 point 4 (cmux issue #2770)
   warns against — and it would do so on day one of Phase 1, not even as a
   temporary rollout artifact. Each open file becomes its own Pane Tab (its own
   `Block`), consistent with every other widget type and with VS Code's own model
   (an editor group's file tabs ARE the group's tabs, not a nested sub-mechanism).
4. **Exact keyboard shortcut bindings** — **resolved with a concrete, collision-checked
   table, §4.9.** Read the actual registry (`keymodel.ts`) rather than guessing;
   found and extended its existing `Cmd:`-for-Window-Tab / `Ctrl:`-for-Pane
   convention one level deeper for Pane Tab actions, and redefined `Cmd:w`'s
   existing close semantics to mean "close the active Pane Tab, falling through to
   closing the Pane only when it's the last one" — the same semantic every
   tabs-plus-windows app already gives `Cmd+W`, not a new binding.
5. **Tearing off ONE Pane Tab from a multi-tab Pane into a floating window/new
   window** — **allowed, as originally recommended.** Implemented as
   `closeBlockInStack` on the origin Pane + the existing, unmodified floating/
   new-window creation flow on the torn-off block. This is genuinely new
   interaction between two previously-independent features, but disallowing it
   would be a worse, more surprising UX (a user would have to first notice their
   target block is "stuck" in a stack before understanding why tear-off doesn't
   work) than handling it — and the mechanism composes cleanly from primitives that
   already exist independently.

---

## References

- [cmux.com](https://cmux.com/) — [Concepts](https://cmux.com/docs/concepts),
  [Keyboard Shortcuts](https://cmux.com/docs/keyboard-shortcuts)
- [github.com/manaflow-ai/cmux](https://github.com/manaflow-ai/cmux) — issues #5091
  (fixed-width tabs), #205 (auto-reorder regression, closed via PR #215), #2770
  (asymmetric tab-in-pane support across content types), PR #12612 (drag focus-drift
  fix), Discussion #1404 (pane-contains-tabs vs. tabs-contain-panes)
- [W3C WAI-ARIA APG — Tabs Pattern](https://www.w3.org/WAI/ARIA/apg/patterns/tabs/)
- [VS Code — User Interface docs](https://code.visualstudio.com/docs/editing/userinterface);
  [tab overflow UX discussion](https://github.com/microsoft/vscode/issues/7987)
- [Zellij — Pane and Tab Management](https://deepwiki.com/zellij-org/zellij/2.3-pane-and-tab-management)
- [Warp — Vertical Tabs / Panes docs](https://docs.warp.dev/terminal/windows/vertical-tabs/)
- [JetBrains — Drag & Drop Editor Tabs](https://www.jetbrains.com/guide/go/tips/drag-and-drop-editor-tabs/)
- `docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` and its polish
  cluster (see header block above)
- `docs/specs/SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md`
- `docs/specs/SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md`
- `docs/specs/SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md`
- `docs/specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md`
- `docs/architecture/PANE_LAYOUT_AND_REFLOW_ARCHITECTURE.md`
