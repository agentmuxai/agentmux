# SPEC: window_hwnds cache stale-HWND fix

**Date:** 2026-05-28
**Author:** AgentA
**Status:** Implementation — fix for a stale-HWND bug breaking the title-bar close button.
**Spec scope:** small fix; doc bundled with code per CLAUDE.md no-doc-only-PRs rule.

---

## The bug

The title-bar close button stops working after the main CEF window has been recreated within a single host process. Clicking the X looks fine — frontend dispatches, the backend `close_window` handler runs — but the window stays alive and nothing logs after the dispatch.

**Reproduction:** observed on v0.39.1 portable. Live diagnostics:

- Host log: `[win-resolve] resolved via window_hwnds cache` → `cache_hwnd=0x2b03e4`, posts `WM_CLOSE` (twice on the user's two clicks, ~158ms apart).
- Live process state: actual main window HWND is `0x1404f0` (PID 3404).
- The two HWNDs do not match. `PostMessage(0x2b03e4, WM_CLOSE)` posts into a dead window. `on_before_close` never fires.

## Root cause

`agentmux-cef/src/commands/window.rs`:

1. **`resolve_window_hwnd` (line 235)** consults `state.window_hwnds` first. On a cache hit it returns the cached value with **no liveness check** — comment explicitly says "we MUST NOT walk `GA_ROOT` on cache hits."
2. **`capture_hwnd_for_label` (line 1320)** is the only writer for non-floater labels. It has an "already registered, preserving" early-out (line 1331) so it can't re-resolve a stale entry.
3. **There is no eviction path.** `state.window_hwnds.lock().remove(...)` is grep-empty across the whole repo. CEF Views can swap the main window's outer HWND on re-init (host pool promotion, multi-window tear-off, internal Views shutdown/restore), and the cache outlives the swap.

Combined: stale entries persist forever, so any `WM_CLOSE` / `WM_SETOPACITY` / `WM_CLOSE_BY_LABEL` etc. routed through the cache silently sends to a dead window.

## Fix

Two complementary changes, both small:

### 1. Defensive read in `resolve_window_hwnd`

Wrap the cache hit in `IsWindow(hwnd)`. If false: evict the stale entry, log at WARN, fall through to the reducer-registry → `EnumWindows` fallback.

```rust
let cached = state.window_hwnds.lock().get(label).copied();
if let Some(raw_isize) = cached {
    let raw = raw_isize as *mut std::ffi::c_void;
    if !raw.is_null() {
        // Validate liveness — CEF Views can swap the outer HWND on
        // window recreate, and the cache has no eviction path.
        if IsWindow(raw) != 0 {
            tracing::info!(
                target: "win-resolve",
                label = %label,
                cache_hwnd = ?raw,
                "[win-resolve] resolved via window_hwnds cache"
            );
            return raw;
        }
        tracing::warn!(
            target: "win-resolve",
            label = %label,
            stale_hwnd = ?raw,
            "[win-resolve] cache hit was stale (IsWindow=false); evicting"
        );
        state.window_hwnds.lock().remove(label);
    }
}
```

### 2. Cleanup on `on_before_close`

In `agentmux-cef/src/client/mod.rs::on_before_close`, after the existing handler logic, remove this label's entry from `window_hwnds` so a subsequent open of the same label re-captures cleanly.

```rust
fn on_before_close(&mut self, browser: Option<&mut Browser>) {
    // … existing trace + browser_list bookkeeping …

    if let Some(label) = /* derive label from browser */ {
        let removed = self.state.window_hwnds.lock().remove(&label);
        if removed.is_some() {
            tracing::debug!(
                target: "win-resolve",
                label = %label,
                "[win-resolve] evicted on close"
            );
        }
    }
    // … existing on_before_close_browser_pane forwarding …
}
```

The label-from-browser derivation already exists in the surrounding code (used by the `browser_list` index); reuse whatever path the existing handler takes.

### Belt + suspenders

#1 catches stale entries even when `on_before_close` doesn't fire (process crash without OnBeforeClose, CEF lifecycle quirks). #2 keeps the cache clean during normal closes so #1 rarely activates. Both ship together — the fix surfaces too few times to warrant landing them serially.

## Out of scope

- The floating-pane outer HWND is registered directly in `floating_pane.rs::create_owned_popup` — not via `capture_hwnd_for_label`. Its lifecycle is separate (popup destruction is owner-cascade) and is not implicated in the bug surface. No change.
- `find_own_top_level_window` and `find_main_window` are unaffected — they're fallback paths, never the cache.

## Test plan

- [x] Manual smoke on a fresh portable: open main, dock a tab into a tear-off, close tear-off, close main via title-bar X. Window goes away on first click.
- [x] `cargo test -p agentmux-cef --release window` — no regression on the window-command tests.
- [ ] Reagent + codex.

## Why no automated test

The bug only surfaces when CEF Views recreates the outer HWND, which our test harness can't reproduce without a real Win32 message loop. The fix is small and defensive; the hot-path overhead (`IsWindow` is a syscall, ~hundreds of nanoseconds) is negligible vs. the IPC round-trip we're inside.

## References

- `agentmux-cef/src/commands/window.rs:235` — `resolve_window_hwnd`
- `agentmux-cef/src/commands/window.rs:1320` — `capture_hwnd_for_label`
- `agentmux-cef/src/client/mod.rs:637` — `on_before_close`
- Host log evidence: stale-HWND PostMessage at `2026-05-28T17:16:17.366Z`.
