# SPEC — Chrome-style window snap: drag-to-top maximize, border-drag vertical snap

**Date:** 2026-09-04
**Status:** implemented 2026-09-05 — both features built; pending live
verification of the manual test plan in §6 (native Win32 message-loop code,
not reachable by any test harness). See §7 for what shipped.
**Repo:** agentmuxai/agentmux
**Trigger:** Operator request — dragging the AgentMux window to the top of the
screen should offer to maximize it, and dragging a window *border* to the
top/bottom of the screen should vertically snap that edge to the screen edge
(keep width, extend/shrink height) — "the same behavior as the Chrome
browser." Chrome is the right reference point specifically because Chrome,
like AgentMux, draws its own custom title bar on Windows instead of using
the native one, so it can't just inherit this for free from `DefWindowProc`
either — it re-implements the behavior itself.

---

## 0. Why this doesn't already work

**Amended 2026-09-05 — operator live-tested §0.2's "likely already works"
claim on a real build. It does not.** Border-drag vertical snap is
confirmed broken, not just unverified. This changes the analysis below:
the two symptoms are more likely **one root cause** wearing two faces —
see §0.3 — not two unrelated bugs as the original pass assumed.

### 0.1 Drag-to-top → maximize: genuinely missing, and the gap is on record

AgentMux's title-bar drag is **not** a native OS window move. `agentmux-cef/src/ui_tasks/drag.rs:314-323` documents why: the standard trick
(`WM_NCLBUTTONDOWN(HTCAPTION)` → `DefWindowProc` starts the OS's own modal
move loop, which is what makes Aero Snap fire automatically) does not work
for a CEF/Chromium window — Chromium's `HWNDMessageHandler` swallows the
non-client message before `DefWindowProc` ever gets to start that loop
(instrumented proof in the comment: the `SendMessageW` call returned in
0.4ms, i.e. it never blocked on a real move loop). So `Win32BeginMoveTask`
(`drag.rs:325-568`) runs a **fully manual** move loop instead: `SetCapture`
+ a `GetMessageW` loop + `SetWindowPos` per `WM_MOUSEMOVE`. This is
deliberate, documented, working design (`docs/specs/SPEC_WINDOW_DRAG_MANUAL_MOVE_LOOP_2026_05_29.md`,
which itself supersedes an earlier native-loop attempt that didn't work,
`SPEC_WINDOW_DRAG_NATIVE_MOVE_LOOP_2026_05_29.md`) — but because it's
manual, none of Windows' own Snap Assist/edge-detection logic ever runs
during it. That spec says so explicitly:

> Non-goals (v1): Aero Snap/snap-assist... **Lost** (v1) — can add manual
> edge-snap later.

This is that "later." The fix has to be a **manual re-implementation** of
the top-edge-detection-and-maximize gesture, built into the same loop that
replaced the native one — there's no way to "turn Aero Snap back on" short
of solving the underlying CEF non-client-message problem, which the prior
spec already tried and gave up on for good, load-bearing reasons.

### 0.2 Border-drag vertical snap: CONFIRMED broken (2026-09-05 live test)

Window **resizing** itself is real, unmodified native OS handling — this
part of the original analysis holds. Top-level and floater windows are
created with `WS_THICKFRAME` (e.g. `agentmux-cef/src/commands/floating_pane.rs:809`),
and `WM_NCHITTEST` resolves to `HTTOP`/`HTBOTTOM`/`HTLEFT`/`HTRIGHT`/etc.
normally (`floating_pane.rs:603-649` for the borderless floater case),
letting Windows run its **own** resize loop. So the *mechanism* that would
need to carry Snap Assist is genuinely native — and yet the operator's live
test shows the snap doesn't happen. The likely explanation is §0.3, not a
simple style-flag miss.

### 0.3 Likely shared root cause: the main window is a CEF Views *frameless*
window, and Chromium's frameless-window support doesn't carry Snap
eligibility

