# Linux/macOS: wire startup-time window-pool fill

**Status:** BLOCKED — see "Blockers" below. Investigation 2026-05-08; reclassified 2026-05-10 after PR #788 round-2 codex review.
**Author:** runtime investigation 2026-05-08.
**Owner:** TBD.
**Affects:** Linux + macOS AppImage / .app builds.
**Out of scope:** Windows (already correct).

---

## Blockers (added 2026-05-10)

Two independent blockers — fixing one doesn't unblock startup pool fill.

1. **`promote_pool_window` is `cfg(target_os = "windows")` only.** The non-Windows impl at `agentmux-cef/src/commands/window_pool.rs:872-886` always returns `None` with the comment "Non-Windows: pool isn't built yet (Phase 7). Caller falls back to the cold path." Any pre-warmed pool windows on Linux or macOS can never be consumed by tear-off — they're strictly wasted RAM + CPU. Codex P2 on PR #788 caught this for the macOS path; the same applies to Linux.
2. **(Linux/Wayland only)** `POOL_OFFSCREEN_X = -32000` in `window_pool.rs` is a Win32/X11-era hack that the Wayland compositor ignores. Pool windows appear ON SCREEN as visible blank windows because Wayland doesn't let clients dictate position.

Blocker (1) alone makes startup pool fill the wrong call on any non-Windows platform until Phase 7 lands a working `promote_pool_window`. Blocker (2) additionally requires a Wayland-correct hide mechanism (`xdg_toplevel.set_minimized`? transparent surfaces? `--headless` renderer pool?) before any visible-side approach works.

**Implementation status:** the cfg(macos)-only version of the startup pool fill that landed in PR #788 was reverted in the same PR after the codex finding. App.rs no longer fires `spawn_pool_window` from `on_window_created`; the call site is intentionally absent. When Phase 7 implements `promote_pool_window` for non-Windows platforms, this spec is the right place to re-enable startup fill.

---

## Problem

On Linux (and presumably macOS), the window pool is **never seeded at app
startup**. It sits at zero until incidentally refilled by a pane-close path.
Tear-off therefore always takes the cold-path: a fresh top-level CefWindow +
Browser + Renderer + frontend-bundle load on each tear, instead of promoting a
pre-warmed pool window.

User-visible symptom: tear-off takes ~3-3.7 seconds on Linux. On Windows the
same operation is "instant" because the pool is full at boot.

---

## Evidence

From `~/.agentmux/versions/0.33.723/logs/agentmux-host-v0.33.723.log.2026-05-08`,
session starting `19:29:24` (fresh process), filtering pool events:

```
19:29:24.209  BrowserRegistered  label=main  kind=TopLevel { is_pool: false }   ← only registration in startup
19:29:33.052  [dnd:cef] start_cross_drag
19:29:33.247  WARN [pool] pool exhausted on tear-off — frontend will cold-path
19:29:33.399  WARN [fe] [dnd:cross] pool promote failed, cold-pathing {"error":"Error: pool_exhausted"}
19:29:33.423  [create-window] task entered UI thread          ← cold path begins
19:29:33.461  browser_view_create returned
19:29:33.537  Window registered (the new top-level)
19:29:35.935  [initTauriNewWindow] tear-off                   ← +2.4s of frontend cold-load
19:29:36.767  DragOverlay listening on the new window         ← user-visible "done", T+3.7s
```

Cross-checked `0.33.721` log: the only pool spawn observed across the entire
session was `19:02:55.120 [pool] spawning pool window` — and that was triggered
*after a pane close*, not at startup. The post-close path
(`browser_panes.rs:333`) is currently the only pool refill source on Linux.

Result: every fresh session's first tear-off cold-paths. After that, the pool
gets opportunistically filled by pane-close churn — not by design.

---

## Root cause

`agentmux-cef/src/saga_dispatch.rs:325`

```rust
#[cfg(target_os = "windows")]
pub struct LiveActionRunner { … }

#[cfg(target_os = "windows")]
impl SagaActionRunner for LiveActionRunner {
    fn spawn_pool_window(&self) -> String {
        crate::commands::window_pool::spawn_pool_window(&self.state);
        String::new()
    }
    …
}
```

`LiveActionRunner` is the only production impl of `SagaActionRunner`, and it
is **Windows-only**. The launcher-driven saga path that fires
`SpawnPoolWindow` commands at startup on Windows has no production receiver
on Linux/macOS, so the saga commands fire into the void.

The doc comment on `commands/window_pool.rs:25` even acknowledges the
intended trigger:

