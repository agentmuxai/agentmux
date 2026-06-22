# SPEC: Splash user-info footer + "disable splash" setting

**Date:** 2026-06-21
**Author:** asaf (via Claude Code)
**Status:** Draft
**Related:**
- `SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20.md` (Linux Wayland/X11 backends)
- `SPEC_LINUX_SPLASH_POLISH_2026_06_20.md` (X11 backend)
- Win32 splash `agentmux-launcher/src/splash.rs`; macOS splash `agentmux-launcher/src/splash_mac.rs`

---

## 1. Goals

Two user-facing changes to the **startup splash** (the pulsing brain the launcher
paints before CEF is up), consistent **across Linux, macOS, and Windows**:

1. **Identity footer.** Show, well-placed near the **bottom** of the splash card:
   - **username** (the OS user)
   - **host** (machine hostname)
   - **version** (`0.47.2`)
   - **dev label** when the build is not a stable release (dev / channel / build label)
2. **Disable toggle.** A **setting to turn the splash off entirely** — no splash
   window is created at all when disabled.

Non-goals: redesigning the brain/pulse/fade animation; theming; localization;
showing live/runtime data (the footer is static build/host identity only).

---

## 2. Current state (per platform)

The splash lives in the **launcher** (`agentmux-launcher`) — the only process alive
before the multi-second CEF host load. It currently renders **only a pre-baked brain
image** on a dark rounded card; there is **no text rendering** anywhere in the splash.

| Platform | File | Mechanism | Text today |
|---|---|---|---|
| Linux (Wayland) | `splash_linux/wayland.rs` | `wl_shm` ARGB8888 software buffer, `render_frame` (`splash_linux/mod.rs`) | none |
| Linux (X11/XWayland) | `splash_linux/x11.rs` | override-redirect window, software `PutImage`, same `render_frame` | none |
| macOS | `splash_mac.rs` | Native AppKit — `NSImageView` (brain) over a layer-backed `NSView` backdrop | none (native, trivial to add) |
| Windows | `splash.rs` | Win32 layered window, DIB pixel buffer + `composite()` | none |

Shared assets are **pre-baked by `agentmux-launcher/build.rs`** from `resources/brain.png`
into `brain_rgba.bin` / `brain_bgra.bin` + `brain_dims.rs`. The card size today is
`brain + padding` (a square, e.g. Linux `BRAIN_* + PADDING*2`; mac `220×220`; Win32 a
single square `SPLASH_SIZE` used for **both** window and DIB, `splash.rs:132,159–161`).

**Dismiss/spawn model differs by platform — load-bearing for the disable toggle (§6):**
| | Spawn site | Dismiss signal |
|---|---|---|
| Linux | `main()` → `splash_linux::spawn()` (`main.rs:120`) | `AGENTMUX_SPLASH_READY_FILE` (host writes on first paint) |
| macOS | `main()` → `Splash::show()` (`main.rs:87`, main thread) | `AGENTMUX_SPLASH_READY_FILE` |
| Windows | **inside `launcher_main`** → `spawn_splash(dir_hash)` (needs `dir_hash`) | **named event** `AgentMuxSplash-<dir_hash>` (`CreateEventW`, signaled on first paint) — **no ready-file** |

**Data the launcher already has / can get cheaply:**
- version → `env!("CARGO_PKG_VERSION")` (used in `main.rs:714`, `srv_spawner.rs:113`).
- channel / dev → `AGENTMUX_BUILD_CHANNEL_DEFAULT` (baked via `option_env!`, default
  `"stable"`, see `agentmux-common/build.rs`) and the `AGENTMUX_DEV=1` dev marker.
- username → `USER` (unix) / `USERNAME` (win) env, with a `libc`/Win32 fallback.
- host → `libc::gethostname` (unix) / `GetComputerNameExW` (win), or the `hostname` crate.
- The launcher **already reads an early config file** (`config::load_saga_retention_days`
  → `~/.agentmux/config.toml`), so reading a disable flag before srv/host starts is
  precedented and cheap.

---

## 3. Data model — the footer fields

A single shared struct, gathered **once** in the launcher and handed to each backend.
Proposed home: `agentmux-launcher/src/splash_info.rs` (new), reused by all platforms.

```rust
pub struct SplashInfo {
    pub user: String,     // "snowbark"
    pub host: String,     // "devbox"  (short hostname, not FQDN)
    pub version: String,  // "0.47.2"
    pub dev_label: Option<String>, // Some("dev · main") on non-stable; None on stable
}
```

Sourcing rules:
- **user**: `USER`/`USERNAME` env → fallback `libc::getpwuid`/`GetUserNameW` → `"user"`.
- **host**: short hostname (strip domain). `gethostname` → first `.`-segment → `"host"`.
- **version**: `env!("CARGO_PKG_VERSION")`.
- **dev_label**:
  - stable release (`channel == "stable"` and `AGENTMUX_DEV` unset) → `None`.
  - `AGENTMUX_DEV=1` → `Some("dev")` (optionally `"dev · <branch>"` if the dev branch is
    derivable from the data dir, à la `~/.agentmux/dev/<branch>/`).
  - otherwise → `Some(<channel>)` (e.g. `local-main-1f343290`), truncated to fit.

