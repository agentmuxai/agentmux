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

### 1.4 Resolved: the strip no longer competes with the loading overlay

Before this spec, `.pane-tab-strip` was a flex row entirely outside
`.agent-view`'s own box — geometrically incapable of overlapping anything
inside it. Moving the strip inside `.agent-pane-stack-content` makes it a
sibling of `.agent-view`, whose `.agent-pane-loading-overlay`
(`agent-view.tsx`, `_loading-overlay.scss`) is `position: absolute; inset:
0; z-index: var(--zindex-elem-modal)` (100) — shown from mount until
initial history load resolves, on every pane mount and reconnect. The
strip's own z-index (`var(--z-pane-overlay, 4)`) is far below 100, so
whether this actually manifested depended on whether `.agent-view` isolated
its descendants into their own stacking context.

Earlier rounds of this investigation were unable to settle that with full
confidence — MDN and a W3C spec page gave opposite answers on whether
`container-type: inline-size` alone creates one. reagent P1 on PR #2526's
6th round settled it directly: `container-type: inline-size` computes to
`contain: inline-size style`, and neither `inline-size` nor `style`
containment is in the set of values (`layout`, `paint`, `strict`,
`content`) that create a stacking context — so it does not, and the
loading overlay genuinely was painting over the tab strip on every mount.

**Fixed, narrowly** — not by containing `.agent-view`. A first attempt gave
`.agent-view` itself an explicit `z-index: 0` (a real, unambiguous
stacking-context trigger, unlike `container-type`), reasoning that it would
contain every absolutely-positioned descendant the same way
`.agent-document-scroll-region` already contains its own children
(`_document.scss`). reagent P1 on PR #2526's 7th round caught the flaw:
that also contained `.agent-auth-overlay` (`_auth-overlay.scss`, same
`--zindex-elem-modal` tier as the loading overlay), which deliberately
relies on escaping `.agent-view` to outrank `.block-mask` (`block.scss` —
its own `.block-focused` comment: "No z-index here intentionally... without
a stacking context, `.block-mask` ... is in the parent stacking context").
Containing `.agent-view` capped its resolved priority at 0 from
`.block-mask`'s perspective, so the focus-ring/click-to-focus mask (z-index
10 unfocused, 50 focused) started painting over the "blocks all
interaction" auth overlay whenever the pane was unfocused during an OAuth
wait — a regression in the opposite direction, not scoped to what this
spec was trying to fix.

An interim fix targeted only the one overlay flagged at the time:
`.agent-pane-loading-overlay` got its own explicit `z-index: 1`
(`_loading-overlay.scss`), not the shared `--zindex-elem-modal` token —
comfortably below the tab strip's 4, while `.agent-auth-overlay` kept
`--zindex-elem-modal` (100) and its escape-and-outrank-`.block-mask`
behavior untouched. reagent P1 on PR #2526's 8th round found this was
whack-a-mole: `.slash-autocomplete` (`_slash.scss`, z:50) and
`.agent-search-bar` (`_search.scss`, z:10) have the exact same "escapes
`.agent-view`, could paint over the tab strip" property the loading
overlay had — neither was ever nested inside a stacking-context-
establishing ancestor, so both compete with the tab strip's z:4 directly
whenever they're open (slash-command picker; in-session search).

**Actually resolved**, structurally: a repo-wide grep found
`.agent-auth-overlay` was never rendered anywhere — dead CSS left over
from an earlier auth-UI design (the live one is `AgentDocumentView.tsx`'s
`.agent-auth-panel-card`/`.agent-auth-notice`, an inline notice, not a
full-pane overlay). Deleted `_auth-overlay.scss` and its `agent-view.scss`
import. With nothing left that legitimately needs to escape `.agent-view`
to outrank `.block-mask`, `.agent-view` now carries its own `z-index: 0`
again — this time correctly — containing every current and future
absolutely/sticky-positioned descendant (loading overlay, search bar,
slash-autocomplete, filter toggle, …) at once instead of requiring a new
patch every time reagent finds another one.

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
