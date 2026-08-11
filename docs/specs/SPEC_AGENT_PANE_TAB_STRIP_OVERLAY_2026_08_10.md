# SPEC: Agent pane tab strip floats over the conversation, doesn't reserve a row

**Date:** 2026-08-10
**Status:** implemented.
**Related:** `docs/specs/SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`
(shrink-to-fit width + hide-single-tab-pill, already shipped, unchanged by
this spec), `docs/specs/SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md`
(same investigation thread, same branch/PR — landed first).

---

## 0. Ask, and how this differs from what was already fixed

> the goal is to be able to see more of the conversation while no tabs are
> open. the conversation should be totally unobstructed except for the +
> sign

This is a different, larger ask than the tab-strip transparency fix already
confirmed on `main` (PRs #2282, #2289). That fix made the strip's own
*background* transparent and its *width* shrink-to-fit — real, already-
shipped improvements — but the strip was still a normal flex child of
`.agent-pane-stack`, meaning it always reserved its own 28px-tall **row**,
full pane width in terms of layout impact, regardless of how little of that
row actually had visible content. The conversation could never render (or
scroll) into that reserved space, transparent background or not — a
reserved-but-empty row and true "unobstructed" are different things. This
spec closes that remaining gap, flagged as an open, larger question in
`REPORT_AGENT_PANE_TAB_STRIP_TRANSPARENCY_FEASIBILITY_2026_08_10.md` §6
before the user confirmed it's actually wanted.

---

## 1. Design: overlay, not reserved row

Moved `<PaneTabStrip>` from a sibling-before `.agent-pane-stack-content` (a
normal flex child, reserving height) to a child *inside*
`.agent-pane-stack-content`, absolutely positioned over it:

```scss
.agent-pane-stack-content {
    flex: 1 1 auto;
    min-height: 0;
    position: relative;

    > .pane-tab-strip {
        position: absolute;
        top: 0;
        left: 0;
        z-index: var(--z-pane-overlay, 4);
    }
}
```

`.agent-view` (the conversation + composer, `AgentPresentationView`'s root)
is now a plain sibling of `.pane-tab-strip` inside `.agent-pane-stack-content`,
filling that box entirely — the tab strip floats on top of it rather than
pushing it down. With one conversation open, `PaneTabStrip.scss`'s existing
shrink-to-fit + hide-single-tab-pill behavior (unchanged, still fully in
effect) means the strip's own box is exactly the `+` button's 28×28px —
nothing else in that corner intercepts clicks or paints over anything,
because nothing else is there. The conversation renders, and scrolls,
underneath the rest of that row.

### 1.1 Why this doesn't affect the editor or terminal panes

`PaneTabStrip` is shared (`frontend/app/element/PaneTabStrip.tsx`/`.scss`,
also used by `editor-tab-strip.tsx` and `term.tsx`). This spec does **not**
touch the shared component's own base rule — the new `position: absolute`
override is scoped to `.agent-pane-stack-content > .pane-tab-strip`, a
selector that only matches when the strip is nested exactly where this
spec's JSX change puts it (agent panes specifically). Editor and terminal
tab strips keep reserving their own row, unchanged, unless a future spec
asks for the same treatment there.

### 1.2 Stacking order — why `z-index: var(--z-pane-overlay, 4)` is enough

`.agent-document-scroll-region` (`_document.scss`) has internal children
using `z-index` up to 100 — at first glance a risk that conversation
content could paint over the floating strip. It isn't: that selector sets
`position: relative; z-index: 0` together, which (per the CSS stacking-
context spec) establishes a **new stacking context** — its children's
z-index values are compared only against each other, isolated from
anything outside that box. `.agent-view` itself has `position: relative`
but no explicit `z-index`, so it doesn't establish its own stacking context
either; the strip (an explicit, positioned `z-index: 4`) only needs to
clear `.agent-view`'s effectively-`auto` stacking to paint above it, which
it does regardless of DOM order (the strip is now written *before*
`.agent-view` in JSX, but stacking order for positioned elements doesn't
depend on source order once z-index is explicit).

### 1.3 Accepted tradeoff: minor overlap directly behind the strip, at the very top of a scrolled conversation

If a conversation is long enough to scroll, and the user scrolls all the
way to the top, whatever renders in the top-left ~28px-tall, strip-width
band will sit *underneath* the floating tab strip — for a single
conversation, that's just the `+` button's own small box. This is the
literal, explicitly-granted exception in the ask itself ("unobstructed
except for the `+` sign") — not an oversight. No extra scroll-padding or
fade treatment was added to avoid it; it wasn't asked for, and would add
complexity (asymmetric padding on a scroll region) for a corner case
already accepted as fine.

---

## 2. Files touched

- `frontend/app/view/agent/agent-view.tsx` — `<PaneTabStrip>` moved from a
  sibling of `.agent-pane-progress-bar-slot` (before `.agent-pane-stack-content`)
  to a child of `.agent-pane-stack-content`, first before the `<Show>` for
  `AgentPicker`/`AgentPresentationView`.
- `frontend/app/view/agent/agent-view.scss` — removed
  `.agent-pane-stack > .pane-tab-strip { flex: 0 0 auto; }` (no longer a
  direct child there); added the `position: absolute` override scoped to
  `.agent-pane-stack-content > .pane-tab-strip`.
- No changes to `frontend/app/element/PaneTabStrip.tsx`/`.scss` (§1.1) or
  any other pane type.
- No `agentmux-srv` (Rust) changes. No wire-format changes.

---

## 3. Verification

- `npx tsc --noEmit` — clean.
- Full `frontend/app/view/agent/` suite — 85 files / 1102 tests passing.
- `bash scripts/vite-build.sh --mode production` — compiles cleanly;
  confirmed the compiled CSS contains
  `.agent-pane-stack-content>.pane-tab-strip{position:absolute;top:0;left:0;z-index:var(--z-pane-overlay, 4)}`.
- **Not verified visually** — no display in this sandbox. Needs a live
  `task dev` check: single conversation open, confirm the conversation
  fills the pane right up to the top edge (minus the `+` button's own
  corner) with no reserved dead band; add a 2nd/3rd tab, confirm the strip
  still only occupies exactly its own tabs+`+` width, still floating (not
  pushing content down); scroll a long conversation to the top and confirm
  the only obstruction is directly behind the strip itself, matching §1.3.