`agentmux-cef/src/app/mod.rs:315-317` — `AgentMuxWindowDelegate::is_frameless`
returns `self.frameless as i32`, and for the main window this is true (it's
what makes the custom title bar possible at all instead of a native one).
`can_resize`/`can_maximize` (`app/mod.rs:319-325`) both report `1` at the
CEF Views level — so CEF *itself* isn't refusing resize/maximize — but
`is_frameless` hands control of the actual Win32 non-client-area behavior
to Chromium's internal `views::HWNDMessageHandler`, which we do not control
or have source for in this repo. Windows' Snap Assist (both the
move-to-top-maximize gesture in §0.1 and the border-drag vertical-snap
gesture here) has historically been unreliable-to-absent for Chromium/
Electron-style frameless windows on Windows precisely because the OS's
snap eligibility checks key off non-client-area signals (`WS_CAPTION`
presence, standard `WM_NCCALCSIZE` non-client sizing, etc.) that a fully
frameless window may not present in the shape Windows expects — this is a
known category of issue for this class of app, not unique speculation about
AgentMux specifically, but **not independently confirmed against
Chromium's actual internal implementation** (not in this repo, can't be
grepped) — treat as the leading hypothesis, not a certainty.

**Practical consequence: don't chase a style-flag tweak on the theory that
one bit is wrong.** Given `is_frameless` hands non-client behavior to code
this repo doesn't own, the more tractable fix for BOTH features is the same
shape §0.1 already committed to: **stop relying on Windows to do this and
implement the gesture manually**, at a point AgentMux already subclasses
the main window's WndProc. That infrastructure already exists and is
already proven safe on this exact window — see §3.1.

## 1. What already exists (do not rebuild)

