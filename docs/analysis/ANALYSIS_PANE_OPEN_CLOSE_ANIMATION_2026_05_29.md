# ANALYSIS: Pane Open/Close Animation — why panes "jerk into place" and what a real fix takes

- **Date:** 2026-05-29
- **Author:** AgentX
- **Area:** `frontend/layout/` (SolidJS tiling layout) + `frontend/app/block/` + (for full coverage) `agentmux-cef` host
- **Goal:** When a pane opens/closes/splits, neighbors should glide into their new geometry instead of snapping.
- **Status:** Root-caused. Initial one-line attempt reverted (PR #1156). Awaiting a direction decision (see §7).

---

## 1. TL;DR

- The tiling layout already has a transition system (`.tile-layout.animate .tile-node { transition: width, height, transform }`), gated by `DefaultAnimationTimeS`, which ships as **0** — so it's dormant. I set it to `0.15s`, expecting panes to glide.
- **It did not work**, verified twice in `task dev`. Reason: the visible pane **content** is not sized by the animating wrapper — it's sized by a **debounced inner rect**, and the debounce interval *is* `animationTimeS`. So raising the value doesn't make content ease; it makes content **hold its old size for 150 ms and then snap**. The empty wrapper eases; the thing you see pops at the end.
- That is almost certainly why the maintainers left the default at 0 — the machinery smooths the wrapper box but not the content, so it was never switched on.
- Only **browser panes are native windows** (HWNDs that CSS cannot animate at all). Terminal / agent / editor panes are **DOM** (xterm / CodeMirror) and *are* animatable in principle.
- A real smooth open/close is therefore a **design choice with an effort/coverage tradeoff**, not a one-liner. Options in §7.

---

## 2. Symptom

Opening, closing, splitting, or rebalancing a pane makes the surrounding panes **snap** to their new size/position in a single frame — a visible jerk. The reported expectation is a short, smooth reflow.

---

## 3. What was attempted (and reverted)

PR #1156 changed `frontend/layout/lib/layoutModel.ts:103`:

```ts
const DefaultAnimationTimeS = 0;   // → 0.15
```

…and added a `prefers-reduced-motion` override in `tilelayout.scss`. Rationale at the time: the `.animate .tile-node` transition uses `var(--animation-time-s)` as its duration, and that variable is fed from `animationTimeS` (default 0), so the transition was instantaneous.

**Outcome:** no visible change to pane open/close (verified twice). The change was reverted from the working tree; the commit still sits on branch `agentx/pane-open-close-animation` / PR #1156 pending a decision.

**Why it was wrong:** it only affects the wrapper box, and it *adds* a 150 ms delay to the content settling on open/close (the debounce — see §5). Net: ineffective and mildly regressive.

---

## 4. How the layout renders a pane (evidence)

### 4.1 The wrapper box — animatable, and the part that *does* transition
- `TileLayout.win32.tsx:336` `DisplayNode` renders one `.tile-node` per leaf, keyed by `props.node.id` (persistent DOM element):
  - `:527` `const tileTransform = () => addlProps()?.transform;`
  - `:531-534` `<div class="tile-node" id={node.id} style={tileTransform()}>` — geometry applied as inline style.
- The geometry is produced by `setTransform` (`utils.ts:63-88`), which emits a fully animatable style:
  ```ts
  { position:"absolute", top:0, left:0,
    transform: `translate3d(${left}px,${top}px,0)`,
    width: `${w}px`, height: `${h}px` }
  ```
- It's written per child during tree layout (`layoutGeometry.ts:153-156`, `additionalPropsMap[child.id] = { rect, transform, ... }`).
- The transition is enabled by `tilelayout.scss:164-172`:
  ```scss
  .tile-layout.animate .tile-node {
      transition-duration: var(--animation-time-s);
      transition-timing-function: linear;
      transition-property: width, height, transform;
  }
  ```
- `--animation-time-s` is set from the model (`TileLayout.win32.tsx:142`, `:136`), and the `.animate` class is applied ~150 ms after mount and while not resizing (`:111-116`, `:177`).

➡️ **The `.tile-node` wrapper transitions correctly** once `animationTimeS > 0`. This part of my change worked.

### 4.2 The content — sized by a *debounced* rect, and the part that snaps
- `block.tsx:166` `const innerRect = useDebouncedNodeInnerRect(nodeModel);`
- `block.tsx:172-183` the block's content size comes straight from that rect:
  ```ts
  const rect = innerRect();
  retVal.width  = `calc(${rect.width}  - ${offset.width}px)`;
  retVal.height = `calc(${rect.height} - ${offset.height}px)`;
  ```
- The debounce (`layoutModelHooks.ts:87-119`) delays applying the new rect by **`animationTimeS * 1000`** ms:
  ```ts
  setTimeout(() => setInnerRect(nodeInnerRect), nodeModel.animationTimeS() * 1000);  // :101-103
  ...
  if (prefersReducedMotion || isMagnified || isResizing) {   // :113 — apply instantly
      clearInnerRectDebounce(); setInnerRect(nodeInnerRect);
  } else {
      setInnerRectDebounced(nodeInnerRect);                   // :117 — delayed
  }
  ```

➡️ With `animationTimeS = 0.15`, on open/close the content **keeps its old width/height for 150 ms, then snaps** to the new size — exactly in step with the wrapper *finishing*. During the animation you get "wrapper box eases while content sits at the old size (clipped or with a gap), then content pops." That reads as a snap / no animation.

The debounce exists for a good reason: during a **resize drag** you don't want heavy panes (terminals, native browser windows) to reflow/repaint every frame, so the content is held and settled once. (During a drag, `isResizing` is true → `:113` applies the rect instantly, and `.animate` is off, so the wrapper follows the cursor directly.) The mechanism was designed around drag, not open/close.

---

## 5. Root cause

**The wrapper and the content are driven by two different geometry channels, and only the wrapper is CSS-transitioned.** The content channel is *debounced by the same `animationTimeS`*, so increasing that value delays the content snap rather than animating it. The pane content — the thing the user actually sees — never eases. Hence: no visible open/close animation, and a small added settle-delay.

Secondary constraint: **browser panes are native child windows.** `pane-rect-registry.ts` is explicitly "a registry of live **native browser-pane HWND rects**," and only `browser_pane_create` registers one. Native HWNDs composite above the DOM and cannot be moved/resized/faded by CSS at all — the host repositions them with a discrete `SetWindowPos`. So even a perfect DOM animation will not animate a browser pane.

---

## 6. Pane-type matrix (what's animatable by which mechanism)

| Pane type | Rendering | CSS-animatable? | Notes |
|---|---|---|---|
| Terminal | xterm.js (DOM canvas) — `app/view/term/term.tsx` | ✅ (DOM) | resizing re-fits xterm each frame (cost) |
| Agent | DOM (pty/xterm-style, `usePtyWidth`) | ✅ (DOM) | same reflow cost as terminal |
| Editor | CodeMirror (DOM) | ✅ (DOM) | cheap-ish reflow |
| Sysinfo / Help / Swarm / etc. | DOM | ✅ (DOM) | cheap |
| **Browser** | **native CEF child window (HWND)** | ❌ | needs host-side Win32 animation |

The DOM panes are the majority; browser is the one that genuinely needs host work.

---

## 7. Options (the decision)

Each is a real strategy with a different effort/coverage/risk profile.

### Option A — Sync the DOM-pane content resize (CSS, no Rust) — *recommended for the actual complaint*
Make the content resize **in step with the wrapper** on open/close:
- For non-drag layout changes, apply the new inner rect **immediately** (don't debounce), and add a CSS transition on the block content `width`/`height` matching the wrapper's `--animation-time-s`. Keep the debounce for the resize-drag path (gate on `isResizing`, which already exists at `layoutModelHooks.ts:113`).
- **Covers:** terminal, agent, editor, and all other DOM panes — i.e. the "neighbors jerk into place" complaint directly.
- **Excludes:** browser panes (still snap — native window).
- **Effort:** moderate; touches `block.tsx` (transition on content) + `layoutModelHooks.ts` (open/close = immediate, drag = debounced) + the `animationTimeS` enablement.
- **Risk:** xterm/CodeMirror re-fit on every frame during the ~150 ms transition can itself look busy on heavy panes. Mitigate with a short duration (≤150 ms) and `ease-out`, or by transitioning `transform: scale` of a snapshot rather than true width/height (more work).

### Option B — Host-side native window animation (full coverage incl. browser)
Animate the native pane **HWND rects in the Rust host** — interpolate `SetWindowPos` over ~150 ms — synchronized with the DOM wrapper transition, so *every* pane (including browser) glides.
- **Covers:** everything.
- **Effort:** substantial; new animation loop in `agentmux-cef` driving per-pane window rects, coordinated with the frontend's `--animation-time-s`. Higher risk (native window timing, multi-window, the existing airspace/overlay-clip interaction).
- **Best if:** browser-pane smoothness is a hard requirement.

### Option C — Enter/exit "pop" only (cheapest)
Don't animate neighbor reflow at all. Animate just the **opening** pane (fade + scale in) and the **closing** pane (fade + scale out) via compositor-only opacity/transform on the block content (mirrors the existing drop-zone `.placeholder` enter/exit at `tilelayout.scss:203-231`).
- **Covers:** the appearance/disappearance of the pane you act on; cheap and smooth, no reflow cost.
- **Excludes:** the neighbor reflow still snaps; browser panes can't fade (native).
- **Effort:** small. **Caveat:** may not satisfy "neighbors jerk into place," since that *is* the reflow.

### Option D — Revert / hold
Close PR #1156, leave open/close as-is.

---

## 8. Recommendation

If the goal is specifically the **neighbor reflow** ("jerks into place"), **Option A** is the most on-target without Rust work, accepting that browser panes still snap and that terminal reflow during the transition needs a tight duration. If browser-pane smoothness is required too, that's **Option B** and should be scoped as its own piece of work. **Option C** is the cheapest and worth considering if perceived smoothness of the acted-on pane is enough.

A reasonable phased path: **A now** (covers the common panes and the literal complaint), **B later** if browser panes must also glide.

---

## 9. PR status

- Branch `agentx/pane-open-close-animation`, PR **#1156** currently holds only the ineffective `DefaultAnimationTimeS 0 → 0.15` + reduced-motion change. Working tree is reverted to baseline.
- Recommend **not merging #1156 as-is**. Either repurpose the branch for the chosen option (A/B/C) or close it (D).

---

## 10. Key files

| File | Role |
|---|---|
| `frontend/layout/lib/layoutModel.ts:103,365` | `DefaultAnimationTimeS`; constructor wiring |
| `frontend/layout/lib/layoutModelHooks.ts:87-119` | **inner-rect debounce** (the crux); `:113` instant on resize/reduced-motion |
| `frontend/app/block/block.tsx:166,172-183` | block content sized from the (debounced) inner rect |
| `frontend/layout/lib/TileLayout.win32.tsx:111-116,136,142,177,336,527,531` | `.animate` gate, `--animation-time-s`, `.tile-node` render |
| `frontend/layout/lib/tilelayout.scss:164-172,203-231` | `.tile-node` transition; `.placeholder` enter/exit reference |
| `frontend/layout/lib/utils.ts:63-88` | `setTransform` → `translate3d` + px width/height |
| `frontend/layout/lib/layoutGeometry.ts:145-191` | per-node rect/transform computation |
| `frontend/app/platform/pane-rect-registry.ts` | native **browser-pane** HWND rects (only browser is native) |
| `frontend/app/view/term/term.tsx` | terminal = xterm (DOM) |
