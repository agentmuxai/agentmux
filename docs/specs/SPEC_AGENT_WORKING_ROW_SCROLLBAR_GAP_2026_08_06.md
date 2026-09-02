# Spec: Continuous AgentWorkingRow background through the scrollbar gutter

**Date:** 2026-08-06
**Status:** superseded — 2026-09-01, see `Superseded-by:` below.
**Superseded-by:** [`SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md`](./SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md)

Was implemented in PR #2439 (backdrop-layer fix) and verified in code 2026-08-10.
Superseded **in full**: every element this spec introduced — the
`.agent-working-row-backdrop` layer, the anchor's scrollbar-width inset, and
the stacking-context reasoning that ordered them — existed to serve the
floating-overlay geometry of `SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md`
§3.2. The working row is now a normal-flow sibling below the ActivityDock,
outside `.agent-document-scroll-region` entirely, so it cannot paint over the
message list's scrollbar and the gutter-color gap this spec fixed cannot
occur. Retained for history, not as a description of current code.
**Scope:** `frontend/app/view/agent/agent-view.tsx`, `frontend/app/view/agent/styles/_control-bar.scss`, `frontend/app/view/agent/styles/_document.scss`
**Related:** `SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md` §3.2 (introduced the floating-overlay architecture this spec builds on), the `.agent-working-row-anchor` z-index-obscures-scrollbar comment in `_control-bar.scss` (the earlier bug this spec must not reintroduce)

## 1. Report

