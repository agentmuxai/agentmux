# Linux CEF flags audit and cleanup

**Status:** Draft.
**Author:** runtime investigation 2026-05-08, prompted by user-reported UI
lag (hover latency in hamburger dropdown, slow startup) on AgentMux Linux
AppImage while VSCode (also Chromium-based) feels instant on the same VM.
**Out of scope:** macOS .app, Windows portable. Same audit may be valuable
there but the flag inventories differ.

---

## Problem

We accumulated environment variables and CEF command-line switches across
two distinct eras:

1. **Tauri / WebKitGTK era** (pre-March 2026, pre-CEF migration). Workarounds
   for WebKit's DMABUF renderer, GTK input methods, IBus quirks, etc.
2. **CEF era** (March 2026 onward). New switches for Chromium GPU init,
   subprocess limits, sandbox configuration.

When we migrated from Tauri to CEF (commit `12333fa2`), the WebKit-era env
vars in `scripts/linux-apprun.sh` were carried forward unchanged. Most of
them target a runtime that no longer exists in this codebase (WebKitGTK is
gone; we ship 100% CEF/Chromium on Linux). Some are dead (CEF doesn't read
WebKit env vars at all). Some are still active but were never reconsidered
for CEF and may be tuning CEF for the wrong target.

User-visible symptoms suggesting these stale settings now matter:

- Hover lag on dropdown menus that doesn't appear in VSCode (Electron / same
  Chromium core / same VM / same GPU).
- ~3s cold launch even on a warmed CEF cache.
- WebGL blocklisting in the GPU process despite VSCode showing accelerated
  paint on the same machine.

Goal of this audit: identify every Linux-specific flag we currently set,
classify each as *required for CEF* / *Tauri-era leftover* / *unclear, needs
test*, and propose a minimal-risk cleanup that returns us to "best CEF
defaults" with explicit overrides only where needed.

---

## Inventory

### A. AppImage `AppRun` script (`scripts/linux-apprun.sh`)

```bash
export APPDIR="$this_dir"                                          # (1)
export WEBKIT_DISABLE_DMABUF_RENDERER=1                            # (2)
if [ -n "$WAYLAND_DISPLAY" ]; then export GDK_BACKEND=wayland       # (3)
else export GDK_BACKEND=x11; fi
export XMODIFIERS=""                                                # (4)
export GTK_IM_MODULE=gtk-im-context-simple                          # (5)
bash "$this_dir/install-linux-desktop.sh" "$APPIMAGE" || true       # (6)
export LD_LIBRARY_PATH="$this_dir/usr/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"  # (7)
exec "$this_dir/usr/bin/agentmux" "$@"                              # (8)
```

| # | Var / call | Era | Effect on CEF | Verdict |
|---|------------|-----|---------------|---------|
| 1 | `APPDIR` export | both | Used by AppImage tooling + our `install-linux-desktop.sh`. | **Keep** |
| 2 | `WEBKIT_DISABLE_DMABUF_RENDERER=1` | **Tauri/WebKitGTK** | **Zero effect on CEF.** WebKit-only env var. CEF doesn't read it. | **Remove** |
| 3 | `GDK_BACKEND=wayland|x11` | both | Affects GTK widgets only. CEF uses its own Wayland/X11 path via Ozone, controlled by `--ozone-platform=...` (auto-detected by libcef). The host process does link GTK, but the relevant Linux-side window code in agentmux-cef is delegated to CEF Views, not GTK. | **Remove** (let CEF auto-detect; no GTK widgets to bias) |
| 4 | `XMODIFIERS=""` | **Tauri/WebKitGTK** | Disables IBus on GTK. Originally a WebKitGTK input-glitch workaround. CEF uses its own IME path on Linux (`InputMethodAuralinux`), not GTK's IM module. Keeping this empty potentially blocks Asian-language input on text fields (CTRL+SPACE etc.) for CEF users. | **Remove** |
| 5 | `GTK_IM_MODULE=gtk-im-context-simple` | **Tauri/WebKitGTK** | Same theme as (4). Forces GTK to a stripped IME that drops dead-keys / compose keys for international users. Irrelevant for CEF. | **Remove** |
| 6 | `install-linux-desktop.sh` invocation | CEF era | Registers `~/.local/share/applications/agentmux.desktop`. | **Keep** |
| 7 | `LD_LIBRARY_PATH` | CEF era | Required: `libcef.so` is colocated with `agentmux-cef` and the binary is built without RPATH. The dynamic linker won't find libcef without this. | **Keep** |
| 8 | `exec` | CEF era | Required boilerplate. | **Keep** |

### B. Explicit CEF switches set in code (`agentmux-cef/src/app.rs:347-388`)

