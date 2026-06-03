# Collapsed Tool Overlays Are Laid Out While Hidden → Slow Zoom/Scroll

**Status:** Fix = `content-visibility: hidden` (§4) + virtualizer crash guard (§4.3); verified live. Lazy-mount approach tried and abandoned (§4.1).
**Date:** 2026-06-02
**Author:** AgentA
**Tracking:** open

---

## 1. Symptom

Zooming an agent pane (Ctrl+`±`, Ctrl+Wheel) is visibly slow / janky when the
conversation contains many tool calls. Scrolling and window-resize over the same
content are also heavier than they should be. The cost scales with the number of
tool blocks, not the number of *expanded* ones.

---

## 2. Architecture recap

Each tool call renders a `ToolBlock` (`frontend/app/view/agent/components/ToolBlock.tsx`):
a one-line summary row plus an `<ToolBlockOverlay>` panel holding the **full** tool
output (params, stdout/stderr, result). The panel has two states:

- **expanded** (`pinned`, `running`, `pending_approval`, or the 3 s
  post-completion hold) → `.agent-tool-panel--flow`, in normal flow.
- **collapsed** (everything else — the default, and how *all* history tools load)
  → `.agent-tool-panel--hidden`.

The panel — including the heavy `<ToolBlockOverlay>` child — is **always rendered
in the DOM** regardless of state. Collapse is purely visual via CSS
(`_document-nodes.scss`):

```scss
.agent-tool-panel        { max-height: 50vh; overflow: hidden; /* +120ms transition */ }
.agent-tool-panel--hidden{ max-height: 0; padding: 0; margin: 0; opacity: 0; }
```

`inert` + `aria-hidden` remove the collapsed panel from the focus/a11y tree, but
**not** from layout. This was deliberate (source comment): keeping the markup
mounted lets the 120 ms `max-height` transition animate the open/close.

Per-pane zoom is a single Chromium CSS `zoom` on `.agent-view`
(`agent-view.tsx:725`). Changing `zoom` invalidates layout + paint for the
**entire** subtree underneath it.

---

## 3. Root cause (empirically verified, live CDP)

`max-height: 0; overflow: hidden` clips the panel **visually** but the browser
**still lays out every descendant** to compute intrinsic sizes (paint is largely
culled; layout is not). So all collapsed tool bodies remain full-cost layout
participants. Any full-subtree layout invalidation — most visibly a `zoom`
change — re-lays-out all of them.

### 3.1 Census (one live pane, 56 rows on screen)

| metric | value |
|---|---|
| tool blocks on screen | 32 (all collapsed) |
| hidden panels | 32 |
| **hidden DOM nodes** | **1,220** |
| **hidden text** | **763,766 chars (~746 KB)** |

All of that is clipped behind `max-height: 0` — invisible, but laid out.

### 3.2 A/B zoom-reflow benchmark

Forced synchronous layout time per `zoom` change, median of 18 samples
(zoom ∈ {0.5, 0.8, 1.2, 1.6, 2.0, 1.0} × 3), measured live via CDP:

| collapsed tool bodies | median reflow | max |
|---|---|---|
| **present (current)** | **299 ms** | 434 ms |
| `display:none` on `.agent-tool-panel--hidden` | **29.5 ms** | 49 ms |

**The clipped-but-laid-out tool content is ~90 % of the zoom relayout cost — a
~10× penalty.** The same dead weight taxes scroll and resize; zoom just makes it
most visible because it invalidates the whole subtree at once.

### 3.3 Why the virtualizer doesn't save us here