```
// - App startup → spawn N pool windows after primary first-paint.
```

That trigger is unwired on non-Windows. The comment describes the design;
the implementation only delivers it on Windows.

`spawn_pool_window` itself is fully cross-platform (it uses CEF directly with
no platform-specific gates inside the function body). So the bug is a missing
caller, not a missing implementation.

---

## Options considered

### Option A — Add a Linux/macOS `LiveActionRunner` impl

Make `LiveActionRunner` available on all platforms, route pool-fill saga
commands through `launcher_ipc.rs` on Linux/macOS too.

**Pros:** Architecturally consistent across platforms. Reuses the existing
saga dispatch + report machinery.

**Cons:** On Linux/macOS there is no `agentmux-launcher` binary — the
AppImage / .app *is* the host. Sagas are designed to be sent from the
launcher to the host over IPC; without a launcher there's nothing to issue
the saga. Either the host has to issue the saga to itself (awkward) or we
have to build a launcher binary for non-Windows. Both are large.

### Option B — Direct startup-time call from the host

After the main window's renderer first-paints, call
`spawn_pool_window(state)` `POOL_TARGET_SIZE` times directly. Skips the
saga path entirely; the host owns its own pool fill on platforms with no
launcher.

**Pros:** Tiny change. Single file touched. Honors `is_quitting` /
`any_browser_pane_closing` gates already inside `spawn_pool_window`.
Idempotent in-flight semaphore prevents over-fill if called concurrently
with any other refill source.

**Cons:** Slightly diverges from the Windows architecture (host self-fires
instead of launcher-fires). Tolerable: the host ALREADY self-fires from
`browser_panes.rs::close` and `drain_closed_label`; this just adds a third
self-fire site, the startup one.

### Option C — Defer to first user activity

Fill the pool on first hover over the tab strip, on first tab create, etc.
Avoids paying any pre-warm cost when the user never tears.

**Pros:** Saves ~50-100MB RSS on users who never tear off.

**Cons:** Doesn't actually solve the latency for the *first* tear-off,
which is the user-reported regression. And anyone who tears at all ends up
paying the cold cost on the first tear anyway. Better tackled as a separate
"lazy pool" change later, if the RSS cost matters.

### Recommendation

**Option B.** Smallest change, exact behavior parity with the Windows pool
once the seeds land, and the gating logic that already lives inside
`spawn_pool_window` makes it safe to call freely.

---

## Implementation

### Where to call from

`agentmux-cef/src/app.rs::AgentMuxWindowDelegate::on_window_created` is the
right hook. The Linux/macOS-only `state.windows` registration block already
runs there (lines 70-77). Immediately after registering the *main* window
(label `"main"`), kick off pool fill.

```rust
#[cfg(not(target_os = "windows"))]
if let Some((state, label)) = self.window_registration.as_ref() {
    state.windows.lock().insert(label.clone(), window.clone());
    tracing::info!(
        window_label = %label,
        "[browser-pane] registered Window in state.windows for pane attachment"
    );

    // Linux/macOS startup pool fill — Windows uses the launcher saga path
    // (saga_dispatch.rs::LiveActionRunner is cfg(windows) only). On
    // platforms without a separate launcher binary, the host has to seed
    // the pool itself or first-tear-off cold-paths every time. See
    // docs/specs/linux-pool-startup-fill-2026-05-08.md.
    if label == "main" {
        for _ in 0..crate::commands::window_pool::POOL_TARGET_SIZE {
            crate::commands::window_pool::spawn_pool_window(state);
        }
    }
}
```

Why `label == "main"`: sub-windows (tear-off targets, pop-up windows, pool
windows themselves) all flow through the same `on_window_created` callback.
Filling the pool only when the first main window registers prevents
secondary windows from re-triggering pool fill.

Why `POOL_TARGET_SIZE` direct calls (not just one): `spawn_pool_window`'s
in-flight semaphore single-flights — concurrent calls collapse to one
spawn-in-flight at a time. Each call seeds **at most one** new pool entry.
Calling it twice in a tight loop won't fill two slots; only the first call
runs, the second early-outs against the in-flight flag. The correct
loop-with-callback pattern is: fire one, and rely on the
`mark_pool_window_renderer_ready` → `spawn_pool_window` recursion in
`window_pool.rs:401` to keep refilling until target is reached.

So the loop above is wrong. Use one call:

```rust
if label == "main" {
    crate::commands::window_pool::spawn_pool_window(state);
}
```

