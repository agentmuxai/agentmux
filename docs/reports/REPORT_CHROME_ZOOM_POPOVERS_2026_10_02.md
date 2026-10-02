# Report: Popovers and Chrome Zoom — Why Some Follow It, and One Primitive for All of Them

**Status:** implemented in #4208 (§10). Zoom behaviour measured in headless Chrome 154, the same Chromium as AgentMux's CEF 154 (§4); not yet measured inside the app itself.
**Date:** 2026-10-02
**Verified against:** `da756879d` (main)

---

## 1. The complaint, and the short answer

Some status bar panels follow chrome zoom (the version panel does) and some do not. The status bar panels and the agent pane's model/effort panel should all follow it. Asked for a robust, clean architecture and DRY opportunities.

**Short answer.** Whether a panel follows chrome zoom is an accident of *where it is mounted*, not a decision anyone made per panel:

- The **version panel** (`InstancePanel`) is rendered *inside* `.status-bar`, which carries `zoom: var(--zoomfactor)`. It inherits the zoom, so it scales. But it is placed with real-pixel `bottom`/`right` values that the inherited zoom then scales again, so it drifts off its chip (§4).
- **Every other panel** is portaled to `<body>`, outside any zoomed ancestor, so it renders at 1x whatever the zoom.

Behind that sits a larger problem: nine surfaces each hand-roll the same floating-panel plumbing (portal, positioning, dismiss, pane overlay). The copies have drifted, and several real bugs come from that drift (§5). The fix for zoom and the DRY fix are the same change: **one anchored-popover primitive that owns the shell, the zoom scope, positioning, dismissal and the airspace cut** (§6).

## 2. Background: two zoom systems

- **Chrome zoom.** `--zoomfactor` on `:root`, set by `applyChromeZoomCSS()` (`frontend/app/store/zoom.ts`). Ctrl+wheel over the title bar, status bar or a pane header. Applied with CSS `zoom` on two in-flow containers: `.window-header` (`window-header.{win32,linux,darwin}.scss`) and `.status-bar` (`StatusBar.scss:25`).
- **Per-pane zoom.** `term:zoom` block meta. Ctrl+wheel over a pane. In the agent pane it is applied on `.agent-view-zoomed` (`agent-view.tsx:1265`), and `--agent-pane-zoom` is published on `.agent-view` (`agent-view.tsx:1231`).

CSS `zoom` is inherited down the DOM tree, not across a `<Portal>`. A portaled element only scales if something inside the portal applies `zoom`.

## 3. Inventory

| Surface | Mounted | Zoom today | Positioned by | Outside click | Esc | Airspace cut |
|---|---|---|---|---|---|---|
| Version panel `InstancePanel` | inside `.status-bar` | chrome, inherited, drifts | own `bottom`/`right` math (`InstancePanel.tsx:158`) | `StatusBar.tsx:53`, bubble, `querySelector(".instance-panel")` | yes | `usePaneOverlay` |
| Token usage `TokenBreakdownPopover` | Portal | none | `computeMenuPosition`, static rect | capture | yes | `usePaneOverlay` |
| CPU cores `CpuCoresPopover` | Portal | none | same, static rect | capture | yes | `usePaneOverlay` |
| Disk volumes `DiskVolumesPopover` | Portal | none | same, static rect | capture | yes | `usePaneOverlay` |
| Backend `BackendStatusPanel` | Portal | none | same, static rect | bubble | **no** | `usePaneOverlay` |
| Host `HostPopoverPanel` | Portal | none | same, static rect | bubble | **no** | `usePaneOverlay` |
| Status bar tips `TipBalloon` | Portal | none | same, live element | n/a (hover) | n/a | `usePaneOverlay` |
| Model/effort `AgentRuntimeDropup` | Portal | none (trigger follows pane zoom) | same, live element | bubble + focus-out | yes | `data-pane-overlay` **only** |
| Session stats `AgentSessionStats` | Portal | none (trigger follows pane zoom) | same, live element | bubble + focus-out | yes | `data-pane-overlay` **only** |
| Title bar `MoreDropdown`, `PinnedWidgetFlyout`, `FlyoutMenu` | Portal | none (trigger follows chrome zoom) | same, live element | in `action-widgets.tsx` | partly | `usePaneOverlay` |

"Static rect" means the trigger's `DOMRect` is captured once at click time and handed down as `anchorRect`. "Live element" means the trigger element itself is the floating-ui reference.

### 3.1 Why the status bar popovers lost their zoom

