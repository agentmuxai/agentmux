# Report: "New Window" startup color-flash sequence

**Date:** 2026-07-14
**Author:** AgentX
**Type:** Investigation report, with all three findings implemented in this same PR. See §8 for what shipped and what's a partial/lower-confidence mitigation vs. a complete fix.
**Purpose:** The hamburger menu's "New Window" shows a visible sequence of solid-color changes before settling into the real UI, instead of one solid background with the pulsating brain-logo splash held until the app is truly ready to paint. This report traces exactly where each color in that sequence comes from and why, as a basis for a future ordering fix.
**2026-07-15 update:** §4.3's gap was live-measured the next day, on a real
Windows GUI session — see `docs/specs/REPORT_NEW_WINDOW_COLOR_FLASH_LIVE_MEASUREMENT_2026_07_15.md`.
Confirmed real and confirmed sizeable: 268ms between the native window
becoming visible and CEF's compositor being told to show anything. The
reorder needed to close it is not attempted there either — same
regression-risk reasoning as §8 below, now backed by a live number instead
of a guess.

---

## 0. Headline finding

**The single-color-plus-pulsing-brain splash the user is asking for already exists** — `index.html`'s `#startup-loading` (§3) — and it's already the first thing painted by the DOM/JS layer. It is not the thing that's flashing. The flash comes from **three separate, independently-verified gaps below the DOM layer**, all Windows-specific (this environment's platform), two of which are explicitly self-documented in the code as known-and-only-partially-fixed:

1. **CEF's own native backstop color doesn't match the splash's color** (§4.1).
2. **The window is shown on `on_load_end` (`did-finish-load`), which CEF's own code comment admits "can fire before anything has visually painted" — gated to real-paint on Linux (shipped yesterday, PR #2151), left ungated on Windows/macOS** (§4.2).
3. **The pool-promote path — the one "New Window" actually takes on a warm app — shows the native Win32 window and shows CEF's own compositor in two separate, non-atomic steps, a gap the code's own comments say used to paint fully blank** (§4.3, the most likely dominant cause).

None of these three are new bugs to invent a fix for from scratch — each has an adjacent, already-shipped or already-designed mechanism (the Linux paint-gate, the reveal-gate/splash-fade system) that a fix would extend rather than reinvent. §6 lays out the ordering each one needs.

---

## 1. Where "New Window" is wired up

- **Menu item:** `frontend/app/window/hamburger-menu.tsx:88-93` — `"New Window"` calls `getApi().openNewWindow()`.
- **Host entry point:** `agentmux-cef/src/commands/window/creation.rs:237-287`, `open_new_window()`. Two paths, tried in order every time:
  1. **Pool-first (the path a running app almost always takes):** `promote_pool_window_for_new_window` → on Windows, `promote_pool_window` (`window_pool.rs:1620-1648` → the Windows body starting `:1006`). The app keeps a small pool of fully-loaded, hidden, off-screen windows specifically so "New Window" is instant — see `window_pool.rs:1602-1619` and the pool doc comment at `creation.rs:264-266` ("~3s saved").
  2. **Cold path (pool empty):** `open_window_with_kind` (`creation.rs:350-446`) → `post_create_window` → `CreateWindowTask` (`ui_tasks/window.rs:1349-1493`), a fresh CEF window spun up from scratch (~2.5-3.5s per the comment at `creation.rs:279`).

Because the pool exists precisely to make "New Window" the common, fast case, **the pool-promote path (§4.3) is what a user is actually exercising almost every time**, and is the strongest suspect.

---

## 2. Enumerated color/paint sequence, in order (pool-promote path, Windows)