| Switch | Set with | Reason given in code | Verdict |
|--------|----------|---------------------|---------|
| `--disable-features=CalculateNativeWinOcclusion` | `cmd.append_switch_with_value` | "Prevent empty browser on visibility change (CEF #3638)." | **Keep** — still relevant. |
| `--background-color=ff222222` | same | "Set initial background color via CLI." | **Keep** — UX-driven. |
| `--remote-allow-origins=*` | same | "Allow the DevTools inspector page (served from the remote debugging server) to open its own WebSocket connection back to that same server." | **Keep** — still required for our DevTools server. |
| `--renderer-process-limit=1` | same | "Cap renderer subprocesses. In Alloy mode the frontend runs in the browser process (no renderer spawned), but DevTools popups can spawn additional renderers at ~100GB VA each." | **Investigate** — see § Hot Issues below. |

### C. Switches added by libcef / cef-dll-sys (visible in `cef-debug.log` Crash keys)

These appear at runtime but are not set by our code. We get them by being a
CEF app:

```
--ozone-platform=wayland            (auto-selected by Ozone based on env)
--render-node-override=/dev/dri/renderD128   (DRI render node, auto)
--no-sandbox                        (we don't ship a setuid SUID helper)
--disable-features=...,EyeDropper   (libcef adds EyeDropper to our list)
--lang=en-US
--remote-debugging-port=9222
--log-severity=info
--user-data-dir=...
--locales-dir-path=...
--resources-dir-path=...
--browser-subprocess-path=...
--variations-seed-version
--enable-crash-reporter=,
--change-stack-guard-on-fork=enable
--metrics-shmem-handle=...
--field-trial-handle=...
--pseudonymization-salt-handle=...
--trace-process-track-uuid=...
--shared-files=...
--service-sandbox-type=...
```

These are libcef-internal plumbing. Don't touch.

### D. GPU / WebGL state (from `cef-debug.log`)

```
gpu-gl-renderer = "ANGLE (VMware Inc., SVGA3D; build: RELEASE; LLVM;,
                   OpenGL ES 3.1 Mesa 25.2.8-0ubuntu0.24.04.1)"
gpu-gl-vendor   = "Google Inc. (VMware, Inc.)"
gpu_count       = "0"
[ERROR:gpu/command_buffer/service/context_group.cc:138] WebGL1 blocklisted
```

