# SPEC: Submenu positioning flash + hover-intent (safe-triangle) timing

**Date:** 2026-08-10
**Status:** Draft
**Driving observation (verbatim):** "1) on after right-click to open a menu,
if I hover over a submenu, the subpanel that loads will appear completely
offset in the upper left of the screen away from where it is supposed to
be, but when I hover and the subpanel loads again, it loads correctly
placed. i see that in different forms along all the different types of
menus. 2) The submenus are often hard to peruse with them unpainting
before i can move the mouse. we need to research best practices on timing
and specing, perhaps we need a solid framework for it app-wide... take a
look, write a comprehensive spec."

---

## TL;DR

Two bugs, two root causes, one fix shape:

1. **Positioning flash** — `FlyoutMenu`'s `SubMenu` (`frontend/app/element/flyoutmenu.tsx`)
   paints at its placeholder style (`position:fixed;left:0px;top:0px` — the
   viewport's upper-left corner) **fully visible**, then jumps to the real
   `computeMenuPosition()`-derived position one RAF + one microtask later.
   The sibling implementation (`showJsContextMenu` in `frontend/util/cef-api.ts`)
   already solved this by holding the submenu `visibility:hidden` until the
   position resolves — `SubMenu` never got the same guard. This isn't a
   "sometimes" bug so much as a **flash whose duration depends on how fast
   that RAF/promise resolves** — often imperceptible, occasionally a visible
   snap to the corner, which matches "appears offset, then loads correctly
   on reload."
2. **No hover-intent / safe-triangle grace period** — confirmed by direct
   code reading: **neither** submenu implementation has ever had a
   close-side delay. Both close the instant the cursor leaves the
   triggering row (`FlyoutMenu`: the instant a *different* item is
   hovered; `cef-api.ts`: instant `mouseleave`). Diagonal mouse movement
   from the parent item into the submenu panel routinely crosses "dead"
   space first and slams the submenu shut before the user arrives.

Only **two** submenu-hover implementations exist in the current codebase
(not "many independent" ones as suspected) — but they diverge in exactly
the way that produces "I see it in different forms": one has the
visibility guard, one doesn't; neither has hover-intent. The fix is a
single shared primitive both route through, covering positioning-safety
and close-intent uniformly — see §5.

---

## 1. Current state — exactly two implementations, confirmed by reading current code

| | **Implementation A — `SubMenu`** | **Implementation B — `showJsContextMenu` submenu block** |
|---|---|---|
| File | `frontend/app/element/flyoutmenu.tsx:376-515` | `frontend/util/cef-api.ts:223-273` |
| Render model | SolidJS component, reactive signal | Vanilla DOM, imperative |
| Used by | Hamburger menu's Theme/Opacity submenus (`hamburger-menu.tsx:98,103`), `MenuButton` (block-header menu button) | Every native right-click menu with a nested submenu: pane-header "Pane Color" (`blockframe.tsx:43-73`), pane-body "Replace With…" (`pane-actions.ts`), terminal Themes/Font Size/Zoom/Transparency (`termSettingsMenu.ts`), sysinfo "Plot Type" (`sysinfo-model.ts`), Armory "Bind to Agent" (`bind-to-agent-menu.ts`, `SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md`) |
| Position primitive | `computeMenuPosition()` — `frontend/app/util/menu-position.ts:228` (shared by both) | same `computeMenuPosition()` |
| Position lifecycle | `registerSubMenu` ref → `requestAnimationFrame` → `autoUpdate()` (floating-ui) re-runs `computeMenuPosition` on scroll/resize | one-shot: `mouseenter` → `computeMenuPosition().then(...)`, no `autoUpdate` (acceptable — a native right-click menu doesn't need to track scroll) |
| **Visible before positioned?** | **Yes — the bug.** Initial signal value `"position:fixed;left:0px;top:0px"` (line 384) has no `visibility` property; the `<div>` renders with that style immediately on mount, fully opaque, at the viewport's top-left, until the RAF-gated `computeMenuPosition().then(setSubStyle(...))` chain resolves. | **No.** `sub.style.visibility = "hidden"` is set *before* `computeMenuPosition()` is called (line 253) and only cleared after `.then()` resolves (line 267) — explicitly commented as "avoids a one-frame flash at the cursor." |
| Open delay | 0ms (synchronous `onMouseEnter`) | 0ms (synchronous `mouseenter`) |
| Close trigger | Sibling item's `mouseenter` marks this key `visible: false` (`handleMouseEnterItem`, lines 144-166) — **there is no `mouseleave` handler anywhere in this file** (confirmed via grep, zero occurrences) | `row.addEventListener("mouseleave", () => { sub.style.display = "none"; })` (line 273) — instant |
| Close delay | 0ms | 0ms |
| Safe-triangle / hover-intent | None | None |

### 1.1 Bug 1 root cause — ranked

**Primary (high confidence, structural, confirmed by diff-reading the two implementations):**
Implementation A never adopted the `visibility:hidden`-until-placed guard
that implementation B already has, and that B's own comment explicitly
frames as fixing this exact class of bug ("avoids a one-frame flash at the
cursor"). The RAF gate in `registerSubMenu` (flyoutmenu.tsx:395) protects
against measuring a *zero-sized, unlaid-out* node — a different problem —
but does nothing to hide the node while it sits at the placeholder
coordinates. Every submenu open through Implementation A paints at
`(0, 0)` for at least one frame; whether a user perceives it as a jarring
snap depends on how long the RAF + `computeMenuPosition()` promise chain
takes to settle, which varies with layout cost, font-metric caching, and
general system load — exactly the kind of intermittent, "sometimes I see
it" symptom described.

**Secondary (plausible contributor, needs instrumentation to confirm before fixing):**
`handleMouseEnterItem`'s `setVisibleSubMenus` updater (flyoutmenu.tsx:144-166)
does a shallow copy of the *outer* map but mutates the *inner* per-key
objects in place:
```tsx
setVisibleSubMenus((prev) => {
    const updatedState = { ...prev };              // shallow copy — outer map only
    updatedState[key] = { visible: true, label: item.label };
    ...
    for (const pkey in updatedState) {
        if (!ancestors.includes(pkey) && pkey !== key) {
            updatedState[pkey].visible = false;     // mutates the SAME object prev[pkey] pointed to
        }
    }
    return updatedState;
});
```
Because `updatedState[pkey]` is the same object reference as `prev[pkey]`,
this write is not purely functional — it's possible for a still-committing
render pass to observe a half-mutated state object. Recommend
instrumenting `position()`'s value inside `registerSubMenu`'s RAF callback
on a first-vs-second hover of the same item to confirm or rule this out
before patching (§8, test plan).

**Tertiary (defensive gap, not necessarily the direct cause):**
`registerSubMenu`'s RAF callback has no `el.isConnected` check before
wiring `autoUpdate` (contrast with `assertMenuInPaintableArea`, which does
check `isConnected`). A rapid re-hover within the same frame — mouse
leaves and returns to the same item fast enough that Solid tears down and
recreates the `<SubMenu>` DOM node between the `ref` callback firing and
the RAF firing — could attach `autoUpdate` to an already-detached node
while the new one is never positioned. Worth ruling out alongside the
above.

### 1.2 Bug 2 root cause — confirmed, no ranking needed

Fully established by direct code reading, not inference: `setTimeout` and
`mouseleave` were grepped exhaustively across `frontend/` for any
submenu-adjacent hover-timing logic. **Zero close-delay exists in either
implementation, and no safe-triangle/pointer-trajectory check exists
anywhere in the codebase.** Prior art (`fa167063` / PR #792, "instant
submenu open — drop setTimeout(0) + visibility-hidden + autoUpdate
polling") deliberately removed an *open*-side `setTimeout(0)` for latency
and never considered a *close*-side grace period — that PR's own
description frames the change purely as an open-latency win ("~33ms →
~2ms"), with no UX pass on closing. This spec is the first time close
timing has been looked at.

The codebase already has an established, tested pattern for hover-intent
*elsewhere* — worth mirroring rather than inventing new conventions:
- `frontend/app/view/agent/components/UserMessageBlock.tsx:92-131` —
  `mouseenter` → 150ms delay → expand; `mouseleave` cancels the pending
  timer. Covered by `UserMessageBlock.test.tsx:189`.
- `frontend/app/element/tooltip.tsx` — enter/leave delay handling for
  tooltip show/hide.

Neither is submenu-specific and neither implements safe-triangle geometry
(they gate on time only, not cursor trajectory), but both establish that
timer-based hover-intent is a known, tested pattern in this codebase.

---

## 2. Best-practice research — timing and geometry for submenu hover

*(Web research, 2026-08-10 — sources at the end of this section.)*

The "submenu closes before I can reach it" problem is a 30+ year old,
well-documented UI problem with a standard name: the **safe triangle**
(aka hover triangle, Amazon triangle, hover tunnel, extended mouse
corridor — traced to Bruce Tognazzini/Jim Batson's work on Apple's HID
team). Two implementation families dominate current practice:

### 2.1 Geometry-based: `safePolygon` (Floating UI's approach)

Floating UI — **already a direct dependency of this codebase**
(`@floating-ui/dom`, used throughout `menu-position.ts`) — ships a
`safePolygon()` interaction handler (in its React bindings package,
`@floating-ui/react`) built exactly for this: it computes a polygon from
the cursor's exit point to the floating (submenu) element's near edges,
and keeps the submenu open as long as the cursor stays inside that
polygon while moving toward it. Key parameters from the library's own
defaults and community usage:
- Recommended **open delay ~75ms** paired with `safePolygon` for close —
  the small open delay absorbs accidental hovers while sweeping across
  sibling rows (a secondary win: it also gives `computeMenuPosition()`
  time to resolve *before* the submenu becomes visible at all, which
  independently helps Bug 1).
- `requireIntent: true` (the default) additionally checks cursor
  *velocity* — a cursor that has stopped moving toward the submenu is
  treated as "given up," closing it even if still geometrically inside
  the safe polygon. Prevents the submenu from being stuck open
  indefinitely if the user parks the mouse nearby without committing.
- `blockPointerEvents: true` matters operationally: without it, DOM
  elements between the trigger row and the submenu can themselves
  intercept `mouseleave`/`mouseenter` and cause a premature close —
  directly relevant to Implementation A's sibling-triggered close.
- The polygon test requires the trigger and the floating submenu to
  share a coherent coordinate space / stacking context — a portal that
  renders the submenu somewhere structurally distant from its trigger
  can break the hit-test. Both current implementations already portal
  the submenu (`<Portal>` in `flyoutmenu.tsx`, `document.body` append in
  `cef-api.ts`) — the new shared primitive needs the polygon computed in
  viewport (`position:fixed`) coordinates, not relying on DOM adjacency,
  to stay correct through the portal.

`safePolygon` itself is a React-hook API (`@floating-ui/react`'s
`useHover`/`useInteractions`), not usable directly from SolidJS or vanilla
DOM — but the underlying algorithm is plain geometry (build a
triangle/quad from last-known cursor position to the floating element's
corners, point-in-polygon test on `mousemove`) and is framework-agnostic
to port. This spec recommends porting the **algorithm**, not depending on
the React package.

### 2.2 Delay-based: VS Code's approach

VS Code's own context-menu implementation uses a simpler delayed-trigger:
apply the `:hover` CSS state immediately for visual feedback, but debounce
the actual open/close *action* callback. Less precise than geometry-based
safe-triangle (doesn't know whether the cursor is moving *toward* the
submenu, just that it hasn't left yet), but far simpler to implement
correctly and to reason about, and sufficient for menus with modest
item density. Cited as reasonable prior art for the close-side fallback
below.

### 2.3 Recommendation for this codebase

Combine both, in priority order — geometry primary, timer fallback:

1. **Small open delay (~75-100ms)** before a submenu becomes visible at
   all on `mouseenter`. Serves double duty: absorbs accidental
   triggers while sweeping across sibling rows, *and* — combined with
   holding `visibility:hidden` during that window while
   `computeMenuPosition()` resolves — structurally eliminates Bug 1's
   flash, because the submenu is never shown before both (a) it's
   positioned and (b) the hover has been sustained long enough to be
   intentional.
2. **Safe-triangle polygon check on close.** On `mouseleave` from the
   trigger row, don't close immediately — start tracking `mousemove`
   and test whether the cursor is inside the polygon formed by its
   position and the submenu panel's near corners. Keep the submenu open
   while inside the polygon; close as soon as it exits (or after a
   short absolute timeout as a safety net, ~300ms, in case the
   `mousemove` tracking itself fails to attach for some reason —
   never leave a submenu open indefinitely with no listener).
3. **`requireIntent`-style velocity check** is a nice-to-have, not a v1
   requirement — the absolute timeout safety net (above) already
   prevents "stuck open" in the common case; skip in v1 unless testing
   shows it's needed.

Sources:
- [Better Context Menus With Safe Triangles — Smashing Magazine](https://www.smashingmagazine.com/2023/08/better-context-menus-safe-triangles/)
- [Better Context Menus With Safe Triangles | Medium](https://medium.com/@mike.articonbusiness/better-context-menus-with-safe-triangles-ad0e45b63a95)
- [No More Menu Rage: Smooth Navigation with useSafeArea — Rippling](https://www.rippling.com/blog/no-more-menu-rage-smooth-navigation-with-usesafearea)
- [radix-ui/primitives#2437 — Safe triangles for submenus](https://github.com/radix-ui/primitives/issues/2437)
- [The context menu safety triangle | Medium](https://medium.com/@nielsmanders/the-context-menu-safety-triangle-411c75065374)
- [floating-ui/floating-ui#3420 — Menu with submenu discussion](https://github.com/floating-ui/floating-ui/discussions/3420)
- [Interactive dropdown menus with Radix UI](https://www.joshuawootonn.com/radix-interactive-dropdown)

---

## 3. Why this is genuinely two implementations, not one — and why unify anyway

Unlike the *visual* unification proposed (and never actioned — see
`SPEC_UNIFIED_MENU_SYSTEM_2026_05_11.md`'s 2026-08-07 stale-note), the
*positioning primitive* (`computeMenuPosition`) is already correctly
shared per `SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20.md` — both
implementations call the same function. What is **not** shared is the
lifecycle glue around it: RAF-gating, visibility-hiding, `autoUpdate`
wiring, and (the net-new part) hover-open/close timing. That glue is
hand-rolled twice, has silently diverged (one has the visibility guard,
one doesn't), and has zero hover-intent in both. A third submenu surface
appearing tomorrow (the pattern shows up roughly every few months per git
history — Pane Color #1884, Bind-to-Agent #2485) would almost certainly
reinvent it a third way rather than compose the existing primitive,
exactly per `SPEC_MENU_PAINTABLE_AREA_GUARD`'s own §4 diagnosis of why
positioning drifted in the first place ("nothing enforces a single path").

**Correction to prior research for this spec:** an earlier pass suggested
a `useMenuPosition()` React/Solid-style hook already exists in
`menu-position.ts` as unused/dead code, and a `MenuBuilder` class with a
`.submenu()` method exists elsewhere as dead code. **Neither claim holds
under direct inspection** — `menu-position.ts` exports exactly
`MenuPositionRequest`, `MenuPositionResult`, `getNativePaneRects`,
`getPaintableArea`, `computeMenuPosition`, and `assertMenuInPaintableArea`;
no `useMenuPosition` function exists anywhere in the file (only a stray
comment in `flyoutmenu.tsx:129` that reads as if referencing one). No
`menu-builder.ts` or `MenuBuilder` class exists anywhere in the repo. Both
were an earlier research pass's error — flagged here so it isn't
propagated. The Armory "Bind to Agent" submenu (`bind-to-agent-menu.ts`)
*does* exist (PR context: `SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md`)
and routes through Implementation B (`ContextMenuModel.showContextMenu` →
`showJsContextMenu`) — it is not a third implementation, just a caller
that an earlier grep pass missed by searching the wrong directory
(`view/armory/` instead of `view/identity/`).

---

## 4. Proposed shared primitive — `openSubmenu()` / hover-intent core

One **framework-agnostic** module (not a Solid hook — it must work from
both SolidJS component code and vanilla-DOM `cef-api.ts` code), living
alongside the existing positioning primitive:

`frontend/app/util/submenu-hover.ts`

```ts
export interface SubmenuHoverController {
    /** Call on the trigger row's mouseenter. Schedules the open. */
    onTriggerEnter(): void;
    /** Call on the trigger row's mouseleave, with the current cursor pos. */
    onTriggerLeave(e: MouseEvent): void;
    /** Call on mousemove while a close is pending (safe-triangle tracking). */
    onTrackedMouseMove(e: MouseEvent): void;
    /** Call once the submenu element exists, to register its rect for the polygon test. */
    setSubmenuEl(el: HTMLElement | null): void;
    /** Tear down all timers/listeners — call on unmount / menu close. */
    dispose(): void;
}

export interface SubmenuHoverOptions {
    openDelayMs?: number;          // default 90
    closeSafetyTimeoutMs?: number; // default 300 — absolute fallback if polygon tracking stalls
    onOpen: () => void;             // caller shows the submenu (still visibility:hidden — see below)
    onClose: () => void;            // caller hides/unmounts the submenu
}

export function createSubmenuHover(opts: SubmenuHoverOptions): SubmenuHoverController;
```

This owns **timing and the safe-triangle geometry only**. It does not
know about `computeMenuPosition` — that stays exactly as-is, called by
the caller inside `onOpen`. The caller is responsible for:
1. Keep the submenu `visibility:hidden` (or `display:none`, matching each
   implementation's existing pattern) until `computeMenuPosition()`
   resolves — this is the direct Bug-1 fix, made mandatory by moving it
   into the shared contract instead of leaving it to each call site to
   remember.
2. Feed `setSubmenuEl()` once the submenu node exists, so the polygon
   test has real geometry instead of guessing.

### 4.1 Migration shape per implementation

| Implementation | Change |
|---|---|
| `FlyoutMenu`'s `SubMenu` (flyoutmenu.tsx) | Replace the ad-hoc `handleMouseEnterItem`/sibling-close logic with `createSubmenuHover`; keep `computeMenuPosition`/`autoUpdate` wiring as-is, but gate `subStyle`'s visibility on the controller's open state instead of rendering the placeholder style live. Delete the shallow-copy-but-mutates-nested-objects updater (§1.1 secondary) as part of this pass — the controller's own state replaces `visibleSubMenus`' role for close timing; `visibleSubMenus` keeps its existing role for open state. |
| `showJsContextMenu` submenu block (cef-api.ts) | Replace the raw `mouseenter`/`mouseleave` pair with `createSubmenuHover`; the existing `visibility:hidden`-until-placed logic slots directly into `onOpen`, unchanged in spirit. |

Both implementations keep their existing render strategy (Solid reactive
vs. vanilla DOM) — only the timing/safe-triangle *decision logic* is
shared, not the rendering.

### 4.2 Why not go further and unify the renderers too

Out of scope for this spec. `SPEC_UNIFIED_MENU_SYSTEM_2026_05_11.md`
already proposed (and never shipped) a full renderer unification
(`<Menu>` Solid component replacing both). That's a larger, higher-risk
change or­thogonal to the two concrete bugs here. This spec's shared
primitive is deliberately renderer-agnostic so it doesn't depend on that
unification landing first, but doesn't preclude it — if `<Menu>` ever
ships, it would consume `createSubmenuHover` too, as the one remaining
caller.

---

## 5. Implementation phases

### Phase 1 — `submenu-hover.ts` core + unit tests (0.5 day)
Pure logic, no DOM framework dependency: open-delay timer, safe-triangle
polygon test (point-in-polygon against trigger-exit-point → submenu
corners), absolute close-safety timeout, `dispose()` cleanup. Unit tests:
cursor moving straight toward submenu stays open past the trigger's
bounds; cursor moving away closes promptly; cursor parked motionless
outside both rects closes after the safety timeout; rapid open/close
cycling doesn't leak timers.

### Phase 2 — migrate `cef-api.ts`'s submenu block (0.5 day)
Lower-risk migration first — this implementation already has the
visibility-hiding half of the contract; only the close-timing half is
new behavior. Smoke-test every Implementation-B caller (§1 table).

### Phase 3 — migrate `FlyoutMenu`'s `SubMenu` (1 day)
Higher-risk: touches the Solid reactive state (`visibleSubMenus`,
`hoveredItems`) and removes the sibling-triggered-close logic in favor of
the controller. Also fixes Bug 1 directly here (visibility gating) and
removes the shallow-copy aliasing bug identified in §1.1. Covers
Hamburger Theme/Opacity submenus and any `MenuButton` caller.

### Phase 4 — dev-mode instrumentation for Bug 1 confirmation (0.25 day, can run before Phase 3)
Temporary `console.debug` (behind `AGENTMUX_DEV=1`, same gate as
`assertMenuInPaintableArea`) logging `position()`'s value and elapsed
time-since-mount inside `registerSubMenu`'s RAF callback, to empirically
confirm/deny the §1.1 secondary (state-aliasing) and tertiary
(disconnected-node) hypotheses before or alongside the Phase 3 rewrite.
Removed once Phase 3 ships (the rewrite makes the old code paths moot).

**Total: ~2.25 days.**

---

## 6. Test plan

- [ ] Unit: `submenu-hover.ts` — open delay fires once per sustained hover, cancels on early leave; safe-triangle keeps submenu open for diagonal cursor paths toward it; closes on paths away from it; absolute safety timeout fires if `mousemove` tracking never resumes.
- [ ] `task dev` manual matrix — Bug 1 (repeat each 10x, first-open and re-open):
  - [ ] Hamburger → Theme submenu — no visible flash/snap on first hover
  - [ ] Hamburger → Opacity submenu — same
  - [ ] Right-click pane header → Pane Color submenu — same
  - [ ] Right-click pane body → Replace With… submenu — same
  - [ ] Right-click terminal → Themes / Font Size / Zoom / Transparency submenus — same
  - [ ] Right-click sysinfo pane → Plot Type submenu — same
  - [ ] Right-click Armory account row → Bind to Agent submenu — same
- [ ] `task dev` manual matrix — Bug 2:
  - [ ] From each surface above, move the mouse diagonally from the parent item toward the submenu at a normal human speed — submenu stays open and reachable
  - [ ] Move the mouse away from both parent and submenu — submenu closes within ~300ms
  - [ ] Sweep the mouse quickly across sibling rows without pausing — no submenu flickers open spuriously (validates the open delay)
  - [ ] Park the mouse motionless near but outside the safe-triangle polygon — submenu eventually closes (validates the safety timeout, not just "open forever")
- [ ] `muxlog host '[menu-guard]'` — zero paintable-area violations across the matrix (regression check against `SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20.md`'s existing guard)
- [ ] Existing `menu-position.test.ts` still green
- [ ] New `submenu-hover.test.ts` covers the unit cases above

---

## 7. Risk register

| Risk | Mitigation |
|---|---|
| Safe-triangle polygon test computed in the wrong coordinate space through a portal | Compute in viewport (`position:fixed`) coordinates throughout, matching `computeMenuPosition`'s own output space — never rely on DOM-tree adjacency between trigger and portal'd submenu |
| Open delay (~90ms) makes menus feel less "snappy" than PR #792's zero-delay ideal | Delay is on visibility, not on `computeMenuPosition` — positioning can still start computing during the delay window so there's no added *positioning* latency once shown; 90ms is below common perceptible-lag thresholds (~100-150ms) |
| §1.1's secondary/tertiary hypotheses turn out not to be real contributors | Phase 4's instrumentation runs before committing to the Phase 3 rewrite narrative; the rewrite fixes the confirmed primary cause regardless, so no wasted work either way |
| Migrating `FlyoutMenu`'s close logic could regress sibling-menu switching (moving from one parent's submenu directly to a different parent's item) | Keep `visibleSubMenus`'s open-side ancestor logic; only close-timing routes through the new controller — a `mouseenter` on a different top-level item still closes the previous submenu immediately (no safe-triangle needed there — it's an explicit new selection, not an attempt to reach the current submenu) |

---

## 8. Open questions

1. **Q1** — Should the safety timeout (300ms) be tunable per-surface, or one global constant? Recommend one global constant for v1 — no evidence yet that any surface needs a different value.
2. **Q2** — `requireIntent`/velocity-based closing (§2.3 point 3) — worth adding in v1, or defer? Recommend defer; revisit if the safety-timeout-only approach proves to leave submenus open too long in practice.
3. **Q3** — Does `Implementation A`'s existing `autoUpdate`-driven re-positioning (for scroll/resize) need any interaction with the new hover controller, e.g. does a mid-hover reposition ever need to reset the safe-triangle polygon? Likely yes — the polygon should recompute against the submenu's *current* rect, not a stale one; `setSubmenuEl` should be callable repeatedly, not just once.

---

## 9. Cross-references

- `docs/specs/SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20.md` — the positioning-primitive framework this spec builds on top of, unchanged.
- `docs/specs/SPEC_UNIFIED_MENU_SYSTEM_2026_05_11.md` — the (stale, never-actioned) visual-chrome unification; orthogonal to this spec.
- `docs/specs/SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md` — most recent net-new submenu caller (Implementation B).
- `frontend/app/element/flyoutmenu.tsx` (Implementation A)
- `frontend/util/cef-api.ts:150-314` (Implementation B)
- `frontend/app/util/menu-position.ts` (shared positioning primitive, unchanged by this spec)
- `frontend/app/view/agent/components/UserMessageBlock.tsx:92-131` — existing hover-delay prior art in this codebase
- `frontend/app/element/tooltip.tsx` — existing hover-delay prior art in this codebase

---

*End of spec.*
