# Retro — Portable 0.49.6 Exits on Splash: Pool-Window False-Close from PR #1803

- **Date:** 2026-06-27
- **Symptom:** Portable 0.49.6 shows splash then exits immediately (host respawns in a loop, never stays up)
- **Root cause:** PR #1803's `EVENT_OBJECT_LOCATIONCHANGE` pool-move detector fires a false `report_window_closed` for every warm-pool window the moment it is positioned at `x=-20000`
- **Triggered by:** Per-build channel isolation (introduced before 0.49.5) correctly isolates the portable — so the new startup path creates a fresh cef-cache and hits the pool-window crash for the first time
- **Resolution:** Exclude `window-pool-*` labels from the close-detection filter (one-line fix in `agentmux-cef/src/wrr/win_event.rs`)

---

## 1. Timeline

| Event | Detail |
|-------|--------|
| Before 0.49.5 | Per-build channel isolation bakes a unique `AGENTMUX_BUILD_CHANNEL_DEFAULT` per `task package` run, giving each portable its own `~/.agentmux/channels/<channel>/` tree |
| 0.49.5 portable | Worked; warm-pool windows existed before `EVENT_OBJECT_LOCATIONCHANGE` detection was added |
| PR #1803 merged → 0.49.6 | Added `OFFSCREEN_POOL_THRESHOLD_X` check in the `EVENT_OBJECT_LOCATIONCHANGE` hook to detect OS-level window closes |
| User launches 0.49.6 portable | Splash appears, exits. Launcher respawns host ~4× before giving up |

---

## 2. What the isolation audit found (a false lead)

The initial hypothesis was that the portable was connecting to the running `dev:main` instance — because an earlier CLI test ran the wrong binary (`runtime/agentmux-0.49.6.exe`, which is the CEF host, not the launcher) from a bash shell that inherited `AGENTMUX_RUNTIME_MODE=dev:main`. That binary reads mode from env directly and connected to the dev:main srv pipe. This was an artifact of bypassing the launcher, not a real isolation bug.

**The isolation mechanism is working correctly.** Evidence from `~/.agentmux/channels/`:

```
local-main-b28b7a-5e53c4d7/versions/0.49.6/data/launcher-events.log  ← portable channel ✓
local-main-b28b7a-6f04d4dc/versions/0.49.5/data/launcher-events.log  ← 0.49.5 portable
dev-main-46e1cf3c9a82d88d/data/launcher-events.log                    ← dev session
```

The portable launched under `local-main-b28b7a-5e53c4d7` — its own isolated channel — never touching `dev-main-*`. The per-build channel, compile-time `AGENTMUX_BUILD_CHANNEL_DEFAULT`, and the nested guard in `agentmux-launcher/src/data_dir.rs` all worked as designed.

---

## 3. Actual bug: pool-window false closes from PR #1803

### What PR #1803 did

PR #1803 fixed **Gap A** — OS-level window closes (the user dismisses via taskbar, `Alt+F4`, etc.) that CEF's `on_before_close` never fires for because CEF recycles HWNDs into a warm pool at `x=-20000` instead of destroying them.

The fix: hook `EVENT_OBJECT_LOCATIONCHANGE`, and when an HWND moves to `x < OFFSCREEN_POOL_THRESHOLD_X = -20000` (and is not minimized), look up the label and call `report_window_closed`. Filter out `browser-pane-*` labels because floating browser panes legitimately park at `x=-20000`.

```rust
// agentmux-cef/src/wrr/win_event.rs
EVENT_OBJECT_LOCATIONCHANGE => {
    ...
    if rect.left < OFFSCREEN_POOL_THRESHOLD_X && IsIconic(hwnd) == 0 {
        if let Some(label) = state.label_for_hwnd(hwnd) {
            if !label.starts_with("browser-pane-") {   // ← only this exclusion
                crate::launcher_ipc::report_window_closed(label);
            }
        }
        return;
    }
    ...
}
```

### The missed case

Warm-pool windows (`window-pool-<uuid>`) are also positioned at `x=-20000` when they are **created and parked** in the pool. Their labels do NOT start with `browser-pane-`, so `report_window_closed("window-pool-xxx")` fires immediately when the pool window is initialized.

The gap in the filter: three categories of HWND park at `x < -20000` in the app's lifetime:
1. **Recycled real windows** after user close — the target case. Labels: `main`, `window-N`, named instances.
2. **Floating browser panes** — filtered out with `!starts_with("browser-pane-")` ✓
3. **Warm-pool windows** — **NOT filtered.** Labels: `window-pool-<uuid>` ✗

