# REPORT — Zoom bindings audit, and the case for collapsing preview zoom into pane zoom

**Date:** 2026-09-06
**Author:** Loap #2 @ claudius
**Trigger:** Operator report — zooming into previews is janky, "they currently
have separate zoom systems, the pane level and the preview level, but
sometimes the preview level doesn't even work." Ask: count the zoom bindings
and report on simplifying to pane level only.
**Status:** proposed — analysis only; this report changes no code. §3 is the
recommendation; flip this line to `implemented` when it lands. (A separate
fix, PR #3009, addresses the hover peek panel's own zoom escape — a
different bug in the same area, not part of this proposal.)

---

## 0. Headline

**The operator's instinct is right, and the reason is stronger than
"simpler".** Pane zoom and preview zoom are not two implementations of the
same idea — they use two *different CSS mechanisms* with different
capabilities:

| | Mechanism | Scales hardcoded `px`? |
|---|---|---|
| Pane zoom | CSS `zoom` on the pane root | **Yes** — `zoom` scales the whole box tree |
| Preview zoom | `font-size: N%` on one wrapper div | **No** — only descendants authored in `em`/`%` |

The preview stylesheet is overwhelmingly authored in hardcoded pixels:
**66 `font-size: …px` rules vs 8 relative (`em`/`%`) rules** in
`frontend/app/view/agent/styles/_document-nodes.scss`. So preview zoom can
only ever move ~11% of the type in the panel. That is not a bug to fix — it
is the mechanism working as designed, on a stylesheet that was never written
for it.

**This fully explains "sometimes it doesn't even work."** Whether preview
zoom appears to do anything depends entirely on which renderer you happen to
be looking at. `DiffViewer.tsx`, `BashOutputViewer.tsx`, `HighlightedCode.tsx`
and `CompactResult.tsx` never reference the scale at all — they inherit
whatever cascades in, so any hardcoded-px rule inside them silently ignores
it.

Deleting preview zoom and letting pane zoom cover previews is therefore not
a feature trade — **it replaces a mechanism that can't work with one that
already does.** CSS `zoom` on the pane root scales previews correctly today,
including every hardcoded pixel.

## 1. Count: 13 binding sites, 5 systems, 3 platform copies

**Direct answer to "how many places are there zoom bindings?" — 13 places
that a user gesture can reach, across 5 independent systems.** Plus 3
duplicated platform copies of the core module and 3 compensation sites that
exist only to undo zoom applied elsewhere.

### System 1 — pane zoom (`term:zoom` block meta) — the canonical one

CSS `zoom` on the view root. This is the system to keep.

| # | Binding | File |
|---|---|---|
| 1 | Global Ctrl+wheel → `zoomBlockIn/Out` | `frontend/app/app.tsx:205-220` |
| 2 | Keyboard `Ctrl`/`Cmd` `+` `-` `0` | `frontend/app/store/keymodel.ts:234-266` |
| 3 | Command palette / menu (`view:zoom:in|out|reset`) | `frontend/app/store/command-registry.ts:350-368` |
| 4 | Terminal's own Ctrl+wheel | `frontend/app/view/term/term.tsx:371-386` |
| 5 | Editor's own Ctrl+wheel | `frontend/app/view/editor/editor-view.tsx:117-131` |
| 6 | Armory's own Ctrl+wheel | `frontend/app/view/armory/armory-view.tsx:66-80` |
| 7 | Warden's own Ctrl+wheel | `frontend/app/view/warden/warden-view.tsx:43-57` |
| 8 | Swarm's own Ctrl+wheel **and** keyboard | `frontend/app/view/swarm/swarm-view.tsx:30-71` |
| 9 | Agent shell sub-block's Ctrl+wheel | `frontend/app/view/agent/components/AgentShellSubblock.tsx:219-234` |

Sites 4-9 all write the same `term:zoom` meta key but each re-implements the
step/clamp/apply logic locally instead of calling `zoom.platform`'s shared
`zoomBlockIn/Out`. Swarm is the furthest drifted — its own `zoomFactor` memo,
its own `setZoom`, its own `STEP`, and its own keyboard handling
(`swarm-view.tsx:68-71`) duplicating what `keymodel.ts` already provides.

Core module, **duplicated three times** with near-identical bodies:
`frontend/app/store/zoom.win32.ts`, `zoom.darwin.ts`, `zoom.linux.ts`
(behind `zoom.platform.ts`).

### System 2 — chrome/window zoom (`--zoomfactor`)

| # | Binding | File |
|---|---|---|
| 10 | Ctrl+wheel while over the title/status bar | `app.tsx:205-206` → `chromeZoomIn/Out` (`zoom.win32.ts:133-161`) |

Distinct from pane zoom by design (scales title bar + status bar only, via a
CSS var rather than CSS `zoom`). **Not in scope to change** — it targets a
different surface and doesn't conflict.

### System 3 — preview zoom (`previewFontScale`) — the one to delete

| # | Binding | File |
|---|---|---|
| 11 | Ctrl+wheel over the preview body only | `frontend/app/view/agent/components/ToolBlock.tsx:148-170` |

- State: a **local, ephemeral** `createSignal(1.0)` (`ToolBlock.tsx:146`) —
  not persisted, not block meta.
- Range: `0.7`–`2.0`, step `0.05` (`ToolBlock.tsx:131-133`) — a *different*
  range and step from pane zoom.
- Applied as `font-size: <scale*100>%` on one wrapper
  (`ToolOverlayLog.tsx:344`), reached via a prop chain
  `ToolBlock` → `ToolBlockOverlay.tsx:36,67` → `ToolOverlayLog.tsx:47`.
