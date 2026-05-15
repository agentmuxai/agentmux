# CEF Wayland transparency — consolidated research + plan

**Date:** 2026-05-10
**Status:** Research consolidated; implementation not started.
**Driving question:** Make the AgentMux main window transparent on Linux/Wayland so the CSS layer's `rgba(_,_,_,0.5)` body background composites with the desktop instead of clamping to opaque black.
**Origin:** Six weeks of intermittent work scattered across specs, retros, archive handoffs, and one local CEF source build. This doc reconciles what's confirmed vs. what's folklore vs. what's untried.

---

## TL;DR

CSS-side transparency works. Patched `libcef.so` exists, claims to include Chad Nelson's `SetBackgroundOpaque(false)` plumbing, and now ships in the AppImage via PR #743. The IPC handler `set_window_transparency` is wired through to the host on every platform — but the **Linux branch of the handler is a literal no-op**. One unverified gap stands between "doesn't work" and "might just work."

---

## What's confirmed working today

### CSS layer (pure frontend, no IPC)

- `xterm` theme background set to `#00000000` (`frontend/util/termutil.ts`).
- Block container background set to a semi-transparent theme color (e.g., `#1e1e1e80`) via the `blockBg` memo in `termViewModel.ts`.
- AppBackground div renders the tab wallpaper / gradient over an alpha-aware base.
- Body background sits at `rgba(34,34,34, var(--window-opacity))`. With `--window-opacity` unset, that resolves opaque; with it set, partially transparent.

### Windows transparency end-to-end

- `agentmux-cef/src/commands/window.rs::set_window_transparency` iterates `state.browsers`, finds each top-level HWND via `GetAncestor(GA_ROOT)`, applies `WS_EX_LAYERED` + `SetLayeredWindowAttributes` for alpha, applies `DwmSetWindowAttribute` for Mica/Acrylic backdrop. Both apply and remove paths exist.

### CEF build artifacts on disk