All strings are sanitized to printable ASCII and **length-clamped** (see §5).

---

## 4. Visual layout — "near the bottom"

Grow the card vertically to add a **footer band** below the brain; the brain + pulse
stay as-is, just anchored higher instead of dead-center.

```
┌──────────────────────────┐
│                          │
│          (brain)         │   ← pulsing brain, centered in the UPPER region
│                          │
│                          │
│   snowbark@devbox        │   ← footer line 1  (muted)
│   v0.47.2 · dev          │   ← footer line 2  (muted; "· dev" only when dev_label)
└──────────────────────────┘
```

- **Card size:** keep current width; **add a footer band** of height
  `FOOTER_H` (≈ 2 lines + padding). Define `CARD_W`, `CARD_H = brain_region + FOOTER_H`.
  Re-center the **card** on the primary monitor (Linux X11 already centers on RANDR
  primary; mac centers on `mainScreen`; Win32 centers — all must use the **new** card
  height).
- **Typography:** small, **muted** foreground (e.g. `#8A8A93` on the `#1A1A1F`
  backdrop), centered, monospace, ~11–12 px logical. Two lines:
  - line 1: `"{user}@{host}"`
  - line 2: `"v{version}"` + (`" · {dev_label}"` if `Some`)
- **Overflow:** clamp each line to the card width; middle-ellipsize over-long
  user/host/channel so the card never grows horizontally.
- **Animation:** the footer is **static** (no pulse). It should respect the global
  fade-in/out so it appears/disappears with the card, not independently.

(Exact px values are an implementation detail; pick once and share constants across
platforms so the three look identical.)

---

## 5. Text rendering — the cross-platform crux

The brain is a pre-baked image; **text is dynamic** (user/host/version differ per
machine/build), so it must be rasterized at runtime. Two software-buffer platforms
(Linux, Windows) and one native (macOS).

**Decision: a shared, dependency-light bitmap-font atlas for the software platforms;
native text on macOS.**

- **Shared bitmap font (Linux + Windows).** Pre-bake a small **monospace bitmap-font
  atlas** via `build.rs` (same pattern as the brain → `font_atlas.bin` + glyph metrics),
  covering printable ASCII (0x20–0x7E). Add a shared `draw_text(buf, x, y, s, color,
  scale)` that blits glyphs into the ARGB/BGRA pixel buffer with the footer color and
  the global window-alpha. This keeps the launcher's "tiny binary, no heavy deps" ethos
  (`splash_mac.rs` explicitly avoids heavy deps) — **no `fontdue`/`ab_glyph`/freetype**.
  - Source font: ship a permissively-licensed monospace bitmap (e.g. a classic 6×13 /
    Cozette / Spleen-style) in `resources/`, baked at build time. Record license.
  - Linux: extend `render_frame` (`splash_linux/mod.rs`) to draw the two footer lines
    after the backdrop/brain. Shared by both Wayland and X11 (they already share
    `render_frame`).
  - Windows: draw the same atlas into the DIB inside `composite()` (`splash.rs`), after
    the brain. (Avoids GDI `DrawText`-on-layered-DIB alpha headaches and keeps pixels
    identical to Linux.)
- **macOS:** two `NSTextField`s (or one with a newline) added as subviews of the
  backdrop in `build_window()` (`splash_mac.rs`) — `labelColor`/custom gray, monospaced
  system font, centered, non-editable, transparent background. Native crispness; no
  atlas needed. They inherit the window's fade via the existing `setAlphaValue:` path.

Alternative considered (rejected for v1): native text on **all** platforms (GDI on
Win, CoreText on mac, fontconfig/pango on Linux). Rejected because Linux native text
pulls **fontconfig/freetype/pango** into the launcher — exactly the dependency weight
the launcher avoids — and risks font-availability variance. The baked atlas is
self-contained and pixel-identical across the software platforms.

---

## 6. Disable-splash setting

### 6.1 Where it lives
- **User-facing key in `settings.json`:** `"splash:disabled": true` (bool, default
  `false`), following the existing `namespace:key` convention (`term:fontsize`,
  `widget:icononly`, …). Register it in the settings schema/defaults
  (`agentmux-srv/src/backend/wconfig/`) so it's documented and a future settings UI can
  surface a toggle. Since Settings today opens `settings.json` in an editor (per
  `CLAUDE.md`), the key + a one-line doc comment is the "option in settings."
- **Env override (highest precedence):** `AGENTMUX_SPLASH=0` (or
  `AGENTMUX_NO_SPLASH=1`) to force-disable regardless of the file — handy for CI,
  screenshots, and bisecting. Mirrors the existing `AGENTMUX_SPLASH_HOLD_MS` knob.

### 6.2 How the launcher reads it (must be EARLY)
The splash is spawned by the launcher **before** srv/host (`main.rs:120`
`splash_linux::spawn()`; mac `splash_mac::Splash::show()`; Win inside `launcher_main`).
So the launcher must resolve the flag without srv:
1. Resolve the config dir from the already-computed `paths` (the launcher resolves data/
   config paths early — `main.rs` "paths resolved" log).
