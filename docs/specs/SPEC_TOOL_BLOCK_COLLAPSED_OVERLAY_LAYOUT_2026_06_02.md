# Collapsed Tool Overlays Are Laid Out While Hidden → Slow Zoom/Scroll

**Status:** Root cause empirically verified (§3); fix implemented (§4)
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

## 4. Fix — lazy-mount the overlay child

Keep the lightweight panel **container** always mounted (so the `max-height`
transition still has a stable element to animate), but mount the heavy
`<ToolBlockOverlay>` **only** while the tool is expanded or animating closed:

```tsx
// ToolBlock.tsx
const COLLAPSE_UNMOUNT_MS = 160; // > the 120ms collapse transition
const [mountOverlay, setMountOverlay] = createSignal(expanded());
let unmountTimer: ReturnType<typeof setTimeout> | null = null;

createEffect(() => {
    const exp = expanded();              // the ONLY reactive dep
    if (exp) {
        if (unmountTimer) { clearTimeout(unmountTimer); unmountTimer = null; }
        setMountOverlay(true);
    } else {
        if (!untrack(mountOverlay)) return;   // already unmounted — no timer
        if (unmountTimer) clearTimeout(unmountTimer);
        unmountTimer = setTimeout(() => { setMountOverlay(false); unmountTimer = null; }, COLLAPSE_UNMOUNT_MS);
    }
});
onCleanup(() => { if (unmountTimer) clearTimeout(unmountTimer); });
```

```tsx
<div class={clsx("agent-tool-panel", { "--hidden": …, "--flow": … })}
     inert={!expanded()} aria-hidden={!expanded()}>
    <Show when={mountOverlay()}>
        <ToolBlockOverlay … />
    </Show>
</div>
```

### 4.1 Why this preserves the animation where it matters

- **History tools** (the bulk — the 32 above) load already-collapsed:
  `expanded()` is `false` at init, so `mountOverlay()` starts `false`, the
  `untrack` guard skips even the timer, and the overlay is **never mounted**.
  These never animate (they were never open), so there is nothing to lose — and
  ~90 % of the zoom cost plus 1,220 DOM nodes disappear.
- **Open animation** (pin / auto-expand): `expanded()` → `true` mounts the
  overlay *and* flips the class to `--flow` in the same tick; the panel container
  was sitting at `max-height: 0`, so it animates `0 → 50vh` with content present.
- **Live auto-collapse** (running → 3 s hold → collapse): `expanded()` → `false`
  keeps the overlay mounted for `COLLAPSE_UNMOUNT_MS` so the `max-height → 0`
  shrink animates with content, then the child unmounts and stops costing layout.
- **Re-expand during the collapse window**: the effect re-runs, clears the
  pending unmount timer, and re-asserts `mountOverlay(true)`.

### 4.2 Reactivity discipline

The effect subscribes to **`expanded()` only**; `mountOverlay` is read via
`untrack` and written via `setMountOverlay`, so it never subscribes to its own
write (avoids the self-loop class of bug already documented in this file's
`postCompletionHold` history). The unmount timer is a plain captured variable,
cleared on cleanup and on re-expand.

---

## 5. Edge cases

- **Overlap-safety (virtualizer):** a collapsed row already measures only its
  summary line (`max-height: 0` panel), so removing the hidden child does **not**
  change the row's measured height — no virtualization-overlap regression
  (cross-ref `SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01`).
- **Streaming live-tail:** the collapsed summary row's `↳ live-tail` line lives in
  `.agent-tool-summary`, not the overlay, so it is unaffected by lazy-mounting the
  overlay.
- **Bookmark / open-in-pane actions** live in the overlay action bar; they are
  only reachable when expanded (overlay mounted), which is unchanged from today
  (`inert` already blocked them while collapsed).
- **`inert` / `aria-hidden`:** still applied to the (now possibly empty) panel
  container based on `expanded()`; harmless on an empty container.

---

## 6. Testing

- **Live A/B re-run (verification of record):** re-run the §3.2 benchmark after the
  fix; collapsed-tool zoom-reflow median should drop from ~300 ms toward the
  ~30 ms `display:none` floor. (jsdom can't exercise CSS `zoom`/layout — the CDP
  probe is the verification of record.)
- **Animation manual check:** pin/unpin a tool → smooth open + 120 ms shrink;
  let a running tool complete → 3 s hold then animated collapse.
- **DOM census:** on-screen `.agent-tool-panel--hidden` should contain ~0 child
  nodes for history tools after the fix.
- **Overlap sweep:** re-run the zoom 0.5–2.0 virtualization sweep; still 0 stuck
  rows / 0 overlaps.

---

## 7. Files

| File | Change |
|---|---|
| `frontend/app/view/agent/components/ToolBlock.tsx` | `mountOverlay()` signal + effect; gate `<ToolBlockOverlay>` behind `<Show>` |
| `docs/specs/SPEC_TOOL_BLOCK_COLLAPSED_OVERLAY_LAYOUT_2026_06_02.md` | this spec |

---

## 8. Risks

- **First-expand paint cost:** the overlay now mounts on first expand instead of
  being pre-built. For a single tool that's a one-row render — negligible, and far
  cheaper than laying out all 32 every zoom. If a specific huge tool ever janks on
  first open, revisit with `content-visibility` on the overlay rather than
  reverting to always-mounted.
- **Animation timing coupling:** `COLLAPSE_UNMOUNT_MS` (160) must stay `>` the CSS
  collapse transition (120 ms) or the content unmounts mid-shrink. Both are
  constants in their respective files; keep them in sync if the transition is
  retuned.
