# SPEC: Pane tab strip — compact (shrink-to-fit) sizing + double-click rename

**Date:** 2026-07-22
**Status:** Implemented — §4 resolved via option 2 (dedicated `renameagentdefinitiontitle` RPC)
**Scope:** `frontend/app/element/PaneTabStrip.tsx` / `.scss` (shared strip), agent-pane fork
tabs (`frontend/app/view/agent/agent-view.tsx`, `frontend/app/view/agent/fork/**`), terminal-pane
tabs (`frontend/app/view/term/term.tsx`)
**Author:** Agent3
**Related:**
`docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` (introduced the shared
`PaneTabStrip` component and both consumers this spec refines — Phases 1–5, all merged),
`frontend/app/tab/tab-measure.ts` (the window-tab-bar's existing content-width-measurement
precedent, reused conceptually below),
`frontend/app/block/titlebar.tsx` (the existing per-block click-to-rename precedent, reused
conceptually below)

---

## 1. Intent

Three follow-up polish items on the tab strip shipped by
`SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md`, stated by the user directly:

1. **Shrink-to-fit strip.** When an agent or terminal pane has only one tab open, the strip
   should show just that tab (or, per the user's literal wording, just the `+`) hugging the
   left edge — not a full-pane-width bar with dead, blank space trailing after the `+`. "The
   bar grows as it is used, not a whole bar of blank."
2. **Content-sized tabs.** When `+` is clicked and a new tab opens, that tab's width should be
   exactly the width of its label (plus padding) — not a fixed/flex-stretched width unrelated
   to the text it holds.
3. **Double-click to rename.** A tab should be renamable by double-clicking it.

## 2. Current state (verified against `main` @ `6ab7e60cc`)

### 2.1 Why the strip shows dead space today

`frontend/app/element/PaneTabStrip.scss`:

```scss
.pane-tab-strip {
    display: flex;
    flex-direction: row;
    // no explicit width — a block-level flex container defaults to 100% of
    // its parent, i.e. the full pane width.
}

.pane-tab-tip {           // the Tooltip wrapper that carries the tab's flex sizing
    flex: 1 1 140px;
    min-width: 80px;
    max-width: 200px;
}

.pane-tab-strip-add {     // the trailing "+"
    flex: 0 0 auto;
    width: 28px;
}
```

With one tab open, the tab's `flex: 1 1 140px` grows to fill available space, capped at
`max-width: 200px`. The `+` (`flex: 0 0 auto`, 28px) sits immediately after it. Neither item can
absorb the pane's remaining width past that point, so the flex container — which *is* full pane
width — ends with unclaimed, unstyled background trailing the `+` out to the pane's right edge.
That trailing area is the reported "dead space."

### 2.2 Why tabs don't size to their label today

Same block: `flex: 1 1 140px; min-width: 80px; max-width: 200px` on `.pane-tab-tip`, plus
`.pane-tab { flex: 1 1 auto; width: 100%; }` on the tab itself. Every tab's width is a function
of *available flex space distributed across siblings*, clamped to an 80–200px band — never a
function of the label's actual rendered width. A tab named "x" and a tab named "Claude Code
Review Session #4" get the same width today (both hit the 80–200px band based on sibling count,
not content).

### 2.3 Existing precedent for content-sized tabs: `frontend/app/tab/tab-measure.ts`

