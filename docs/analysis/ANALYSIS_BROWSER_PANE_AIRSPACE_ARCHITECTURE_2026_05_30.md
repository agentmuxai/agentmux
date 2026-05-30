# Browser-Pane Airspace — Architecture Reassessment

**Date:** 2026-05-30 · **Author:** AgentX · **Trigger:** runtime test of #1178 on Windows surfaced
"right-click context menus appear wrong around browser panes — black spots in offset menus, menus
sometimes hidden."

> **Verdict (one line):** Our understanding of the *mechanism* is rigorous and well-documented.
> The reported symptoms are most likely a **concrete, fixable coordinate-unit bug** (overlay rects
> sent in CSS pixels, pane geometry computed in physical pixels) — **not** evidence that the
> architecture is wrong. A new architecture (off-screen / shared-texture rendering) is the
> *principled long-term* fix for a class of residual fidelity gaps, but is **not required** to
> resolve what the user is seeing. **Confirm display scale before doing anything else.**

---

## 1. How it works today (confirmed from source)

On **Windows**, a browser pane is a **native OS child window** — `CefBrowserHost::CreateBrowser`
produces a windowed (not off-screen-rendered) child `HWND`
(`agentmux-cef/src/browser_panes.rs:4`). A native child HWND **always composites above** the
Chromium-rendered DOM, regardless of CSS `z-index`. This is the classic **"airspace" problem**
(`pane-overlay.ts:4-16`, `browser_panes.rs:500-507`).

The workaround — **punch a rectangular hole** in the pane HWND wherever a DOM overlay needs to show:

1. Any DOM element that can overlap a pane tags itself `data-pane-overlay`
   (`pane-overlay-auto.ts:7-17`). FlyoutMenu/context menus and their submenus do this
   (`flyoutmenu.tsx:173`, `:316`).
2. A `MutationObserver` + per-element `ResizeObserver` measure each tagged element's rect via
   `getBoundingClientRect()` (`pane-overlay.ts:190-198`, `pane-overlay-auto.ts:65-96`).
3. Rects are unioned, rAF-coalesced and deduped, and sent over `browser_panes_set_overlay_clip`
   (`pane-overlay.ts:75-147`). A CSS-pixel pane-rect registry gates the IPC out when nothing
   intersects (`pane-rect-registry.ts`).
4. The host calls `SetWindowRgn` with `RGN_DIFF` to subtract the overlay rects from the pane's
   region (`browser_panes.rs:516-727`). The pane is transparent inside the hole, so the DOM
   overlay painted at the same screen position shows through.

**#1178** (the PR under test) adds `pane_clip_cache` — a per-pane signature so a redundant
`SetWindowRgn` is skipped when the region is unchanged (`browser_panes.rs:132-170`, `:603-711`).
It is a **performance cache layered on top of this model**; it does not change the model.

**macOS/Linux** do *not* use child HWNDs. Panes are `CefBrowserView`s added as overlay views
(`add_overlay_view`). There is **no way to punch a hole** through an Aura view, so that path's
"workaround" is to **hide the entire pane** whenever any overlay rect intersects it
(`browser_panes.rs:728-862`, `compute_pane_visible`). So the airspace problem is **unsolved on
every platform** — Windows hides a *rectangle* of the pane, macOS/Linux hide the *whole* pane.

---

## 2. Symptom → cause mapping

The user reported three things: **black spots**, **offset**, **hidden** — specifically around
right-click context menus over browser panes. Ranked by likelihood:

### (A) PRIMARY HYPOTHESIS — CSS-px vs physical-px coordinate mismatch  *(high confidence; needs runtime confirm)*

- Overlay rects are produced from `getBoundingClientRect()` **with no `devicePixelRatio` multiply**
  — i.e. **CSS pixels** (`pane-overlay.ts:190-198`; explicitly stated in `pane-rect-registry.ts:13-19`:
  *"the overlay rects are produced from `getBoundingClientRect()` without a dpr multiply"*).
- The Rust clip computes the pane's geometry from `GetWindowRect` + `MapWindowPoints` — **physical
  pixels** — and subtracts the overlay rect directly: `left = ox - pane_rect.left`
  (`browser_panes.rs:633-691`). **No DPI scaling is applied to the overlay rects on the host side.**
- `browser-view.tsx` is the **only** place `devicePixelRatio` is used, and only to compute the
  pane's *resize* rect (CEF/`SetWindowPos` want physical) — never the overlay rects.

