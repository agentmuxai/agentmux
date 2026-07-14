# SPEC: Linux Startup Blank/White-Flash — Paint-Gated Window Show

**Date:** 2026-07-13
**Author:** analysis session (agentu agent, Linux)
**Status:** Proposed — diagnosis complete, fix not yet implemented
**Baseline:** agentmux main @ `66f0ce8e`
**Related:** `docs/analysis/cef-white-flash-on-startup.md` (2026-03-31, Windows),
`docs/analysis/cef-white-flash-retro.md` (2026-04-01, Windows — the precedent this spec
extends to Linux), `docs/specs/SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01.md` (a related but
distinct Linux rendering bug — see "Not the cause" below), `docs/specs/SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_2026_07_02.md`
(sibling telemetry gap on macOS), `agentmux-launcher/src/startup_events.rs`.

---

## Problem

On Linux only, after the native launcher splash finishes and the main window appears, the
user sees: a blank/transparent window, then a white flash, then a loading indicator, then a
brief blank moment again, before the UI settles into its final state. On Windows the
equivalent startup is smooth with no flash. This is a UX regression specific to the Linux
build — the same host binary source runs on both platforms.

## Profiling data (captured 2026-07-13, fresh `task package:linux` build, v0.51.0)

The frontend already emits a `[startup-bench]` timeline into the host log
(`frontend/app-init.ts`, `frontend/util/startup-bench.ts`) — no new instrumentation was
needed to capture this. One real run:

```
   0.0ms  (start)     bootstrap-start
  +3.8ms               setupCefApi-start
  +4.9ms               initCefApi-start
+100.7ms               backend-endpoints-cached
  +2.2ms               invoke-batch-start
+389.7ms               invoke-batch-done      <- ~15 IPC get_* calls, 130-380ms EACH
  +3.3ms               setupCefApi-done
  +5.3ms               initApp-start
+1165.1ms              fonts-ready            <- single biggest chunk (document.fonts.ready)
+165.9ms               isMainWindow-start/done
= 3318.7ms total: bootstrap-start -> window "settled"
```

Reusable for future profiling: `muxlog host -i <branch|version> --grep startup-bench`
(or grep the per-channel host log directly for `\[startup-bench\]`).

Two independent findings fall out of this:

1. **A real ~3.3s frontend-perf gap** between window-create and content-settled, dominated
   by (a) a chain of local IPC round-trips that appear to run serially rather than
   overlapping (~15 calls at 130-380ms apiece for what should be sub-10ms local calls), and
   (b) a 1.16s wait on `document.fonts.ready` (capped at a 2s timeout,
   `frontend/app-init.ts:571-580`). This alone explains the "some loading" portion of the
   symptom and is worth fixing independent of the native flash (see "Follow-up" below).
2. **No telemetry currently spans host-spawn -> first real paint on any platform**
   (confirmed against `agentmux-launcher/src/startup_events.rs` and
   `docs/specs/SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_2026_07_02.md:99`, which flags the
   same gap for macOS). We can see frontend-side bootstrap timing and native splash timing,
   but nothing today confirms *when Chromium actually painted a visible frame* relative to
   when the OS window was shown. That gap is what Phase 1 below closes.

## Root cause: `on_load_end` shows the window on load-complete, not paint-complete

`agentmux-cef/src/client/navigation.rs:122-150` (function `on_load_end`, not
`#[cfg]`-forked by platform) shows the native window:

```rust
// Show window via CEF Views API after content paints.
// All windows (main + secondary) now use CEF Views.
...
if !is_pool_window {
    if let Some(bv) = browser_view_get_for_browser(browser_cloned.as_mut()) {
        if let Some(window) = bv.window() {
            if window.is_visible() == 0 {
                window.show();
                if let Some(ref mut b) = browser_cloned {
                    if let Some(host) = b.host() {
                        host.set_focus(1);
                    }
                }
            }
        }
    }
}
```

The comment claims this runs "after content paints", but `on_load_end` actually fires on
**main-frame load complete** (DOM/resources loaded), which is not the same event as "the
compositor has produced and presented a frame to the window." The same callback also writes
the cross-process splash-dismiss signal (`navigation.rs:107-120`,
`AGENTMUX_SPLASH_READY_FILE`) that the launcher's native Linux splash
(`agentmux-launcher/src/splash_linux/mod.rs`) polls for before tearing itself down. So on
Linux, both "dismiss the splash" and "show + focus the real window" are triggered from the
same load-complete event, with nothing confirming a frame was actually composited in
between.

