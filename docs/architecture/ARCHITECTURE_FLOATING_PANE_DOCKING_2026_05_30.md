# Architecture: Floating Panes, Tear-Off, Redock & Browser-Pane Lifecycle

**Date:** 2026-05-30
**Author:** AgentA
**Status:** Living reference — written to stop the redock/floating regression cycle
**Audience:** anyone touching tear-off, floating windows, redock, browser-pane lifecycle, or window-HWND resolution

> **Why this doc exists.** This subsystem has regressed repeatedly (#1157, #1159, #1165, #1166, #1168, #1173, #1177, and the 2026-05-30 parent-HWND bug). Each fix touched one of ~7 entangled systems and broke another. This document maps the whole thing **once**, names the recurring failure modes, and proposes a modularization that makes the failure modes structurally impossible — so we stop playing whack-a-mole.

---

## 0. TL;DR — the recurring root causes

Every regression in this area is an instance of one of these five themes:

1. **Wrong-window HWND resolution.** Four overlapping resolvers (`resolve_window_hwnd`, `find_main_window`, `find_own_top_level_window`, `capture_hwnd_for_label`) + a lazily-evicted cache. `find_own_top_level_window` returns the *first visible top-level* — which is the **floater** (owned windows draw above their owner), not main. Anything that resolves a window **without a label** silently targets the floater. *(Latest instance: the 2026-05-30 browser-pane parent bug — a redocked pane parented to the dying floater → black.)*
2. **Create-while-closing / redock re-create races.** Redock = the *same* `block_id` closes in the floater and re-creates in the target **at nearly the same time**. The old browser's async `on_before_close` can evict the new entry; the new create can race the old close. #1168 fixed one path; residual intermittent black is another.
3. **Owner cascade.** Floaters are Win32 *owned* popups of the source window → owned windows are force-destroyed with the owner (close cascade), pinned above the owner (Z-order), and `GetParent(floater)` returns the owner (so `GA_ROOT` of the floater's CEF child lands on **main**).
4. **Coordinate-system drift.** CSS px ↔ physical px ↔ DIP ↔ DPR. Pane rects are CSS×DPR; floaters may spawn on another monitor.
5. **Fire-and-forget IPC coordination.** Redock is a *sequence* of frontend→host IPC calls (`resolve_window_at_cursor` → `RedockFloatingPane` → block moves → floater auto-closes → pane re-creates) with no transaction boundary; the frontend layout (`onNodeDelete → DeleteBlock`) races the backend move.

**The proposal (§9):** collapse window resolution to one canonical label→HWND function (ban label-less resolution), thread the **target window label** explicitly through pane creation, and model redock as an **atomic backend MOVE** owned by the reducer rather than a frontend-orchestrated close-then-recreate.

---

## 1. Entities & glossary

| Term | What it is | Lifetime / owner |
|---|---|---|
| **Docked pane** | A pane (`Block`) rendered inside a window's tile layout. | The window's workspace/tab. |
| **Floater / floating pane** | A torn-off pane in its own frameless `WS_POPUP` window (`floating-<uuid>` label). | Its own workspace+tab; the OS window is owned by the source window (today). |
| **Window** | A top-level CEF/host window: `main`, `window-*`, promoted pool windows. | Host process. |
| **Browser pane** | A pane whose content is a *native child browser* (`browser-pane-<block_id>-<seq>` label) embedded via `set_as_child`. Has a **second HWND** (the web-content child) layered over the frontend. | Reducer `HostState.browser_panes`, keyed by `block_id`. |
| **`window_hwnds` cache** | `Mutex<HashMap<String/*label*/, isize/*HWND*/>>` on `AppState`. The authoritative label→outer-top-level-HWND map. | Host (`state.rs:763`). |
| **Pool window** | Pre-warmed off-screen CEF window (parked at x=-32000) for fast tear-off/promote. | `window_pool.rs`. |

**Label conventions:** `main`, `window-*`, `floating-<uuid>`, `browser-pane-<block_id>-<seq>`, `window-pool-*`. The `-<seq>` on browser panes is a monotonic counter so close-then-recreate of the same block doesn't collide (`reducer/mod.rs:202`).

---

## 1.1 Lifecycle policy — **floaters do NOT keep the instance alive**

**Invariant FP-LIFE: a floating pane dies with the last top-level window.** Closing the last
`main`/`window-*` (real top-level) window quits the instance even if one or more floaters are still
open — they are torn down with it. A floater is never the sole thing keeping the process tree alive.

**Why.** A floater is part of the user's session, not an independent instance. The #1676 win_event
last-window quit trigger (`count_visible_user_windows`) already excludes floaters by window-class, so
at the OS-quit level this has always been the behavior; this invariant makes the *reducer count*
agree — it previously, accidentally, counted direct `floating-<uuid>` floaters as instance-keeping
windows but **not** `floating-pool-<uuid>` ones (same user-facing thing, opposite behavior, decided
purely by which code path created the floater).

**How it's enforced — by TYPE, not by label.** Floaters are their own `BrowserKind::Floater` variant
(warm pane-pool = `is_pool: true`, promoted/visible = `is_pool: false`). The last-window gate
`reducer::quit::is_live_user_window` counts only `TopLevel { is_pool: false }`, so **all** floaters
are excluded *by type*, regardless of `floating-<uuid>` vs `floating-pool-<uuid>` labeling. This
replaced a fragile `!label.starts_with("floating-pool-")` string check. See
`SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md` (finding L4) and
`SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md`.

**If this ever changes** (e.g. "a *visible* floater should keep the instance alive"), it is a
deliberate decision that MUST update BOTH the win_event quit trigger (`count_visible_user_windows`)
AND the reducer count together — never just one, or they desync (which is the bug class this whole
doc exists to prevent).

---

## 2. Operation flows (frontend → host)

### 2.1 Tear-off (docked pane → floater)
1. **Drag start** — pragmatic-dnd `draggable()` on `[data-role="block-header"]` (`TileLayout.win32.tsx:439`). On Win32 the dragHandle API is avoided (breaks WebView2 dragstart).
2. **Drop detection** — `CrossWindowDragMonitor.win32.tsx`: `dragend`, or an **800ms fallback timer** after `dragleave` that polls `get_mouse_button_state` (OLE drops over native apps don't deliver `dragend`).
3. **`performTearOff()`** (`CrossWindowDragMonitor.win32.tsx:265`): `measureSourcePaneSize()` (CSS px — *not* ×DPR, floater may land on another monitor) → `TearOffBlock` RPC (new workspace+tab) → **`open_floating_pane_window` IPC FIRST, delete docked layout node only on success** (avoids orphaned workspace).
4. **Host** (`floating_pane.rs`): `open_floating_pane_window` validates, DPI-scales CSS→physical, allocates `floating-<uuid>`, posts `CreateFloatingWindowTask`.
5. **New renderer** loads `?workspaceId=…&windowLabel=floating-…&floatingPaneId=…`. `app.tsx:375` detects `floatingPaneId` → renders `<FloatingPaneWorkspace/>` (no tab bar / status bar; just the 33px `BlockFrame_Header`).

### 2.2 Header move (JS-driven window drag)
`floating-pane-workspace.tsx` `onMount`:
- `onMouseDown` on `[data-role="block-header"]` (skipping `INTERACTIVE_SELECTOR`) → **`preventDefault()` is load-bearing** (blocks the HTML5 dragstart that would *re-tear-off* the block — "double tear-off") → `get_window_position` → `dragging=true`, guarded by `currentMouseDownId`.
- `onMouseMove` → coalesced `set_window_position` (one IPC in-flight, last wins, DPR-scaled delta).
- During drag, throttled `update_floating_redock_hover` (~50ms) drives the drop-target highlight.

### 2.3 Edge-resize (floater only)
`floating-pane-workspace.tsx:130-260` — an 8px (`FLOATER_EDGE_RESIZE_BORDER`) invisible **pointer** grab-band → `get_window_rect`/`set_window_rect`. Browser floaters inset their web-content child by the band depth (`use-pane-rect-sync.ts:70`) so the frontend owns the band. **Note:** the band overlaps the top of the 33px header → grabbing the top edge resizes instead of moving (a known UX sharp-edge; reagent flagged it on #1177). Shipped at 12px, regressed to 4px as a side effect of PR #1829's browser-pane matte fix, restored to 8px — see `docs/retro/retro-floating-pane-resize-hit-target-2026-07-27.md`.

### 2.4 Redock (floater → drop on a window)
`onMouseUp` → `clear_floating_redock_hover` → `tryRedockAtCursor()` (`floating-pane-workspace.tsx:409`):
1. `resolve_window_at_cursor({x, y, exclude_label: self})` → target `{label, window_id}` (walks real top-level frames, matches `window_hwnds`).
2. Load target workspace via WaveObj graph (non-pinning `reloadWaveObject`, non-reactive `getObjectValue` — avoids refcount leak in the async mouseup with no reactive owner).
3. `RedockFloatingPane(blockId, sourceTabId, sourceWsId, targetTabId, targetWsId)` → backend moves the block → source `tab.blockids` empties → **auto-close watcher** dismisses the floater.

### 2.5 Browser-pane lifecycle (frontend side)
`browser-view.tsx`: `createPane` → `browser_pane_create({block_id, url, window_label, ...rect})`; placeholder rect → `browser_pane_resize` (deduped, RAF during reflow); `onCleanup` → **`browser_pane_close` BEFORE disconnecting observers** (flips backend to `Closing` so late IPC no-ops). Focus: `main_window_focus` on address-bar mousedown/focus; `browser_pane_focus` on content click.

---

## 3. Browser-pane lifecycle (host) — the state machine

**States** (`HostState.browser_panes`, keyed by `block_id`, `state.rs:134`): `Live` → `Closing{since}` → *removed*. Every op short-circuits when `Closing`.

**Create** (`browser_panes.rs:157`): `TryRegisterBrowserPaneLive` → `RegisterResult`:
- `Fresh(label)` → post `CreateBrowserPaneTask`.
- `AlreadyLive(label)` → re-navigate existing.
- `Closing` → **stash the create params in the reducer under the same lock that observed `Closing`** (`reducer/panes.rs:104`), replay on close-completion. **This is the #1168 fix.** Comment (`browser_panes.rs:204`): *"old CEF Browser mid-teardown — don't overwrite (its `on_before_close → DrainBrowserPaneByLabel` would evict the NEW entry)."*

**CreateBrowserPaneTask::execute** (`browser_pane/creation.rs`): resolve parent HWND → `set_as_child(parent, rect)` → `browser_host_create_browser`.
> ⚠️ **THE 2026-05-30 BUG:** parent was resolved via `find_own_top_level_window()` (first visible top-level). On redock the floater is still alive and on top, so the redocked pane (`window_label=main`) was parented to the **dying floater** → cascade-destroyed mid-load (`ERR_ABORTED`) → black. **Fix (proven, parked on `agenta/browser-pane-redock-fix-wip`):** resolve from `window_label` via `resolve_window_hwnd`. See §8.

**Close** (`browser_panes.rs:302`): `EnqueueBrowserPaneClose` (Live→Closing) → `close_with` → `UnregisterBrowser` (atomic take, `removed_browser`) → `destroy_hwnd`: **`ShowWindow(SW_HIDE)` then `DestroyWindow` then `InvalidateRect(parent)`** (DWM otherwise leaves the last GPU frame "stuck"). We deliberately **do not** `host.close_browser(force)` — Alloy treats pane+main as one close unit and quits the app. → `CompleteBrowserPaneClose` (remove entry, return any deferred create to replay) → `replay_pending_create` → `spawn_pool_window`.

**Async drain** (`on_before_close_browser_pane` → `drain_closed_label`, idempotent): drains the reducer entry, replays deferred create, refills pool.

**App-exit gate** (`client/mod.rs:843`): counts *user-facing* browsers via `user_visibility_snapshot()` (single atomic lock) — **excludes pool windows and `browser-pane-*`, but counts `floating-*`** → a surviving floater keeps the app alive. The `"no backend window ID registered … shells may orphan"` warning (`client/mod.rs:1063`) fires when a closing label has no backend window id.

**Reducer events:** `BrowserPaneCreateRequested` / `BrowserPaneLive` / `BrowserPaneClosing` / `BrowserPaneClosed` / `BrowserUnregistered`. **TOCTOU lesson (#1168 / PR #660):** cross-arm state (deferred create, removed browser) must live *in* `HostState` and move atomically with the observation — never a sidecar `Mutex` on `AppState`.

---

## 4. Window-HWND resolution — the fragile core

`commands/window/lifecycle.rs`. **Three-tier `resolve_window_hwnd(state, label)`** (line 207):
1. **Cache** (`window_hwnds[label]`) guarded by `IsWindow` (lazy-evict on stale — `SPEC_WINDOW_HWND_CACHE_STALE_FIX`). **Cache hits are returned verbatim — never GA_ROOT-walked** (that would land on the floater's owner=main).
2. **Reducer registry** (`get_browser(label).host().window_handle()` → `GA_ROOT`) — GA_ROOT needed because `set_as_child` returns the inner `WS_CHILD`.
3. **EnumWindows fallback:** `label=="main"` → `find_main_window` (skips `AgentMuxFloatingPane` class **and** off-screen pool windows < `OFFSCREEN_POOL_THRESHOLD_X = -20000`); else → `find_own_top_level_window` (first visible — **the trap**).

**Cache writers:** `floating_pane.rs:145` (pre-registers the floater outer HWND at create — without it, `close_window_by_label("floating-…")` would WM_CLOSE **main**), and `capture_hwnd_for_label` (walks to GA_ROOT once at window-ready, guarded against overwriting pre-registered floaters and against binding to off-screen pool windows).

**`find_own_top_level_window` is the recurring footgun.** Doc comment (`lifecycle.rs:99`): *"returns the first visible top-level… owned windows draw ABOVE their owner, so as soon as a floating-pane window exists, every label-less `get/set_window_position` accidentally targets the floater — dragging the main window moves the floater instead."* It must **never** be used when a label is available.

**The dual of the footgun — an on-screen pool window mislabeled as main (proven 2026-05-30).** `find_main_window`'s pool skip (`is_offscreen_pool_window` = `rect.left < OFFSCREEN_POOL_THRESHOLD_X`) only catches pool windows *parked* off-screen. A warm-pool window **promoted on-screen** to serve as the user's main window — which *every* fresh launch does (windows boot in `pool mode — deferring init until promote`) — but never **relabeled** evades it: `window_hwnds` still maps `window-pool-<uuid> → its HWND` while `main` is left with *no live HWND* (`resolve_window_hwnd("main")` logs *"no available HWND for label=main"*) and `find_main_window` returns the pool HWND *as* main. The frontend, meanwhile, defaults `myLabel = ?windowLabel ?? "main"`, so it identifies as `main`. Captured via the permanent `redock-resolve` trace: `resolve_window_at_cursor` over the on-screen main handed back the stale `window-pool-*` label, which never matched the target's `myLabel === "main"` ghost gate → **no landing ghost, no dock onto main** (terminal *and* browser panes; this is the long-running "redock-onto-main doesn't work" class). **Fix:** when the HWND under the cursor is the cache-independent main frame (`find_main_window`) *and* its reverse-map label is a lingering `window-pool-*`, resolve it as `main`, reconciling the OS cache with the frontend's identity. The deeper cure is to make **promotion relabel** the window (`window-pool-* → main`) so the cache, the backend, and the frontend agree — tracked as a P1/P2 follow-up.

---

## 5. The floater OS window (`floating_pane.rs`)

`create_owned_popup` → `CreateWindowExW(WS_POPUP | WS_THICKFRAME, WS_EX_TOOLWINDOW, owner = source-main-HWND)` + `DwmExtendFrameIntoClientArea(-1)` + `SW_SHOWNOACTIVATE`. **The owner = source window** buys no-taskbar/no-Alt-Tab *and* the minimize/restore/**destroy** cascade — but also: pinned-above-owner Z-order, and `GetParent(floater)=owner` (so the floater's CEF child's `GA_ROOT` = main, the reason the cache pre-register exists).

`floating_pane_wndproc`: `WM_NCCALCSIZE→0` (frameless), `WM_NCHITTEST` maps a 6px (`RESIZE_BORDER_CSS`) native band (mostly moot — CEF child consumes hit-testing; edge-resize is JS-driven per #1177), `WM_SIZE` resizes the **bottom-most direct child** (the frontend browser) — *not* `GW_CHILD` (the web-content child), which would stretch the page over the header (#1173).

> **Parked experiment** (`agenta/floater-independence-wip`): create the popup **unowned** (`owner=null`) → floaters survive the source window's close + follow normal activation Z-order (last-clicked-to-front). Verified to deliver those two behaviors; not shipped because it surfaced the latent `onNodeDelete`-on-redock race separately.

---

## 6. Coordinate systems

| Layer | Unit | Notes |
|---|---|---|
| Frontend DOM | CSS px | `getBoundingClientRect()` |
| Win32 / `SetWindowPos` | physical px | CSS × `devicePixelRatio` |
| CEF Views (Linux/macOS) | DIP | × zoom factor |
| Tear-off size | CSS px (no DPR) | floater may spawn on another monitor |
| Browser-pane rect | CSS × DPR | retro 2026-04-19: hardcoded DPR=1 broke HiDPI |

**Rule:** convert at the IPC boundary; label every rect with its unit.

---

## 7. Fix history & the themes they map to

| PR | What it fixed | Theme (§0) |
|---|---|---|
| #1157 | Co-evict pane window-placement state on close | 2 |
| #1159 | Reducer-backed floater maximize/restore | — |
| #1165 | Never bind a label to an off-screen pool HWND (window-drag) | 1 |
| #1166 | Deterministic redock-onto-main (don't depend on HWND-capture race) | 1, 5 |
| #1168 | Deterministic re-create-after-close (redocked panes always load) | 2 |
| #1173 | Maximize sizes the frontend child, not the web-content child | 5 (owner/children) |
| #1177 | Edge-resize floaters by dragging edges/corners | — |
| **2026-05-30** | **Browser-pane parent = `window_label`, not first-visible** | **1** |
| **2026-05-30** | **Redock-onto-main when main is an unrelabeled on-screen pool window** — `resolve_window_at_cursor` resolves the `find_main_window` frame as `main` over a stale `window-pool-*` reverse-map label | **1** |

Cross-cutting themes from the docs catalog: HWND/label-binding races (1), create-while-closing TOCTOU (2), CEF Alloy renderer-process sharing, coordinate drift (4), reactive-owner cascade, dual drag mechanisms, Linux/macOS Views gaps.

---

## 8. Current open issue (2026-05-30)

**Symptom:** redocking a **browser** pane sometimes renders it **black** (terminal/agent redock is unaffected).

**Proven (via the new `browser_pane::trace` `[pane-trace]` instrumentation — on the wip branch):** the redocked pane is created with `requested_window=main` but `parent_hwnd == the floater's HWND`; the floater's HWND dies → its child (the pane) is destroyed first (child-before-parent) → `ERR_ABORTED` → black. **Deterministic** part fixed by resolving the parent from `window_label`.

**Residual:** even with the fix the pane *occasionally* still goes black. Hypothesis: the create-while-closing race (theme 2) is still reachable in a narrow window — the floater pane's `Closing` and the target's `Fresh` create interleave such that the target create resolves a parent or load that the close then tears down. Needs the `[pane-trace]` `seq`-ordered log under a fresh profile to confirm whether it's (a) `create-deferred-closing` → replay landing late, or (b) a second wrong-parent slip.

**Parked artifacts (nothing lost):**
- `agenta/browser-pane-redock-fix-wip` — the parent-HWND fix + the **permanent** `browser_pane/trace.rs` lifecycle instrumentation (keep it; this area keeps needing it).
- `agenta/floater-independence-wip` — the unowned-floater (survival + z-order) experiment.

---

## 9. Modularization proposal — make the failure modes impossible

The five themes share a root: **identity (which window?) and ownership (whose lifecycle?) are resolved implicitly, late, and in multiple places.** Make them explicit and single-sourced.

### P1. One canonical window resolver; ban label-less resolution
- Collapse `resolve_window_hwnd` / `find_main_window` / `find_own_top_level_window` / `capture_hwnd_for_label` behind **one** API: `window_hwnd(label) -> Option<HWND>`. Internally it owns the cache + the fallbacks.
- **Delete `find_own_top_level_window` from any call site that has a label.** It (and `move_window_by`) are the recurring wrong-window source. If a caller truly has no label, that's a bug to fix, not a fallback to use.
- *This single change would have prevented #1165, #1166, and the 2026-05-30 parent bug.*

### P2. Thread the target window explicitly through pane creation
- `CreateBrowserPaneTask` already carries `window_label`. Make the parent HWND come **only** from `window_hwnd(window_label)` (the fix), and have the `OpenFloatingPaneArgs`/create path carry the source/target window label end-to-end (the pre-existing TODO in `floating_pane.rs`/`creation.rs`). No `find_own_top_level_window` in any create path.

### P3. Model redock as an atomic backend MOVE, not frontend-orchestrated close+recreate
- Today redock = `RedockFloatingPane` (block move) **+** floater auto-close **+** pane close in floater **+** pane create in target **+** frontend `onNodeDelete → DeleteBlock` — five independently-scheduled steps that race (the black render *and* the "block is in tab A not B" `onNodeDelete` error both come from this).
- Introduce a single reducer-owned **PaneLocation** state machine: `Docked(window, tab)` ⇄ `Floating(floater)` with one transition `Redock{from_floater, to_window, to_tab}` that (a) re-parents the browser's HWND to the target window **without** a destroy+recreate where possible, or (b) if a recreate is unavoidable, sequences close-then-create deterministically (extending #1168) and suppresses the frontend `DeleteBlock` for a block that is *moving* (the `markRedocking` idea, but enforced in the reducer/move, not a frontend flag).

### P4. Decouple floater OS-window lifetime from the source window
- Adopt the unowned-floater model (parked branch) so closing a window can't cascade-destroy floaters, and Z-order is last-clicked-to-front — removing theme 3 entirely. Re-anchor no-taskbar via `WS_EX_TOOLWINDOW` (already independent of owner).

### P5. Keep the lifecycle trace permanent + add `seq` to the frontend
- `browser_pane::trace` (`[pane-trace] seq=… block=… event=…`) stays in. Add the matching frontend `[pane-trace]` (create/close/move with the same `block_id`) so a single `muxlog host pane-trace` reconstructs the full cross-window churn. (Per user: do **not** remove instrumentation; remove only in the far future.)

### P6. A redock/floating integration test harness
- The retros show manual testing misses intermittent races (and a test harness was silently broken for weeks). Add a deterministic harness that scripts tear-off → redock × N for terminal **and** browser panes and asserts `load-end` (not `load-error`) on every redock — so a regression is caught by CI, not by the user on the 11th browser.

**Sequencing:** P1+P2 are small, high-leverage, and fix the current deterministic bug class. P3+P4 are the structural wins (one PaneLocation owner). P5+P6 are the guardrails. Do them as **separate, single-concern PRs**, each verified on a **fresh profile** against the full checklist (dock · redock · terminal+browser resize · z-order · survival · browsers-render), reverting any PR that reddens the checklist before stacking the next — the discipline that this session proved we need.

---

## 10. Appendix — authoritative file map

| Concern | File(s) |
|---|---|
| Tear-off (frontend) | `frontend/layout/lib/TileLayout.win32.tsx`, `frontend/app/drag/CrossWindowDragMonitor.win32.tsx`, `frontend/app-init.ts`, `frontend/app/app.tsx` |
| Floater workspace (frontend) | `frontend/app/workspace/floating-pane-workspace.tsx`, `frontend/app/workspace/floater-resize.ts` |
| Browser pane (frontend) | `frontend/app/view/browser/browser-view.tsx`, `frontend/app/platform/pane-rect-registry.ts` |
| Layout / onNodeDelete | `frontend/app/tab/tabcontent.tsx`, `frontend/layout/lib/{layoutTree,layoutPersistence}.ts` |
| Floater OS window (host) | `agentmux-cef/src/floating_pane.rs`, `agentmux-cef/src/commands/floating_pane.rs` |
| Window resolution (host) | `agentmux-cef/src/commands/window/lifecycle.rs`, `…/motion.rs`, `…/mod.rs` |
| Browser-pane lifecycle (host) | `agentmux-cef/src/browser_panes.rs`, `agentmux-cef/src/browser_pane/{creation,callbacks,trace}.rs` |
| Reducer | `agentmux-cef/src/reducer/{mod,panes,browsers}.rs`, `agentmux-cef/src/state.rs` |
| App-exit / orphan gate | `agentmux-cef/src/client/mod.rs` |
| `window_hwnds` cache | `agentmux-cef/src/state.rs:763` |

| Spec / analysis | Path |
|---|---|
| Tear-off | `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` |
| Edge-resize | `docs/specs/SPEC_FLOATING_PANE_EDGE_RESIZE_2026_05_29.md` |
| Header drag | `docs/analysis/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md` |
| HWND cache stale | `docs/specs/SPEC_WINDOW_HWND_CACHE_STALE_FIX_2026_05_28.md` |
| Redock load race | `docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_LOAD_RACE_2026_05_29.md` |
| Browser-pane state | `docs/specs/browser-pane-state-catalog.md`, `…/browser-pane-reducer-roadmap.md` |
| Per-window opacity (cache origin) | `docs/specs/SPEC_PER_WINDOW_OPACITY_2026-05-14.md` |