The *window*-level tab bar (`frontend/app/tab/tab.tsx`, a different, older tab strip — the
outer app's workspace switcher, not `PaneTabStrip`) already solves "tab width = label width" via
`measureTabWidth(label)`: a singleton `<canvas>` 2D context measures the label's rendered text
width (no DOM reflow), adds a fixed non-text padding budget (52px — left/right pad + gap +
close-button + slack), and clamps the result to `[TAB_STANDARD_WIDTH=232, TAB_MAX_WIDTH=260]`,
with a floor at `TAB_STANDARD_WIDTH` so short labels don't shrink below a normal-looking tab.
The result is written to a `--tab-natural-width` CSS custom property consumed by drag-reorder
layout math.

That canvas-measurement machinery exists there specifically because that tab bar supports
drag-to-reorder, which needs a precise numeric width up front for transform math. `PaneTabStrip`
has no drag-reorder. **Recommendation: don't port the canvas-measurement approach — use plain
CSS shrink-to-fit instead** (§3.2), which is simpler and gets the same visual result without a
JS measurement pass on every rename/relabel.

### 2.4 Existing precedent for double-click rename

Two real, shipped patterns to choose between:

- **Inline `<input>` swap** (`frontend/app/view/editor/editor-tab-strip.tsx`'s Save-As flow):
  `PaneTabStrip`'s existing `renderLabel?: (tab: T) => JSX.Element` prop lets a caller swap the
  plain `<span class="pane-tab-label">` for a custom element. The editor uses this today (for
  Save-As, not rename) with a local `SaveAsInput` component: autofocuses on mount, tracks value
  via a signal, commits on Enter, cancels on Escape, commits-as-cancel on blur, with a
  `committed` flag guarding double-fire between blur and Enter.
- **`contentEditable` toggle** (`frontend/app/tab/tab.tsx`'s window-tab rename): a
  `contentEditable={isEditable()}` div toggled on `onDblClick`, auto-selected on entry, commits
  via `ObjectService.UpdateTabName` on blur, Enter confirms + blurs, Escape reverts text + blurs.

`PaneTabStrip` already exposes `onTabDoubleClick?: (tab: T) => void` (used today by the editor to
pin a preview tab) and `renderLabel` (used today for Save-As). **No changes to the shared
component are needed** — rename is entirely a per-consumer concern, following the editor's
`renderLabel`-swap pattern (§2.4's first bullet), since it composes more simply with
`getTooltip`/`getTabClass` than a raw `contentEditable` node would.

### 2.5 Persistence: where would a renamed label actually get saved?

**Agent fork tabs** — `ForkSetEntry.title` (`frontend/app/view/agent/fork/fork-set.ts`,
`titleOf()`) resolves to `branch_label` when set, else `AgentDefinition.name`. The only mutation
RPC is `updateagent` (`agentmux-srv/src/server/agent_handlers/core.rs`,
`RpcApi.UpdateAgentDefinitionCommand`), which can change `name` — but **`branch_label` is
explicitly immutable post-insert**:

```rust
// parent_id + branch_label describe provenance and
// are immutable post-insert (forks are separate rows,
// not in-place edits).
parent_id: old.parent_id.clone(),
branch_label: old.branch_label.clone(),
```

This is a real conflict with the rename feature: **for any tab whose `branch_label` is set (every
fork except a lineage root), renaming via `name` would have zero visible effect**, because
`titleOf()` prefers `branch_label` over `name` unconditionally. §4 calls this out as an open
question — it needs a product decision, not a workaround.

**Terminal tabs** — `frontend/app/view/term/term.tsx`'s `termTabs` memo synthesizes
`Terminal ${i+1}` unconditionally today; there is no persisted label at all. The code already
anticipates this as a follow-up:

```ts
// Position-based labels ("Terminal 1", "Terminal 2", …) — matches
// common terminal-app convention for an unnamed session. ... a
// cwd-derived or user-renamable label is a follow-up, not something
// cheaply available here yet.
```

A real persistence mechanism already exists and fits without inventing anything new: per-block
`meta["pane-title"]`, set via `RpcApi.SetMetaCommand(client, { oref: WOS.makeORef("block", id),
meta: {"pane-title": title} })` — this is exactly what `frontend/app/block/titlebar.tsx`'s
existing click-to-edit pane title already does. Block meta is readable independent of whether a
block's view is mounted (`WOS.getObjectValue(oref)`), which matters here: a dormant (non-active)
terminal tab's block has no live `TermViewModel` to read a title from (per `termTabs`'s own
comment), but its meta is always fetchable directly from the WOS object cache regardless of
mount state.

## 3. Proposed design

### 3.1 Shrink-to-fit strip

`.pane-tab-strip` stops being an implicit full-width block-level flex container and becomes
shrink-to-fit:

```scss
.pane-tab-strip {
    display: inline-flex;   // was: flex — shrinks to content width instead of filling the pane
    max-width: 100%;        // still can't overflow the pane; falls back to internal scroll/ellipsis
    flex-direction: row;
    align-items: stretch;
    height: 28px;
    border-bottom: 1px solid var(--border-color);
    background: var(--secondary-bg-color, var(--block-bg-color));
}
```

One tab today would render as: `[tab sized to its label] [+ 28px]`, full stop — no trailing
background. As more tabs open, the strip naturally grows tab-by-tab; it only reaches full pane
width once enough tabs are open to fill it, and only then does horizontal overflow/scroll
behavior (already `overflow-x: hidden` today — worth revisiting to a scroll-on-overflow pattern
in a later pass, out of scope here) kick in.

Callers that render the strip inside a parent using `justify-content` or that assumed a
full-width strip (none currently do — both `agent-view.tsx` and `term.tsx` just insert
`<PaneTabStrip>` as a flex child of a column layout) are unaffected; `inline-flex` still
participates in the parent's block/flex flow normally, it just no longer *claims* full width for
itself.

### 3.2 Content-sized tabs

Replace the flex-grow/clamp band with shrink-to-fit + a reasonable upper bound, so a single very
long label can't push a tab arbitrarily wide:

```scss
.pane-tab-tip {
    flex: 0 0 auto;         // was: flex: 1 1 140px
    max-width: 240px;       // was: 200px cap; kept as a ceiling, no floor
    display: flex;
    align-items: stretch;
}

.pane-tab {
    flex: 0 0 auto;         // was: flex: 1 1 auto
    width: auto;            // was: width: 100%
    min-width: 0;
    // ... padding/gap/etc. unchanged — these already determine the tab's
    // "natural" width once flex-grow is no longer forcing it wider.
}
```

`.pane-tab-label`'s existing `overflow: hidden; text-overflow: ellipsis; white-space: nowrap;`
still applies once a label would exceed the 240px ceiling, so pathological names degrade the same
way they do today (truncated + full text in the hover tooltip) rather than blowing out the strip.

No JS measurement pass needed (§2.3) — the browser's own intrinsic/shrink-to-fit flex sizing
does this for free once `flex: 1 1 …` stops forcing growth.

**Interaction with §3.1:** with both changes, a lone tab renders at its natural (short) width,
immediately followed by `+`, immediately followed by nothing — matching the user's stated
target exactly ("only show the + at the left ... no dead space").

### 3.3 Double-click to rename

No changes to `PaneTabStrip.tsx` itself. Per consumer:

**Terminal tabs** (`term.tsx`):
- `termTabs` memo gains a per-stack-member label read: for each `id` in `stack`, read
  `WOS.getObjectValue<Block>(WOS.makeORef("block", id))?.meta?.["pane-title"]`, falling back to
  today's `Terminal ${i + 1}` when unset. This makes the memo depend on block meta as well as
  layout state — needs a reactive read (the existing `wos.ts` object-value accessors are already
  Solid-signal-backed for the currently-open blocks, matching the pattern `blockData()` already
  uses for the pane's own block).
- New local `renamingBlockId` signal (mirrors editor's `saveAsTabId`), set via
  `onTabDoubleClick={(tab) => setRenamingBlockId(tab.blockId)}`.
- `renderLabel` swaps in an inline `<input>` (same commit/cancel/blur contract as the editor's
  `SaveAsInput`) when `tab.blockId === renamingBlockId()`; on confirm, calls
  `RpcApi.SetMetaCommand(TabRpcClient, { oref: WOS.makeORef("block", tab.blockId), meta: {
  "pane-title": trimmedValue } })`.

**Agent fork tabs** (`agent-view.tsx`) — blocked on the §4 decision below. Once resolved, the
wiring is the same shape: a `renamingDefinitionId` signal, `onTabDoubleClick`, a `renderLabel`
swap, and a commit handler that calls `RpcApi.UpdateAgentDefinitionCommand` with whichever field
the §4 decision lands on.

## 4. Open question — how does renaming a *fork* tab actually persist?

`branch_label` is immutable by explicit prior design decision (§2.5), and it's also the field
`titleOf()` prefers whenever it's set — which is every fork tab except a lineage root. Renaming
a fork tab therefore cannot be implemented by calling today's `updateagent` RPC alone; it would
silently no-op for exactly the tabs users are most likely to want to rename (the forks, not the
original). Three ways to resolve this, for the user to pick before agent-tab rename is
implemented:

1. **Make `branch_label` mutable via `updateagent`.** Reverses the earlier "immutable
   provenance" decision. Simplest change, but that decision may have been made for a reason not
   visible from the code alone (e.g. some other part of the system keying off `branch_label` as
   a stable identifier) — worth confirming it's safe before reversing.
2. **Add a dedicated `renameagentfork` RPC** that updates only `branch_label`, leaving
   `updateagent`'s immutability contract untouched for every other caller. More backend surface,
   but the immutability guarantee stays intact for non-rename callers.
3. **Restrict double-click-rename to the root tab only** (the one with no `branch_label`, where
   renaming `name` already changes the visible title today with zero backend changes). Fork tabs
   would not be renamable via this feature. Ships fastest; likely reads as an inconsistent
   limitation to users ("why can I rename this tab but not that one").

**Recommendation: option 2.** It's a small, additive RPC, doesn't require re-litigating the
provenance-immutability decision, and gives every tab (root or fork) the same renamable
behavior the user asked for.

## 5. Suggested phasing

1. **Phase A — sizing only** (§3.1 + §3.2): pure CSS, no backend, no open questions, affects
   editor/agent/terminal strips identically since they share `PaneTabStrip.scss`. Lowest risk,
   ships immediately.
2. **Phase B — terminal tab rename** (§3.3, terminal half): no open questions, reuses existing
   `pane-title` meta + `SetMetaCommand` plumbing verified in §2.5.
3. **Phase C — agent fork tab rename** (§3.3, agent half): blocked on §4's decision.

## 6. Verification plan

- Manual, in `task dev`: open a single agent pane → confirm the fork strip is exactly
  `[agent name tab][+]` wide with no trailing space. Fork it (or open a second terminal tab) →
  confirm the strip grows by exactly the new tab's natural width. Open a tab with a long name →
  confirm it truncates at the 240px ceiling with a working hover tooltip showing the full text.
- Double-click a terminal tab → confirm the input appears, Enter commits and the label persists
  across a pane close/reopen (meta survives), Escape reverts, blur commits (matching the
  Save-As/`titlebar.tsx` convention already used elsewhere).
- `npx vitest run app/view/agent app/view/term app/element` + `npx tsc --noEmit` — no regressions
  in the existing `PaneTabStrip.test.tsx` (11 tests) or the editor tab strip's own tests, which
  exercise the sizing CSS indirectly via class assertions, not layout math, so should be
  unaffected by the SCSS-only sizing change.