| Step | What happens | What paints / why | Citation |
|---|---|---|---|
| 1 | `SetWindowPos` repositions the still-**hidden** native HWND to its final on-screen rect | nothing visible yet | `window_pool.rs:1336` |
| 2 | `set_taskbar_hidden(raw_hwnd, false)` → internally `SW_HIDE` → style change → `SW_SHOWNA` | **the native Win32 window becomes visible on screen.** CEF's own Views `Window`/compositor has *not* been told to show yet (that's step 4, below, and it's asynchronous) — see §4.3 | `window_pool.rs:1354` |
| 3 | `ShowWindow(raw_hwnd, SW_SHOW)`, then a second `SetWindowPos` | further shows/activates the same not-yet-compositor-synced native window | `window_pool.rs:1357, 1361` |
| 4 | `post_promote_pool_window_views_show` **posts** (asynchronously, to a separate UI-thread task queue — not synchronous with steps 2-3) a task that calls CEF Views `window.set_bounds()` + `window.show()` | **this is the first point the browser's own compositor visibility actually flips.** Whatever painted between step 2 and here was CEF's raw backstop color or a stale cached frame, not real content | `window_pool.rs:1401-1412` → `ui_tasks/pool.rs:24-53` |
| 5 | `pool:promote` event emitted to the frontend | frontend's `awaitPoolPromote()` resolves; the splash (`#startup-loading`, already loaded since the window was pre-warmed) is what's visible at this point, body still hidden | `window_pool.rs:1418-1427`, `frontend/app/init/pool.ts:44-93` |
| 6 | `initHostNewWindow()` → `initWave()` fetches full config via RPC, **then** `render(App, elem)` mounts the Solid.js tree | still behind the splash | `frontend/app/init/app-init.ts:1036, 1042` |
| 7 | `App` mounts → theme effect sets `data-theme`/`data-theme-polarity` on `<html>` | theme SCSS overrides activate; still behind the splash | `frontend/app/app.tsx:192-205` |
| 8 | Reveal gate decides the frame has settled (quiet-paint heuristic, 80ms clean / 800ms hard cap) → splash cross-fades 200ms, body unhidden | **first time the real app is visible** | `frontend/app/store/tab-reveal.ts:42-53, 107-178`, `frontend/app/init/startup-splash.ts:17-38`, `app-init.ts:747-750` |

**The flash the user sees is almost certainly steps 2→4**: a real, on-screen native window whose CEF compositor hasn't been told it's visible yet, for a gap that is not bounded by anything synchronous — it's whatever the OS task-queue scheduling happens to take. The `document.body.style.visibility = "hidden"` guard (`app-init.ts:581`) **cannot mask this**, because it's a DOM-level hide; this gap is happening at the native-window/compositor level, entirely below the DOM.

---

## 3. The splash that already exists (and already works, at the DOM layer)

`index.html:8-72, 107-157`:
- `html { background: rgb(34, 34, 34); }` (`:12-14`) — matches the splash's own background, so there's no flash *within* the static HTML/CSS by itself.
- `#startup-loading` — `position: fixed; inset: 0`, same `rgb(34,34,34)`, `z-index: 99999`, forced `visibility: visible !important; opacity: 1 !important` specifically so it survives `app-init.ts`'s later `document.body.style.visibility = "hidden"` (comment at `index.html:29-30` calls this out explicitly: "Without this the spinner disappears for ~200ms during the FOUC guard").
- The pulsing brain is the AgentMux logo SVG inline at `:108-156`, animated via `@keyframes startup-pulse` (`:57-60`), with `will-change: transform, opacity` specifically to keep the pulse smooth under GPU compositing (comment at `:49-56` — without it, Chromium re-rasterizes the 1200×1200-viewBox SVG every frame, which "reads as a blink/flicker instead of a smooth pulse").
- Cross-fade out via `.fading` (`:39-43`, 200ms), driven by `startup-splash.ts:18` (`FADE_MS = 200`, kept in sync by comment).
- **This is exactly the "one solid color + pulsating brain, held until ready" behavior the user is asking for.** It is already the correct design at the layer it controls. It is simply not the last word — steps below the DOM (native window show, CEF compositor show) can produce visible paint before this splash's own JS/CSS machinery ever gets a chance to run.

`BrainSpinner.tsx` (`frontend/app/element/BrainSpinner.tsx`) is the same asset/animation extracted as a reusable component for other loading states (agent-pane blank-load, browser-pane external-site boot — see §5). No new visual asset is needed for any fix; the existing splash is already the right one.

---

## 4. The three below-the-DOM gaps, verified directly against source

### 4.1 CEF backstop color mismatch