User-reported, in the agent pane: the `AgentWorkingRow` ("Working…" / "✓
Worked") status row has an opaque colored background that stops short of
the pane's true right edge, leaving a narrow vertical strip unpainted right
where the message list's scrollbar lives. When the conversation is scrolled
away from the very bottom (scrollbar thumb elsewhere), that strip clearly
reads as a gap — the row's color looks like it's cut off rather than
extending the full width of the pane.

## 2. Root cause (traced, not guessed)

### 2.1 The DOM/stacking structure today

Inside `.agent-document-scroll-region` (`agent-view.tsx:1551-1626`), two
absolutely-positioned siblings share the same box:

```
.agent-document-scroll-region (position: relative; overflow: hidden)
├── AgentDocumentView → .agent-document
│     position: absolute; inset: 0; overflow-y: auto;
│     z-index: auto (unset)
│     owns the native scrollbar (styled via ::-webkit-scrollbar)
└── .agent-working-row-anchor
      position: absolute; left: 0; bottom: 0;
      right: var(--agent-document-scrollbar-width, 14px);  ← the inset
      z-index: 2
      └── AgentWorkingRow → .agent-working-row (opaque background)
```

### 2.2 Why the inset exists (`_control-bar.scss:168-181`)

This inset is not an oversight — it's the fix for a *previous* bug,
documented in the code: the row's background used to be flush to `right: 0`
(no inset). Because `.agent-working-row-anchor` has explicit `z-index: 2`
and `.agent-document` has `z-index: auto`, the anchor's whole subtree
(including its opaque background) painted **above** `.agent-document` —
which includes `.agent-document`'s own custom-styled scrollbar, since a
CSS `::-webkit-scrollbar` renders as part of its owning element's own box,
not as a separate always-on-top browser-chrome layer in this engine. The
scrollbar was still there and clickable, just visually painted over —
invisible. Insetting the anchor by exactly the scrollbar's width
(`--agent-document-scrollbar-width`, `_document.scss:12`, kept in sync with
`.agent-document::-webkit-scrollbar`'s width in `app.scss:99-102`) was the
fix: nothing inside the anchor can geometrically reach that strip, so the
scrollbar is guaranteed visible.

### 2.3 Why that fix produces today's visible gap

The inset makes the anchor's *own* background never reach the strip, but
it does nothing to make `.agent-document`'s content **paint a matching
color** in that strip either — it just leaves the strip entirely to
whatever `.agent-document` puts there, which is:

- `::-webkit-scrollbar-track` → `background-color: var(--scrollbar-background-color)`, and
  `--scrollbar-background-color: transparent` (`theme.scss:123`). The track
  is **fully transparent** — wherever the thumb isn't currently covering
  it, the strip shows whatever is *behind* `.agent-document`, i.e. the
  general pane background, not the working row's color.
- `::-webkit-scrollbar-thumb` → `var(--scrollbar-thumb-color)`, `rgba(255,
  255, 255, 0.5)` (`theme.scss:124`) — translucent white, also not the
  row's color, only present where the thumb currently sits.

So the "gap" is real and structural, not a rendering glitch: a
`--agent-document-scrollbar-width`-wide (14px) strip at the bottom of the
pane that the working row's background is deliberately excluded from, and
that `.agent-document`'s own scrollbar styling never fills with a matching
color. It's most visible exactly when the report describes — scrolled away
from true bottom, thumb elsewhere, track transparent — because that's when
the strip shows plain transparent track (background bleed-through) instead
of at least a translucent white thumb partially masking it.

## 3. Constraint: the fix must not reintroduce §2.2's bug

Any fix must keep `.agent-document`'s scrollbar (track and thumb) **visibly
on top** of whatever fills that strip. Simply removing the inset (`right:
0` again) is not an option — that's the exact regression the current code
comments warn against.

## 4. Proposed fix: a decorative backdrop layer, stacked *below* `.agent-document`

The two concerns currently conflated by a single z-index value need to be
split:

- The row's **text/interactive content** must stay above `.agent-document`'s
  scrolling message content (unchanged requirement — this is what
  `z-index: 2` on the anchor already provides, and must keep providing).
- The row's **background color** should additionally reach a layer that is
  below `.agent-document`'s own scrollbar rendering, so the scrollbar keeps
  painting on top of it — the opposite ordering from the anchor's text.

A single element can't sit both above and below the same sibling at once,
so this needs a **third layer**: a new, purely decorative element that:

1. Spans the **full** width (`left: 0; right: 0`, no scrollbar inset),
   same `bottom`/height as the current anchor.
2. Sits at a stacking level **below** `.agent-document` (e.g. `z-index: 0`,
   or simply no explicit `z-index` and placed earlier in the DOM than
   `.agent-document` — either is sufficient since `.agent-document` itself
   has `z-index: auto`; giving the backdrop a negative or lower explicit
   value is the more robust choice and doesn't depend on DOM-order
   tie-breaking).
3. Uses the **same background** as whichever `AgentWorkingRow` variant is
   currently showing (`--loading`'s `color-mix(in srgb, var(--accent-color)
   4%, var(--elevated-bg-color))`, or `--worked`'s `var(--elevated-bg-color)`
   plain) — or, more simply, always paints `--elevated-bg-color` and lets
   the existing anchor's own (inset) background carry the accent tint on
   top of it, since the two colors are close enough that a 14px strip of
   the untinted base likely won't read as a mismatch. **Needs a visual
   check once implemented** — if the tint difference is visible, thread the
   loading/worked state through as a class on the backdrop too (see §5).
4. Is `pointer-events: none` and needs no click handling — it sits below
   `.agent-document`, so pointer events over that strip already reach
   `.agent-document`'s scrollbar naturally without this layer intercepting
   anything.
5. Only renders under the same condition the anchor's `<Show>` already uses
   (`agent-view.tsx:1596-1600`) — no idle placeholder, matches current
   behavior of collapsing to nothing when neither loading nor holding
   completed stats.

With this in place, in the 14px strip: the backdrop paints first (bottom),
`.agent-document` paints next including its scrollbar (so the scrollbar
stays visibly on top, satisfying §3), and the existing anchor's text layer
paints last, unchanged, in front of everything. The previously-transparent
track now shows the backdrop's color instead of bleeding through to the
general pane background; the translucent-white thumb now sits over a
color-matched backdrop instead of a mismatched one.

## 5. Implementation sketch

- `agent-view.tsx` (~line 1595, inside `.agent-document-scroll-region`,
  *before* `AgentDocumentView` in source order): add a new sibling `<div
  class="agent-working-row-backdrop">`, gated by the same `<Show when=...>`
  condition currently on the anchor's contents (lines 1596-1600) — either
  duplicate the condition or hoist it to a shared `createMemo` so the two
  `<Show>`s can't drift out of sync.
  - If the loading/worked color distinction turns out to matter visually
    (see §4.3's caveat), also mirror the `--loading`/`--worked` boolean
    onto this element via a class, driven by the same `props.loading`
    logic `AgentWorkingRow` already computes — simplest path is exposing
    that boolean from `agent-view.tsx`'s own already-computed
    `showingLaunchActivity() || workingFromPhase(...)` expression (already
    inlined twice at lines 1597-1598/1602; a `createMemo` wrapping it once
    would also de-duplicate that repetition as a side benefit).
- `_control-bar.scss`: new `.agent-working-row-backdrop` rule near
  `.agent-working-row-anchor` (~line 165) — `position: absolute; inset:
  auto 0 0 0;` (or explicit `left/right/bottom`), height matching the row
  (reuse `--agent-working-row-height` if it's convenient, otherwise let it
  size to content the same way the anchor does), `z-index: 0` (or `-1`;
  either is fine as long as it's below `.agent-document`'s implicit `auto`
  level — confirm empirically since `auto` and unset both resolve to the
  same default paint order, and a negative z-index on a descendant of a
  non-stacking-context ancestor can behave surprisingly if
  `.agent-document-scroll-region` itself ever gains a stacking-context
  trigger — check computed z-index of `.agent-document-scroll-region`
  before picking `-1` vs `0`), `pointer-events: none`.
- No changes needed to `.agent-working-row-anchor` itself, `.agent-document`,
  or the scrollbar-width custom property — this is additive.

## 6. Risks / open questions to resolve during implementation

1. **Color match precision** — is a flat `--elevated-bg-color` backdrop
   visually indistinguishable enough from the `--loading` state's subtly
   accent-tinted background in a 14px strip, or does it need the exact
   per-state color threaded through? Cheap to check once built; §5 already
   describes the fallback if it does need threading.
2. **Thumb-over-backdrop appearance** — when the scrollbar thumb sits in
   this strip, it'll now render over a color-matched backdrop instead of
   the general pane background. This is the intended fix (continuous
   color story) but is a genuine visual change worth a deliberate look,
   not just inferred from CSS — the translucent white thumb will read
   slightly differently over an accent-tinted background than over the
   plain dark pane background.
3. **Stacking-context correctness** — confirm via devtools (computed
   z-index / paint order, not just reasoning about the CSS) that the
   backdrop actually lands behind `.agent-document`'s scrollbar and not
   just behind its content, across both the loading and worked visual
   states, before considering this done.
4. No interaction with `SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY`'s
   §3.3 known trade-off (interposing rows like the retry bar) — the
   backdrop is scoped to the same visibility condition as the existing
   anchor, so it inherits that trade-off unchanged rather than worsening it.

## 7. Verification plan

- Visual: start a turn, scroll the conversation up away from true bottom
  while it's still streaming — confirm the working row's background now
  reads as one continuous bar to the pane's right edge, with the scrollbar
  thumb/track still clearly visible and clickable on top of it. Repeat
  scrolled to true bottom. Repeat in the `--worked` (completed) state.
  Repeat with the pane narrow enough that the row's text truncates, to
  confirm the backdrop's width is independent of the text-bearing anchor's
  content width (it should always span full width regardless of text
  length — that's the fix for the exact "the blue tracks the text" framing
  that's *not* what we're implementing, worth explicitly ruling out during
  a build-time smoke check since it'd look superficially similar to
  "half-fixed").
- Confirm scrollbar remains clickable/draggable over the strip (the
  original bug this can't regress) — drag the thumb through that region.
- `npx tsc --noEmit` — no logic changes expected to break typing, but the
  new conditionally-rendered element and any shared-memo refactor should
  still typecheck cleanly.