They used to declare `zoom: var(--zoomfactor)` themselves. PR #2736 removed it (`docs/specs/SPEC_STATUS_BAR_POPOVER_DOUBLE_ZOOM_OFFSET_2026_08_22.md`), because a self-zoomed popover placed in real pixels was scaled twice and landed far from its anchor (case B in §4). That spec named the remaining work: keep the *content* scaling with a "non-position-affecting alternative". It was never done, so the popovers have sat at 1x since 2026-08-22. The comments "Deliberately NOT `zoom: var(--zoomfactor)`" mark the spots (`_token-usage.scss:62`, `_cpu-cores-popover.scss:41`, `_disk-volumes-popover.scss:38`, `StatusBarTip.scss:16`, `StatusBar.scss:250`).

## 4. Measurements: which zoom pattern actually works

I built a minimal page: a zoomed bar at the bottom with a chip in it, positioned with the app's own `@floating-ui/dom` 1.8.0 and the same middleware stack as `computeMenuPosition` (`offset(4)`, `flip`, `shift`, `size`, `strategy: "fixed"`, `placement: "top-end"`). I ran it in headless Chrome 154.0.8037.93 at three zoom levels. The panel is 200×100 CSS px, so it *should* render at 200z × 100z, flush with the chip's right edge and 4px above it.