| Window | `background_color` | Value | Citation |
|---|---|---|---|
| Global CEF init | `Settings.background_color` | `0x00000000` (transparent) | `agentmux-cef/src/lib.rs:841` |
| **Main window**, non-transparent (the common case) | `BrowserSettings.background_color` | **`0xFF000000` — opaque pure black** | `agentmux-cef/src/app.rs:1180-1189` — comment explicitly explains the opaque-vs-transparent split: "opaque windows use 0xFF000000 so Chromium's compositor treats layers as opaque (better performance, subpixel LCD text, **no opacity flash**)" |
| **Secondary window, cold path** (`CreateWindowTask`, used when the pool is exhausted) | `BrowserSettings.background_color` | **Always `0x00000000` — fully transparent, unconditionally, regardless of the user's `window:transparent` setting** | `ui_tasks/window.rs:1371-1388` — own comment: this mirrors the transparency cascade so floating panes/secondary windows aren't forced opaque when the user wants transparency, but it applies even when the user does NOT want transparency |

Verified: the splash's actual color is `rgb(34, 34, 34)` = `#222222` (`index.html:12-14`, confirmed by direct read). **Neither `0xFF000000` (black) nor `0x00000000` (transparent) is `#222222`.** Any frame the native compositor presents before the page's own stylesheet has taken over shows one of these CEF backstops, then jumps to `#222222` once the page paints — a literal solid-color-to-solid-color transition, exactly the symptom described.

### 4.2 `on_load_end` → `window.show()` is unguarded on Windows (fixed on Linux, yesterday)

`agentmux-cef/src/client/navigation.rs:363-371`:

```rust
#[cfg(not(target_os = "linux"))]
{
    window.show();
    if let Some(host) = b.host() {
        host.set_focus(1);
    }
}
```

verified directly — `window.show()` fires unconditionally on Windows/macOS the moment `on_load_end` fires (CEF's "main-frame HTML finished loading" signal). The surrounding code's own comment (`backend.rs:390-399`, referenced from `navigation.rs`) states plainly:

> "CEF's `on_load_end` … only means 'main-frame HTML finished loading' and can fire before anything has visually painted."

**This exact bug class was fixed for Linux one day before this report** — commit `2587b85c`, "fix(linux): gate startup window-show/splash-dismiss on real first paint (#2151)," 2026-07-14. Its own commit message: *"showing/focusing the real window and dismissing the native splash from that event... let Linux's slower GPU/EGL init path reveal a blank or white surface before real content painted. Adds a `report_first_paint` IPC call fired via double-rAF... and on Linux only, defers window.show()/focus... until that signal arrives or a 1.5s safety-net timeout fires... **Windows/macOS behavior is unchanged.**"* The real first-paint signal was measured landing ~2.08s after `on_load_end` in that PR's own verification run — i.e., on Linux, up to ~2 seconds of "loaded but not actually painted" was being masked incorrectly before the fix. There is no reason to believe Windows doesn't have an analogous (if smaller) gap; it's simply never been measured or gated.

**Documentation gap, worth fixing alongside any code change:** both `bootstrap.ts:22,391` and `navigation.rs:17,57,277,311` reference `docs/specs/SPEC_LINUX_STARTUP_PAINT_GATING_2026_07_13.md` — and the commit message itself says "See ... for the full diagnosis and profiling data this is based on" — but **that file does not exist anywhere in the repository** (confirmed via `find`/`ls`). The diagnosis and profiling data this report would want to build on for a Windows equivalent was apparently never actually committed.

**Important scope note, also verified directly:** the same PR's own commit message states pool-window prewarms landed at "1.36-1.84s, unaffected by this gate" — i.e., **the Linux fix does not touch the pool-promote path at all.** Whatever ships for Windows needs to separately address §4.3 below; extending the Linux `report_first_paint` gate to Windows would fix the cold path (§1, path 2) but not the common pool-promote path (§1, path 1).

### 4.3 Pool-promote: native window shown before CEF's own compositor is told (self-documented, most likely dominant cause)

This is the path an actual "New Window" click almost always takes (§1), and it's the one with the most direct, explicit self-documentation of the exact bug class in question.

`window_pool.rs:1309-1324` (comment, verified directly):

> "position the window at its final ON-SCREEN rect while it is still HIDDEN, then perform the FIRST show THERE... The previous order re-showed it first... at the OFF-SCREEN pool position, and THEN moved it on-screen. On Windows that binds the browser compositor's visibility/surface state to the off-screen show, and the subsequent move+resize never re-syncs it, so **the promoted window paints BLANK despite a valid DOM**."

So this exact code was already rewritten once to fix a *worse* version of this bug (fully blank window). The current sequence (verified directly against `window_pool.rs:1330-1362`):

1. `SetWindowPos` — move hidden HWND to final on-screen rect.
2. `set_taskbar_hidden(raw_hwnd, false)` — **the actual native hidden→visible transition** (internally `SW_HIDE → style change → SW_SHOWNA`).
3. `ShowWindow(raw_hwnd, SW_SHOW)` + a second `SetWindowPos`.
4. Only **after** steps 1-3 complete, `post_promote_pool_window_views_show` is called (`window_pool.rs:1401-1412`), which **posts a task to a separate UI-thread queue** (verified: `ui_tasks/pool.rs:24-53`, `wrap_task!`/`post_task` pattern) — this task is what actually calls CEF's own Views `window.set_bounds()` + `window.show()`.

`ui_tasks/pool.rs:14-22` (comment, verified directly):

> "The Windows promote positions the raw HWND via Win32 and never touched the Views `Window`, so the browser's view-hierarchy/compositor visibility never flipped from hidden → the promoted window painted BLANK despite a valid DOM. **This is the macOS-vs-Windows asymmetry.**"

**The two "show"s are not atomic — step 2/3 is synchronous on the IPC thread; step 4's actual compositor-show is asynchronous, posted to and executed on a different thread's task queue.** Between those two points, the OS is presenting a real, on-screen, focused HWND whose CEF browser/compositor has not yet been told it's visible. What paints in that gap is whichever of CEF's backstop colors applies to that window (§4.1) or a stale cached frame from the window's last known (off-screen, pool-holding) state — **not** the splash, because the splash is DOM content and the compositor hasn't been shown yet at all.

Additionally: step 4's `window.set_bounds()` performs an actual resize (from the pool's off-screen holding size to the real target size) on an *already-visible* window, which can itself force a further visible re-layout/re-paint pass on top of the show-timing gap.