Chromium has detected the VMware SVGA3D adapter and put it on the WebGL
blocklist. WebGL1 fails to initialize → 2D compositing falls back to a path
that may or may not be hardware-accelerated. VSCode on the same machine
uses Electron's defaults and visibly does NOT show this lag, so the
blocklist alone isn't sufficient explanation; whatever VSCode does to side-
step it (we don't yet know which switch / feature flag), we can probably
adopt.

---

## Hot issues (suspected lag contributors)

### Issue 1 — `--renderer-process-limit=1`

The comment justifies this with the Tauri/Alloy-era assumption that "the
frontend runs in the browser process (no renderer spawned), but DevTools
popups can spawn additional renderers at ~100GB VA each."

In our **current Linux CEF build**, this is wrong:

- The frontend is loaded into a top-level browser window with its own
  renderer process (not Alloy mode for the user-visible UI — verified by the
  zygote/utility/network process tree we observe).
- We have **at least 4 renderers concurrently in normal use**: main
  window + 2 pool windows + any open browser-pane (sub-renderer per pane).
- `--renderer-process-limit=1` forces ALL of them to share ONE renderer
  process. Every JS event loop competes with every other.

Hover-event handler (e.g. dropdown highlight) on the main window has to
schedule against pool-window idle JS, browser-pane idle JS, etc., on a
single thread. Hover lag is exactly what this looks like under contention.

**Verdict:** **Remove.** Default is "no cap"; let Chromium spawn one
renderer per top-level + per OOPIF as designed.

VA-space concern: Chromium is 64-bit on Linux x86-64. The "100GB VA per
renderer" cited in the comment is virtual address space, not physical
memory. On Linux 64-bit there is effectively unlimited VA and Chromium
relies on this. There is no real-world cost to letting renderers spawn.

### Issue 2 — `WEBKIT_DISABLE_DMABUF_RENDERER` confusion

The CLAUDE.md memory reads:

> `WEBKIT_DISABLE_DMABUF_RENDERER=1` is required for AppImage to work on
> this system. `window.show()` hangs without it (DMA-BUF renderer
> incompatibility)

That note was correct **for the Tauri/WebKitGTK build**. The current
CEF build's `window.show()` doesn't go through WebKit at all. The
"requirement" is folklore that needs to be retested under CEF and removed
from memory if it doesn't reproduce.

**Action:** Remove the env var from AppRun, build a fresh AppImage, launch.
Two outcomes:

1. **Window appears normally** → confirm the var was inert under CEF, drop
   it permanently, update CLAUDE.md memory to scrub the obsolete advice.
2. **Window hangs at show()** → some path in CEF 146.7.0 is incidentally
   reading this env (unlikely but possible — Chromium has hundreds of
   conditional code paths checking various WebKit-named env vars for
   compatibility), or there's a NEW required workaround. File a separate
   bug + keep the env var for now with a "WHY" comment.

### Issue 3 — IBus / IM module disable (`XMODIFIERS=""`, `GTK_IM_MODULE=...`)

CEF on Linux uses `InputMethodAuralinux`, which integrates with
`IBus` and `fcitx` directly via D-Bus, not via GTK's IM module. Disabling
GTK IM modules has zero effect on CEF text input. Empty `XMODIFIERS` was
historically a WebKitGTK input-glitch workaround.

Side effects of keeping these for CEF users:

- Asian-language users (CJK) lose IBus on the host process's GTK widgets if
  any are visible (probably none in our codebase, but worth verifying).
- Compose-key sequences (`<dead_acute>` + `e` → `é`) may not fire.

**Verdict:** Remove. If we discover a CEF-specific input bug after removal,
we'll address it with a CEF-aware workaround, not a WebKit-era one.

### Issue 4 — `GDK_BACKEND=wayland|x11`

CEF on Linux uses Ozone, which auto-detects the platform from
`WAYLAND_DISPLAY` / `DISPLAY` env vars and from `--ozone-platform=` switch
(libcef sets this for us). GDK_BACKEND influences GTK widget code only.

Our host process links GTK as a transitive dep of CEF, but our code
doesn't draw GTK widgets directly. If GDK_BACKEND has any observable
effect on our app, it's incidental to GTK init paths in Chromium itself.

**Verdict:** Remove. Reasoning: forcing the GDK backend at AppRun time is
a Tauri-era hack that biases GTK in a direction CEF may not want. Let the
defaults work, observe behavior. If something breaks, we have a real
GTK-related issue worth its own investigation.

---

## Comparison with VSCode (Electron) on the same machine

Both AgentMux and VSCode embed Chromium. Same VM, same Mesa, same
WebGL-blocklisted GPU. VSCode hover is instant; AgentMux hover lags.

What VSCode does that we don't (preliminary, needs deeper diff):

- VSCode does NOT set `--renderer-process-limit=1`. Each major UI surface
  gets its own renderer.
- VSCode does NOT set `WEBKIT_*` env vars (Electron isn't WebKit).
- VSCode does NOT touch `GTK_IM_MODULE` / `XMODIFIERS`.
- VSCode lets Electron's defaults handle Wayland / IBus / IM — modern
  Electron has tuned this stack.

This audit's recommended cleanup essentially **moves us toward VSCode's
flag profile**.

---

## Cleanup proposal (single PR)

### Touch list

1. **`scripts/linux-apprun.sh`** — replace WebKit-era env-var block with a
   minimal CEF-appropriate version:

   ```bash
   #!/usr/bin/env bash
   # AppImage AppRun for AgentMux on Linux (CEF runtime).
   # See docs/specs/linux-cef-flags-audit-2026-05-08.md for what is and
   # isn't set here, and why.
   set -e
   this_dir="$(readlink -f "$(dirname "$0")")"
   export APPDIR="$this_dir"
   if [ -n "$APPIMAGE" ] && [ -x "$this_dir/install-linux-desktop.sh" ]; then
       bash "$this_dir/install-linux-desktop.sh" "$APPIMAGE" || true
   fi
   # libcef.so + EGL/GLESv2 sit in usr/bin alongside agentmux-cef. Binary
   # is built without RPATH so we set LD_LIBRARY_PATH explicitly.
   export LD_LIBRARY_PATH="$this_dir/usr/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
   exec "$this_dir/usr/bin/agentmux" "$@"
   ```

   Removed: `WEBKIT_DISABLE_DMABUF_RENDERER`, `GDK_BACKEND`,
   `XMODIFIERS`, `GTK_IM_MODULE`. Kept: `APPDIR`, desktop registration,
   `LD_LIBRARY_PATH`, `exec`.

2. **`agentmux-cef/src/app.rs::on_before_command_line_processing`** —
   delete the `--renderer-process-limit=1` switch. Update the surrounding
   comment to reflect that we now allow per-top-level renderers because
   the user-visible Linux build is no longer Alloy-mode and the VA-space
   concern is mooted on 64-bit.

3. **CLAUDE.md memory** (`/home/snowbark/.claude/projects/-home-snowbark/memory/`)
   — scrub the "WEBKIT_DISABLE_DMABUF_RENDERER required for AppImage to
   work" entry, replace with a pointer to this spec.

### Why one PR, not four

These flags are all entangled — the diagnosis of each one only makes sense
in the context of the others. Splitting them would force the reviewer to
mentally reconstruct the full audit four times.

---

## Test plan

### Sequence

For each of the four flag removals, do an isolated test build:

| Build | What removed | Pass criteria |
|-------|--------------|---------------|
| T1 | `WEBKIT_DISABLE_DMABUF_RENDERER` only | Window appears, no `window.show()` hang |
| T2 | + `GDK_BACKEND` | Same as T1 + Wayland/X11 detection works either way |
| T3 | + `XMODIFIERS` + `GTK_IM_MODULE` | T2 passes + plain ASCII input works in any text field |
| T4 | + `--renderer-process-limit=1` removal | T3 passes + hover lag measurably reduced; pool windows have separate PIDs |

If any test fails, stop the cascade and document which flag is actually
required for CEF. (The rest of the cleanup still ships.)

### Quantitative metrics to track

- **Cold launch ms** (page-load → mainwin-done): currently ~2.4s on a warmed
  cache. Target: ≤ 1.5s after `--renderer-process-limit` removal.
- **Hover lag on hamburger dropdown**: currently visibly perceptible
  (~50-150ms), VSCode hover on same machine ~10-20ms. Target: < 30ms.
- **Process count after launch**: currently ~7 (1 host + 1 srv + 5 zygote/
  utility). After fix: + 2-3 renderer processes (one per pool window, one
  per main window).
- **Tear-off elapsed time** with full pool: reported separately in
  `linux-pool-startup-fill-2026-05-08.md`. Should improve once renderers
  aren't queued behind each other.

### Smoke checks (every build)

1. App window appears. Frontend loads.
2. Click into a text field, type ASCII — letters arrive.
3. Open the hamburger menu, hover items — repaints crisp.
4. Open a browser pane, navigate — content loads.
5. Tear off a tab — new window opens (cold path acceptable for now).
6. Close pane — host stays alive (already verified in 0.33.723 smoke).

---

## Risks

### Risk: removing `WEBKIT_DISABLE_DMABUF_RENDERER` makes window.show() hang on this exact VM

If true, the env var was inadvertently load-bearing through some
Chromium-side conditional check. Unlikely but possible. Mitigation: T1 is
the first build; if it hangs, we keep the var and document why explicitly
in a comment.

### Risk: removing `GDK_BACKEND=wayland` causes Wayland → X11 fallback

If GTK in our process incidentally creates a top-level (it shouldn't, but
some menu / dialog paths in Chromium do), the GDK-default of X11 might
apply to it. The user-visible main window is Wayland-correct because CEF's
Ozone path drives it. Worst case: a side-popup looks slightly off-position.

### Risk: removing `--renderer-process-limit=1` increases memory

Each renderer adds ~30-50MB RSS (real, not VA). With 3 windows + 1 pane,
peak could grow by ~150MB. Acceptable on modern desktops. Worth measuring
on tear-off-heavy sessions.

### Risk: input method removal breaks IBus on some specific desktop env

CEF's `InputMethodAuralinux` is supposed to handle this, but reports of
IBus regressions in Chromium-on-Linux are real. Mitigation: T3 explicitly
tests text input. If it regresses, we re-add the IBus-adjacent vars with
a "WHY" comment.

---

## Open questions

- **Why does VSCode work and we don't on this VM?** This audit *narrows*
  the question (most of our extra flags are stale and identical between
  Linux distros), but doesn't fully answer it. After the cleanup lands,
  if hover lag persists, the next thing to investigate is GPU-process
  initialization differences (Chromium has many `--use-gl=*` modes;
  VSCode/Electron may pick a different default than libcef does).
- **Does removing `--renderer-process-limit=1` introduce a CEF lifecycle
  bug we haven't seen?** The original commit message implied an Alloy-mode
  reason. We're no longer Alloy on Linux, but verify by looking at
  current `frontend` integration mode under CEF before removal.
- **Does VSCode use code caching, code splitting, or other startup
  optimizations we're not?** That's relevant to the cold-launch tax but
  separate from this flag audit.

---

## See also

- `docs/specs/linux-pool-startup-fill-2026-05-08.md` — startup pool fill
  (already merged in 0.33.724).
- `docs/specs/linux-appimage-cold-launch-tax-2026-05-08.md` — the FUSE /
  SquashFS / V8-cold-cache tax (separate fix, not this PR).
- `scripts/linux-apprun.sh` — file under review.
- `agentmux-cef/src/app.rs::on_before_command_line_processing` — file under
  review.
- `CLAUDE.md` (root) and the auto-memory file in
  `~/.claude/projects/-home-snowbark/memory/MEMORY.md` — both reference the
  removed env vars and need follow-up edits.
