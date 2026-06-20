# SPEC: Session-aware Linux startup splash (X11 + Wayland)

**Date:** 2026-06-20
**Status:** Draft / proposed
**Owner:** launcher / Linux platform
**Supersedes the splash portion of:** [`SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05.md`](./SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05.md) Workstream B (X11-only assumption)
**Builds on:** [`SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31.md`](./SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31.md) (dismiss protocol, threading precedent)

---

## 0. TL;DR

Linux is the only platform with **no startup splash** — the launcher spawns `srv` + the CEF host and the user stares at a blank desktop for the ~1–3 s of `srv` handshake + CEF init before the host paints its first frame. Windows (`splash.rs`, Win32) and macOS (`splash_mac.rs`, Cocoa) both paint a pulsing brain logo in that window; Linux falls straight through to `splash_event_name = None` (`agentmux-launcher/src/main.rs:1356`).

The previously-specced Linux plan (Workstream B) was **X11-only**, justified because the host was pinned to `--ozone-platform=x11`. **That pin was just removed** — [PR #1611] made **native Wayland the default** when `WAYLAND_DISPLAY` is present (`agentmux-cef/src/app.rs:599`). The old spec explicitly said to **skip the splash on native-Wayland hosts**, which now means *no splash on the common GNOME/Wayland path* — the opposite of the goal.

This spec replaces that with a **session-aware, dual-backend splash** in the launcher:

| Session (mirrors host ozone) | Splash backend | Fidelity |
| --- | --- | --- |
| X11, or `AGENTMUX_OZONE_PLATFORM=x11` (XWayland) | **`splash_x11`** via `x11rb` — `override_redirect` + EWMH `_NET_WM_WINDOW_TYPE_SPLASH` + RANDR centering | Full (positioned, on-top, no taskbar) |
| Wayland (default when `WAYLAND_DISPLAY` set) | **`splash_wayland`** via `smithay-client-toolkit` + `wl_shm` xdg_toplevel | Degraded but present (compositor-placed, no forced on-top) |

Both share one dismiss protocol (the macOS `AGENTMUX_SPLASH_READY_FILE` file signal), one asset pipeline (`brain.png` → `build.rs`), and one animation curve.

---

## 1. Goals / non-goals

### Goals
- A branded splash (pulsing brain on dark backdrop) covering the launcher→`srv`→host→CEF-first-frame gap on Linux, on **both** X11 and native-Wayland sessions — GNOME/Mutter being the primary target.
- Reuse, not reinvent: same dismiss protocol, asset pipeline, animation params, and launcher integration shape as Windows/macOS.
- No GPU dependency (software draw only) — the launcher is a tiny binary that must paint *before* the CEF/GPU stack exists.
- Graceful degradation and clean teardown: a host that crashes before first frame must not leave a stuck splash.

### Non-goals
- Pixel-perfect parity of placement/stacking on Wayland (the protocol forbids it — see §3).
- A `wlr-layer-shell` implementation (unavailable on GNOME — see §3.1).
- Covering the *dev-mode* Vite module-load lag — that's a dev-only artifact, not the shipped cold-start gap. (The packaged AppImage already loads fast.)
- Multi-window / per-monitor splashes; one splash on the primary/active output.

---

## 2. Motivation & the new Wayland wrinkle

The launcher is up in milliseconds; `agentmux-cef` *is* the multi-second cold start (CEF init, GPU channel, page load). A splash can only be painted by a process that is alive *before* CEF — i.e. the launcher, in its own process. This is exactly why Windows/macOS put the splash there, and why a host-side / HTML splash **cannot** cover the gap (CEF can't paint until it's initialized).

Until #1611 the Linux host ran XWayland (`--ozone-platform=x11`) to dodge the CEF 146 native-Wayland frame-stall bug. Workstream B leaned on that: an X11 splash matched an X11 host, and `AGENTMUX_OZONE_PLATFORM=wayland` (the rare regression-test path) simply skipped the splash.

CEF 148 fixed the native-Wayland bug (verified: no `OnTrancheFlags Not implemented`, no `DidNotProduceFrame`, smooth typing), so #1611 flipped the default to **native Wayland on Wayland sessions**. Consequently:

- On a GNOME/Wayland desktop (now the default), the host is a **native-Wayland** client. An XWayland splash would float as a *foreign* X11 surface over a Wayland host, and the old spec's rule would skip it entirely → **no splash for most users**.
- To show a splash on that path we need a **native-Wayland** splash — which runs into Wayland's deliberate UX constraints (§3).

The splash backend must therefore track the **same** session decision the host makes, so splash and host always speak the same display protocol.

---

## 3. Research findings (Wayland/GNOME feasibility)

### 3.1 GNOME/Mutter does not support `wlr-layer-shell`
`wlr-layer-shell` is the wlroots/KDE protocol for stacked, screen-edge-locked overlay surfaces (what panels/launchers/splashes use). **Mutter does not implement it and GNOME has declined to** (open since [mutter#973], [gnome-shell#1141]). `gtk4-layer-shell` / `gtk-layer-shell` exist but are thin wrappers over that protocol — so they work on sway/Hyprland/KDE/COSMIC, **not GNOME**. The clean "always-on-top overlay" mechanism is therefore **off the table for our primary target**.

### 3.2 Wayland forbids client-controlled positioning and stacking — by design
Wayland intentionally does **not** expose absolute window position to clients, and there is **no client-side "always on top."** "Throughout the design of Wayland the approach has been for the client to make requests and the server to make the final decisions." A standalone `xdg_toplevel` therefore:
- **Cannot self-center** — the compositor places it. (Mutter places small new toplevels roughly centered on the work area, which is acceptable for a splash.)
- **Cannot force itself above** the host window — best effort only.
- **Cannot avoid the taskbar/overview** the way EWMH `_NET_WM_STATE_SKIP_TASKBAR` does on X11 (no equivalent without extension protocols).

These are accepted degradations on Wayland, not bugs to fix.

### 3.3 `smithay-client-toolkit` + `wl_shm` is the viable software path
The sctk `simple_window` example shows the minimal flow: `Connection::connect_to_env()` → `registry_queue_init()` binds `wl_compositor`/`wl_shm`/`xdg_wm_base` (`CompositorState`, `Shm`, `XdgShell`) → `XdgShell::create_window()` for an xdg_toplevel → `SlotPool` allocates an **ARGB8888** `wl_shm` buffer → draw raw pixels into the canvas → damage + `commit`; redraw on the `frame` callback (~16 ms). Decorations are requested via `WindowDecorations` (we request **none**). Core draw is ~50 LOC; a full minimal toplevel with handler boilerplate is ~450 LOC. **No GPU** — pure shared-memory software blit, exactly what the launcher needs.

### 3.4 Precedent
Electron went Wayland-native (CSD via `ClientFrameViewLinux`), but there is **no established native pre-init splash pattern** on Wayland — Chromium/Electron apps generally accept CSD and have no cold-start splash. So we are slightly off the beaten path; the degradations in §3.2 are inherent, and we design around them rather than expecting a polished overlay.

**Conclusion:** keep the full-fidelity **X11** splash for X11/XWayland sessions, add a **best-effort native-Wayland** splash (sctk + `wl_shm`) for Wayland sessions, and share everything else.

---

## 4. Architecture

```
launcher main()  (ms-fast, pre-CEF)
   │
   ├─ session = detect()            // mirrors agentmux-cef/src/app.rs ozone logic
   │     AGENTMUX_OZONE_PLATFORM set? → that
   │     else WAYLAND_DISPLAY set?   → "wayland"
   │     else                         → "x11"
   │
   ├─ splash = match session {
   │     "wayland" => splash_wayland::spawn(dir_hash),   // sctk + wl_shm, own thread
   │     _         => splash_x11::spawn(dir_hash),       // x11rb, own thread
   │   }   // → Option<SplashHandle { ready_file: PathBuf }>
   │
   ├─ spawn srv + host  (host env gets AGENTMUX_SPLASH_READY_FILE = splash.ready_file)
   │
   └─ host paints first frame → writes ready_file (on_load_end)
         → splash thread polls the file, fades out (~160 ms), tears the surface down
         → safety timeout (10 s) dismisses if the host never signals (crash before paint)
```

Key choices:
- **Threaded model (like Windows), not main-thread (like macOS).** Neither X11 (x11rb/xcb) nor Wayland (sctk) requires the main thread, so each backend owns a **dedicated thread** with its own display connection + event loop. `spawn()` returns a lightweight `SplashHandle`. This avoids the macOS CFRunLoop-on-main dance and keeps `main()` free to start the supervisor immediately.
- **One dismiss protocol for all of Linux:** the macOS **`AGENTMUX_SPLASH_READY_FILE`** file signal (§6). Simpler than Windows named events and already implemented host-side; Linux just needs the host's `cfg` gate widened.
- **Splash backend == host backend, always.** Because both read the same env/`WAYLAND_DISPLAY`, an X11 host gets an X11 splash and a Wayland host gets a Wayland splash — never a cross-protocol mismatch.

---

## 5. Backend A — X11 (`splash_x11.rs`)

Essentially Workstream B of the prior spec, retained for X11 and XWayland sessions where we get full control.

- **Window:** `create_window` with `override_redirect = 1` (no WM decoration/management), `SPLASH_SIZE × SPLASH_SIZE`, centered on the **primary** output via RANDR (`xrandr` `GetMonitors`, pick `primary`).
- **EWMH hints** (best-effort, harmless under `override_redirect`): `_NET_WM_WINDOW_TYPE = _NET_WM_WINDOW_TYPE_SPLASH`, `_NET_WM_STATE = {_NET_WM_STATE_ABOVE, _NET_WM_STATE_SKIP_TASKBAR, _NET_WM_STATE_SKIP_PAGER}`.
- **Paint:** one `Pixmap` sized to the window; each tick, software alpha-blend the brain RGBA over the solid backdrop into the pixmap, then `CopyArea` to the window. ~60 Hz (16 ms).
- **Pulse curve:** shared (§8).
- **Dismiss:** poll `AGENTMUX_SPLASH_READY_FILE`; on signal, fade alpha→0 over ~160 ms, then unmap + destroy + close the connection. 10 s safety timeout.
- **Crate:** `x11rb` `0.13`, `default-features = false`, features `["randr"]` (add `xinerama` only if multi-GPU monitor enumeration needs it). Pure `libxcb` (present on every X11 stack).

---

## 6. Backend B — Wayland (`splash_wayland.rs`)

New. Best-effort native-Wayland splash for the now-default path.

- **Connection/thread:** dedicated thread; `Connection::connect_to_env()`, `registry_queue_init()`, bind `CompositorState` / `Shm` / `XdgShell`.
- **Surface:** `XdgShell::create_window(surface, WindowDecorations::None, &qh)`; `set_title("AgentMux")`, `set_app_id("ai.agentmux.AgentMux")` — **same app_id as the host** so Mutter groups/associates them and is more likely to stack the splash with the app. Set a fixed `min`/`max` size = `SPLASH_SIZE` so the compositor gives us exactly that (a splash is non-resizable).
- **Buffer/draw:** `SlotPool` → **ARGB8888** `wl_shm` buffer; blit the dark backdrop + alpha-modulated brain each `frame` callback (~16 ms); `damage` + `commit`. (Note: `wl_shm` ARGB is **pre-multiplied** straight-alpha per Wayland — match the backdrop opaque and the brain alpha-pulsed.)
- **Placement / stacking — accepted limitations (§3.2):** we cannot center or force-on-top. We rely on Mutter's default centered placement for small toplevels and on dismissing *the instant* the host paints (so the two-window overlap window is sub-frame in practice). Document this as expected.
- **Taskbar/overview:** the splash may flash briefly in the overview/alt-tab. Mitigations: minimal lifetime + matching `app_id`; if a suitable hint protocol is available at impl time (e.g. `xdg-toplevel-tag-v1`), set it, but do **not** depend on it.
- **Dismiss:** identical file-poll + fade + teardown as X11.
- **Crates:** `smithay-client-toolkit` (pulls `wayland-client`, `wayland-protocols`). Software-only; no GPU/EGL.

> **Decision call-out (§12-A):** the Wayland backend is intentionally "good enough," not pixel-perfect. If review decides the degradations aren't worth it for v1, ship X11-only and have Wayland sessions *skip* the splash (the prior behavior) — but that leaves the default desktop splash-less, which defeats the motivation.

---

## 7. Shared — dismiss protocol & host signaling

Reuse the **macOS file mechanism** verbatim:

- Launcher (`splash_*::spawn`) creates `std::env::temp_dir().join(format!("agentmux-splash-ready-{}", std::process::id()))`, returns it in `SplashHandle`.
- Launcher threads it into the host env as **`AGENTMUX_SPLASH_READY_FILE`** (see §10 — `spawn_host_unix` must learn to set it).
- Host writes the file on first paint. The existing code in `agentmux-cef/src/client/mod.rs:1310` is gated `#[cfg(target_os = "macos")]`; **widen to `#[cfg(any(target_os = "macos", target_os = "linux"))]`** (the one-character change the prior spec noted). The Windows `AGENTMUX_SPLASH_EVENT` branch is untouched.
- Splash thread polls `ready_file.exists()` on its tick; on hit → fade + teardown.
- **Safety timeout: 10 s** (mirror `splash_mac.rs` `DISMISS_TIMEOUT`). If the host crashes before first paint, the splash self-dismisses cleanly; the launcher's supervisor independently handles the crash/restart. On a restart, the same ready-file path is reused so a late first-frame still dismisses a splash left pending.

---

## 8. Shared — assets, `build.rs`, animation

### Asset
`agentmux-launcher/resources/brain.png` — 256×256 RGBA8 transparent PNG (already in tree; macOS embeds it directly, Windows decodes it in `build.rs`).

### `build.rs`
Currently `#[cfg(target_os = "windows")]`-only (emits pre-multiplied **BGRA** `brain_bgra.bin` + `brain_dims.rs`). Add a `#[cfg(target_os = "linux")]` arm:
- Decode `brain.png` → **straight RGBA8** → emit `brain_rgba.bin` (X11 pixmap wants native RGBA; the Wayland path converts RGBA→pre-multiplied ARGB at blit time, or emit a second pre-multiplied buffer if cheaper).
- Emit the shared `brain_dims.rs` (`BRAIN_W`, `BRAIN_H` consts).
- ~30 LOC; no runtime PNG decoder dependency.

### Layout & animation (shared constants, match macOS feel)
- Backdrop: opaque **RGB(26, 26, 31)** = `#1A1A1F`.
- Brain display size and padding: follow the macOS convention (brain 150 px, 35 px padding → 220×220, 16 px corner radius where the backend supports rounding; X11 `override_redirect` can use a shaped/rounded pixmap, Wayland uses a rounded ARGB blit). Sizing constants live in one shared module.
- Pulse: fade-in 0→peak over **200 ms**, then **1.1 Hz** sine; alpha range matches the platform's compositing (macOS 0.73–1.0; for software ARGB use the same normalized 0.73–1.0 on the brain, backdrop stays opaque). ~60 fps (16 ms ticks).

---

## 9. Launcher integration & the `spawn_host_unix` gap

- **Session dispatch** in `main.rs` replaces the current `#[cfg(not(target_os="windows"))] let splash_event_name = None;` with a Linux arm that runs `detect()` and calls the matching `splash_*::spawn`. macOS keeps its existing early-show path; Windows keeps `spawn_splash`. Introduce a small enum so the host-spawn path is platform-agnostic:
  ```rust
  enum SplashDismiss { None, WindowsEvent(String), ReadyFile(PathBuf) }
  ```
- **`spawn_host_unix` must set the splash env.** The Explore audit confirms the Unix host-spawn path currently does **not** export `AGENTMUX_SPLASH_READY_FILE` (nor `AGENTMUX_HOME`/`AGENTMUX_LAUNCHER_PIPE` — those are separate A1 work). Add: when `SplashDismiss::ReadyFile(p)` is present, set `AGENTMUX_SPLASH_READY_FILE = p` on the host `Command`. This is the **prerequisite** that makes any Linux splash dismiss.
- **Supervisor flow:** the splash thread is independent of the supervisor; it dies on signal or timeout. No change to crash/restart/OOM-`disable_gpu` ladders.
- **AppImage timing:** `AppRun` execs the launcher; splash `spawn` should run immediately after single-instance acquisition, before `srv` spawn, so the brain appears within ~50 ms of AppRun (per the prior spec's acceptance bar).

---

## 10. Dependencies

```toml
[target.'cfg(target_os = "linux")'.dependencies]
x11rb = { version = "0.13", default-features = false, features = ["randr"] }
smithay-client-toolkit = "0.19"   # pulls wayland-client + wayland-protocols
```

- Both are **software-only** (X11 `Pixmap` / Wayland `wl_shm`); no EGL/GPU, no new C deps beyond `libxcb` (X11) and `libwayland-client` (present in any Wayland session).
- Binary-size budget: x11rb ≈ +150 KB stripped; sctk + wayland-client ≈ +250–400 KB stripped. Acceptable for a launcher; both are `cfg(linux)`-gated so Windows/macOS are unaffected.
- Feature-flag both backends behind a `linux-splash` cargo feature if we want a no-splash build for headless/CI.

---

## 11. Phasing

| Phase | Deliverable | Notes |
| --- | --- | --- |
| **P0** | Dismiss plumbing | Widen `client/mod.rs` cfg to include linux; teach `spawn_host_unix` to set `AGENTMUX_SPLASH_READY_FILE`; `build.rs` Linux arm emits `brain_rgba.bin`; shared constants module. **No visible splash yet** — but everything downstream depends on this. |
| **P1** | `splash_x11.rs` | Full-fidelity X11 splash; verify on an X11 session and via `AGENTMUX_OZONE_PLATFORM=x11` (XWayland). |
| **P2** | `splash_wayland.rs` | sctk + `wl_shm` toplevel; verify on GNOME/Wayland (the default). Accept §3.2 degradations. |
| **P3** | Polish | Rounded corners, fade-out tuning, `app_id` grouping, optional taskbar-hint protocol if available, multi-monitor primary-output selection. |

P1 and P2 are independent after P0 and can land in either order; P2 is the higher-value one given native Wayland is the default.

---

## 12. Decision points (need a call before/within implementation)

- **(A) Wayland fidelity vs. scope.** Ship the best-effort Wayland splash (recommended — it's the default desktop) accepting compositor placement + no forced on-top, **or** ship X11-only and skip on Wayland (simpler, but splash-less for most users)?
- **(B) Single dual-backend module vs. two files.** `splash_x11.rs` + `splash_wayland.rs` behind a `splash_linux` dispatcher (recommended) vs. one large file.
- **(C) Binary size.** Accept ~+0.4 MB for sctk on the launcher, or gate the Wayland backend behind a feature that distro packagers can drop?
- **(D) Rounded corners on Wayland.** Worth the per-pixel alpha mask, or square is fine for v1?

---

## 13. Testing & acceptance criteria

X11 session (or `AGENTMUX_OZONE_PLATFORM=x11`):
- [ ] Brain appears within ~50 ms of AppRun, centered on the primary monitor, above other windows, not in the taskbar.
- [ ] Smooth 1.1 Hz pulse; fades out within ~200 ms of host first frame.
- [ ] Multi-monitor: centered on the primary output.

Wayland session (GNOME/Mutter, default):
- [ ] Brain appears within ~50 ms, roughly centered (compositor-placed), pulsing smoothly.
- [ ] Dismisses within ~200 ms of host first frame; no lingering surface; minimal/no taskbar-overview flash.
- [ ] No protocol errors in the launcher log; `wl_shm` buffer released cleanly on teardown.

Both:
- [ ] **Host crash before first paint** → splash self-dismisses at the 10 s timeout, no zombie window, supervisor still restarts the host.
- [ ] `AGENTMUX_OZONE_PLATFORM` override routes the splash to the matching backend (forced x11 → X11 splash even on a Wayland session via XWayland; forced wayland → Wayland splash).
- [ ] Windows/macOS splash behavior unchanged (no regressions from the shared refactor).
- [ ] `--features no/linux-splash` (if added) builds and runs splash-less.

---

## 14. Risks & mitigations

| Risk | Likelihood | Mitigation |
| --- | --- | --- |
| Wayland splash not on-top / placed oddly on some Mutter versions | Medium | Accept (§3.2); dismiss on first frame so overlap is sub-perceptible; matching `app_id`. |
| Splash flashes in GNOME overview/alt-tab | Medium | Minimal lifetime; optional tag protocol; document as known v1 limitation. |
| Two Wayland clients (launcher + host) confuse the compositor | Low | Independent connections, standard pattern; splash torn down as host maps. |
| sctk API churn (0.19→) | Low–Med | Pin version; the surface/`wl_shm` path is stable; thin usage. |
| `wl_shm` pre-multiplied-alpha mistakes (dark fringing) | Med | Pre-multiply at blit; unit-test a few pixels; compare against macOS render. |
| Binary-size creep on the launcher | Low | `cfg(linux)`-gate; optional feature flag (§12-C). |
| Non-GNOME Wayland (KDE/wlroots) behaves differently | Low | xdg_toplevel + `wl_shm` is universal; layer-shell explicitly **not** used so no wlroots-only dependency. |

---

## 15. References

Codebase:
- `agentmux-launcher/src/splash.rs` (Windows), `splash_mac.rs` (macOS), `main.rs:1354–1357` (gating), `build.rs` (asset embed)
- `agentmux-cef/src/client/mod.rs:1310` (macOS ready-file write), `app.rs:599–637` (ozone/session selection — mirror for `detect()`)
- `resources/brain.png` (256×256 RGBA)
- Prior specs: `SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05.md` (Workstream B), `SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31.md`

External research:
- GNOME/Mutter declines `wlr-layer-shell`: <https://gitlab.gnome.org/GNOME/mutter/-/issues/973>, <https://gitlab.gnome.org/GNOME/gnome-shell/-/work_items/1141>
- `wlr-layer-shell` protocol (wlroots/KDE only): <https://wayland.app/protocols/wlr-layer-shell-unstable-v1>
- Wayland window positioning is compositor-controlled (no client placement/always-on-top): <https://canonical-mir.readthedocs-hosted.com/stable/explanation/window-positions-under-wayland/>, <https://wayland-book.com/xdg-shell-basics/xdg-toplevel.html>
- sctk software-rendered window example: <https://github.com/Smithay/client-toolkit/blob/master/examples/simple_window.rs>
- gtk4-layer-shell (works on wlroots/KDE, not GNOME): <https://github.com/wmww/gtk4-layer-shell>
- Electron Wayland-native (CSD; no native splash precedent): <https://www.electronjs.org/blog/tech-talk-wayland>