**Consequence:** at display scale = 100% (`dpr = 1`) CSS px == physical px and everything lines up.
At **125% / 150% / 175%** the punched hole is **offset and undersized** relative to where the menu
actually paints:
- where the hole lands but the menu does **not** paint → you see straight through the pane to the
  parent window background = **black spot**;
- where the menu paints but **no** hole was punched → the native pane covers it = **hidden**;
- the net visual is a menu and its "show-through" region **drifting apart** = **"offset menus."**

This matches all three reported symptoms with a single root cause. The intersection *gate* is
computed CSS-vs-CSS (`pane-rect-registry.ts`) so it stays correct and the IPC still fires — the
corruption is purely in the host-side hole math.

> **The drag work already flagged this neighbourhood:** the title-bar drag was *"worse at 125%
> scale"* and the manual move loop was praised for *"no DPI math"*
> (`HANDOFF_UX_LATENCY_DRAG_2026_05_30.md`, `SPEC_WINDOW_DRAG_*`). DPI handling is a known soft spot.

### (B) SECONDARY — geometry fidelity of a rectangle vs painted pixels  *(structural, minor for this menu)*

`getBoundingClientRect()` returns the **rectangular border-box**. It excludes `box-shadow` /
`filter: drop-shadow`, and a rectangle cannot represent rounded corners or inter-element gaps.
For the *flyout* menu specifically this is **not** the main culprit: `flyoutmenu.scss` shows
`border-radius: 0` and an opaque padded background, so its border-box is fully painted. But this
gap is real for any overlay with a shadow, radius, or transparent padding, and for the **gap
between a menu and its offset submenu** (two separate portaled rects — `flyoutmenu.tsx:311-391`).

### (C) TERTIARY — temporal lag  *(structural; worsens "rapid hover" + "hidden")*

The clip trails the paint through a long async pipeline: DOM mutation → MutationObserver → track →
ResizeObserver/`updateRect` → `sendClip` (rAF) → `flushClip` → IPC → tokio handler → `SetWindowRgn`
→ `InvalidateRect` → next paint (`pane-overlay-auto.ts`, `pane-overlay.ts:75-147`). For a quick
right-click→read→dismiss the menu can paint *before* the hole lands → momentarily hidden/flickering.
The rAF coalescing and #1178's cache improve throughput but **widen** the window between "menu
painted" and "hole applied."

### (D) #1178's specific role

