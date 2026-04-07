# DevTools: Secondary Window Investigation

**Branch:** `agenta/fix-devtools-native-window`  
**Date:** 2026-04-06  
**Status:** Primary DevTools fix merged; secondary-window DevTools not working — root causes identified

---

## What Works

- Main window DevTools: clicking the widget opens a standalone native OS window with Chrome DevTools (native title bar, close/minimize/maximize). Second click closes it.
- `RuntimeStyle::CHROME` fix prevents crash when DevTools popup window is created.

---

## What Doesn't Work

DevTools for secondary windows (Ctrl+Shift+N or tearoff) is a **noop** — nothing happens on click.

---

## Root Causes Identified

### 1. `create_isolated_request_context` fails for secondary windows

**Log evidence (v0.33.55):**
```
ERROR:cef\libcef\browser\chrome\chrome_browser_context.cc:116
Cannot create profile at path
C:\Users\area54\AppData\Roaming\ai.agentmux.cef.v0-33-55\browser-contexts\window-df50140247a34f238143ca7b6d271df2
```

**Code path:** `commands/mod.rs:create_isolated_request_context` → `request_context_create_context(settings)` where `settings.cache_path = <data_dir>/browser-contexts/<label>/`

**Root cause hypothesis:** In CEF v146 (Alloy/Views), `RequestContext` with a custom `cache_path` goes through Chrome's `chrome_browser_context.cc` even for Alloy-style browsers. Chrome's profile creation may be failing because:
- The directory is created empty by `create_dir_all` before CEF tries to initialize it, and Chrome's profile init rejects a non-virgin path
- OR CEF v146 has changed how isolated contexts work in Alloy mode and `cache_path` is no longer the right mechanism

**Impact:** `request_context_create_context` returns `None`; `CreateWindowTask` passes `None` as request context → browser is created with the **shared** main context. This means all secondary windows share state (cookies, JS heap, etc.), but the browser still opens.

**What to try:**
- Remove `create_dir_all` — let CEF create the directory itself
- Try passing `persist_session_cookies: false` and `persist_user_preferences: false` in `RequestContextSettings`
- Try NOT using isolated contexts at all (pass `None`) — if all we need is a new renderer process, `RuntimeStyle::ALLOY` with `browser_view_create` may already do that
- Check if CEF v146 requires `ChromeRuntime` flag for isolated profile support

---

### 2. `pending_window_labels` push was missing for both secondary window paths

**Fixed in this branch** (`fix(cef): push window label to pending_window_labels`).

`on_after_created` pops `pending_window_labels` to determine the browser's key in `state.browsers`. Neither `open_new_window` nor `open_window_at_position` (tearoff) was pushing the label before calling `post_create_window`. The fallback generated a random UUID, so the browser was stored under a different key than what the frontend URL contained.

**Both paths now push before posting:**
- `commands/window.rs:open_new_window`
- `commands/drag.rs:open_window_at_position`

---

### 3. Secondary window frontend logs don't appear in chrome_debug.log

All `[fe]` console logs appear with `source: http://127.0.0.1:<port>/assets/index-*.js (2)` — browser ID 2. Secondary windows would be browser ID 3+. Their console output is absent from the log, suggesting the secondary window's frontend may not be loading at all (likely a consequence of issue #1 — isolated context fails, and the fallback shared context may not have the right IPC credentials or the window may be crashing before render).

**To investigate:** Add `[dlog]` / `tracing::info!` to `on_after_created`, `on_load_end`, and `CreateWindowTask::execute` to confirm whether the secondary window browser reaches the loaded state.

---

## Suggested Next Issue

File a follow-up: `fix(cef): isolated RequestContext creation fails for secondary windows in CEF v146`

Simplest fix to try first: remove `create_dir_all` from `create_isolated_request_context` and let CEF manage the directory:
```rust
// Remove this line:
std::fs::create_dir_all(&ctx_path).ok();
```
If CEF creates it, the "Cannot create profile" error may go away.
