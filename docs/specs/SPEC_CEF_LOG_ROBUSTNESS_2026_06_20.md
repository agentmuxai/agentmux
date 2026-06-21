# SPEC: Harden two CEF-init log errors (cache_path + debug-port bind)

**Date:** 2026-06-20
**Author:** AgentA
**Status:** Draft — ready to implement
**Found in:** the `0.46.6+gb80dc396` (Windows sandbox Phase 3, #1633) launch — `cef-debug.log` at startup
**Scope:** `agentmux-cef` host CEF/Chromium configuration. Not sandbox-related (surfaced while verifying the sandbox build, but both predate it).

---

## 0. Summary

The host's `cef-debug.log` logs two non-fatal errors at every launch. Both
degrade gracefully today (CEF continues), but each silently loses a real
capability:

| # | Error | Who it hits | Lost capability |
|---|-------|-------------|-----------------|
| 1 | `cache_path … is not a child of the root_cache_path … Defaulting to in-memory storage` | **every** user (per extra/pool/browser window) | disk-backed browser state for non-main windows — cache/cookies/storage live only in RAM, lost on close |
| 2 | `bind() returned an error: Only one usage of each socket address … (0x2740)` | **multi-instance** (AgentMux's core design) | the 2nd+ instance gets no DevTools remote-debugging server → the browser DOM API (`/agentmux/browser/*`, pane CSS queries) can't connect |

Both are config-shape bugs, not logic bugs. Fixing them makes the host robust
under its own design (multiple instances; per-window contexts).

---

## 1. Error 1 — `cache_path` is not a child of `root_cache_path`

### 1.1 Evidence
```
ERROR:cef\libcef\browser\context.cc:161] The cache_path directory
  (…\versions\0.46.6\data\browser-contexts\window-pool-<id>)
  is not a child of the root_cache_path directory (…\versions\0.46.6\cef-cache)
ERROR:cef\libcef\browser\context.cc:185] The cache_path is invalid.
  Defaulting to in-memory storage.
```

### 1.2 Root cause
- **`root_cache_path`** is set once in `agentmux-cef/src/lib.rs` (~l.672, l.715):
  `cache_dir = data_dir` where that `data_dir` is the **CEF cache dir**
  `…/versions/<ver>/cef-cache`.
- **Per-window `cache_path`** is built in
  `agentmux-cef/src/commands/mod.rs` `create_isolated_request_context()`
  (~l.46-48) from `state.version_data_dir` →
  `…/versions/<ver>/data/browser-contexts/<label>`.

`cef-cache/` and `data/` are **siblings** under `…/versions/<ver>/`. CEF
requires every `RequestContextSettings.cache_path` to be a **descendant** of
the global `Settings.root_cache_path` (a hard Chromium invariant —
`context.cc:161`). It isn't, so CEF rejects it and falls back to in-memory
storage for that window's context.

### 1.3 Impact
Every isolated browser window (pool windows, tear-offs, browser panes) runs
with an **in-memory** request context: no disk cache, session cookies, IndexedDB,
or Local Storage persistence. Cosmetic-looking, but it means browser-pane state
silently doesn't survive a window close, and the cache is rebuilt every open
(slower, more memory).

### 1.4 Fix
Root the per-window browser-contexts **under** `root_cache_path` instead of the
sibling `data` dir:

```
…/versions/<ver>/cef-cache/browser-contexts/<label>/      ← child of root ✓
```

Implementation:
1. Expose the resolved `root_cache_path` (the `cef-cache` dir) to
   `create_isolated_request_context` — either store it in `AppState` next to
   `version_data_dir` (e.g. `cef_cache_dir: Mutex<Option<String>>`, set in the
   same place `lib.rs` builds `cache_dir`), or add a `DataPaths` helper both
   sites call. Single source of truth — do **not** recompute `cef-cache` in two
   places.
2. In `create_isolated_request_context`, build
   `ctx_path = <cef_cache_dir>/browser-contexts/<label>`.
3. Keep the existing "don't pre-create the dir" rule (CEF's Chrome profile
   initializer fails on an existing-but-empty dir — `mod.rs:49`).
4. `<label>` is already sanitized for path use upstream; keep that.

### 1.5 Test / verify
- Launch; grep `cef-debug.log` → **no** `context.cc:161/185` lines.
- Open a browser pane, set a cookie / localStorage value, close + reopen the
  window → value persists (was lost before).
- The on-disk `cef-cache/browser-contexts/<label>/` dir exists after a window
  opens.

---

## 2. Error 2 — `bind()` collision on the remote-debugging port

### 2.1 Evidence
```
ERROR:net\socket\tcp_socket_win.cc:530] bind() returned an error:
  Only one usage of each socket address (protocol/network address/port)
  is normally permitted. (0x2740)
```
Observed with multiple AgentMux instances running (0.46.4 + 0.46.5-dev +
0.46.6). On a single-instance machine the preferred port is free and this does
not fire.

### 2.2 Root cause
`agentmux-cef/src/lib.rs:675`:
```rust
let debug_port: u16 = if is_dev { 9223 } else { 9222 };   // hardcoded
…
remote_debugging_port: debug_port as i32,                 // l.714
```
The remote-debugging server binds a **hardcoded** port (`9222` release / `9223`
dev). A second instance of the same channel/profile binds the same port and
fails (`WSAEADDRINUSE`, 0x2740). AgentMux is **designed** to run many instances
in parallel (different versions, dev + portable, per-build channels — see the
isolation invariants I1–I6), so a fixed port contradicts the model. (This is the
same hardcoded port that made cross-instance CDP driving impossible during the
provider-isolation verification.)

### 2.3 Impact
The instance whose bind failed has **no** CEF DevTools server. The browser DOM
API (`browser_api/*`: `/agentmux/browser/*`, the pane CSS-selector lookups in
`browser_api/resolver.rs` / `routes.rs`) connects to
`ws://127.0.0.1:<debug_port>/devtools/page/<target>` — which doesn't exist for
that instance → browser-pane introspection / DOM queries silently break for the
2nd+ instance. `state.debug_port` still holds the (unbound) hardcoded value, so
the failure is invisible until a DOM-API call times out.

### 2.4 Fix
Pick a **free** port instead of a fixed one, and record the *actual* port:

1. Try the preferred port first (`9223` dev / `9222` release) for muscle-memory
   convenience: bind-test `TcpListener::bind(("127.0.0.1", preferred))`. If it
   binds, use `preferred`; drop the listener immediately so CEF can take it.
2. If the preferred port is taken, fall back to an OS-assigned free port:
   `TcpListener::bind(("127.0.0.1", 0))` → read `.local_addr().port()` → drop.
3. Set `remote_debugging_port` to the chosen port **and** store it in
   `app_state.debug_port` (already the value `browser_api` reads — so the DOM
   API automatically targets the right port).
4. Accept the small TOCTOU window between the bind-test and CEF's own bind
   (another process could grab the port in that gap). Mitigate by doing the
   probe immediately before constructing `Settings` and logging the chosen port;
   if CEF still can't bind, the DOM API degrades exactly as today (no worse).

Note: CEF/Chromium does **not** reliably auto-assign on `remote_debugging_port =
0`, so we must choose the concrete port ourselves (hence the bind-probe).

### 2.5 Test / verify
- Launch **two** instances; grep both `cef-debug.log` → neither logs the
  `tcp_socket_win.cc:530 bind()` error.
- `state.debug_port` differs between the two instances; `GET
  http://127.0.0.1:<port>/json` succeeds for **each** (DevTools server up).
- A browser-pane DOM query (`/agentmux/browser/*`) works in the **second**
  instance (was broken before).

---

## 3. Non-goals / notes

- The `[getApi] called before window.api exists` console line is a benign
  frontend startup-order log (window.api injects a tick later) — not in scope.
- These are independent fixes; either can land alone. Bundle them in one PR
  (both are `agentmux-cef` CEF-config robustness) with a `patch` changeset.
- Neither changes the sandbox; they were merely surfaced by the Phase 3 launch.

## 4. Change surface
- `agentmux-cef/src/lib.rs` — §1.4 (resolve + expose `root_cache_path`),
  §2.4 (free-port selection for `remote_debugging_port`, store in
  `app_state.debug_port`).
- `agentmux-cef/src/commands/mod.rs` — §1.4 (`create_isolated_request_context`
  roots `browser-contexts` under the cef-cache dir).
- `agentmux-cef/src/state.rs` — if option (a): add `cef_cache_dir` field beside
  `version_data_dir` / `debug_port`.