#1178 is a cache on a model whose fidelity is already imperfect. The one real regression it
introduced (recording a clip signature without checking `SetWindowRgn`'s return) was caught by
claude-opus review and **already fixed** in `89962f6` (`browser_panes.rs:608/619-620, 698/707-711`).
A stale-cache wrong-skip *could* contribute to a "hidden" case, but it cannot produce **black
spots** — those require a hole in the wrong place, which is the coordinate bug (A). So #1178 is
**not** the source of what the user saw; at most it can be a minor amplifier of (C).

---

## 3. Is our architecture understanding rigorous?

**Yes, for the mechanism.** The code is unusually well-documented: it correctly names the airspace
problem, explains why `SetWindowRgn` is the only lever on a child HWND, scopes clips per window
(Codex P1 #544), handles the clear-on-non-intersect transition (#1097 fix #1), coalesces per-frame
(#1097 fix #2), and reasons carefully about HWND reuse and cache desync (#1178). The
review history shows real adversarial scrutiny.

**The blind spot is fidelity, not mechanism.** The model assumes a DOM overlay can be represented to
the host as **one or more axis-aligned rectangles in a coordinate space the host shares.** Three
places that assumption leaks:
1. **Coordinate space** — CSS px vs physical px (bug A). *This is a defect, not a law of nature —
   fixable.*
2. **Shape** — rectangles can't express shadows / radius / gaps (B). *Inherent to rect-punching.*
3. **Time** — the hole is always at least one async hop behind the paint (C). *Inherent to
   "frontend measures, host punches."*

(1) is a straightforward bug. (2) and (3) are **inherent** to compositing a native child-HWND
surface against DOM by subtracting rectangles — the same wall every windowed-embed approach hits
(legacy CEF windowed, Electron native `<webview>`, windowed WebView2).

---

## 4. Do we need a new architecture?

**Not to fix the reported symptoms.** Symptom (A) — overwhelmingly the most likely cause — is a
coordinate-unit fix within the current model. Ship that first and the black-spots/offset/hidden
very likely disappear at non-100% scale.

**Yes, eventually, if we want pixel-correct overlays without per-overlay babysitting.** Rect-punch
will always carry (2) and (3), plus the macOS/Linux path is an even cruder hack (hide the whole
pane). The principled fix is to **stop putting the pane in its own airspace**:

| Option | What | Pros | Cons / Cost |
|---|---|---|---|
| **0. Fix coordinate units** *(do now)* | Multiply overlay rects by `devicePixelRatio` before the clip IPC (or send CSS px + scale on the host using the pane's DPI). Mirrors what `browser-view.tsx` already does for pane resize. | Tiny, surgical; almost certainly resolves the report; no architecture change. | Leaves (2)/(3) residuals (shadows, fast-hover flicker). |
| **0b. Tighten the existing model** | Outset rects by shadow extent; apply the clip synchronously on menu-open (skip first rAF); audit #1178 cache for wrong-skips during gestures. | Cheap; reduces residual artifacts + flicker. | Still rectangles; still trailing; growing special-case surface. |
| **1. Native overlay popups** | Render menus/tooltips/context-menus in their own borderless top-most child HWND so they share the pane's airspace and stack above it by Z-order — no punching. | Eliminates airspace for the worst offenders without re-architecting panes. | Fragments rendering — DOM menu content must live in a separate native surface (effectively OSR-for-menus); input/focus plumbing. |
| **2. Off-screen / shared-texture rendering (OSR)** *(principled, long-term)* | Render the pane to a GPU texture (CEF windowless + `OnAcceleratedPaint`, D3D11 shared handle) and composite it as a layer the **main** Chromium compositor draws. | Removes airspace **entirely, on all platforms**; DOM overlays stack by z-index; no hole math, no DPI mismatch, no lag; unifies the Windows HWND path with the macOS/Linux Views hack. | Largest change: input/IME routing, per-frame texture compositing, perf tuning. Historically slower, but shared-texture OSR is production-grade today (Chrome embedders, OBS). Team already has `docs/research/cef-transparency-research-2026-05-10.md` as a starting point. |

**Recommendation:** Option **0 now** (verify scale → apply DPR), Option **0b** as fast-follow for
residual polish, and **scope a spike for Option 2 (shared-texture OSR)** as the real end state —
explicitly framed as "retire the airspace workaround on all platforms," not just a Windows menu fix.

---

## 5. Action plan

1. **VERIFY (decisive, do first):** ask the user / check the Windows display scale (Settings →
   System → Display → Scale). Re-run the right-click-over-pane test at **100%**.
   - Symptoms vanish at 100% but present at 125%+ → confirms hypothesis (A); go to step 2.
   - Symptoms persist at 100% → (A) is not it; re-weight toward (B)/(C) and instrument the actual
     overlay rect vs the painted menu rect (log `rectsToSend` and compare to the menu's on-screen box).
2. **FIX (A):** apply `devicePixelRatio` to overlay rects in `pane-overlay.ts` (and the auto-clip
   path) before dispatch — or pass CSS px + DPR and scale host-side — matching `browser-view.tsx`'s
   resize path. Add a regression note/test for the unit contract. Small PR.
3. **POLISH (0b):** outset rects by computed shadow extent; apply the first clip synchronously on
   overlay open; confirm #1178's cache can't wrong-skip mid-gesture.
4. **DECIDE (2):** open a discussion / spike for shared-texture OSR as the cross-platform airspace
   retirement. Reference `docs/research/cef-transparency-research-2026-05-10.md`.

## 6. Bearing on #1178

#1178 is sound *as a cache* and opus-approved on current HEAD; it neither causes nor fixes these
visuals. Recommend: keep it on its merge track (after the Windows build is restored by the
`agentx/fix-windows-build-redock-cfg` PR), but **do not treat its merge as "airspace fixed."** The
airspace correctness work is steps 1–4 above and is independent of #1178.

---

### File references
- `agentmux-cef/src/browser_panes.rs` — `set_pane_overlay_clip` (Win32 `SetWindowRgn`), `pane_clip_cache`, non-Windows hide-whole-pane path
- `frontend/app/platform/pane-overlay.ts` — overlay rect collection, `rectFromElement` (CSS px), rAF coalesce + dispatch
- `frontend/app/platform/pane-overlay-auto.ts` — `data-pane-overlay` auto-discovery observers
- `frontend/app/platform/pane-rect-registry.ts` — CSS-px pane registry + intersection gate (documents the no-DPR contract)
- `frontend/app/element/flyoutmenu.tsx` / `flyoutmenu.scss` — context-menu DOM, `data-pane-overlay` tagging, square/opaque styling
- `frontend/app/view/browser/browser-view.tsx` — the only `devicePixelRatio` user (pane resize, not overlays)
- `docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md`, `docs/research/cef-transparency-research-2026-05-10.md`
