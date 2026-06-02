# CEF 148 fork update + 7680→7778 customization audit

**Date:** 2026-06-02
**Repo:** `agentmuxai/cef` (fork of `chromiumembedded/cef`)
**Local CEF tree:** `~/cef-build/chromium/chromium/src/cef` @ upstream `0d9d52a65` (148.0.7778.180)
**Context:** We moved AgentMux from CEF 146 (7680) to CEF 148 (7778) for the macOS 26
work. This report covers (a) what was landed in the fork for reproducibility and
(b) an audit of the old 7680 customizations against upstream 148.

---

## (a) Fork update — DONE

### What was wrong
Our only CEF-148-specific source patch — `agentmux_process_requirement.patch`
(forces `GetPeerValidationPolicy() → kNoValidation`, the -67030 renderer fix) —
existed **only as an untracked file** in the local tree and was **not registered
in `patch/patch.cfg`**. A clean `cef_create_projects.sh` would not apply it, so
the patched-framework build was **not reproducible** off this one machine.

### What was done
- Created **`agentmux/7778-process-requirement`** in `agentmuxai/cef`, based on
  upstream `0d9d52a65` (148.0.7778.180).
- Added `patch/patches/agentmux_process_requirement.patch` and **registered it in
  `patch/patch.cfg`** (entry `'name': 'agentmux_process_requirement'`), so
  `cef_create_projects.sh` applies it automatically on a fresh checkout.
- Also created **`7778`** in the fork (upstream mirror) so the fork tracks the
  148 line. (Created via the GitHub shared-fork object network — no large push.)
- Commit message documents the **`dcheck_always_on=false`** build requirement (see
  `docs/retro/retro-macos26-cef-dcheck-root-cause-2026-06-02.md`).

### To reproduce the patched CEF 148 framework now
```
# fetch CEF 148 + the agentmux patch branch
... automate-git --branch=7778 ...
cd chromium/src/cef && git fetch <fork> agentmux/7778-process-requirement && git checkout FETCH_HEAD
./cef_create_projects.sh            # applies agentmux_process_requirement automatically
# build the framework with PRODUCTION assertion config:
GN_DEFINES="is_official_build=false is_debug=false symbol_level=1 dcheck_always_on=false"
ninja -C out/Release_GN_arm64 cef_framework
```

Note: `dcheck_always_on=false` is a **build flag, not a patch** — it does not live
in the fork. It is required (a from-source `is_official_build=false` build defaults
`dcheck_always_on=true`, and macOS-26 DCHECKs then crash on drag/close).

---

## (b) 7680 → 7778 customization audit

The fork's `agentmux/7680-drag-rightclick-and-transparency` branch (CEF 146)
carried three custom CEF features (authors: snowbark, asaf, Chad Nelson). Status
of each against upstream 7778 (CEF 148):

| 7680 customization | In upstream 7778? | AgentMux uses it? | Verdict |
|---|---|---|---|
| **`CefWindow::BeginWindowDrag()`** (native HTCLIENT-region window drag) | **NO** | **YES** — `ipc.rs:252 start_window_drag` → `ui_tasks.rs:215 begin_window_drag` | **Port if native drag is wanted** (esp. Linux) |
| **Right-click on HTCAPTION falls through to renderer** (asaf "Patch A") | **NO** (upstream `window_view.cc` returns HTCAPTION → WM handles it) | indirect (title-bar right-click menus) | Port if title-bar right-click menus are needed |
| **Transparent Views windows** (transparency cascade: RWHView/WebContents bg) | **PARTIAL** — upstream 7778 has Views transparency plumbing; basic opacity works (`set_window_transparency … opacity=0.8` confirmed in logs) | YES (opacity) | Verify fully-transparent windows; likely OK |

### The one that matters: `BeginWindowDrag`
- **Not in upstream CEF 148.** Our 7778 build is stock CEF + only the
  process_requirement patch, so `_cef_window_t.begin_window_drag` does not exist.
- **AgentMux already guards for this.** `ui_tasks.rs` checks the runtime
  `_cef_window_t` struct size and logs: *"libcef.so ABI mismatch … was not built
  from agentmux/7680-…; skipping native drag"*, then falls back (manual move loop
  on Windows; no-op otherwise). So it **degrades gracefully, but the native drag
  feature is silently absent** on a stock-148 framework.
- **macOS is currently unaffected:** the `patched-libcef` Cargo feature is
  default-off on macOS, and macOS drag uses AppKit `-webkit-app-region: drag`
  native regions (+ our `beginDraggingSession` swizzle), not `BeginWindowDrag`.
- **Linux is the regression risk:** Linux/Wayland window-move was JS-driven via
  `start_window_drag → BeginWindowDrag` on the 7680 fork. On stock 148 that path
  no-ops. If Linux ships on 148, native window drag needs the port.

### Why these were on a fork at all
Some agentmux-team CEF work was **upstreamed** into CEF 148 (good — less to
carry): `CefV8BackingStore`, `CefComponentUpdater`, and the AI-agent-oriented
`blink_ax_viewport_collapse` (CDP accessibility viewport collapse) and
`chrome_browser_task_manager` shutdown-crash fix are all in upstream 7778's
`patch.cfg` now. `BeginWindowDrag`, the HTCAPTION right-click fall-through, and
the full transparency cascade were **not** upstreamed and remain fork-only.

---

## Recommendations / decisions

1. **Done — reproducibility:** `agentmux/7778-process-requirement` lands the only
   148 source patch in a registered, reproducible form. ✅
2. **Decide on `BeginWindowDrag` for 148.** If Linux (or a future macOS native
   drag) is in scope, port the `CefWindow::BeginWindowDrag()` commits from
   `agentmux/7680-…` onto `agentmux/7778-process-requirement` (rebase the 3
   BeginWindowDrag commits; they touch `include/views/cef_window.h`,
   `window_impl.{cc,h}`, `window_view.cc`, `browser_view_impl.cc`). Then enable the
   `patched-libcef` feature for that platform. **Not needed for the current
   macOS-only goal.**
3. **Decide on right-click HTCAPTION fall-through.** Port only if title-bar
   right-click menus are a requirement; otherwise drop (one small patch).
4. **Transparency:** verify a fully-transparent Views window on 148 before
   assuming parity; basic opacity is confirmed working.
5. **Keep the fork's `7778` branch synced** to upstream as CEF tags 148.x, and
   rebase `agentmux/7778-process-requirement` on top.

### One-line answer to "do we need to update our fork for CEF 148?"
Yes — and it's now done for the mandatory part (the reproducible -67030 patch).
The optional part is a product decision: whether to forward-port `BeginWindowDrag`
(and the HTCAPTION right-click patch) from the 7680 fork, which only matters for
native window drag on non-macOS / future macOS.
