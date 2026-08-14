# SPEC: Bind the pane tab strip to its own pane's zoom, and fix top scroll-clearance for short agent conversations

**Date:** 2026-08-12 (Part A corrected before implementation — see §A.0's
note).
**Status:** implemented, with two further correctness bugs caught in
PR #2566 review after this doc's own live verification missed them —
fixed, not yet independently re-verified live a third time. History:

1. An earlier draft bound to the wrong zoom control entirely (global
   chrome zoom, `--zoomfactor`) — implemented, live-verified, then
   corrected to the per-pane design below before landing anywhere.
2. This per-pane version was implemented and live-verified — **but only
   on the agent pane.** Live check used a real Ctrl+Wheel zoom gesture
   (CDP-dispatched) on an agent pane: tabs, `+`, and conversation content
   scaled together (0.97 → 1.3), `--pane-tab-strip-zoom` and
   `--agent-pane-zoom` agreed exactly, and the strip's right edge stayed
   pixel-identical to the pane's real right edge at zoom (zero drift,
   confirming §A.1's landmine stayed avoided). None of this exercised
   editor's zoom at all, and the agent padding-top check's own numbers
   (a `docPaddingTop` reading that didn't match the expected formula)
   were misread as `getComputedStyle` ambiguity inside nested `zoom`
   rather than recognized as the double-scaling bug it actually was.
3. `reagentx-workflow[bot]`'s PR review caught both: editor's
   `zoomFactor` prop double-compounds with `.editor-view`'s own ambient
   `zoom` (§A.3), and `_document.scss`'s `padding-top` double-scaled the
   same way (§B.3). Both fixed below — the editor fix is to *not* pass
   `zoomFactor` at all (ambient zoom already covers it "for free"), and
   the padding-top fix is to drop the zoom multiplication entirely (same
   reason). Terminal was re-checked and confirmed to have no ambient
   `zoom` anywhere in its tree, so its wiring was correct as shipped.
**Related:** `docs/specs/SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md` (agent
strip floats over content), `docs/specs/SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md`
(shipped — made the agent strip a full-width, 28px-tall blurred band,
which is what makes Part B below worse than before that change),
`docs/specs/zoom-architecture.md` (stale — see §A.0).

This is two related asks from the same conversation, both about
`frontend/app/element/PaneTabStrip.tsx`/`.scss`. They're bundled in one
doc because Part B's fix needs a value Part A introduces (the strip's
real height as a computable formula) — implementing B first with a
hardcoded `28px` would just go stale the moment A ships.

---

# Part A — Bind the pane tab strip to its own pane's content zoom (not chrome zoom)

## A.0 Correction: this is per-pane zoom, not chrome zoom