### Failure mode (confirmed from launcher-events.log)

Every time the host created a pool window:
1. `pool_window_added` logged by launcher
2. HWND registered in `AppState.window_hwnds` with label `window-pool-xxx`
3. HWND moved to `x=-20000` (pool position)
4. `EVENT_OBJECT_LOCATIONCHANGE` fires → `label_for_hwnd()` returns `window-pool-xxx`
5. `!label.starts_with("browser-pane-")` → `true` → `report_window_closed("window-pool-xxx")` called
6. Launcher receives close event → `pool_window_removed` logged
7. CEF error: `Cannot create profile at path .../browser-contexts/window-pool-xxx` (profile creation starts before the pool window is reported closed; the closure races the profile setup)
8. Host exits with code 0
9. Launcher respawns — loop

This repeats for every pool window on every startup. The host never stabilizes; the launcher retries ~4× then gives up. Splash disappears.

### Why it didn't manifest in 0.49.5 portables

PR #1803 was not in 0.49.5. The 0.49.5 portables predate the `EVENT_OBJECT_LOCATIONCHANGE` detection entirely. The dev:main session (which contains PR #1803 code) runs against a pre-existing cef-cache where pool windows already exist — the initialization HWND-move doesn't fire because the windows were created before the hook was registered in this process lifetime.

The 0.49.6 portable starts with a **fresh cef-cache** (new per-build channel, no existing pool state) — pool windows are created for the first time, triggering the LOCATIONCHANGE move detection every time.

---

## 4. Fix

**Files:** `agentmux-cef/src/state.rs`, `agentmux-cef/src/wrr/win_event.rs`

Gate on the authoritative `BrowserKind::TopLevel { is_pool: false }` flag instead of a label prefix.

Added `AppState::is_live_top_level_browser(label: &str) -> bool` to `state.rs`:

```rust
pub fn is_live_top_level_browser(&self, label: &str) -> bool {
    self.host_state
        .lock()
        .browsers
        .get(label)
        .map(|h| matches!(h.kind, BrowserKind::TopLevel { is_pool: false }))
        .unwrap_or(false)
}
```

In `win_event.rs`, replaced the prefix check with:

```rust
if state.is_live_top_level_browser(&label) {
    crate::launcher_ipc::report_window_closed(label);
}
```

**Why this is correct over the prefix approach:**

- A promoted `window-pool-xxx` window keeps its original label but acquires `is_pool: false` atomically at promotion → it IS a real user window → report fires correctly.
- A naïve `starts_with("window-pool-")` would suppress that report, leaving the launcher with a stale window count — the same regression #1803 set out to fix.
- Warm-pool HWNDs (`is_pool: true`), Floaters, Panes, and HWNDs whose browser hasn't fired `OnAfterCreated` yet all return `false` → skipped.

**Negative-filter debt eliminated:** the codebase now uses a positive filter (only `TopLevel { is_pool: false }` triggers close reports), matching the pattern already used by `count_live_user_windows` and `is_live_user_window`.

---

## 5. Invariant violated

From CLAUDE.md §Isolation invariants:

> **I3 Bounded blast radius** — a launcher failure may terminate only processes in its own job

Not violated in the blast-radius sense, but the pool-window false-close breaks the host's ability to create a CEF browser context — an implicit invariant that pool windows are not lifecycle-managed by the external close detector.

---

## 6. Open question: isolation mechanism robustness

The isolation audit found that the `AGENTMUX_RUNTIME_MODE` env var takes priority over the portable marker in `RuntimeMode::current()` (step 1 beats step 2). The nested guard in `data_dir.rs` only fires when `AGENTMUX=1` is set (i.e., when launched from inside an agent pane shell).

**Gap**: a portable launched from a terminal that inherited `AGENTMUX_RUNTIME_MODE=dev:main` but NOT `AGENTMUX=1` (e.g., a CMD window that is a non-agent child of the dev session) would mis-classify as dev:main and share its data dir with the running dev instance — exactly the CEF singleton failure seen in the flawed CLI test.

This is a **latent risk**, not the cause of the current bug. The fix from §4 unblocks the user immediately. The env-priority inversion (noted in `runtime_mode.rs:current()` step 1 vs step 2) should be addressed as a follow-up: portable marker detection should short-circuit before any env-var check.