2. Read `settings.json` from that dir; parse **only** `splash:disabled` (a tiny,
   best-effort, dependency-free read — on any error, default to *enabled*, since a
   broken read must never silently suppress the splash).
3. Env override wins over the file.
4. If disabled → **do not create any splash window** on any platform. The *how* differs
   by dismiss model (§2):
   - **Linux/macOS** (ready-file): skip `splash_linux::spawn()` / `Splash::show()` **and**
     don't set `AGENTMUX_SPLASH_READY_FILE`, so the host's first-paint write is a no-op.
   - **Windows** (named event): simply **don't call `spawn_splash(dir_hash)`** — no
     `CreateEventW`, no window. There is no ready-file to suppress; the host's
     splash-dismiss event signal harmlessly no-ops when nothing listens.

### 6.3 Interaction with `AGENTMUX_SPLASH_HOLD_MS`
`HOLD_MS` (linger time) is unaffected; disabling is orthogonal — disabled means "never
shown," HOLD_MS only matters when shown.

---

## 7. Cross-platform consistency checklist

| Concern | Linux | macOS | Windows |
|---|---|---|---|
| Card resized for footer | `render_frame` + card dims | `SPLASH_PX` → W×(H+footer) | **`SPLASH_SIZE` square → separate `W`×`H`** (DIB `biWidth`/`biHeight` + `CreateWindowExW`) |
| Re-center on new height | RANDR primary (x11), compositor (wl) | `mainScreen` | `SM_CXSCREEN/CYSCREEN` (full-screen, current) |
| Footer text | baked atlas via `render_frame` | `NSTextField` ×2 | baked atlas in `composite()` |
| Reads `splash:disabled` | launcher `spawn()` guard | `Splash::show()` guard | `launcher_main` guard |
| Env override `AGENTMUX_SPLASH=0` | ✓ | ✓ | ✓ |

Shared code (gather `SplashInfo`, the atlas, the disable-flag read) lives **once** in
the launcher and is consumed by all three backends so they cannot drift.

---

## 8. Edge cases

- **Missing user/host** → fall back to `"user"` / `"host"`; never crash, never block
  the splash.
- **Very long hostname / channel** → middle-ellipsize to the card width.
- **No window manager / no monitor info** → footer still renders; centering falls back
  to whole-screen (existing behavior).
- **Disabled-flag read fails** → treat as **enabled** (fail safe — the splash is the
  expected default; a parse error must not hide it).
- **HiDPI / scaling** → footer font scales with the same factor the brain/card use
  (Linux software buffer is unscaled today; if/when scaled, scale the atlas blit too).
- **Dev label absent on stable** → line 2 is just `v{version}` (no trailing `·`).

---

## 9. Test plan / acceptance

- **AC1:** On all 3 platforms, the splash shows `user@host` and `v<version>` near the
  bottom, centered, muted, not overlapping the brain.
- **AC2:** A dev build (`task dev`, `AGENTMUX_DEV=1`) shows the dev label; a stable
  release does **not**.
- **AC3:** `settings.json` `"splash:disabled": true` → no splash window appears (verify
  no splash process/surface; host still starts normally).
- **AC4:** `AGENTMUX_SPLASH=0 <launch>` → no splash even if the file says enabled;
  `AGENTMUX_SPLASH=1` (or unset + file false) → splash shows.
- **AC5:** Over-long hostname is ellipsized; card width unchanged.
- **AC6:** Footer fades in/out with the card; does not pulse.
- **AC7 (regression):** brain still centers (Linux via XWayland per the
  2026-06-21 centering fix), pulses, and dismisses on host first-paint.

Manual eyeball with `AGENTMUX_SPLASH_HOLD_MS=4000` on each platform; screenshot diff
the three for layout parity.

---

## 10. Implementation slices

1. **`SplashInfo` gatherer** (`splash_info.rs`) + unit tests for sourcing/clamping.
2. **Disable flag**: env + `settings.json` read in the launcher; guards in all 3 spawn
   sites; register `splash:disabled` in the settings schema/defaults. (Ship-able alone.)
3. **Baked font atlas** in `build.rs` + `draw_text` for the software buffers.
4. **Linux footer** (`render_frame` + card resize + re-center). Verify Wayland→XWayland
   path still centers.
5. **Windows footer** (`composite()` + DIB/window resize + re-center).
6. **macOS footer** (`NSTextField`s + card resize + re-center).
7. Cross-platform screenshot parity pass.

---

## 11. Open questions

- **Exact footer content/format:** `user@host` + `v0.47.2 · dev` as drafted — confirm
  wording, or do we want the channel/build-label verbatim on dev?
- **Bitmap font choice + license** for the baked atlas (needs a permissive monospace).
- **Settings UI:** is editing `settings.json` sufficient, or should a real toggle be
  added to a settings surface now (vs. tracked as follow-up)?
- **Privacy:** `user@host` is shown on the splash — fine for a single-user desktop;
  flag if splash screenshots are ever shared in support channels (the disable toggle
  mitigates).