| Piece | Where | Reusable for this feature? |
|---|---|---|
| Manual move loop | `Win32BeginMoveTask` (`ui_tasks/drag.rs:325-568`) | **Yes — this is where the new detection logic goes.** Already tracks cursor position every `WM_MOUSEMOVE` in physical screen px (`GetCursorPos`, `cur`), already has a `cancelled` flag (Esc), already has an established "emit a hover-ish event mid-drag" precedent (see next row). |
| Mid-drag hover event pattern | `update_floating_redock_hover`/`clear_floating_redock_hover`, called from inside the same `WM_MOUSEMOVE` arm at a 50ms cadence (`drag.rs:461-481`), consumed by `frontend/app/workspace/floating-pane-workspace.tsx` (redock ghost/highlight state machine, ~line 130-670) | **Model, don't reuse directly.** That mechanism is floater-redock-specific (different renderer component, different semantics). The new snap-preview affordance should follow the identical *shape* — host emits a hover-state event mid-drag, a dedicated frontend consumer renders/clears an overlay — as a sibling mechanism, not a branch inside the floater code. |
| Drag-end/cancel events | `window_drag_ended` (`drag.rs:551-560`, carries `label`, `moved`, physical `cursor_x`/`cursor_y`) and `window_drag_cancelled` (`drag.rs:515-521`) | Reuse the same emission point (drag.rs's loop exit / Esc arm) to also clear any snap-preview state — don't invent a third exit-signaling mechanism. |
| Monitor work-area lookup | `agentmux-cef/src/app/monitor.rs::get_monitor_work_area(px, py) -> Option<(x, y, w, h)>` (`MonitorFromPoint` + `GetMonitorInfoW`) | **Reuse the underlying Win32 calls, not the function as-is.** This helper converts physical → DIP pixels (`monitor.rs:42-50`, `scale = dpi_x / 96.0`) for CEF's `Window::set_bounds`, which expects DIP. The drag loop works entirely in **physical** px (its own comment: "no devicePixelRatio math"). Mixing the two here would reproduce exactly the physical/DIP unit-confusion bug class this codebase has already been bitten by elsewhere — call `MonitorFromPoint`/`GetMonitorInfoW` directly inside `drag.rs` and compare against `rcWork.top`/`rcWork.bottom` in physical px, don't route through `get_monitor_work_area`. |
| Maximize toggle | `agentmux-cef/src/commands/window/chrome.rs::maximize_window(state, args)` (`GetWindowPlacement`/`ShowWindow(SW_MAXIMIZE\|SW_RESTORE)`) | The *logic* (lines 48-61) is what's needed, but it's shaped as an IPC handler taking `{label}` args and re-resolving the HWND from a label. Inside `Win32BeginMoveTask` we already hold the raw `HWND` (`self.hwnd`) on the UI thread — extract the placement-toggle body into a small `maximize_hwnd(hwnd: HWND)` helper both call sites share, rather than round-tripping through label resolution + JSON args from inside the drag loop. |
| Frontend title-bar drag region | `frontend/app/window/window-header.tsx`, `useWindowDrag.win32.ts` (arms on mousedown, fires `start_window_drag` IPC past a 4px threshold) | Unchanged — the new behavior is entirely host-side once the drag has started. |
| Native resize `WM_SIZING` subclass | `agentmux-cef/src/client/wndproc.rs::window_edge_resize_wndproc` (already installed on the main window via `SetWindowLongPtrW`/`GWLP_WNDPROC`) | **Yes — this is where Feature B's fix goes (§3).** Already intercepts every `WM_SIZING` tick and reads the dragged edge; currently pure-observer (forwards to the renderer, never modifies the message). Extending it to also clamp the proposed `RECT` is the whole fix — no new subclass needed. |

## 2. Design — Feature A: drag-to-top → maximize

### 2.1 Detection

Inside `Win32BeginMoveTask`'s `WM_MOUSEMOVE` arm (`drag.rs:444-483`), after
computing `cur` (current cursor position, physical px):

1. Resolve the work area of the monitor under `cur` via `MonitorFromPoint`/
   `GetMonitorInfoW` directly (§1 — not `get_monitor_work_area`, unit
   mismatch).
2. If `cur.y <= rcWork.top + SNAP_ZONE_PX` (a small threshold — Windows'
   own Snap Assist uses a few px; start with something like 2-5px past the
   work-area top, tune from feel, not a hard requirement here), the drag is
   "in the top snap zone."
3. Track this as a new `in_snap_zone: bool` local in the loop (parallel to
   `cancelled`), transitioning it only on actual state change (entering/
   leaving the zone) — don't re-emit every single mousemove tick.

### 2.2 Visual affordance — "provide the maximize option"

The trigger's phrasing ("provide the maximize option," not "just maximize
immediately") matches Chrome/Windows' own UX: a preview appears while
still dragging, and the user commits by releasing inside the zone (or
backs out by dragging away). On the `in_snap_zone` transition:

- **Entering the zone**: emit a new host→renderer event (same shape as
  `update_floating_redock_hover`, different name/payload — e.g.
  `window_snap_hover` carrying the target rect: the full work area, since
  that's what maximizing would produce) so a new, dedicated frontend
  listener can render a translucent preview outline. This should live
  alongside `window-header.tsx`/the main window's own chrome, not inside
  `floating-pane-workspace.tsx` (that component is floater-specific and
  the main window doesn't mount it).
- **Leaving the zone** (cursor moves back down past the threshold) or
  **drag ends without committing**: emit a corresponding clear event —
  reuse the existing `window_drag_ended`/`window_drag_cancelled` emission
  points (drag.rs:515-521, 551-560) to also signal "and clear any snap
  preview," rather than adding a fourth place that needs to remember to
  clean this up.

Exact overlay rendering (a layered Win32 window? a frontend-drawn
absolutely-positioned div matching Chrome's own approach on custom-chrome
Windows builds?) is an implementation-time decision, not fixed here — the
constraint that matters is the event-driven handshake described above, so
the overlay's lifecycle can't outlive the drag or desync from it the way
the floater redock ghost's own bug history (see the extensive
Windows/non-Windows race commentary already in `floating-pane-workspace.tsx`)
shows is easy to get wrong.

### 2.3 Commit — maximize on release inside the zone

In the `WM_LBUTTONUP` arm (`drag.rs:484-490`) and the synthesized-up path
in the button-already-released check (`drag.rs:411-429`): if `in_snap_zone`
is true and the drag was not cancelled, call `maximize_hwnd(h)` (§1's
extracted helper) **instead of** leaving the window at the dropped
position. The existing `SetWindowPos` calls per-mousemove already moved the
window during the drag (per §2.1, that's unavoidable — the loop can't know
in advance the user will release inside the zone), so the maximize call
here is a real position/size change, not a no-op; that's expected and
matches Chrome/Windows' own feel (the window visibly snaps into place on
release).

### 2.4 Scope

- **Main/`FullInstance` top-level windows**: in scope — this is what the
  trigger describes.
- **Floaters**: **out of scope.** They're borderless `WS_POPUP` windows
  with their own maximize semantics already (`toggle_floating_maximize`,
  `chrome.rs:96` — explicitly documented as NOT using
  `ShowWindow(SW_MAXIMIZE)` because borderless popups have no usable
  native placement) and their own drag-loop branch already does something
  very different at the top of the screen (redock-hover / tear-off-back-in
  detection). Adding snap-to-maximize on top of that would need its own
  design pass against the redock interaction, not a bolt-on here.
- **Subwindows** (`open_subwindow`, tied to a parent `FullInstance`):
  open question — likely fine to include (same maximize semantics as main),
  but not the trigger's stated case; confirm before extending, don't
  assume.

### 2.5 Interaction with Esc-cancel

The existing Esc handler (`drag.rs:491-522`) restores the window to its
pre-drag position and sets `cancelled = true`, but keeps looping (so the
eventual `WM_LBUTTONUP` still balances Chromium's mousedown — a real,
documented invariant, do not break it). The new snap logic must respect
`cancelled`: once cancelled, stop updating `in_snap_zone` (mirroring how
`WM_MOUSEMOVE`'s `SetWindowPos` call already gates on `!cancelled`) and
clear any visible snap preview immediately, same as the existing
`clear_floating_redock_hover` call in that same arm does for the floater
case.

## 3. Design — Feature B: border-drag vertical snap

Confirmed broken (§0.2). Fix by **intercepting the native resize, not
replacing it** — a much smaller change than Feature A needed, because an
interception point already exists and is already installed on the main
window.

### 3.1 The interception point already exists

`agentmux-cef/src/client/wndproc.rs:297-360ish`,
`window_edge_resize_wndproc` — a real `SetWindowLongPtrW(hwnd, GWLP_WNDPROC,
...)` subclass already installed on the main window (same family as
`install_top_level_focus_restore_hook`, `wndproc.rs:33`, whose own doc
comment records that subclassing the main CEF Views window's WndProc this
way is "SAFE" and already proven). It already intercepts `WM_SIZING` on
every tick of a native border-drag resize and reads the dragged edge
(`WMSZ_TOP`/`WMSZ_BOTTOM`/etc. from `wParam`) — today purely to forward
`windowresize:tick` events to the renderer for pane redistribution
(`SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26.md` §3.4). Its
own doc comment currently states an invariant that this change deliberately
breaks: *"Pure observer: every message ALWAYS passes through to the
original WndProc."*

**`WM_SIZING`'s `lParam` is a pointer to the proposed `RECT`, and the OS
contract explicitly allows the handler to modify it before returning** —
this is standard, documented Win32 behavior (it's the whole mechanism apps
use to enforce min/max size or aspect-ratio constraints during a native
resize), not something being invented here. This is the same shape as
`WM_SIZING` being the *only* per-tick point during a native resize where
the app can influence that tick's outcome, mirroring why `WM_MOUSEMOVE`
was the right interception point for the manual move loop in Feature A.

### 3.2 The fix

Extend `window_edge_resize_wndproc`'s `WM_SIZING` arm: when `edge` is
`"top"` or `"bottom"` (or a corner combination that includes one), read the
proposed `RECT` from `lparam`, resolve the current monitor's work area
(`MonitorFromPoint`/`GetMonitorInfoW` — same physical-px caveat as §1's
table entry: don't route through `get_monitor_work_area`'s DIP conversion),
and if the proposed top/bottom is within a snap threshold of the work
area's top/bottom, clamp that field of the RECT to the work-area edge
exactly before returning. Write the modified RECT back through `lparam`
and return `1` (`TRUE`) to tell Windows the app adjusted the rect, per
`WM_SIZING`'s documented contract — do not `CallWindowProcW` first and
modify after; the original wndproc must see (or produce) the final,
already-clamped rect, not have its own answer overwritten after the fact.

This is a **narrower, more surgical change** than Feature A's manual move
loop: no new modal loop, no new drag-state machine — one existing
message handler gains a conditional rect mutation on top of the
forwarding it already does. Keep the existing `windowresize:tick` emission
unchanged (still fires, still carries the real edge) — this fix only
changes what final rect that tick converges to, not the pane-redistribution
notification pipeline built on top of it.

### 3.3 Scope

- **Main window**: primary target, matches the trigger.
- **Floaters**: borderless `WS_POPUP`, no native maximize placement (per
  §2.4) — likely wants the SAME `WM_SIZING`-clamp treatment for visual
  consistency, but floaters resize via their own `WM_NCHITTEST` zone
  mapping (`floating_pane.rs:603-649`), a different code path than the main
  window's `window_edge_resize_wndproc` — confirm whether that hook is also
  installed on floater HWNDs or whether floater resize needs its own
  parallel clamp. Don't assume parity without checking.
- **Subwindows**: same open question as §2.4.

### 3.4 What this does NOT need

Unlike Feature A, this does **not** need a snap-preview overlay — a native
resize drag already shows the window's outline moving live (that's what
resizing *is*), so clamping the rect during `WM_SIZING` is both the
detection and the visual feedback in one step; there's no separate
"preview, then commit on release" phase to design the way §2.2 needed one
for the move case.

## 4. Platform scope

This spec is **Windows-first**, because Aero Snap is inherently a Windows
concept and §0's root cause (a Windows-specific CEF frameless-window
limitation, worked around via Windows-specific WndProc subclassing) doesn't
describe macOS or Linux:

