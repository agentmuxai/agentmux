# SPEC: Linux splash polish (fade-out, multi-monitor centering, rounded corners)

**Date:** 2026-06-20
**Status:** Draft / in progress
**Owner:** launcher / Linux platform
**Builds on:** [`SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20.md`](./SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20.md) — this is its **Phase 3 (polish)**.

---

## 0. TL;DR

The shipped Linux splash (PR #1618) is deliberately minimal: an **opaque, square** window that **vanishes abruptly** on dismiss, **centered on the whole X screen** (wrong on multi-monitor). This brings it to parity with the Windows/macOS splashes:

1. **Fade-out** (~160 ms) on dismiss, on both backends — matching `splash.rs`/`splash_mac.rs`.
2. **Rounded corners** (16 px) — matching macOS.
3. **Primary-monitor centering** on X11 via RANDR (Wayland is compositor-placed, unchanged).

The enabler on X11 is switching the splash window to a **32-bit ARGB visual** (per-pixel alpha) when a compositor is present — Wayland already has per-pixel alpha via `wl_shm` ARGB8888.

---

## 1. Goals / non-goals

### Goals
- Smooth ~160 ms opacity fade-out when the splash dismisses (host first-paint *or* safety timeout), then tear down.
- 16 px rounded corners on the dark backdrop, both backends.
- X11: center on the **primary** monitor (RANDR), not the union of all screens.
- Graceful fallback: if X11 has **no compositor** (no per-pixel alpha), keep today's behavior (opaque, square, abrupt) rather than render garbage.

### Non-goals
- Wayland positioning/stacking changes — still compositor-controlled (protocol limitation; unchanged from the base spec).
- Drop shadows / blur / fancy easing — straight linear opacity fade is enough.
- Per-monitor DPI scaling of the brain (the asset is fixed-size; revisit only if HiDPI looks wrong).

---

## 2. Shared rendering changes

`render_frame` currently writes an **opaque** BGRX frame (alpha byte = 0xFF, square). Polish needs **per-pixel alpha** with **rounded corners** and a **global fade multiplier**, and the output must be **pre-multiplied** (what both X Render compositors and `wl_shm` ARGB8888 expect).

New signature (shared in `splash_linux/mod.rs`):

```rust
/// Composite one frame, pre-multiplied, into a 4-byte/pixel buffer.
/// `brain_alpha` is the pulse (0..1); `window_alpha` is the global fade (0..1);
/// `radius` rounds the backdrop corners (0 = square). `bgr` selects channel
/// order (B,G,R,A on LE for both X11 ARGB and wl_shm ARGB8888).
fn render_frame(buf, w, h, brain_alpha, window_alpha, radius, bgr)
```

Per pixel:
1. `cov` = rounded-rect coverage at (x,y) — `1.0` inside, `0.0` outside, anti-aliased on the corner arcs (distance-to-corner-center vs `radius`, 1-px smoothstep).
2. Composite brain over backdrop in straight RGB (as today).
3. `a = cov * window_alpha` (backdrop is otherwise opaque); **pre-multiply**: each channel `*= a`; alpha byte `= a*255`.

A square, fully-opaque frame (`radius=0`, `window_alpha=1`) reduces to today's output, so the **opaque-fallback path keeps using the existing fast opaque fill**.

`pulse_alpha(t)` is unchanged (drives `brain_alpha`). A new helper computes `window_alpha` during the fade phase (see §5).

---

## 3. X11 backend (`splash_x11.rs`)

### 3.1 ARGB visual + compositor gate
- Detect a running compositor: an owner exists for the `_NET_WM_CM_S<screen>` selection (`GetSelectionOwner`). GNOME/Mutter (and XWayland under it) always have one.
- If composited: find a **depth-32, TrueColor** visual on the screen; create a **colormap** for it; create the window at depth 32 with that visual + colormap. Required extra `CreateWindowAux`: `colormap`, `border_pixel` (0) — X requires both when the window depth differs from the parent. The pixmap + GC are created at depth 32. → per-pixel alpha works; fade + rounded corners enabled.
- If **not** composited (or no depth-32 visual): keep the current **depth-24 opaque** path (`window_alpha` forced to 1, `radius` forced to 0, abrupt dismiss). Log which path was taken.

### 3.2 Primary-monitor centering (RANDR)
- Enable the `randr` feature on `x11rb`.
- Query the **primary** output: `randr::get_output_primary(root)` → output; `get_crtc_info` of its CRTC → `(x, y, width, height)`. Center the splash within that rect.
- Fallbacks: no primary set → first connected/enabled CRTC; RANDR unavailable → today's whole-screen centering.

### 3.3 Loop
Unchanged structure; the draw call now passes `window_alpha` (1.0 until fading) and `radius`, and the put-image strips already handle 32-bit depth.

---

## 4. Wayland backend (`splash_wayland.rs`)
- `wl_shm` ARGB8888 already carries per-pixel alpha and is always composited, so **no visual gymnastics** — just pass `radius` and `window_alpha` to `render_frame`.
- Set an **opaque region** only when not rounded/fading (optimization); with rounded corners + fade we leave the surface fully alpha-blended (correctness over the minor perf win for a ~1 s splash).
- App-id grouping unchanged. No taskbar-hint protocol — GNOME honors none for a plain toplevel (documented limitation, base spec §3.2).

---

## 5. Fade-out (both backends)
- Dismiss is now two-phase. When `should_dismiss()` first returns true, record `fade_start` and keep looping; compute `window_alpha = 1.0 - clamp01((now - fade_start)/FADE_OUT)` with `FADE_OUT = 160 ms`. When `window_alpha` reaches 0, tear down.
- On the **opaque-fallback** X11 path (no compositor), skip the fade (can't alpha a depth-24 window) — dismiss immediately, as today.
- The safety timeout (10 s) also fades out (host crashed before paint) rather than vanishing.

Helper in `mod.rs`:
```rust
const FADE_OUT: Duration = Duration::from_millis(160);
/// 1.0 while not fading; ramps to 0.0 over FADE_OUT once `fade_start` is set.
fn fade_alpha(fade_start: Option<Instant>, now: Instant) -> f32 { … }
```

---

## 6. Constants
- `CORNER_RADIUS_PX = 16` (matches macOS).
- `FADE_OUT = 160 ms` (matches macOS).
- Backdrop, pulse, padding, `min_hold` — unchanged from the base spec.

---

## 7. Testing & acceptance

X11 (composited GNOME / XWayland, via `AGENTMUX_OZONE_PLATFORM=x11`):
- [ ] Rounded corners visible; backdrop alpha-blended (no black square box).
- [ ] Centered on the **primary** monitor (verify on a multi-monitor setup, or at least correct on single).
- [ ] Smooth ~160 ms fade-out on dismiss; no abrupt pop.

X11 non-composited (e.g. plain `Xorg` + no compositor, or `picom` off):
- [ ] Falls back cleanly to opaque square, abrupt dismiss — **no garbage/black-corner rendering**.

Wayland (GNOME default):
- [ ] Rounded corners + fade-out both render via `wl_shm`.
- [ ] No protocol errors; buffer released on teardown.

Both:
- [ ] Host-crash-before-paint → fade-out at the 10 s timeout.
- [ ] Windows/macOS splash unchanged.

Validation will use the same `AGENTMUX_SPLASH_HOLD_MS` long-hold trick to inspect the fade/corners, and a fresh-channel cold start (`env -u AGENTMUX … AGENTMUX_CHANNEL=…`).

---

## 8. Risks & mitigations

| Risk | Likelihood | Mitigation |
| --- | --- | --- |
| X11 ARGB window renders black/garbage with no compositor | Med | `_NET_WM_CM_S*` gate → opaque-24 fallback; never create ARGB without a compositor. |
| No depth-32 TrueColor visual on some servers | Low | Fall back to opaque-24. |
| RANDR absent / headless | Low | Fall back to whole-screen centering. |
| Pre-multiplied-alpha mistakes (dark fringing on the brain/edges) | Med | Pre-multiply once at the end; eyeball vs macOS; unit-check a corner + center pixel. |
| Rounded-corner AA cost per frame | Low | Compute coverage cheaply (only the corner boxes need the arc test; interior is a fast opaque fill). |

---

## 9. Files touched
- `agentmux-launcher/src/splash_linux/mod.rs` — `render_frame` rework (per-pixel alpha, rounded mask, fade), `fade_alpha`, constants.
- `agentmux-launcher/src/splash_linux/x11.rs` — ARGB visual + compositor gate, RANDR primary centering, two-phase dismiss/fade.
- `agentmux-launcher/src/splash_linux/wayland.rs` — pass radius + window_alpha, two-phase dismiss/fade.
- `agentmux-launcher/Cargo.toml` — `x11rb` `randr` feature.