- No keyboard binding, no menu entry, no persistence, no reset.

### System 4 — help view (`help:zoom`)

| # | Binding | File |
|---|---|---|
| 12 | Own Ctrl+`+`/`-`/`0` and Ctrl+wheel | `frontend/app/view/helpview/helpview.tsx:36-70` |

A second, parallel copy of System 1 against a different meta key
(`help:zoom`), with its own MIN/MAX/step — but it *does* use CSS `zoom` and
*does* persist, so it works correctly. Out of scope for the jank fix;
in scope for §4's consolidation if that's ever wanted.

### System 5 — drone canvas viewport

| # | Binding | File |
|---|---|---|
| 13 | Bare wheel (no Ctrl) pans/zooms the canvas | `frontend/app/view/drone/drone-view.tsx:145-172` |

`transform: scale()` on in-memory viewport state. Legitimately different — a
pannable canvas, not document zoom. **Leave alone.**

### Compensation sites (exist only to undo zoom)

Not bindings, but part of the cost of the current design:

- `PaneTabStrip.tsx:38-45,182-191` — `--pane-tab-strip-zoom`, so the tab
  strip isn't re-zoomed by the pane's `zoom`.
- `PeekOverlay.tsx` (`readPaneZoom`/`withPaneZoom`) — re-applies pane zoom to
  a Portal that escaped it. Added by PR #3009.
- `window-header.*.scss` — `calc(100vw / var(--zoomfactor,1))` to undo chrome
  zoom.

## 2. Why the two systems fight

Three concrete conflicts, all reachable with one gesture:

1. **Same gesture, different outcome a few pixels apart.** Ctrl+wheel over
   the preview *body* zooms the preview; over the *filename header* it falls
   through to pane zoom (`ToolBlock.tsx:152-162`, an explicit `overPreviewBody`
   hit-test). This is deliberate and documented — and is exactly the "janky"
   the report describes. There is no visual cue where the boundary is.
2. **Two different scales compose multiplicatively.** A preview inside a
   zoomed pane is at `paneZoom × previewScale`, with different clamps
   (`0.5–2.0` vs `0.7–2.0`), so the effective range is neither and the two
   ratchet against each other.
3. **Preview zoom silently resets.** It lives in a component-local signal in
   a **virtualized** transcript. Scroll the row out of view and back and the
   scale is gone. Worse, `ToolBlock.tsx:186-196` documents that `<Index>`
   reuses component instances across *different nodes* when the streaming
   buffer advances — so a stale scale can land on an unrelated tool block.

## 3. Recommendation — delete System 3

Per §0, this loses nothing that currently works and removes all three
conflicts.

**Remove:**
- `previewFontScale` signal, the `PREVIEW_ZOOM_*` constants, and the `wheel`
  listener + `overPreviewBody` hit-test (`ToolBlock.tsx:131-133,146-170`)
- the `previewFontScale`/`fontScale` prop chain through
  `ToolBlockOverlay.tsx:36,67` and `ToolOverlayLog.tsx:47,344`
- the now-obsolete "leave Ctrl+wheel alone" carve-out in
  `ToolOverlayLog.tsx:159-163`

**Result:** Ctrl+wheel anywhere in the pane — preview body included — zooms
the pane, which already scales previews correctly.

**What is genuinely lost:** the ability to enlarge a dense diff *without*
enlarging the surrounding transcript. Worth stating plainly rather than
pretending the feature was pure cost. But it is a capability that today only
functions on ~11% of the panel's text and silently resets on scroll — so in
practice it is being removed from users who mostly can't use it anyway. If
it's wanted back later, the correct implementation is CSS `zoom` on the
preview container plus persistence in block meta (mirroring System 1), not
font-size on a wrapper.

**Sequencing note:** deleting System 3 makes previews *start* obeying pane
zoom in places they previously didn't. That is the fix, but it is also a
visible behavior change on first launch — worth calling out in the changeset
rather than letting it read as a regression.

## 4. Optional follow-ups (do NOT bundle with §3)

Each is independent and none blocks the main fix:

1. **Collapse sites 4-9 onto the shared helpers.** Six views re-implement
   step/clamp/apply against the same `term:zoom` key. Routing them through
   `zoom.platform`'s `zoomBlockIn/Out` would delete the duplication and make
   the step size consistent — swarm in particular has drifted furthest.
2. **De-duplicate `zoom.{win32,darwin,linux}.ts`.** The three copies are
   near-identical; the platform split appears to predate the current shape.
3. **Fold `help:zoom` into `term:zoom`** (site 12). It's a working parallel
   copy — pure consolidation, zero user-visible change if done right.
4. **Consider `em`-ifying `_document-nodes.scss`** *only* if per-preview zoom
   is ever genuinely wanted again. 66 rules; not worth doing speculatively.

## 5. Verification behind this report

Every count and claim was checked against the working tree at `main`
(`f946cb730`), not inferred:

- `66` vs `8` — `grep -c 'font-size:.*px'` and
  `grep -cE 'font-size:[^;]*(em|%)'` on `_document-nodes.scss`.
- The `font-size: N%` application — read directly at
  `ToolOverlayLog.tsx:344`, and its prop chain traced back through
  `ToolBlockOverlay.tsx:36,67` to `ToolBlock.tsx:146,540`.
- Swarm's and helpview's independence — read directly
  (`swarm-view.tsx:30-71`, `helpview.tsx:36-70`), correcting an initial
  survey that had mislabeled swarm as using the shared helpers.
- `85` files in `frontend/` mention "zoom" at all (non-test); the 13 above
  are the subset a user gesture can actually reach.