---

## 5. Prior art already in this codebase (fix should extend these, not invent new mechanisms)

- **`docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md`** (2026-05-09) — origin of the reveal-gate/splash-fade system (`frontend/app/store/tab-reveal.ts`), built for the *identical* symptom description ("3-5 visible paint stages read as flicker") on tab switches, later generalized to also cover window opens — `app-init.ts:732-747`'s own comment explicitly calls out "'New Window' from the hamburger menu... is covered by the same reveal-gate mechanism as tab-switch." This is the most directly reusable pattern for anything at the JS/DOM layer (§4.2's Linux fix already plugs into an equivalent mechanism at the native layer via `report_first_paint`).
- **Commit `2587b85c` / PR #2151** (§4.2) — the closest possible prior art: "gate window show on real paint, not on load-complete," already built and shipped, just Linux-scoped. Its `report_first_paint` (double-rAF signal from `bootstrap.ts`) → `on_frontend_first_paint`/`reveal_gated_window` (`navigation.rs:17-153, 315-353`) pattern is the mechanism to extend to Windows for §4.2 — though see §4.3's scope note: it would not by itself fix the pool-promote path.
- **`docs/research/RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md`** (referenced from `window_pool.rs:1309` and `ui_tasks/pool.rs:22`) — the research that produced the *current* (still non-atomic, per §4.3) Windows pool-promote ordering. Whoever picks up §4.3 should start here rather than re-deriving the Windows CEF Views/HWND relationship from scratch.
- **`docs/specs/SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md`** and **`docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md`** — confirm `BrainSpinner`/the brain-pulse asset is already the established, cross-codebase loading indicator (used for agent-pane blank-load and browser-pane external-site boot too), sharing its animation with the startup splash. No new visual component is needed for any fix here.
- **`docs/specs/SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md`** — checked; unrelated (launcher quit/teardown, not startup paint). Worth noting only because it confirms there are **two separate splash layers**: the launcher's own **native pre-splash** (`agentmux-launcher/src/splash.rs`/`splash_mac.rs`, referenced from `navigation.rs:264-269`) which only covers the very first window of a cold app launch, versus `index.html`'s in-page splash (§3) which covers every window including "New Window" on an already-running instance. This report is entirely about the latter; don't conflate the two if a fix touches launcher code.

---

## 6. Theme timing (context for why the splash must stay up as long as it does)

Verified: the real theme is **not** available synchronously.