- **macOS**: `run_macos_native_drag_loop` (per the win32-move-loop's sibling
  in `drag.rs`) — uses `NSWindow`'s own drag tracking rather than a manual
  `SetCapture`/`GetMessage` loop, since the CEF/Chromium non-client-message
  problem that forced the manual loop is Windows-specific. macOS has no
  Aero-Snap-equivalent "drag to top = maximize" convention in the first
  place (its analogue is the green traffic-light button / Option-drag for
  fill-screen); Chrome on macOS doesn't attempt to replicate Windows' snap
  gesture either. Out of scope for Feature A; not a gap to close.
- **Linux**: the frontend drag hook delegates to `CefWindow::BeginWindowDrag`/
  `xdg_toplevel.move` (a real Wayland/X11 protocol request), which most
  compositors implement their own edge-snap/tiling behavior for at the
  window-manager level, independent of the app. This may already provide
  equivalent behavior for free, or may not depending on the user's WM —
  genuinely out of AgentMux's control either way, and not this spec's
  concern.
- Confirm both of the above with a quick live check rather than taking this
  section's reasoning as certain, but treat Windows as the actual scope of
  implementation work.

## 5. Non-goals

- **Windows 11 Snap Layouts flyout** (hovering the maximize button shows a
  grid of layout options). Related, visually similar, but a distinct
  feature with its own `WM_NCHITTEST`/`ISnapLayoutSuggestions`-adjacent
  integration surface. Not requested; would be its own spec.
