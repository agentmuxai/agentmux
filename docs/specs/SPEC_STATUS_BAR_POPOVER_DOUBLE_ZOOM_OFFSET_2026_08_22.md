# SPEC — Status bar popovers render offset (and undersized) under Chrome zoom

**Date:** 2026-08-22 (fixed 2026-08-22, PR #2736)
**Type:** Bug diagnosis + fix (implemented, verified live via CDP)
**Status:** Resolved
**Scope (confirmed affected):** `frontend/app/statusbar/TokenBreakdownPopover.tsx`
(+ `_token-usage.scss`), `frontend/app/statusbar/CpuCoresPopover.tsx`
(+ `_cpu-cores-popover.scss`), `frontend/app/statusbar/DiskVolumesPopover.tsx`
(+ `_disk-volumes-popover.scss`), `frontend/app/statusbar/StatusBarTip.tsx`
(+ `StatusBarTip.scss`), and the shared `.status-bar-popover` class
(`StatusBar.scss:244` — confirmed also covers `HostPopover.tsx`, which
renders with `class="status-bar-popover host-popover"`, i.e. via the same
shared, now-fixed class, not a separate one). **Not affected:**
`InstancePanel.tsx` — despite living in the same `frontend/app/statusbar/`
directory and looking superficially similar, it positions itself via its
own `bottom`/`right` fixed-positioning logic (`InstancePanel.tsx:411-421`),
NOT `computeMenuPosition`, and `_instance-panel.scss` has no `zoom:`
declaration at all — an earlier draft of this spec incorrectly listed it as
a likely-affected consumer without individually verifying it; corrected
per codex's review of PR #2736. Also not affected: `MoreDropdown`,
`PinnedWidgetFlyout`, `AgentRuntimeDropup`, the generic `flyoutmenu.tsx` —
none of these self-apply `zoom:` in their own SCSS (verified by direct grep),
so they don't hit this bug despite using the same `computeMenuPosition`
positioning utility.

## Reproduction

Observed live via `mcp__agentmux__UIQuery`/`UIClick` against a real running
instance, at Chrome zoom `--zoomfactor: 0.65` (reachable via ordinary
`Ctrl+scroll-wheel`-out on the status bar/title bar — `WHEEL_STEP = 0.05`,
7 steps down from the default `1.0`; see `zoom.win32.ts:22-23`):

1. Click the status bar's Token Usage indicator (`.token-usage-indicator`).
2. `TokenBreakdownPopover` opens. Its inline style (written by
   `computeMenuPosition`, see below): `width: 320px; position: fixed; left: 225px; top: 866px;`
3. Its ACTUAL rendered rect (`getBoundingClientRect()`, as read by `UIQuery`):
   `x: 146.25, y: 562.890625, width: 208.0, height: 124.578125`.
4. Every dimension is scaled down by **exactly 0.65×** from what the popover's
   own CSS declares: `208/320 = 0.65`, `146.25/225 = 0.65`, `562.89/866 ≈ 0.6499`.

The popover renders in the middle of the pane instead of anchored near the
status bar (bottom of the window) — visually appearing both mis-positioned
*and* shrunk, because both position and size are subject to the same
compounding scale.

For comparison, the SAME window's `.agent-view` (the per-agent-pane container)
reports its own, unrelated zoom: `zoom: 0.75; --agent-pane-zoom: 0.75;` — a
completely different value, confirming per-pane zoom is not the cause; this
is purely a Chrome-zoom (`--zoomfactor`) bug.

## Root cause

This is a genuine **double application of the same zoom factor** to the same
value — not a wrong-coordinate-space capture, and not ambiguous:

1. **`TokenUsageIndicator.tsx`** captures `anchorRect` via
   `indicatorRef.getBoundingClientRect()` at click time. Because the button
   lives under `.status-bar`, which itself has `zoom: var(--zoomfactor)`
   applied (`StatusBar.scss:25` — a *deliberate* decision, see below), this
   rect is already **real, on-screen, post-zoom pixel coordinates**. This
   part is correct and not the bug.
2. **`computeMenuPosition`** (`frontend/app/util/menu-position.ts:228-296`)
   takes that already-real anchor rect, runs floating-ui's
   `computePosition(..., { strategy: "fixed", middleware: [offset, flip, shift, size] })`,
   and writes back `left`/`top` as plain `px` values. It never reads `zoom`,
   `--zoomfactor`, `--agent-pane-zoom`, or `devicePixelRatio` anywhere — by
   design, it's a flat, 1:1-coordinate-space utility, and that's the correct
   contract *as long as the element it's positioning has no zoom of its own*.
3. **`.token-usage-breakdown` (`_token-usage.scss:62`) — and identically
   `.agent-cpu-cores-popover`, `.agent-disk-volumes-popover`,
   `.status-bar-tip`, `.status-bar-popover` — each ALSO declare
   `zoom: var(--zoomfactor)` directly on themselves.** Chromium's `zoom`
   property rescales an element's own used values for a `position: fixed`
   box's `left`/`top`/`width` by that factor at paint time. So the already-
   real-pixel coordinates from step 2 get scaled down a **second** time by
   the exact same factor that was already baked into the anchor rect in
   step 1.

This is why the math is exact: `225 × 0.65 = 146.25`, `320 × 0.65 = 208`,
`866 × 0.65 = 562.9`. It is not a rounding coincidence or a compounding of
two *different* zoom sources — it's the literal same `--zoomfactor` value
applied twice to the same coordinate.

### Why the self-`zoom:` rule exists at all (not a random mistake)

`docs/specs/zoom-architecture.md` (§3, "Option A") is the origin: it
proposed and this codebase adopted `zoom: var(--zoomfactor)` as the
architecture for scaling **in-flow chrome containers** — `.window-header`,
`.status-bar`, and (per that doc's own "what responds" table, written before
Option A shipped) `.status-bar-popover`'s font-size. That's the right call
for an element that **stays in its normal DOM position** and needs its own
fonts/icons/padding/children to scale together — "zero changes needed in
child SCSS files… new UI elements just work" (zoom-architecture.md:75-76).

The gap: that document only ever discusses **in-flow containers**. It never
anticipated a **Portal'd, `position: fixed`, floating popover positioned via
absolute-pixel math derived from an anchor that is ITSELF inside a zoomed
container** — which is exactly what most status-bar popovers are (per
`docs/specs/SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17.md`'s own
confirmed-consumer list for the AIRSPACE-CLIP mechanism specifically:
`TokenBreakdownPopover`, `CpuCoresPopover`, `DiskVolumesPopover`,
`InstancePanel`, all sharing `usePaneOverlay`. That list is about airspace
clipping, not `computeMenuPosition` — `InstancePanel` is on it but, per the
correction above, positions itself via its own `bottom`/`right` math, not
`computeMenuPosition`, and was never actually affected by *this* bug).
Someone (reasonably, by analogy with `.window-header`/`.status-bar`)
applied the same "just add `zoom: var(--zoomfactor)`" pattern to the
Portal'd, `computeMenuPosition`-driven popovers specifically, not realizing
their *position* was already computed in real, post-zoom pixels via their
anchor — unlike `.window-header`, which has no anchor-derived position at
all, just normal flow layout.

### `usePaneOverlay` — confirmed NOT involved

`usePaneOverlay` (`frontend/app/platform/pane-overlay.ts:265-316`, the
"airspace cut" mechanism) only measures the popover's already-rendered
`getBoundingClientRect()` and forwards it to the Rust host so native browser-
pane HWNDs can be clipped around it (`SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17.md`).
It has no effect on the popover's own position or scale — confirmed by
reading its implementation in full. Likewise, the Portal itself (Solid's
`<Portal>`, `TokenUsageIndicator.tsx:95`) genuinely re-parents the popover's
DOM node to `document.body` — but `--zoomfactor` is a root-level CSS custom
property (`:root`, `tailwindsetup.css:74`), so it inherits into the Portal'd
node regardless of where in the DOM tree that node physically lives. Moving
the element via Portal does not escape the variable; only removing the
element's own `zoom:` declaration (or compensating for it) would.

## Blast radius

Confirmed via direct grep for `zoom:\s*var\(--zoomfactor\)` across
`frontend/`:

| File | Selector | Self-applies zoom? |
|---|---|---|
| `_token-usage.scss:62` | `.token-usage-breakdown` | Yes — **repro'd live above; fixed in PR #2736** |
| `_cpu-cores-popover.scss:41` | `.cpu-cores-popover` | Yes — same bug; **fixed in PR #2736** |
| `_disk-volumes-popover.scss:38` | `.disk-volumes-popover` | Yes — same bug; **fixed in PR #2736** |
| `StatusBarTip.scss:16` | `.status-bar-tip-balloon` | Yes — same bug (confirmed positioned via `computeMenuPosition`, per its own doc comment); **fixed in PR #2736** |
| `StatusBar.scss:244` | `.status-bar-popover` (shared class) | Yes — same bug for every popover composing it, **confirmed** for `HostPopover` (renders `class="status-bar-popover host-popover"`); **fixed in PR #2736** |
| `_instance-panel.scss` | `.instance-panel` | **No `zoom:` declaration at all** — positions via its own `bottom`/`right` math (`InstancePanel.tsx:411-421`), not `computeMenuPosition`. Not affected; not touched by the fix. |
| `StatusBar.scss:25` | `.status-bar` (the status bar itself, in-flow) | Yes — **correct**, this is the container the architecture doc intended; unchanged |
| `window-header.{win32,linux,darwin}.scss` | `.window-header` (in-flow) | Yes — **correct**, same reasoning; unchanged |

Not affected (verified no self-`zoom:` in their own SCSS):
`more-dropdown.tsx`, `pinned-widget-flyout.tsx`, `AgentRuntimeDropup.tsx`,
`flyoutmenu.tsx` — all also use `computeMenuPosition`, but their floating
elements don't self-zoom, so they position correctly regardless of Chrome
zoom.

## Fix (implemented, PR #2736)

Two viable approaches were considered, not mutually exclusive — **option 1
was chosen and shipped**:

1. **Strip `zoom:` from each popover's own root rule.** The anchor rect
   `computeMenuPosition` starts from is already real/post-zoom (since the
   anchor button lives under a zoomed `.status-bar`), so the popover itself
   needs no additional scaling of its *position* — only its own font-size/
   padding/icon sizing would need to keep scaling with `--zoomfactor` to
   stay visually consistent with the status bar that triggered it, which
   would need to move to individual `calc(Npx * var(--zoomfactor))`
   declarations (the pre-Option-A approach `zoom-architecture.md` explicitly
   moved away from for exactly the "tedious, easy to miss a hardcoded value"
   reason documented there) or a scoped, non-position-affecting alternative.
2. **Have `computeMenuPosition` (or its caller) divide the target
   `left`/`top` by the anchor's ambient zoom before writing them**, so the
   popover's own subsequent `zoom:` self-application lands back on the
   originally-intended real-pixel position. This keeps the "self-zoom scales
   everything uniformly" convenience for font/icon/padding sizing but
   requires every `computeMenuPosition` caller whose target self-zooms to
   pass that factor in — a new, easy-to-forget contract for any *future*
   zoom-scoped popover, unlike option 1's "just don't self-zoom a
   Portal'd/positioned element" rule of thumb.

Applied uniformly to all five affected selectors in one pass. No automated
regression test was added (CSS `zoom` has no layout effect in jsdom, so
nothing in that environment could distinguish the buggy state from the
fixed one) — instead, an explanatory comment was added at each fix site,
and the fix was verified live via CDP against a running `task dev`
instance: forced `--zoomfactor: 0.65`, confirmed `TokenBreakdownPopover`'s
computed `zoom` is now `1` and its rendered rect exactly matches its
declared inline `left`/`top`/`width` (previously diverged by exactly
0.65× on every dimension, matching the reproduction at the top of this
document).