- The pre-JS fallback color (`index.html`'s `rgb(34,34,34)` and `theme.scss`'s `:root` defaults, `frontend/app/theme.scss:9-26`) is a **static, hardcoded** literal that happens to match the default dark theme — not a theme-aware read.
- The user's actual configured theme is only known after `RpcApi.GetFullConfigCommand` resolves (`app-init.ts:1036-1038`) — an IPC round trip, not a local synchronous read (no `localStorage`-cached theme value is read anywhere in `bootstrap.ts`/`app-init.ts`).
- Only after that RPC resolves does `render(App, elem)` run (`:1042`), and only after `App` mounts does the `data-theme` attribute get set (`app.tsx:192-205`), activating the real per-theme SCSS.
- **This means for any non-default (especially light) theme, there is a real, unavoidable window where only the hardcoded dark splash color is known** — which is fine, and is exactly why the reveal-gate (§5) exists: keep the splash up until the theme has actually applied and the frame has settled. This part of the design is sound. It's undermined only by §4.1-4.3 happening *underneath* it, where the JS-level splash/body-hidden mechanism has no ability to mask a native-window or compositor-level paint at all.

---

## 7. Summary — what produces the flash, ranked by likely visibility

1. **§4.3 (pool-promote non-atomic show)** — almost certainly the dominant cause, since it's the path a real "New Window" click takes on a warm app, it's a native-window-level gap the DOM-level splash cannot mask at all, and the code's own history/comments confirm this exact class of gap has already caused a *worse* symptom (fully blank window) once.
2. **§4.2 (`on_load_end` unguarded on Windows)** — a real, self-documented gap, proven to matter on Linux (up to ~2s of mis-timed show before yesterday's fix), unmeasured but structurally identical on Windows; applies mainly to the cold path (§1 path 2), not pool-promote.
3. **§4.1 (backstop color mismatch)** — compounds whichever of the above windows is open by determining *which* wrong color is seen (black vs. transparent vs. whatever was last cached) rather than causing a gap on its own.

A fix should treat these as three separable pieces of work, likely three separate PRs given how differently they're layered (native Win32/CEF-Views ordering vs. CEF backstop-color config vs. an IPC-gated show), each following the pattern its nearest prior art already established (§5) rather than inventing new machinery.

---

## 8. What shipped in this PR (2026-07-14, same day)

All three findings were implemented in the same PR as this report, at three different confidence levels — stated plainly here rather than left implicit, since §4.3's fix in particular is a partial mitigation, not a complete resolution.

**§4.1 (backstop color) — high confidence, straightforward.** Added `app::OPAQUE_BG_ARGB = 0xFF222222` (matches the splash's `rgb(34,34,34)`), used for the main window's opaque case (`app.rs`) and, made transparency-aware for the first time, the secondary/cold-path/pool-window case (`ui_tasks/window.rs`'s `CreateWindowTask`, previously hardcoded transparent `0x00000000` unconditionally — now `OPAQUE_BG_ARGB` when `window:transparent=false`, unchanged `0x00000000` when `true`, preserving the transparency-cascade fix that literal existed for). This is low-risk: it only changes what color paints during a gap that already exists, not whether/when the gap exists.

**§4.2 (`on_load_end` unguarded on Windows) — medium-high confidence, direct port of working Linux code.** Widened every `#[cfg(target_os = "linux")]` gate around the paint-gate mechanism (`reveal_gated_window`, `handle_first_paint_signal`, the safety-timeout task, the `report_first_paint` IPC handler) to `#[cfg(any(target_os = "linux", target_os = "windows"))]`. The frontend signal (`bootstrap.ts`'s double-rAF `report_first_paint` call) already fires unconditionally on every platform, so no frontend changes were needed. **Also moved the Windows launcher-splash-dismiss signal** (`AGENTMUX_SPLASH_EVENT` via `OpenEventW`/`SetEvent`) out of its old unconditional position in `on_load_end` into `reveal_gated_window` (new `signal_windows_splash_dismiss` helper) — this was necessary, not optional: leaving it unconditional while gating `window.show()` would have dismissed the native pre-splash *before* the now-deferred window actually shows, a strictly worse gap than the one being fixed. This mirrors how the Linux/macOS ready-file write already only fires from `reveal_gated_window`. **Scope reminder from §4.2 itself:** pool windows skip this whole gated block (`is_pool_window` check), so this fix covers the main window's first show and the cold "New Window" path (pool empty) — not the common pool-promote path, which is §4.3's job.

**§4.3 (pool-promote non-atomic show) — the dominant cause per §7, and the one I have the least confidence is fully resolved.** Investigated reordering the Win32-show-then-CEF-Views-show sequence directly, but found no safe insertion point: `set_taskbar_hidden`'s own internal `SW_HIDE → style change → SW_SHOWNA` cycle *is* "the genuine first show" the existing code comments refer to, and it unconditionally hides-then-shows regardless of prior state — so calling CEF's `window.show()` any time before it would just cause `set_taskbar_hidden` to hide the now-visible window again, a *new* flicker, not a fix. The prior research behind the current ordering (`RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md`) already tried and rejected the naive "CEF-show-first" reorder for causing a worse (fully blank) regression once. Reordering further without the ability to visually verify on a real Windows GUI session felt like exactly the kind of change that could silently trade one flash bug for another.

Instead, shipped the change I *am* confident is a strict improvement: made `post_promote_pool_window_views_show` **block** (via the same `mpsc::sync_channel` + `post_task` + `recv_timeout` idiom already used elsewhere in this file for `get_window_position_blocking`) until the CEF Views `set_bounds()+show()` task actually completes on the UI thread, instead of firing it and immediately returning. This does not shrink the native-show-to-compositor-show gap itself (that's still governed by however fast the UI thread's task queue drains, which posting synchronously vs. asynchronously doesn't change) — but it closes a separate, real race: previously, `promote_pool_window` emitted `pool:promote` to the frontend and kicked off a pool refill immediately after posting the CEF-show task, with **no guarantee** the compositor show had even started yet. Now those downstream actions are guaranteed to happen after CEF's own view hierarchy has actually flipped to visible (or after a generous 500ms timeout, so a wedged UI thread can never hang "New Window" indefinitely). Also added `elapsed_ms` timing telemetry (`target: "pool:new-window"`) so a real Windows run can measure the actual gap the same way PR #2151's own verification run measured Linux's first-paint latency before tuning its safety timeout — that measurement is what should inform whether §4.3 needs the riskier reorder after all, rather than guessing from code reading alone.

**Net honest assessment:** §4.1 and §4.2 should meaningfully reduce or eliminate the flash for the main window's first show and any cold "New Window" (pool exhausted). §4.3's fix removes a real downstream race and adds the telemetry needed to validate further work, but the core native-show-timing gap on the common pool-promote path is very likely still narrower than before (color now matches, at least) rather than fully closed — this needs a real Windows visual smoke test to confirm, which wasn't possible from this environment.

---

## 9. References

- Internal: `agentmux-cef/src/commands/window/creation.rs:237-446`, `agentmux-cef/src/commands/window_pool.rs:1006-1433, 1602-1648`, `agentmux-cef/src/ui_tasks/window.rs:1349-1493`, `agentmux-cef/src/ui_tasks/pool.rs:1-81`, `agentmux-cef/src/client/navigation.rs:17-153, 315-396`, `agentmux-cef/src/commands/backend.rs:390-414`, `agentmux-cef/src/lib.rs:841`, `agentmux-cef/src/app.rs:1180-1189`, `index.html:8-157`, `frontend/app/init/app-init.ts:581-583, 712-751, 1029-1042`, `frontend/app/init/pool.ts:44-93`, `frontend/app/init/startup-splash.ts`, `frontend/app/store/tab-reveal.ts:42-53, 107-178`, `frontend/app/app.tsx:192-205`, `frontend/app/theme.scss:9-26`, `frontend/app/element/BrainSpinner.tsx`, `frontend/app/window/hamburger-menu.tsx:88-93`.
- Commit `2587b85c` (PR #2151, "fix(linux): gate startup window-show/splash-dismiss on real first paint") — the direct prior-art fix for §4.2, Linux-only; its own referenced spec (`docs/specs/SPEC_LINUX_STARTUP_PAINT_GATING_2026_07_13.md`) does not exist in the repo — a documentation gap worth closing alongside any Windows follow-up.
- `docs/research/RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md` — prior research behind the current (still non-atomic) §4.3 ordering.
- `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md`, `docs/specs/SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md`, `docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md`, `docs/specs/SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md`.