- **Left/right edge half-screen snap** (drag title bar to a side edge →
  window fills that half of the screen). Not requested here (the trigger
  is specifically top-drag-to-maximize and border-drag-to-vertical-snap),
  but the mechanism in §2.1-2.3 generalizes to it almost directly (same
  zone-detection-and-preview shape, different target rect and different
  `SetWindowPos` on commit) — worth a one-line callout if scope grows
  later; not building it now.
- **Multi-monitor DPI-mismatch edge cases beyond what §2.1 already
  handles** (dragging across monitors with different scale factors mid-drag,
  right at a snap zone). `MonitorFromPoint` already re-resolves per current
  cursor position each check, so ordinary cross-monitor dragging is
  covered; exotic per-monitor-DPI edge timing is not specifically
  hardened here.

## 6. Test plan

**Feature A (drag-to-top maximize) — needs a real running `task dev`
instance, manual verification (no existing harness reaches `Win32BeginMoveTask`,
per its own file — this is native Win32 message-loop code, not something a
unit test can exercise):**

1. Drag the title bar to the very top of the screen — a preview affordance
   appears (exact visual TBD per §2.2's implementation-time decision).
2. Release while still in the zone — window maximizes.
3. Drag to the top, then drag back down before releasing — no maximize; the
   window ends up wherever it was actually dropped, preview is gone.
4. Esc-cancel while in the snap zone — window returns to its pre-drag
   position (existing behavior), no maximize, preview cleared.
5. Multi-monitor: drag to the top of a *non-primary* monitor — maximizes
   onto that monitor's work area, not the primary's.
6. Confirm floaters are unaffected (dragging a floater to the top still
   does whatever the existing redock-hover/tear-off-back-in logic does
   today, not a new maximize).

**Feature B (border-drag vertical snap) — same "needs a real running `task dev`
instance" caveat as Feature A; `window_edge_resize_wndproc` is native Win32
message-loop code, not unit-testable:**

1. Drag the top border to the screen top — height extends so the top edge
   reaches the work area's top; width and x-position unchanged. This is
   what distinguishes correct behavior from an accidental full maximize.
2. Same for the bottom border, against the work area's bottom.
3. Drag a corner (e.g. top-left) — only the axis actually being dragged
   near an edge should clamp; confirm the OTHER axis is untouched (dragging
   top-left near the top shouldn't also clamp the left edge to the screen's
   left unless that edge is independently near it).
4. Multi-monitor: perform the drag with the window straddling two monitors
   with different work-area geometry (and, if available, different DPI) —
   confirm it clamps against the monitor the relevant edge is actually
   over, not a stale/wrong one.
5. Confirm `windowresize:tick`/pane-redistribution behavior is unchanged
   when NOT near an edge (the existing forwarding pipeline this hook
   already drives must not regress).
6. Floaters and subwindows — per §3.3, confirm whether they need (and
   have, or don't have) the same treatment; don't assume from the main
   window's result.

## 7. What shipped (2026-09-05)

**New — `agentmux-cef/src/client/window_snap.rs`.** All the actual decisions,
as pure integer geometry with no Win32 types and deliberately NOT
`#[cfg(windows)]`, so its 14 tests run on every CI target rather than only
`windows-latest`. Both call sites below do nothing but read the answer.
Covers: `WMSZ_*` → dragged-vertical-edge mapping (corners included, and
pinned against `window_edge_resize_wndproc`'s own edge-string map so the two
can't drift), the snap-fill decision, and the cursor-in-top-zone predicate.

**Feature B — border-drag vertical snap** (`client/wndproc.rs`). Extended
`window_edge_resize_wndproc`'s `WM_SIZING` arm to clamp the proposed rect,
per §3. Notes worth carrying forward:

- The clamp runs **after** the passthrough to the original (Chromium)
  WndProc, not before. Chromium's own `WM_SIZING` handling can rewrite the
  proposed rect (min-size / aspect-ratio constraints live there), so
  clamping first would let it silently discard the snap. Clamping after
  makes this the final word on the vertical axis; the clamp can only move
  an edge by at most `SNAP_THRESHOLD_PX`, so it can't realistically push
  the window under Chromium's minimum height.
- The hook's doc comment previously advertised "pure observer: every message
  ALWAYS passes through" — that invariant is now explicitly narrowed (still
  always passes through, but `WM_SIZING`'s rect is no longer left alone) and
  the comment was rewritten to say so rather than left stale.
- Monitor resolution keys off the proposed rect's **center**, not a corner:
  during a top-edge drag the top corner is exactly the point crossing the
  screen edge, so a corner-based lookup would flip to the monitor *above*
  at the moment the snap should engage.

**Feature A — drag-to-top maximize** (`ui_tasks/drag.rs`,
`ui_tasks/snap_preview.rs`, `commands/window/chrome.rs`).

- `snap_preview.rs` (new): a borderless, click-through
  (`WS_EX_TRANSPARENT|WS_EX_NOACTIVATE`), non-taskbar, topmost layered
  window painting one translucent rect — same shape as Windows' own Snap
  Assist preview. It must be a separate OS window: the preview covers the
  full work area while the dragged window is small and following the
  cursor, so no in-window surface could render it. Created lazily once and
  hidden between drags (creating a window mid-modal-loop on every zone
  entry would be slow and a needless mid-gesture failure point). Every
  failure path is logged-and-swallowed — losing the preview must never
  break the snap itself.
- Zone detection keys off the **cursor**, not the window's own top edge:
  the window follows the cursor at whatever offset the user grabbed it, so
  its edge carries no intent. Preview is shown/hidden on zone
  **transitions** only, not re-issued per mousemove tick.
- Commit and preview-teardown both live at the single post-loop exit point
  every `break` funnels through — that placement is what guarantees the
  overlay can't outlive the drag no matter how the loop ended (release,
  Esc-then-release, `WM_QUIT`, or the wake-tick stolen-capture safety net).
- `chrome.rs` gained `toggle_maximize_hwnd` (extracted from
  `maximize_window`, shared) **and a separate non-toggling `maximize_hwnd`**.
  The gesture deliberately uses the non-toggling one: dragging an
  already-maximized window means "maximize [again, here]", and a toggle
  would restore-down instead — the opposite of the gesture's meaning.
- Floaters are excluded by label prefix (`floating-`), per §2.4 — their
  top-of-screen drag already means redock/tear-back-in.

**Not done, deliberately:** §3.3's open question about whether floaters and
subwindows want the same `WM_SIZING` clamp. `window_edge_resize_wndproc` is
installed on "every top-level non-popup window" per its own doc comment, so
floaters (`WS_POPUP`) do NOT currently get the clamp; whether they should is
left to the live pass (§6 item 6) rather than guessed at.