The first draft of this spec bound the tab strip to **chrome zoom**
(`--zoomfactor`, global, currently consumed only by `.window-header`/
`.status-bar`) — wrong. The actual ask is the **per-pane content zoom**
each agent/editor/terminal pane already has independently (`term:zoom`
block metadata, user-adjustable per pane, e.g. `Ctrl+scroll` or whatever
this app's per-pane zoom control is). Confirmed three separate existing
implementations, all reading the same underlying meta key but computed
differently per pane type:

- **Agent**: `zoomFactor` — a `createMemo` inside `AgentPresentationView`
  (`frontend/app/view/agent/agent-view.tsx:1772-1777`):
  ```ts
  const zoomFactor = createMemo(() => {
      const meta = block()?.meta;
      const z = meta?.["term:zoom"];
      if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
      return Math.max(0.5, Math.min(2.0, z));
  });
  ```
  applied via `style={{ zoom: zoomFactor(), "--agent-pane-zoom": String(zoomFactor()) }}`
  on `.agent-view`'s root (`:1833-1837`).
- **Editor**: `model.zoomAtom` (`Accessor<number>`,
  `frontend/app/view/editor/editor-model.ts:167,344`, via
  `useBlockAtom(blockId, "editor-zoom", ...)` reading the same
  `term:zoom` key), applied via `style={{ zoom: model.zoomAtom() }}` on
  the editor root (`editor-view.tsx:705`).
- **Terminal**: `model.termZoomAtom` (`Accessor<number>`,
  `frontend/app/view/term/termViewModel.ts:66,237`, same
  `useBlockAtom(blockId, "termzoomatom", ...)` pattern) — applied not via
  CSS `zoom` but by recomputing xterm's own font size
  (`term.tsx:341,354-362`). Different mechanism for the terminal's own
  content, but the same numeric factor is available and reusable for the
  tab strip regardless of how the terminal body itself renders zoom.

This is the right target: tabs zooming with *that pane's own* content
(so zooming into one conversation makes its own tabs bigger too, matching
what you're looking at) rather than a single global chrome-wide size.
The tradeoff, correctly: tab size is no longer uniform across panes —
an agent pane zoomed to 150% has visibly bigger tabs than a terminal
neighbor at 100%. That's the intended behavior for content zoom, not a
bug (chrome zoom would've kept them uniform; that's not what was asked
for here).

`docs/specs/zoom-architecture.md` is stale (references `wave.ts`/
`blockframe.tsx`/`tab.scss`, none of which match current file names) —
treat it as history either way, not a guide for this change.

## A.1 The landmine still applies — unchanged from the corrected-away draft

This part of the original analysis was never wrong, independent of which
zoom value drives it: `window-header.win32.scss:34,36` /
`.linux.scss:35` combine `zoom: var(--zoomfactor)` with explicit width
compensation (`calc(100vw / var(--zoomfactor, 1))`); the identical fix
is explicitly wrong on macOS —
`window-header.darwin.scss:5-9`:

> On macOS/WebKit, CSS `zoom: factor` scales children so they see
> `width/factor` pixels of layout space. Use `width: 100%` ... **DO NOT**
> use `calc(100vw / var(--zoomfactor, 1))` here — that double-divides:
> children would see `100vw/factor²`.

Chromium (Win/Linux) and WebKit (macOS) resolve `zoom` on an edge/
viewport-anchored box differently. `.agent-pane-stack-content > .pane-tab-strip`'s
`left: 0; right: 0` (added by
`SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md`) is exactly that shape
of box — the landmine is about combining `zoom` with edge-anchored
sizing, full stop, regardless of whether the zoom value comes from
`--zoomfactor` or a per-pane accessor. The design in §A.2 avoids it the
same way regardless of source.

## A.2 Design: zoom an inner wrapper, never the edge-anchored outer box — unchanged architecture, different value source

- **Outer `.pane-tab-strip`** — never zoomed; keeps its real-pixel
  positioning job. `right: 0` keeps spanning the pane's true right edge
  on any platform, no compensation needed, because the box doing
  edge-anchored sizing is never the zoomed one.
- **New inner `.pane-tab-strip-inner`** (wraps the `<For>` + `<Show>` in
  `PaneTabStrip.tsx`) — carries `zoom`, sourced from a **new prop**, not
  a global CSS variable:

```ts
// PaneTabStripProps
zoomFactor?: Accessor<number>;   // per-pane content zoom; 1 (unzoomed) if omitted
```

The outer div sets a scoped custom property from that prop, the same
pattern `.agent-view` already uses for its own `--agent-pane-zoom` — just
named for the strip specifically, since each pane type feeds it a
different source:

```tsx
// PaneTabStrip.tsx
<div
    class="pane-tab-strip"
    style={{ "--pane-tab-strip-zoom": String(props.zoomFactor?.() ?? 1) }}
    onDblClick={...}
>
    <div class="pane-tab-strip-inner">
        <For each={props.tabs}>...</For>
        <Show when={props.onAdd}>...</Show>
    </div>
</div>
```

```scss
.pane-tab-strip {
    height: calc(var(--pane-tab-strip-height, 28px) * var(--pane-tab-strip-zoom, 1));
    // (position/left/right per consumer, border-bottom, background,
    // overflow — unchanged from today)
}

.pane-tab-strip-inner {
    display: flex;
    flex-direction: row;
    align-items: stretch;
    height: var(--pane-tab-strip-height, 28px);
    zoom: var(--pane-tab-strip-zoom, 1);
}
```

The custom property is set on the **outer** div specifically (not inner)
because it needs to be readable by both layers: the outer box's own
`height` calc, and the inner wrapper's `zoom` — custom properties cascade
down to descendants, so setting it on the outermost element makes it
visible to both.

## A.3 Per-pane-type wiring

**Correction (caught in review on PR #2566, after this design first
shipped): whether a pane type passes `zoomFactor` at all depends on
where its `<PaneTabStrip>` renders relative to that pane's own
`zoom`-scaled root — not just on whether a zoom accessor happens to be
in scope.** The inner-wrapper's `zoom` (§A.2) is only correct to apply
when `<PaneTabStrip>` is *outside* the pane's own already-zoomed
subtree (the agent case, a DOM sibling of `.agent-view` — §A.0). Passing
`zoomFactor` when the strip is *already inside* a zoomed ancestor
double-compounds: CSS `zoom` on nested elements multiplies
(`ambient_zoom × own_zoom`), not adds, so a pane zoomed to 1.5× would
render its tabs at `1.5² = 2.25×`.

- **Editor** (`editor-tab-strip.tsx`) — **do NOT pass `zoomFactor`**.
  `<EditorTabStrip>` renders inside `.editor-view`
  (`editor-view.tsx:701-742`), which already has
  `style={{ zoom: model.zoomAtom() }}` on that root — `<PaneTabStrip>` is
  a *descendant* of the already-zoomed element, not a sibling like
  agent's. Ambient zoom already scales the entire tab-strip subtree
  (`.pane-tab-strip`'s height calc defaults to the plain
  `--pane-tab-strip-height` baseline, `.pane-tab-strip-inner`'s own
  `zoom` defaults to `1` — both correct, with the prop simply omitted) —
  the earlier version of this doc called this wiring "trivial" and got
  it backwards; the trivial part was that `model.zoomAtom` was *in
  scope*, not that passing it was correct.
- **Terminal** (`term.tsx`) — passes `zoomFactor={model.termZoomAtom}`,
  confirmed correct (not just "trivial because in scope," verified
  properly this time): terminal has **no ambient CSS `zoom` anywhere in
  its own tree** — grepped `term.tsx` for `zoom`, only hits are the
  `term:zoom` meta key and this prop itself — it applies its zoom via
  xterm font-size recomputation instead (`term.tsx:341,354-362`), a
  wholly different mechanism from agent/editor's `style={{ zoom }}`. No
  ambient zoom to inherit, so the strip legitimately needs its own
  explicit `zoom`, same reasoning as agent.
- **Agent** (`agent-view.tsx`) — **not trivial, the one open question
  left in this spec.** `<PaneTabStrip>` is called from
  `AgentViewWrapper` (`:408-429`), a *parent* of `AgentPresentationView`
  (`:463+`), where the existing `zoomFactor` memo (§A.0) actually lives —
  it is not in scope at the tab strip's own call site. `AgentViewWrapper`
  has its own `block = model.blockAtom` (`:150`) and a separate
  `activeBlockId` memo (`:255-257`, resolves to whichever tab/fork is
  currently active, which is **not necessarily the same block** as
  `model.blockAtom` once more than one tab is open — `model.blockAtom` is
  the pane's own root block, `activeBlockId()` is whichever tab you're
  actually looking at). The correct zoom value for the tab strip is
  whichever tab's content is currently visible, i.e. `activeBlockId()`'s
  own `term:zoom`, not `AgentViewWrapper`'s own root block's.

  **Needs resolving before implementation**: how to reactively read an
  *arbitrary* block's meta by id (`activeBlockId()`) from
  `AgentViewWrapper`'s scope — likely a `WOS.useWaveObjectValue`/
  `WOS.getObjectValue`-style call (patterns already used elsewhere in
  this file; grep for how `AgentPresentationView`'s own `block` accessor
  is ultimately backed, and whether an equivalent "get me this other
  block's live meta" helper already exists rather than needing a new
  one). Once resolved, add a small memo in `AgentViewWrapper` mirroring
  §A.0's exact same read-and-clamp logic, keyed off `activeBlockId()`
  instead of `model.blockAtom`, and pass it as
  `zoomFactor={thatNewMemo}` on the `<PaneTabStrip>` call at `:408`.

  This is deliberately a *second*, independent memo reading the same
  `term:zoom` key — not a refactor to share `AgentPresentationView`'s
  existing one across component boundaries. Both are pure reads of the
  same live reactive source (block meta), so there's no drift/staleness
  risk from computing it twice; threading one value across a
  parent/child boundary that doesn't currently pass it would be a larger,
  riskier change for no real benefit here.

## A.4 What does NOT need to change

- Agent pane's override (`agent-view.scss:91-141`) — `right: 0`,
  `background`, `backdrop-filter`, `pointer-events` all stay on the outer
  `.pane-tab-strip` selector, unaffected by the inner/outer split.
- `backdrop-filter: blur(2px)` stays fixed, not zoom-scaled — decorative
  background on the never-zoomed outer box, not document content.
- Editor/terminal need no width compensation — their shrink-to-fit width
  comes from the inner wrapper's own (zoomed) content size; only the
  agent override's edge-anchored `right: 0` had any exposure to §A.1's
  landmine, and it's now on the never-zoomed layer.

## A.5 Files touched

- `frontend/app/element/PaneTabStrip.tsx` — new `zoomFactor?: Accessor<number>`
  prop; wrap `<For>`+`<Show>` in `.pane-tab-strip-inner`; set
  `--pane-tab-strip-zoom` inline on the outer div.
- `frontend/app/element/PaneTabStrip.scss` — split base rule per §A.2.
- `frontend/app/theme.scss` — `--pane-tab-strip-height: 28px` (unchanged
  from the prior draft — still needed, still not zoom-related itself).
- `frontend/app/view/editor/editor-tab-strip.tsx` — touched, but
  deliberately does **not** pass `zoomFactor` (§A.3 correction) — a
  comment explaining why, so a future reader doesn't "fix" the
  omission.
- `frontend/app/view/term/term.tsx` — pass `zoomFactor={model.termZoomAtom}`.
- `frontend/app/view/agent/agent-view.tsx` — new `tabStripZoomFactor`
  memo in `AgentViewWrapper`, keyed off `activeBlockId()`, passed to the
  `<PaneTabStrip>` call.
- **No `theme.scss`/global `--zoomfactor` involvement at all** — that
  variable is uninvolved in the corrected design; nothing here touches
  chrome zoom or `window-header`/`status-bar`.

---

# Part B — Top scroll-clearance for short/unscrolled agent conversations

*(Design unaffected by the Part A correction — the concept and formula
shape were always right. Correction check: the shipped code, on
inspection, had also literally written `--zoomfactor` here, same mistake
as Part A, not `--agent-pane-zoom` as an earlier revision of this doc
claimed — fixed now to reference `--agent-pane-zoom`, which was always
the intended value and needed no new plumbing, unlike the tab strip
itself.)*

## B.0 The problem

The agent pane's tab strip floats over live conversation content
(`SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md`) and, as of the blur
spec, is a full-width 28px band (`agent-view.scss:91-141`) rather than
just a small `+`-sized box. The overlay spec's one accepted tradeoff was
about *scrolling* a long conversation to the top — reversible by
scrolling back down. A **short conversation that never overflows enough
to scroll** has no scroll position that reveals the first message(s) —
they render permanently underneath the strip.

## B.1 Root cause

`.agent-document` (`frontend/app/view/agent/styles/_document.scss:100-141`)
had only 2px of top clearance (`var(--space-0-5)`) against a 28px-tall
strip — the first `.agent-document-row` rendered essentially flush to
y=0, directly behind the strip's full width, whenever the conversation
didn't overflow past ~26px of missing clearance.

## B.2 Best practice: reserve clearance via `padding-top` on the scrolled content — not `scroll-padding-top`

`scroll-padding-top` only affects scroll-snap/scroll-into-view
targeting, not resting layout — a conversation that never scrolls never
triggers a scroll operation, so it would do nothing for exactly the case
being fixed. (Zero existing uses of `scroll-padding` anywhere in
`frontend/` either way — not an established pattern here.)

This codebase already solves the identical problem symmetrically at the
*bottom* of the same scroll region: `AgentWorkingRow` floats over the
bottom (`_control-bar.scss:76-100`), and `.agent-document`'s
`padding-bottom` reserves its live height via a `ResizeObserver`-fed
custom property (`agent-view.tsx:1372-1384`, `attachWorkingRowAnchor`;
consumed at `_document.scss:120`, `--agent-working-row-height`). The top
fix follows the same *concept* (padding on the scrolled content, not
scroll-snap) but not the same *mechanism* — see §B.3.

## B.3 Implemented fix

**Correction (caught in review on PR #2566, same double-scaling class of
bug as §A.3's editor mistake): do NOT multiply by any zoom variable
here.** The strip's real (on-screen) height is
`--pane-tab-strip-height × --pane-tab-strip-zoom` — a quantity computed
entirely in real, un-zoomed screen pixels (§A.2's outer box is
deliberately never itself zoomed). But `.agent-document` is a descendant
of `.agent-view`, which already has `style={{ zoom: zoomFactor() }}`
applied — so *any* length this rule writes gets auto-scaled by that
ambient zoom by the browser, for free, as part of normal `zoom`
semantics. Multiplying by `--agent-pane-zoom` *again* here double-scales
the result (`baseline × zoom²` once ambient zoom applies on top of an
already-multiplied value) — over-reserving space at zoom > 1 and, more
seriously, under-reserving at zoom < 1 (the clamp allows down to `0.5`),
reintroducing the exact "first message hidden" bug this rule exists to
fix. The two zoom factors cancel out exactly (see derivation below), so
the correct fix is the *unscaled* baseline:

```scss
// _document.scss:113-114, as shipped (corrected)
padding: var(--space-0-5) 0 var(--space-0-5) var(--space-1);
padding-top: calc(var(--space-0-5) + var(--pane-tab-strip-height, 28px));
```

A separate `padding-top` longhand, not merged into the shorthand —
matching how `padding-bottom` is already its own separate longhand
rather than folded into the same `padding:` line.

**Why the plain baseline is exactly right, not an approximation**: write
`padding-top: P` inside a `zoom: Z` ancestor, and the browser renders it
on-screen as `P × Z`. Setting `P = --pane-tab-strip-height` (unscaled)
renders on-screen as `height × Z`. The strip's own real on-screen height
is `height × --pane-tab-strip-zoom`. Since `--agent-pane-zoom` (this
pane's ambient `Z`) and `--pane-tab-strip-zoom` (the tab strip's own,
§A.2) are two *separately plumbed* CSS custom properties that always
hold the *same* runtime number for the agent pane specifically — both
derive from the exact same active block's `term:zoom` meta, just
delivered via two different DOM paths because the strip and
`.agent-document` aren't in the same subtree (§A.3) — the two expressions
are equal: `height × Z = height × --pane-tab-strip-zoom`. No `calc()`
multiplication needed on this side at all; the ambient zoom *is* the
multiplication.

### B.3.1 Sequencing

Still true: land Part A (specifically, `--pane-tab-strip-height`) before
or together with Part B — Part B already references it.

## B.4 Scope: agent-pane only

Unaffected by the Part A correction. Editor/terminal reserve a normal
row for `PaneTabStrip` — no overlap, no gap to fix there.

## B.5 Files touched

- `frontend/app/view/agent/styles/_document.scss` — `padding-top` (§B.3),
  corrected alongside Part A to reference `--agent-pane-zoom`.

---

# Verification plan

Both parts need a fresh pass — Part B's `calc()` shape and location were
always right, but it also had the same literal `--zoomfactor` mistake as
Part A until just now:

- `npx tsc --noEmit`, `stylelint`, full test suite, prod `vite build`.
- `task dev`, per pane type, using each pane's own zoom control (not a
  simulated `--zoomfactor` override, which no longer does anything
  useful for this component post-correction — set the actual `term:zoom`
  meta, or use whatever UI control adjusts it, on an agent, editor, and
  terminal pane independently):
  - Confirm tabs/`+`/(agent only) the blurred trailing panel scale with
    *that pane's own* zoom, and stay independent of a sibling pane's
    zoom level (open two agent panes at different zoom levels
    side-by-side — their tab strips should visibly differ in size).
  - Re-run the same right-edge-alignment check the prior (wrong) draft
    did — `strip.getBoundingClientRect()` right edge vs.
    `.agent-pane-stack-content`'s right edge, at a couple of zoom levels
    — this is the one measurement that directly confirms §A.1's landmine
    is still avoided with the new value source.
  - Short conversation + zoomed-in agent pane: confirm the first message
    is still fully clear of the strip (Part B's calc should track
    whatever zoom the strip itself is now at).
- Not yet re-verified at all post-correction — everything above needs a
  fresh pass; the prior "Actual results" section (measured
  `--zoomfactor`, the wrong variable) has been removed from this doc as
  no longer meaningful.
