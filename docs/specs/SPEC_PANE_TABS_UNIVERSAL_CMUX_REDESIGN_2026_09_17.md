# SPEC: Universal Pane Tabs — Every Pane Header Becomes a Tab Strip (cmux-Style Redesign)

**Date:** 2026-09-17
**Status:** Draft — analysis complete, phased implementation proposed
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

### 2.1 Vocabulary, confirmed from the codebase

| Term | What it is | Where |
|---|---|---|
| **Block** | The durable content object — `{oid, parentoref, version, runtimeopts, meta, subblockids, ...}`. One `Block` = one widget *instance*. | `agentmux-srv/src/backend/obj.rs:480-495` |
| **Widget** (`defwidget@X`) | A template in `widgets.json` (`blockdef` + display metadata) used to CREATE a `Block`. Not itself persisted once instantiated. | `agentmux-srv/src/config/widgets.json` |
| **LayoutNode** | A split-tree position (leaf or split-container). A *leaf* LayoutNode is what most people mean by "pane." | `agentmux-common/src/layout_types.rs:71-91` (frontend mirror: `frontend/layout/lib/types.ts:185`) |
| **Tab** (workspace-level) | The OUTER, browser-style top tab bar — one `Tab` = one whole split-pane tree (one `LayoutModel`). **Not** what this spec is about — see §2.2. | `agentmux-srv/src/backend/obj.rs:408-419`, `frontend/app/tab/tabbar.tsx` |
| **Pane tab** (this spec's subject) | A member of a LayoutNode leaf's `block_stack` — multiple Blocks sharing one leaf, switched via a strip rendered in that leaf's header. | `agentmux-common/src/layout_types.rs:32-56`, `frontend/layout/lib/layoutStack.ts` |

### 2.2 Disambiguation: two different things are called "tabs" in this codebase

`SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` already had to resolve exactly this
naming collision when it shipped, and this spec inherits the same disambiguation:

- **Workspace-level tabs** (`frontend/app/tab/tabbar.tsx`) — the top-of-window
  browser-style strip. Each one owns an entire split-pane tree (its own
  `LayoutModel`). Out of scope for this spec; unchanged.
- **Pane tabs** (this spec) — a strip rendered *inside* one pane (one LayoutNode
  leaf's header), switching between blocks that share that leaf. This is the layer
  being generalized.

Do not let an implementer or reviewer conflate the two — every mention of "tab" in
this document means the inner, per-pane kind unless explicitly qualified as
"workspace tab."

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
  split, with no notion of "the currently focused pane." This is the exact call site
  this spec repoints (§4.2).

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

Exposed as a setting (`pane:tabstrip = always | multi-only`, default `always`) purely
for the case of hiding the tab-pill-as-title distinction when a pane has exactly one
tab and a user prefers the older plain-title look — but note this setting, if
`multi-only`, controls the row's *content* (one plain title vs. one tab pill), not
whether there are one or two rows; the two-row structure itself is not coming back
under any setting. Mirrors VS Code's own `workbench.editor.showTabs` precedent
(default on, user-overridable) for the general shape of offering this as a setting
rather than a hard constraint.

### 4.2 Left-click "add widget" becomes "add as tab to the focused pane"

`handleWidgetSelect` → `createBlock` (`block-layout-actions.ts:68-88`) currently
always dispatches `InsertNode` (new split). This spec changes the default path to:

```ts
export async function addWidgetToFocusedPane(blockDef: BlockDef): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const blockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    markBlockRecentlyCreated(blockId);
    const focusedNodeId = layoutModel.treeState.focusedNodeId;
    if (focusedNodeId) {
        pushBlockOntoStack(layoutModel, focusedNodeId, blockId);
    } else {
        // No focused pane (empty layout, or focus not yet established) — fall
        // back to today's behavior: a fresh top-level pane.
        layoutModel.treeReducer({ type: LayoutTreeActionType.InsertNode, node: newLayoutNode(undefined, undefined, undefined, { blockId }), magnified: false, focused: true });
    }
    return blockId;
}
```

`pushBlockOntoStack` (`frontend/layout/lib/layoutStack.ts:61`) already does exactly
what's needed — attach an already-created block to a leaf's stack and activate it —
and is already view-agnostic. **The explicit "I want a new split, not a tab" path is
preserved, not removed**: the right-click widget-bar menu (`action-widgets-menu.ts`,
this session's own `buildWidgetOpenActions()`) gains two new entries, **"Split
Right"** and **"Split Down"**, driving the existing `createBlockSplitHorizontally`/
`createBlockSplitVertically` primitives (`block-layout-actions.ts:24-66`) unchanged.
A modifier-click (Option/Alt+click, matching the common editor convention of
modifier-click-to-open-in-split) is a candidate fast-path for split-without-a-menu,
flagged as an open question in §7 rather than mandated here.

**Backend note:** no new backend schema is needed (§2.3), but a generic reducer path
for "push an arbitrary newly-created block onto an existing leaf's stack" needs to
exist for non-agent/non-term widget types — today only agent-fork and
terminal-shell-tab creation populate `block_stack` at all. This is new server-side
work (a `Command` variant or an extension of the existing `pane.open` RPC to accept
a `stack_onto_node_id`/similar parameter), scoped per-widget-type as each type rolls
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

### 4.5 New-tab affordance: a "+" at the end of every pane's strip, uniform across types

`PaneTabStrip` already has an `onAdd` prop, but today it's wired per-view-type to
narrow, type-specific behavior (agent forks a new agent; term opens a new shell).
This spec generalizes it: every pane's "+" opens the same **widget picker** the
widget bar's right-click menu already builds (`buildPaneWidgetMenuItems`,
`action-widgets-config.ts:180-216` — already takes an `onSelect(blockdef)` callback
of exactly the right shape), filtered to widget types that have completed §5 rollout.
Selecting a widget from the picker calls `pushBlockOntoStack` against the CURRENT
pane (not the focused pane generally — the pane whose "+" was clicked), reusing
§4.2's primitive.

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

Proposed shortcuts (scoped to "within the focused pane," to be reconciled against
AgentMux's existing keybinding config during implementation — not audited as part of
this spec):
- Next/previous tab in focused pane
- Close current tab in focused pane
- New tab in focused pane (opens the §4.5 picker)
- Reorder current tab left/right within its strip (the Phase 3 keyboard equivalent
  required by §4.8)

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

## 7. Open questions — needs sign-off before implementation starts

1. **Default for `pane:tabstrip`** (§4.1) — `always` (this spec's recommendation,
   matching the literal ask: "the pane header... will be turned into a row of tabs")
   vs. `multi-only` (denser, but means the header component visually changes
   identity when tab count crosses 1↔2).
2. **Modifier-click for split** (§4.2) — should Option/Alt+click on a widget-bar
   entry be a fast-path for "split instead of add-as-tab," or should split always
   require the right-click menu / a dedicated button? No existing AgentMux
   convention was found for modifier-click during this spec's research; needs a
   decision rather than an assumption.
3. **Editor files-tabs migration onto `blockStack`** (§2.4, §5 Phase 1) — confirmed
   as this spec's recommendation, but is real migration work (editor's tab state
   currently lives in `editor-pane-state-store.ts`, not `layoutStack.ts`) that
   needs its own scoping pass before Phase 1 starts.
4. **Exact keyboard shortcut bindings** (§4.9) — proposed set needs reconciliation
   against whatever global keybinding config/conflicts already exist; not audited
   here.
5. **Tearing off ONE tab from a multi-tab pane into a floating window/new window** —
   today's "Open in Floating Pane"/"Open in New Window" actions operate on a whole
   block being created fresh (this session's Phase 2 widget-menu work). Once a block
   can live inside another pane's stack, does tearing it off remove it from that
   stack (leaving the origin pane down to N-1 tabs) or is it disallowed for
   already-stacked blocks in an early phase? Recommendation: allow it, treating "tear
   off" as `closeBlockInStack` on the origin + the existing floating/new-window
   creation flow on the torn-off block — but this needs explicit sign-off since it's
   new interaction between two previously-independent features.

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