| Pattern | z | Right edge error | Bottom edge error | Rendered size |
|---|---|---|---|---|
| **A** in-flow under the zoomed bar, real-px `bottom`/`right` (today's `InstancePanel`) | 0.65 | +1.4 | **+4.3** (overlaps the chip) | 130×65 ✓ |
| | 1.5 | −4.5 | **−14.2** (gap) | 300×150 ✓ |
| **B** portaled, `zoom` on the popover itself (before #2736) | 0.65 | −413 | −241 | 130×65 |
| | 1.5 | +588 | +337 | 300×150 |
| **C** as B, with `left`/`top` divided by z | 0.65 | −70 | −35 | 130×65 |
| | 1.5 | +100 | +51 | 300×150 |
| **D** portaled, no zoom (today's other popovers) | 0.65 | 0 | 0 | 200×100 ✗ |
| | 1.5 | 0 | 0 | 200×100 ✗ |
| **E** portaled **shell (no zoom) + zoomed inner box** | 0.65 | 0 | 0 | 130×65 ✓ |
| | 1.5 | 0 | 0 | 300×150 ✓ |
| **F** portaled into a full-window zoomed layer, `left`/`top` divided by z | 0.65 | −70 | −35 | 130×65 |
| | 1.5 | +100 | +51 | 300×150 |

All patterns are exact at z = 1.

What this shows:

- **E is the only pattern that both scales and stays on its anchor.** The shell has no zoom of its own and shrink-wraps the zoomed inner box, so floating-ui measures the true on-screen size and writes real-pixel `left`/`top`.
- **C and F fail for a reason #2736 did not cover.** Dividing the position by z is not enough: floating-ui reads the floating element's size in CSS px (`offsetWidth` = 200), while it renders at 200z. Every edge-aligned placement (`top-end`, `top-start`) is then off by the size difference. So "compensate inside `computeMenuPosition`" is ruled out, not just inelegant.
- **A, the version panel, scales but drifts.** The error is the anchor's distance from the window edge times (z − 1): small near the corner, but 14px at 150% and growing to the 200% maximum. The #2736 spec marked `InstancePanel` "not affected" because its SCSS has no `zoom:`, but the zoom is inherited from `.status-bar`.

The harness (`lab.html`) is outside the repo, in my workspace scratch folder. It can be added to the repo as a regression check if wanted.

## 5. What the duplication has already cost

The same floating-panel lifecycle has been copied into each surface. These are the differences that are bugs, not choices:

1. **Backend and Host popovers ignore Esc.** Every other status bar popover closes on Esc (`TokenUsageIndicator.tsx:65`, `SystemStats.tsx:104`, `StatusBar.tsx:48`). `BackendStatus.tsx:373` and `HostPopover.tsx:579` only register outside-click.
2. **The latent `autoUpdate` leak `StatusBarTip` already fixed is still in five copies.** `TipBalloon` cancels its pending `requestAnimationFrame` on cleanup, after a leak was seen live on 2026-09-22 (4,718 throws in an hour, `StatusBarTip.tsx:60-72`). `TokenBreakdownPopover`, `CpuCoresPopover`, `DiskVolumesPopover`, `BackendStatusPanel` and `HostPopoverPanel` have the identical unfixed `registerFloating`. A popover dismissed in its first frame (for example a fast double-click on the trigger) starts an `autoUpdate` that nothing stops.
3. **Model/effort and session stats panels have no airspace cut on macOS/Linux.** They carry only `data-pane-overlay` (`AgentRuntimeDropup.tsx:451`). That attribute is auto-discovered on Windows only (`pane-overlay.ts:281-283`), so over a browser pane on macOS/Linux these panels paint behind the native view. Every status bar popover calls `usePaneOverlay` and is fine.
4. **Status bar popovers anchor to a stale rect.** They capture `getBoundingClientRect()` at click time. If chrome zoom or the window size changes while one is open, the chip moves but the popover stays where the chip used to be. The agent pane panels anchor to the live element and do not have this problem.
5. **Outside-click is inconsistent.** Some use capture phase (token, CPU, disk), others bubble (backend, host, version, agent panels). The version panel finds itself with `document.querySelector(".instance-panel")` instead of a ref.
6. **Ctrl+wheel over an open popover does nothing.** `isOverChrome()` (`app.tsx:209`) checks `.window-header`, `.status-bar` and pane headers. A portaled popover is none of those, so the gesture is swallowed (`preventDefault`) and nothing zooms.
7. **The visual frame is defined five times, two ways.** Token, CPU, disk, `.status-bar-popover` and the tip each restate the same `background: var(--modal-bg-color)`, `1px` border, `radius 0` and shadow. `InstancePanel` uses `--main-bg-color`, a 6px radius, a heavier shadow and a different z-index (`--z-popover` vs `--zindex-modal-wrapper`). The menu panels use a third frame (`menu-frame.scss`).

Line count of the copied plumbing: about 40–60 lines per surface for positioning and lifecycle (`registerFloating`, `floatingStyle`, `autoUpdate` cleanup), plus about 20–30 per trigger for anchor capture, outside click and Esc. Across the nine in-scope surfaces that is roughly 500–700 lines that become one module.

## 6. Recommendation: one `AnchoredPopover` primitive

A single Solid component (for example `frontend/app/element/anchored-popover.tsx`) that every in-scope panel renders through. It owns the six concerns that are copied today; the panel keeps only its own content.

```tsx
<AnchoredPopover
    open={open()}
    onClose={() => setOpen(false)}
    anchor={triggerEl}             // a live element, never a captured rect (fixes §5.4)
    placement="top-end"
    zoom="chrome"                  // "chrome" | "pane" | "none"
    class="token-usage-breakdown"  // styles the zoomed inner box
    role="dialog" aria-label="Token usage breakdown"
    dismiss={{ outside: true, escape: true, focusOut: false }}
>
    …panel content…
</AnchoredPopover>
```

What it renders and does:

1. **Portal → shell → body.** The shell (`.anchored-popover-shell`) is `position: fixed`, has no zoom and no size of its own, and takes `left`/`top` from `computeMenuPosition`. The body (`.anchored-popover-body`) carries the panel's class, the frame, all sizes, and `zoom` set from the `zoom` prop: `var(--zoomfactor, 1)` for chrome, `var(--agent-pane-zoom, 1)` for pane (the property is inherited only inside the pane, so the primitive needs to read it from the trigger), and none for `"none"`. This is pattern E: the only one measured correct.
2. **Positioning lifecycle in one place.** `autoUpdate(anchor, shell, …)` with the pending-frame cancellation `TipBalloon` already has (fixes §5.2). The shell's ResizeObserver fires when zoom changes the body's size, so an open popover re-places itself.
3. **Dismiss in one place.** Outside-click (one phase for everyone, checking the trigger and shell by ref), Esc, and optional focus-out (the agent panels' current behaviour). This fixes §5.1 and §5.5.
4. **Airspace cut always.** `usePaneOverlay` on the shell plus `data-pane-overlay`, so it works on every platform (fixes §5.3).
5. **Ctrl+wheel.** The shell carries `data-zoom-scope="chrome"` (or `"pane"`), and `isOverChrome()` gains one selector, so Ctrl+wheel over an open status bar popover zooms the chrome with the bar (fixes §5.6).
6. **One frame.** A `popover-frame` mixin next to `menu-frame` for the status bar style (fixes §5.7). Whether the version panel keeps its own rounder look, or the menu and popover frames merge, is a design call (§8).

Sizes move from inline `width: 320px` on the root to the body, so they scale. Nothing else in the panels' own content changes, and none of their content SCSS needs `calc(… * var(--zoomfactor))`.

**`.menu`-styled panels need one adjustment.** `.menu` sets `position: absolute` (`flyoutmenu.scss:9`). On the body inside a shell, that takes the body out of flow and collapses the shell to 0×0. For the model/effort and session stats panels, the body should get `position: static` or use the `menu-frame` mixin directly instead of the `.menu` class.

**Why not the alternatives:** re-adding self-zoom is the #2736 bug (B). Compensating in `computeMenuPosition` is ruled out by measurement (C). A zoomed layer root fails the same way (F). Per-property `calc()` scaling is what `docs/specs/zoom-architecture.md` §3 rejected as tedious and error-prone, and it would spread across every panel's content SCSS.

## 7. Migration plan

Each phase is one PR. Each phase is checked live over CDP at `--zoomfactor` 0.65, 1.0 and 1.5: the popover's rect against its anchor, its size against 1x times z, and re-placement on a zoom change while it is open.

1. **Primitive + token popover.** Add `AnchoredPopover`, the `popover-frame` mixin and the `isOverChrome` selector. Move `TokenUsageIndicator`/`TokenBreakdownPopover` onto it. Smallest real consumer, and it already has tests.
2. **The rest of the status bar.** CPU, disk, backend, host, the tip balloon (hover-driven, `dismiss` off), and the version panel. The version panel moves from in-flow to the portal, which fixes its drift and lets `StatusBar.tsx` drop its `querySelector` dismiss.
3. **Agent pane panels.** `AgentRuntimeDropup` and `AgentSessionStats`, with `zoom="chrome"` and `dismiss.focusOut`. Gains the macOS/Linux airspace cut.
4. **Optional: title bar menus.** `MoreDropdown`, `PinnedWidgetFlyout`, `FlyoutMenu`. They share `.menu` with every context menu in the app, so this is the widest change. Leave it until 1–3 have baked.

**Tests.** jsdom has no layout for `zoom`, so unit tests cannot check scaling. They can check what the primitive owns: Esc and outside-click close it, clicking the trigger does not, a close inside the mounting frame starts no `autoUpdate` (the §5.2 regression), `usePaneOverlay` registers, and the body gets the right `zoom` value per scope. The existing `TokenBreakdownPopover`, `HostPopover`, `StatusBarTip`, `AgentRuntimeDropup` and `AgentSessionStats` tests should keep passing as the migration proceeds. The CDP check above, or the §4 harness checked into the repo, covers the visual part.

## 8. Decisions for you

1. **Agent panels: chrome zoom or pane zoom?** You asked for chrome zoom. Their triggers sit in the pane and follow pane zoom, so at, say, chrome 100% and pane 140%, the panel would come out smaller than its button. The primitive supports both with the same one-word prop, so this is a choice, not a cost. My recommendation: chrome zoom, as you asked, since the panel then matches the status bar and title bar menus and not whatever zoom the pane happens to be at.
2. **Title bar menus (phase 4): in or out?** They have the same defect, but changing `.menu` touches every context menu.
3. **One popover look or two?** The version panel is styled differently (rounded, darker shadow) from the other status bar popovers. Unifying it is cheap once the frame is a mixin, but it is a visible change.

## 9. What I did not check

- The measurements in §4 come from a minimal page in Chrome 154, not from the app. The app's own popovers have not been measured at a zoom other than 1.0.
- Defects §5.1–§5.6 are read from the code. §5.2 (the leak) and §5.3 (no airspace cut on macOS/Linux) have not been reproduced.
- Modals, toasts, context menus and other portaled surfaces outside the status bar, the agent pane composer strip and the title bar were not inventoried.

## 10. What shipped (feat/chrome-zoom-popovers)

`AnchoredPopover` (`frontend/app/element/anchored-popover.tsx`) as recommended in §6, with phases 1–3 of §7 in one PR:

- **On it:** token usage, CPU cores, disk volumes, backend, host, the status bar tips, the version panel, and the agent pane's model/effort and session stats panels.
- **Decisions taken (§8):** the agent panels follow chrome zoom; the title bar menus stay out (phase 4); the version panel keeps its own look, only its stacking moved to the shell.
- **The status bar popovers anchor to their trigger element**, not a rect captured on click (§5.4).
- **One dismiss rule** for the status bar popovers and the version panel: capture-phase mousedown outside the panel and its anchor, and Esc (§5.1, §5.5). Esc in a field inside the panel stays with the field, so the version panel's rename keeps working. The two agent panels keep their own keyboard and focus handling.
- **`popover-frame` mixin** (`frontend/app/element/popover-frame.scss`) for the shared status bar frame (§5.7).
- **Ctrl+wheel over an open popover zooms the chrome** (`data-zoom-scope="chrome"` in `isOverChrome`, §5.6).

Two more defects found while migrating, both gone with the copies:

8. `BackendStatus`'s outside-click effect returned its cleanup (`return () => removeEventListener(…)`). Solid's `createEffect` does not run a returned function as cleanup, so every open left one more `mousedown` listener on `document`.
9. `AgentRuntimeDropup` tied its `autoUpdate` to the trigger's lifetime, not the panel's, so after closing it kept repositioning a detached element until the pane closed. `AgentSessionStats` had fixed the same thing (Codex P2 on #3435).