This exact class of bug (white flash between window-show and first real paint) was already
investigated for Windows in March/April 2026
(`docs/analysis/cef-white-flash-on-startup.md`, `cef-white-flash-retro.md`) and resolved
there — the shipped fix (visible today in `agentmux-cef/src/app.rs:1207-1209` and
`agentmux-cef/src/client/lifecycle.rs:212`, "No DwmExtendFrameIntoClientArea — it causes the
white flash") was narrower than the retro's full recipe: skip the DWM surface-reset call, and
defer `show()` to `on_load_end`. That was apparently sufficient on Windows (fast GPU/D3D init,
and/or CEF's stock Windows build honoring `background_color` for the Views window quickly
enough that the gap is imperceptible). **Nothing analogous was ever verified for Linux** —
the Linux GPU/EGL/GLX init path and window-manager compositing behave differently, and nobody
re-ran the Windows testbed methodology (`docs/specs/cef-white-flash-testbed.md`) against a
Linux target. Given the same source runs on both platforms, the likeliest explanation for
"Windows smooth, Linux flashes" is that the on_load_end-timing race is simply *slower* to
close on Linux (longer GPU-process/ANGLE init, XWayland/ozone-platform selection
(`agentmux-cef/src/app.rs:799-843`) adding steps not present on Windows), making a
previously-sub-perceptual gap visible.

### Not the cause: the Track 2 per-pixel transparency bug

`docs/specs/SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01.md` and
`docs/retro/cef-linux-transparency-consolidated.md` document a real, CONFIRMED Linux bug
where Blink's `page_base_background_color_` defaults to opaque white and CEF's Linux
renderer never calls `WebViewImpl::SetBaseBackgroundColorOverrideTransparent(true)`, so
*promoted compositing layers* paint opaque white instead of blending with the desktop. That
bug is real but scoped to the "glass"/see-through window feature (Track 2, not yet shipped)
— it affects layers that are supposed to show the desktop through them. It does not explain
the startup flash: our `html`/`body` have explicit opaque dark backgrounds
(`index.html:12-19`), so they aren't relying on that override at all. This was checked and
ruled out during this investigation; worth stating explicitly so a future reader doesn't
conflate the two Linux white/transparency issues.

## Proposed plan

**Phase 1 — close the profiling gap (low risk, additive only).**
Add a "frontend painted" signal: the first `requestAnimationFrame` after the `#startup-loading`
splash SVG (`index.html:82-130`) is in the parsed DOM, sent back to the host over the existing
IPC bridge (new `getApi()` call, mirroring the pattern of `setWindowInitStatus`). Log it on the
host side alongside the existing `on_load_end` / splash-ready-file timestamps. This gives an
exact, reproducible "OS-window-visible-but-blank" duration per run instead of inference, and
fills the gap flagged in `SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_2026_07_02.md:99` for
Linux too (worth doing for both platforms in one pass).

**Phase 2 — gate window-show on that signal instead of `on_load_end` (Linux only, main fix).**
Hold `window.show()` + `host.set_focus(1)` until the Phase 1 signal arrives (with a safety-net
timeout matching the existing `MAX_GATE_MS`-style pattern in `frontend/app/store/tab-reveal.ts`,
so a stalled renderer can't leave the window permanently hidden). Same for the
`AGENTMUX_SPLASH_READY_FILE` write, so the native splash and the real window both wait for
actual paint confirmation rather than load-complete. No CEF rebuild required.

**Phase 3 — empirically test `--disable-gpu-compositing` on Linux.**
The Windows testbed (`cef-white-flash-retro.md`) found this flag "essential" — it forces
software compositing, which respects `background_color` from frame 1, eliminating the
GPU-process-startup-delay gap. It is not currently applied by default on any platform
(`agentmux-cef/src/app.rs:702-728` only sets `--disable-gpu` as a low-memory degraded rung).
Needs local A/B testing on Linux since it trades away GPU-accelerated compositing; only ship
if it measurably shrinks/removes the flash without a perceptible perf regression for
terminals/xterm/mermaid content.

**Phase 4 — frontend perf follow-up (independent of the native-flash fix).**
Investigate why the ~15 `get_*` IPC calls in the startup invoke-batch run serially at
130-380ms each instead of overlapping/batching, and whether the `document.fonts.ready` wait
(1.16s observed, 2s cap) can be shortened via `font-display` strategy or preloading. This
won't fix the native flash but will shrink the "some loading" duration the user also called
out.

## Open questions

- Does Phase 2's paint-confirmation round-trip add any perceptible additional latency to the
  Windows path (where the flash isn't currently a problem)? Should probably be Linux-gated
  behind `#[cfg(target_os = "linux")]` unless Phase 1 data shows it's cheap everywhere.
- Is the "blank" (as opposed to "white flash") part of the symptom actually the OS compositor
  showing an unpainted/transparent surface (consistent with `window:transparent=true` being
  the default in local builds — see host log line `window:transparent=true -> ozone-platform=x11`),
  as opposed to a genuinely blank window? Worth re-testing with `window:transparent=false` to
  isolate whether Track 1 (uniform alpha, `SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01.md`) interacts
  with this gap at all.
- Should Phase 3's software-compositing flag be permanent for Linux or only used as a
  diagnostic during Phase 3's A/B test?

## References (file:line)

- `agentmux-cef/src/client/navigation.rs:41-152` (`on_load_end`, window-show + splash-ready-file)
- `agentmux-cef/src/app.rs:702-728` (`--disable-gpu` degraded rung, only low-memory gated)
- `agentmux-cef/src/app.rs:799-843` (ozone-platform selection)
- `agentmux-cef/src/app.rs:1207-1209`, `agentmux-cef/src/client/lifecycle.rs:212` (no
  `DwmExtendFrameIntoClientArea` — the actual shipped Windows fix)
- `agentmux-launcher/src/splash_linux/mod.rs:224-230,341-387` (native Linux splash, ready-file poll)
- `frontend/app-init.ts:544-583` (bootstrap timing, fonts-ready wait)
- `frontend/app/store/tab-reveal.ts` (existing in-page reveal-gate pattern to mirror for the
  paint-confirmation safety timeout)
- `frontend/util/startup-bench.ts`, `index.html:82-130` (`#startup-loading` splash overlay)
- `docs/analysis/cef-white-flash-on-startup.md`, `cef-white-flash-retro.md` (Windows precedent)
- `docs/specs/SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01.md`,
  `docs/retro/cef-linux-transparency-consolidated.md` (related but distinct Linux bug, ruled out)