- `~/cef-build/chromium_git/chromium/src/out/Release_GN_x64/libcef.so`, ~613 MB stripped, BuildID `b22aa49bc6b17bfa`. Built 2026-05-01.
- Source fork: `github.com/a5af/cef`, branch `agentmux/7680-drag-rightclick-and-transparency`, HEAD `5ab41b6`.
- Two patches on top of upstream CEF 7680:
  1. `CefWindow::BeginWindowDrag()` — **verified working in production** (PR #663, drag retro).
  2. Transparency broadening (claimed to bring Chad Nelson's `SetBackgroundOpaque(false)` IPC support into views-hosted browsers) — **not exercised yet**.

### Distribution

- PR #743's `scripts/resolve-cef-runtime.sh` finds the patched libcef.so and ships it inside the AppImage. As of 0.33.761 the AppImage on Desktop carries the patched binary.

---

## What's confirmed NOT working

- Linux/Wayland window transparency: the CEF window paints opaque ~black (`0xFF222222` from our `--background-color=ff222222` switch in `app.rs::on_before_command_line_processing`) regardless of CSS layer alpha. The desktop is never visible through the window.
- The Linux branch of `set_window_transparency` is literally:
  ```rust
  #[cfg(not(target_os = "windows"))]
  let _ = (transparent, opacity);
  ```
  It accepts the IPC and discards both arguments.

### Why (per the 35-day-old root-cause memory; not freshly verified)

The renderer's `cc::LayerTreeHost` defaults to `has_transparent_background = false`. With that flag false, the compositor clamps all per-pixel alpha to 1.0 in the final Wayland surface buffer. Reaching `has_transparent_background = true` requires the IPC chain `Browser::SetBackgroundColor(0) → RenderWidgetHostViewBase::SetBackgroundColor(0) → SetBackgroundOpaque(false)`.

The 35-day-old memory cites CEF PR #4086 (unmerged upstream) as the patch that adds this. The local fork's transparency commit claims to be the same plumbing rebased onto 7680, but **the call from AgentMux to trigger the renderer-side change is never issued**.

---

## Tried and failed (with reason, sourced from the docs)

| What was tried | Outcome | Why it failed |
|----------------|---------|---------------|
| CSS-only transparency on CEF (no window-level changes) | Showed AgentMux's dark body bg, not the desktop | CEF Views window background is hardcoded opaque before any CSS paints |
| Widget-layer opacity patches (`kOpaque → kTranslucent`) | Cited as ✓ in the root-cause memory | Necessary but not sufficient — Views surface format change without renderer alpha is invisible |
| LD_PRELOAD shim to NULL the opaque region passed to Wayland | Cited as ✓ in the root-cause memory | Necessary but not sufficient — same reason; renderer still emits RGB |
| Verifying renderer CAN emit alpha via CDP screenshot | ✓ | Proves the renderer is *capable*; doesn't make the surface ARGB |
| Building patched `libcef.so` from `a5af/cef` `agentmux/7680-...` (2026-05-01) | ✓ Built (`5ab41b6`) | Build succeeded; **no integration test from AgentMux side** |
| Bundling the patched libcef in the AppImage (PR #743, 2026-05-08) | ✓ Shipping | Resolver picks it up, AppImage contains it; **still no integration test** |
| `windowless_rendering_enabled` (off-screen rendering) | Not attempted | Documented (`cef-transparency-architecture.md` Option C) as a complete architectural rewrite. Out of scope. |
| `wl_subsurface` embedding (upstream CEF #2804) | Not attempted | Not implemented upstream; even if implemented, lacks Hide/Show/SetBounds (`cef-pane-research-2026-05-03.md`) |

---

## The single unverified gap

We don't actually know:
1. Whether the transparency commit on `agentmux/7680-...` is *exactly* Chad Nelson's PR #4086, *rebased and adjusted* for 7680, or something else entirely.
2. Whether the patched libcef.so exposes a stable C API for AgentMux to call (e.g., a new `cef_browser_host_t` method, or just a `CefBrowserSettings::background_color` field that takes alpha values).
3. Whether `Browser::SetBackgroundColor(0)` via the existing cef-dll-sys bindings actually reaches the renderer (the bindings were not regenerated for the patched libcef — same situation as `BeginWindowDrag` before its raw-FFI override).

Until (1)–(3) are answered, every minute spent designing the AgentMux-side wiring is gambling.

---

## Proposed plan (in order, each step has its own pass/fail)

### Step 1 — Read the patch

**Action:** Read the actual transparency commit in `~/cef-build/chromium_git/cef/` (and the mirrored copy at `chromium_git/chromium/src/cef/` if they've drifted). Identify:
- Which CEF API surface it adds (new method? new setting field? new feature flag?).
- Whether the C-side header `include/capi/...` has a new entry (cef-dll-sys would expose it) or if it's renderer-internal only (we'd need a different trigger).
- Whether there's a CLI switch / `--enable-features=...` toggle required at startup.

**Pass:** Single concrete API name + call site identified.
**Fail:** Patch is incomplete or doesn't expose a public surface → block on rebuilding libcef with the proper PR #4086.
**Time budget:** 30 min.

### Step 2 — Verify the API is callable from AgentMux

**Action:** Check whether the identified API is in cef-dll-sys's generated bindings. If yes, use directly. If no, follow the `BeginWindowDrag` precedent: raw FFI override + a one-file patch to cef-dll-sys (see memory `agentmux_drag_fix.md`).

**Pass:** A Rust call site compiles and links against the patched libcef.so.
**Fail:** Bindings are missing AND can't be raw-FFI'd (e.g., the API is C++-only). → block on a different transparency approach.
**Time budget:** 30 min.

### Step 3 — Wire the Linux branch of `set_window_transparency`

**Action:** Replace the no-op in `agentmux-cef/src/commands/window.rs` with:
- Iterate `state.browsers` (mirror Windows path).
- For the main browser(s), post a UI-thread task that calls the API identified in Step 1.
- For `transparent = false`, call the inverse (set background to the default opaque dark).

**Pass:** Code compiles. IPC fires when the user toggles transparency in settings. The host log shows the API call landing on the UI thread.
**Fail:** Compile error (Step 2 wrong) or no IPC fires (Step 1 wrong).
**Time budget:** 1 hour.

### Step 4 — Test end-to-end

**Action:** Run the AppImage. Enable transparency in settings. Look at the screen.

**Pass:** Desktop visible through the AgentMux window. CSS alpha layers actually composite with the desktop wallpaper.
**Fail (case A — opaque):** Renderer didn't switch to ARGB. Capture the Wayland surface buffer (`weston-screenshot`, `wlrctl`, or compositor-debug) to confirm format. Likely cause: patched libcef.so doesn't actually plumb the IPC; rebuild with a known-good PR #4086 application.
**Fail (case B — black/white flash):** Views layer paints opaque before renderer takes over. Documented in `cef-white-flash-on-startup.md`; needs an additional widget-layer patch (`kOpaque → kTranslucent` on the root NonClientView).
**Time budget:** 30 min for the happy path, multi-hour debug if it fails.

### Step 5 — Retro / spec

**Action:** Write a retro doc capturing what worked, what didn't, what additional CEF patches were needed, and the verified API surface. Replace the 35-day-old memory entry.

---

## Risks and known gotchas

- **`tools/patcher.py` is NOT idempotent** (memory: `cef_build_in_progress.md`). If we need to amend the transparency patch, always `git reset --hard HEAD && find . -name '*.rej' -delete` first.
- **CEF rebuild cost:** 3–6 hours wall-clock on a 32-core box if we have to amend the transparency patch.
- **OOM avoidance:** `ninja -j 12 -l 16` under `systemd-run --user --scope` (memory: `cef_build_in_progress.md`). Default `-j` cascades into reboots.
- **DWM equivalent on Wayland:** there is no Wayland-side "Mica / Acrylic" effect. The closest is a compositor-side blur (KWin / Hyprland) requested via `--enable-features=WaylandBlur` or similar; we'd be at the mercy of each compositor. Mutter (GNOME) does not implement client-requested blur.
- **The cefclient sample app's transparency demo was disabled in our rebase** (memory: `cef_build_in_progress.md` — `use_transparent_painting` flag in `main_context_impl.cc`). If we want a smoke test that ISN'T AgentMux, we have to re-enable that flag in a follow-up commit on the fork.
- **`--background-color=ff222222`** is set unconditionally in `agentmux-cef/src/app.rs::on_before_command_line_processing`. If the renderer reads this *before* the IPC-driven alpha switch fires, it may paint opaque dark on first frame regardless. Likely needs to change to `00000000` when transparency is enabled — or be removed and let CSS dictate.

---

## Files cited (verify before quoting)

- `agentmux-cef/src/commands/window.rs` — `set_window_transparency` handler (Linux no-op currently).
- `agentmux-cef/src/app.rs` — `on_before_command_line_processing` (where `--background-color=ff222222` is set).
- `frontend/app/app.tsx::AppSettingsUpdater` — sends the IPC.
- `frontend/util/cef-api.ts` — IPC wrapper.
- `scripts/resolve-cef-runtime.sh` — selects the patched libcef.so for bundling.
- `docs/cef-build/build-patched-libcef.md` — how to rebuild if Step 1 reveals a missing piece.
- `docs/specs/cef-transparency-architecture.md` — 2026-03-29 Tauri-era spec; useful for the CSS-layer diagram but most "current state" claims are stale.
- `docs/analysis/opacity-inconsistency.md` — Windows-side fix already shipped; not directly relevant to Wayland.
- `docs/research/cef-pane-research-2026-05-03.md` — cites the Wayland subsurface limitations.
- Memory `cef_transparency_root_cause.md` (35 days old) — IPC chain documented; not re-verified against current libcef.so.
- Memory `cef_build_in_progress.md` (9 days old) — build infrastructure + gotchas; libcef.so location + branch verified.

---

## Open questions to resolve before any AgentMux-side code changes

1. **What API does the patched libcef.so expose?** Step 1 above.
2. **Is the CLI switch `--background-color=ff222222` going to fight us?** Need to read the call order in libcef; possibly route the value through the IPC too so transparency=on switches it to `0x00000000`.
3. **Does the patch require a feature flag?** Read the patch's `--enable-features=` annotations if any.
4. **What's the cefclient demo's status post-rebase?** Per memory, the demo flag was dropped during the rebase fixup; re-enabling it gives us a smoke test that's not AgentMux.
5. **Compositor-side considerations:** Mutter vs. KWin vs. Hyprland may differ in how they handle ARGB surfaces. The user's machine is Mutter. Document the spectrum, scope to Mutter for the initial validation.
