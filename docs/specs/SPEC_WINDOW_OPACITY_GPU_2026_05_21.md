# SPEC — Window transparency: one cross-platform problem (GPU-composited Chromium)

**Status:** Diagnosis complete · fix direction for decision
**Date:** 2026-05-21 (rev. 2 — widened from Windows-only after reviewing the
transparency history)
**Author:** AgentA
**Supersedes:** the withdrawn `SPEC_WINDOW_OPACITY_SYSTEM_2026_05_21.md`.
**Area:** `agentmux-cef/src/commands/window.rs`, the CEF host launch path,
the `a5af/cef` fork, `frontend` CSS transparency.

---

## 1. Summary

Per-window / window transparency **does not fully work on any platform**, and
the Windows breakage and the long Linux/Wayland struggle are **the same
problem**: transparency for a **GPU-composited Chromium window** is ignored by
whatever layer the GPU compositor draws past.

- **Windows** — the per-window slider (PR #868) uses Win32 `WS_EX_LAYERED` +
  `SetLayeredWindowAttributes(LWA_ALPHA)`. Chromium's GPU process presents via
  **DirectComposition**, which bypasses the window redirection bitmap that
  `LWA_ALPHA` controls. The call "succeeds"; the window stays opaque. It works
  *only* with `--disable-gpu`.
- **Linux/Wayland** — the team made the CEF *window* transparent (CSS
  `--window-opacity` + `a5af/cef` fork patches), but the renderer **still
  rasterizes pane interiors opaque** — borders/gaps bleed the wallpaper,
  panes don't. Traced in the `cef-transparency` retros, never resolved.

It is one wall, hit from two sides. The fix is not a Win32 trick — it is in
the **renderer/compositor (the CEF fork)**.

---

## 2. The three mechanisms in the codebase

| # | Mechanism | How | Platform | Status |
|---|---|---|---|---|
| A | **CSS `--window-opacity`** | `:root` var → `--main-bg-color: rgba(34,34,34,var(--window-opacity))` → panes inherit a translucent bg. Renderer-layer alpha — no GPU dependency. | all | The *correct layer*. Works only if the CEF window itself is transparent (see B). On Windows the CEF browser bg is opaque black, so CSS alpha just darkens — no see-through. |
| B | **CEF transparent window** (`a5af/cef` fork) | Fork patches make the browser/window background `SK_ColorTRANSPARENT` and propagate it down the Views → WebContents → `cc::LayerTreeHost` cascade. | Linux/Wayland (patched `libcef.so`) | **Partial.** Window borders bleed through; pane interiors still rasterize opaque — suspected `cc::RasterSource` / `PictureLayer` tile-cache opaque-clear bug (`session-3` retro). |
| C | **Win32 `SetLayeredWindowAttributes`** (PR #868) | `set_window_opacity` → `apply_window_opacity` → `WS_EX_LAYERED` + `LWA_ALPHA` on the host HWND. | Windows only | **Broken with GPU** (DirectComposition bypass). Works only `--disable-gpu`. |

A + B are the real architecture (`docs/specs/cef-transparency-architecture.md`).
C is a separate, Windows-only attempt added by #868 that sidesteps A/B — and is
a dead end.

---

## 3. Root cause

GPU-accelerated Chromium composites window content on a path the host's
transparency mechanism never reaches:

- **Windows:** `LWA_ALPHA` sets the alpha of the window's *redirection
  bitmap*. A GPU Chromium window renders into a **DirectComposition** visual
  that DWM composites directly — the redirection bitmap is unused, so the
  layered alpha is irrelevant. (`--disable-gpu` → software render into the
  redirection bitmap → `LWA_ALPHA` works again.)
- **Linux:** making the window/surface ARGB and the browser background
  transparent is necessary but **not sufficient** — the renderer's raster
  pipeline still fills pane-interior tiles with an opaque clear colour
  (`contents_opaque` / `RasterSource::ClearForOpaqueRaster` path). The
  compositor faithfully shows opaque tiles.

Both are "the GPU compositor honours what *it* rasterized, not the alpha you
set on an outer layer." Real transparency must be produced **inside the
renderer/compositor** — which is why it has to live in the CEF fork.

---

## 4. Evidence

**Windows (this investigation):**
1. 0.37.8 host log — ~40 `Applied window opacity: 0.X (alpha=N)` during one
   drag; `SetLayeredWindowAttributes` returned success every time.
2. Window enumeration — a real visible 920×800 app window carried
   `WS_EX_LAYERED` + layered alpha 127; still fully opaque on screen.
3. Launcher log — every instance where opacity "worked" (v0.37.1, dev v0.37.6)
   had crashed and been relaunched `--disable-gpu` by the supervisor.
4. Controlled test — 0.37.8 relaunched `--disable-gpu` (same build, GPU the
   only variable): transparency worked. Confirmed by the user.

**Linux (prior history):** `docs/retros/cef-transparency-session-2-2026-05-11.md`
and `…-empirical-…` — after the fork's transparency-cascade patches,
*"pane interiors still render opaque rgb(34,34,34) … window borders/gaps
continue to show wallpaper bleed-through."* Root cause located in the
`cc` raster pipeline; Session 3 ended unresolved.

---

## 5. The real fix — finish it in the renderer (the CEF fork)

The `a5af/cef` fork already carries the transparency cascade
(commits `68e0dc668` "propagate to WebContents", `3e041ad2f` "deferred
top-level transparent bg") plus Chromium-mirror edits to `web_view_impl.cc`
and `content_layer_client_impl.cc`. It is **~80% done and stalled on one
issue**: pane-interior tiles rasterize opaque.

The fix direction:

1. **Resolve the opaque-raster bug in the fork.** Make per-layer rasterization
   honour `contents_opaque=false` for the pane-background layers (the
   `RasterSource` / `PictureLayer` tile-cache path the Session-3 retro
   identified). This is the one remaining blocker for B.
2. **Apply B to Windows too.** The fork's transparency-cascade work was
   built/shipped for Linux (`libcef.so` in the AppImage). The same patched
   `libcef.dll` + transparent CEF browser window must be produced for Windows.
   With a transparent CEF window, CSS mechanism A then yields true see-through
   on Windows — **GPU stays on**, because the alpha is produced in the
   renderer, not via `LWA_ALPHA`.
3. **Retire mechanism C.** Once A+B work, delete `set_window_opacity` /
   `apply_window_opacity` / the `SetLayeredWindowAttributes` path. It is a
   Windows-only dead end that only ever worked on crashed (`--disable-gpu`)
   instances.
4. The per-window opacity **slider** then drives the CSS `--window-opacity`
   for that window (mechanism A) — no Win32 IPC at all.

This fixes Windows and Linux with one body of work, keeps GPU acceleration,
and removes the broken path.

---

## 6. Interim Windows-only workaround (only if a stopgap is needed)

If a shippable Windows result is needed before the fork work lands:

- **`--disable-direct-composition`** — keeps GPU raster, routes presentation
  through the redirection bitmap so `LWA_ALPHA` (mechanism C) works. Cheaper
  than `--disable-gpu`. Untested — would need confirmation.
- **`--disable-gpu`** — proven to work; full software-compositing perf cost.

Both are process-wide flags and Windows-only. Treat as a stopgap, not the
fix — they keep the dead-end mechanism C alive.

---

## 7. Red herrings — do not re-chase

- **PR #947** (slider `%` value display) and the session's store-reactive redo
  — frontend-only, unrelated. The `%`-display work (PR #954) is correct and
  can merge independently.
- **An "HWND-capture race"** — the host does capture an HWND and apply to a
  real visible window. Not the cause.

The single variable was always GPU compositing.

---

## 8. Recommendation

1. **Treat this as one cross-platform transparency task**, owned in the
   `a5af/cef` fork — resume the stalled `agentmux/7680` transparency cascade
   and resolve the opaque-raster bug (§5.1).
2. Produce the patched **Windows `libcef.dll`** alongside the Linux
   `libcef.so` (§5.2).
3. **Retire mechanism C** once A+B land (§5.3).
4. Do **not** ship the current `SetLayeredWindowAttributes` slider as a
   feature — it silently no-ops on every healthy GPU build.
5. Use a §6 flag only as an explicit, labelled stopgap if one is needed.

---

## 9. Verification

Transparency is fixed only when, on a **healthy GPU-on** build (no
`--disable-gpu`), across 5+ fresh launches: the window — **including pane
interiors, not just borders** — visibly shows the desktop through it at the
set opacity. The Win32 API "succeeding" is not evidence — that was the whole
bug.

---

## 10. Source history

- `docs/specs/cef-transparency-architecture.md` — intended A+B design.
- `docs/research/cef-transparency-research-2026-05-10.md`,
  `docs/retros/cef-transparency-empirical-2026-05-11.md`,
  `docs/retros/cef-transparency-session-2-2026-05-11.md` — the Linux attempts
  and the unresolved opaque-raster finding.
- `docs/analysis/opacity-inconsistency.md`, `specs/SPEC_WINDOW_TRANSPARENCY.md`.
- `scripts/resolve-cef-runtime.sh` — how the patched `libcef` is bundled.
- `docs/analysis/CRASH_GPU_PROCESS_FATAL_2026_05_20.md` — the GPU crashes that
  masked this bug (separate problem).