That single call kicks off the recursion that fills the pool to target. The
internal capacity check at `window_pool.rs:128` (`pool_queue_size() >=
POOL_TARGET_SIZE` → return) provides the upper bound.

Verify this assumption during implementation by reading
`mark_pool_window_renderer_ready` end-to-end. If the recursion is broken /
gated, the right fix may be to add a startup-only loop that polls
`pool_queue_size()` until target.

### Timing — should it be deferred?

The user-visible main window first-paint happens around `+2.4s` from process
start (per startup-bench). Spawning a pool window takes ~370ms (per the
tear-off trace `[create-window] task entered UI thread → window_create_top_level returned`).
Two pool windows in a tight chain after main first-paint adds ~700ms of
background CPU work on the UI thread.

Options for when:

1. **Immediately after main-window registration (`on_window_created`):**
   simplest; pool fill races slightly with main-window's own first-paint.
   Risk: paint of main window is delayed by the off-screen pool window's
   construction. Should measure.

2. **After main window's `on_load_end` fires:** pool fill starts ~50ms after
   the user can already see and click on the main window. No paint contention.
   Slightly more wiring (callback in `client/mod.rs` or `callbacks.rs`).

3. **Delayed task, ~500ms post first-paint:** absolute simplest, no risk of
   contention; but adds ~500ms latency before pool is ready, which matters
   if the user tears off immediately.

Recommendation: **(2) — fire from `on_load_end` of the main window**, with
a `cfg(not(target_os = "windows"))` guard. Lowest contention risk; pool is
ready ~1s after first paint.

---

## Validation plan

### Functional

1. Fresh launch of an AppImage with no warmed pool. Wait for first paint.
2. `muxlog host '[pool]'` should show `[pool] spawning pool window` twice
   (or `POOL_TARGET_SIZE` times) within ~1-2 seconds of `on_load_end`.
3. Reducer events: two `BrowserRegistered` entries with
   `kind=TopLevel { is_pool: true }`.
4. Tear off a tab. Should NOT see `pool exhausted on tear-off`. Should see
   `[pool] promote` instead, and a single `[pool] spawning pool window` for
   the refill.

### Performance

Measure end-to-end tear-off latency before and after:

- Before (current 0.33.723): `start_cross_drag → DragOverlay listening` =
  ~3.7s on cold AppImage, ~2.5s on warm cache.
- Target after fix: ~500-800ms (the time to assign the pre-warmed window's
  workspace + frontend's pool-promote handshake). Close to Windows parity.

### Regressions to watch

- **Quitting:** `spawn_pool_window` already early-outs on `is_quitting()`.
  Verify by quitting the app while pool windows are still spawning — should
  drain cleanly with `[wrr] quit_state=Draining (drain mode)`.
- **Pane-mid-close:** `H.7 invariant` gate at `window_pool.rs:111` already
  defers refill if a pane is mid-close. The startup fill must respect this.
  Trivial because we use the existing `spawn_pool_window` entry point.
- **Multiple top-level windows:** if user opens a second top-level (not via
  tear-off, e.g. a sub-window), the `on_load_end` for that window must NOT
  re-trigger the startup fill. Hence the `label == "main"` guard.

---

## Risks / open questions

- **Does `spawn_pool_window` actually self-recurse to fill to target?**
  Code path goes through `mark_pool_window_renderer_ready →
  spawn_pool_window` (window_pool.rs:401) when each pool window's renderer
  reports ready. Verify experimentally: does a single call from `on_load_end`
  end up with two pool windows in the queue, or just one? If just one, add
  an explicit loop driven by reducer's pool_queue_size.
- **Does pool spawning on Wayland have the same WeakPtr race as tear-off
  did?** The pool windows are created off-screen; if their creation overlaps
  any pending pane teardown, the same crash class might trip. The deferred-
  destroy fix (PR landed in 0.33.723) should cover this — but the startup
  fill happens before any panes have closed, so no race window exists at
  startup time.
- **Memory cost:** each pool window is a full CefWindow + Browser + Renderer
  process. Two pool windows ≈ 100-150MB RSS. Document this; may be worth
  making `POOL_TARGET_SIZE` configurable for memory-constrained users.

---

## See also

- `agentmux-cef/src/commands/window_pool.rs` — pool implementation.
- `agentmux-cef/src/saga_dispatch.rs` — Windows launcher saga path.
- `agentmux-cef/src/app.rs::AgentMuxWindowDelegate` — main window
  registration site.
- `docs/specs/linux-appimage-cold-launch-tax-2026-05-08.md` — sister spec
  for the orthogonal AppImage cold-launch issue.