The agent-document virtualizer only mounts on-screen rows, and
`content-visibility: auto` on the row wrapper (`_document.scss:194`) skips
*off-screen* rows. But the 32 collapsed tools above are all **on-screen** (that's
why they're mounted), so neither mechanism skips their hidden bodies. The waste is
strictly the *expanded-markup-kept-while-collapsed* decision in `ToolBlock`.

---

## 4. Fix — `content-visibility: hidden` on the collapsed panel

Keep the overlay **mounted** (DOM stable — see §4.2 for why this matters) but tell
the browser to **skip laying it out** while collapsed:

```scss
// _document-nodes.scss
.agent-tool-panel--hidden {
    max-height: 0;
    /* …existing padding/margin/opacity:0… */
    content-visibility: hidden;   // skip layout + paint of descendants
}
```

`content-visibility: hidden` applies `contain: size layout style paint` to the
element and **does not render its descendants** — their layout and paint are
skipped entirely (the element sizes from `contain-intrinsic-size`, here 0, which
agrees with `max-height: 0`). It is the CSS primitive for exactly this case:
"present in the DOM, but cheap." It gives the same ~10× win as `display:none`
(§3.2) while keeping the subtree attached and its rendering state, so showing it
again is fast and — critically — nothing mounts or unmounts.

### 4.1 Why NOT lazy-mount (`<Show>`) — the abandoned approach

The first implementation gated `<ToolBlockOverlay>` behind a `mountOverlay()`
signal so collapsed tools rendered an empty container. It hit the perf target but
**regressed two ways**, both traced live via CDP:

1. **Virtualizer measurement corruption.** Mounting/unmounting the overlay
   changes a virtualized row's height *abruptly, mid-animation*. The virtualizer
   re-measures during that churn and cached transient/garbage sizes — observed a
   collapsed row at virtualizer-size **0** (real 24 → its neighbor placed 24px too
   high = **−20.9px overlap**) and an expanded row at **784** (≈50vh, real 149 →
   phantom gap). These stuck in the measurement cache and did not self-correct.
2. **Crash.** The extra mount/unmount per expand amplified a latent virtualizer
   render race (§4.3) into a reproducible `TypeError: Cannot read properties of
   undefined (reading 'index')` that tore down the whole list via the error
   boundary.

`content-visibility` avoids both because **the DOM never mutates** on collapse/
expand — it is a pure paint/layout-containment toggle, exactly like the old
`max-height` animation the virtualizer already handled cleanly. The collapse
transition also keeps working (the container animates `max-height`; the content
becomes visible/again-hidden via `content-visibility`).

### 4.2 Why the overlay must stay mounted

The virtualizer measures each row and positions the next via `translateY`. Any
mechanism that changes a row's height **as a DOM mutation** (mount/unmount) rather
than a **style change** (max-height / content-visibility) feeds the virtualizer
abrupt transients it caches wrong. Keep height changes in the CSS layer.

### 4.3 Companion: guard the virtualizer against an undefined virtual item

Independently of the overlay work, `AgentDocumentVirtualList`'s
`<For each={virtualizer.getVirtualItems()}>` callback read `virtualItem.index`
with no guard. During the reflow that follows *any* tool expand/collapse height
change, `getVirtualItems()` can transiently yield an `undefined` entry, throwing
`Cannot read properties of undefined (reading 'index')` **during render** — caught
by the error boundary, which tears down the entire list. This is latent on `main`;
the abandoned lazy-mount made it frequent. Fix:

```tsx
{(virtualItem) => {
    if (!virtualItem) return null;   // drop one transient row, don't crash the list
    const nodeAccessor = () => partition().virtualizedNodes[virtualItem.index];
    …
}}
```

---

## 5. Edge cases

- **Overlap-safety (virtualizer):** the DOM is identical to before this change
  (overlay still mounted); `content-visibility: hidden` keeps the collapsed
  panel at 0 height (as `max-height: 0` already did), so row measurements are
  unchanged — no virtualization-overlap regression (cross-ref
  `SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01`).
- **Open animation:** removing `--hidden` drops `content-visibility: hidden`; the
  content lays out and the container animates `max-height 0 → 50vh`. A one-frame
  content "pop" is possible (content-visibility transitions aren't interpolated)
  but is masked by `overflow: hidden` + the opacity fade.
- **Streaming live-tail / bookmark / open-in-pane:** unaffected — live-tail is in
  `.agent-tool-summary`; the action bar is only reachable when expanded.

---

## 6. Testing (verified live via CDP)

- **Zoom-reflow:** median **299 ms → 31 ms** with `content-visibility: hidden`
  (matches the `display:none` floor) — overlay still mounted (32/32),
  `content-visibility: hidden` confirmed applied.
- **Static overlap sweep:** 0 stuck rows / 0 overlaps across scroll, fresh load.
- **Exception:** none on gentle expand/collapse after the `virtualItem` guard;
  the abandoned lazy-mount reproduced the `(reading 'index')` crash.
- **Known pre-existing (OUT OF SCOPE, see §8):** repeated *rapid* expand/collapse
  churn still accumulates measurement drift (mismatched sizes on **non-tool** rows
  too), present on `main` independent of this change. Not addressed here.

---

## 7. Files

| File | Change |
|---|---|
| `frontend/app/view/agent/styles/_document-nodes.scss` | `content-visibility: hidden` on `.agent-tool-panel--hidden` |
| `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` | guard `<For>` callback against an undefined `virtualItem` (§4.3) |
| `docs/specs/SPEC_TOOL_BLOCK_COLLAPSED_OVERLAY_LAYOUT_2026_06_02.md` | this spec |

---

## 8. Risks & out-of-scope

- **First-show paint after content-visibility:** showing a previously-hidden
  panel re-renders its descendants. For one tool that's a single-row render —
  negligible, and far cheaper than laying out all 32 on every zoom.
- **`content-visibility` browser support:** stable in the bundled CEF/Chromium
  runtime (Chrome 85+). No fallback needed; on an unsupporting engine it degrades
  to the prior always-laid-out behaviour (slow, not broken).

### Out of scope — pre-existing measurement drift under churn

Repeated **rapid** expand/collapse (and scroll) churn accumulates virtualizer
measurement drift: rows whose cached size diverges from their rendered height,
including **non-tool** (markdown / user-message) rows that this change does not
touch. It is present on `main` independent of this work, reproduces with
`content-visibility` toggled off, and is a separate, deeper virtualizer
re-measurement issue (the cache not reconverging after dynamic height changes).
The `virtualItem` guard (§4.3) stops the *crash*; the *drift* is a follow-up,
**tracked in issue #1235**, not in this PR. Clean controlled repro recorded there
(fresh reload 0/0 → 9 expand/collapse cycles → 5 mismatched rows incl. non-tool, 1
overlap).
