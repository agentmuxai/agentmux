# SPEC: Launcher in packaged Linux builds + Linux splash

**Date:** 2026-06-05
**Repo state:** branch `agentu/cef-148-bundle-runtime-files` (CEF 148 + Vulkan SwiftShader bundle), companion to `agentu/cef-148-patched-libcef` (workspace `[patch.crates-io]` for native window-drag)
**Author:** AgentU-asaf (driven by Claude)
**Status:** Spec — ready to implement (phased)
**Motivated by:** Linux AppImage launches the CEF host directly, bypassing the launcher; no startup splash; no `launcher → srv + host` supervisor tree.
**Builds on:** [`SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31.md`](./SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31.md) (macOS parity precedent), [`SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30.md`](./SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30.md) (flat-layout convention reused on Linux), [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md), [`SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md`](./SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md) §0 (X11 ozone default on Linux).

---

## 0. TL;DR

1. **Linux is the only supported platform where the launcher is not in the launch path.** macOS (#1263 + the May 31 spec) and Windows (since B.1) both ship `launcher → srv + host`. The Linux AppImage `AppRun` (`scripts/linux-apprun.sh:39`) does `exec usr/bin/agentmux`, so every `launcher_ipc::report_*` call from the CEF host silently no-ops — the **window pool, instance numbering, single-instance enforcement, durable saga coordination, and srv supervision are all degraded** on Linux exactly the way the macOS spec described as the architectural gap to close.
2. **Linux has no splash.** `agentmux-launcher/src/main.rs:107` falls through `#[cfg(not(target_os = "macos"))]` straight to `launcher_main()`. `splash.rs` is `#![cfg(target_os = "windows")]`, `splash_mac.rs` is `#![cfg(target_os = "macos")]`. The 200–600 ms cold-start window on Linux is currently blank.
3. **PR #1261 (CEF 148 §0) defaulted Linux to X11 ozone** (XWayland). This collapses the Wayland-vs-X11 surface area for the splash: the host runs as an X11 client even under Mutter/KWin/Sway, so a **pure X11 splash via `x11rb` covers ~100% of the supported install base** without pulling in Wayland deps. Opt-out (`AGENTMUX_OZONE_PLATFORM=wayland`) gets no splash in v1 — that's a deliberate scope cut (§B.2.4).
4. **Workstreams:** **(A) wire the launcher into AppRun** as the AppImage entry point; **(B) write `splash_linux.rs`** — X11/`x11rb` borderless override-redirect window, brain bitmap, pulse animation, fade-out, dismissed via the existing `AGENTMUX_SPLASH_READY_FILE` ready-file protocol macOS already uses. A0 unlocks all the launcher-coordinated machinery the host already has IPC wiring for. B requires A0.

---

## 1. Current state — evidence

### 1.1 AppRun bypasses the launcher

`scripts/linux-apprun.sh`:

```bash
run_normally() {
    export APPDIR="$this_dir"
    if [ -n "$APPIMAGE" ] && [ -x "$this_dir/install-linux-desktop.sh" ]; then
        bash "$this_dir/install-linux-desktop.sh" "$APPIMAGE" || true
    fi
    export LD_LIBRARY_PATH="$this_dir/usr/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    exec "$this_dir/usr/bin/agentmux" "$@"      # ← host directly, launcher skipped
}
```

The launcher binary is **built but not actually shipped**: `task build:host:linux` writes `target/release/agentmux-launcher` → `dist/cef/agentmux-launcher`, but `scripts/build-appimage-linux.sh` never copies it into `AppDir/usr/bin/` (it stages only `agentmux-cef` → renamed to `agentmux` at `:84` and the srv at `:88`). So today the launcher sits in `dist/cef/` unused and the AppImage contains no launcher at all. A0 has to copy it in too — see §3.

### 1.2 No Linux splash module

```
agentmux-launcher/src/
├── splash.rs       ← #![cfg(target_os = "windows")]   312 lines
├── splash_mac.rs   ← #![cfg(target_os = "macos")]     372 lines
└── (no splash_linux.rs)
```

`main.rs:108-113`:

```rust
#[cfg(not(target_os = "macos"))]
{
    tokio::runtime::Runtime::new()
        .expect("failed to build Tokio runtime")
        .block_on(launcher_main());
}
```

That `not(macos)` block covers **both** Windows and Linux. On Windows, `launcher_main()` is the right entry because `splash::spawn_splash()` is called on a thread inside `launcher_main()` (`saga/mod.rs`) and returns immediately — it doesn't need main-thread ownership like AppKit does. On Linux there's no equivalent spawn site at all — the splash code path simply doesn't exist.

### 1.3 X11 ozone is the default on Linux (PR #1261)

`agentmux-cef/src/app.rs:418-440` (paraphrased):

```rust
// Linux default: x11 ozone (XWayland). Wayland-native is opt-in
// via AGENTMUX_OZONE_PLATFORM=wayland.
let ozone_choice = std::env::var("AGENTMUX_OZONE_PLATFORM")
    .unwrap_or_else(|_| "x11".to_string());
let oz_key = CefString::from("ozone-platform");
args.push_key_value(&oz_key, &CefString::from(&ozone_choice));
```

This **frames the design space for the splash** (§B.1). Under any compositor (Mutter, KWin, Sway, GNOME-on-Wayland, …) the host appears as an X11 client because XWayland is the in-between layer. A splash that owns its own X11 connection plays nicely with the same translation layer — and we get one display-server abstraction instead of two.

### 1.4 Host already has cross-platform IPC wiring the launcher would re-activate

`agentmux-cef/src/launcher_ipc.rs` has the full set of `report_window_*`, `report_instance_*`, `report_pool_*` IPC calls. On every platform they fall back to silent no-ops when the launcher isn't reachable (which today is **always** on Linux). Wiring the launcher into the Linux launch path costs zero changes to the host — the IPC is already there waiting.

---

## 2. What Linux loses vs macOS/Windows parity (impact table)

Adapted from §2.6 of the macOS spec, with one Linux-specific row at the bottom.

| Capability | Owner | Linux today | After A0 |
|---|---|---|---|
| **Single-instance enforcement** (pipe + recovery) | Launcher | ❌ second AppImage launch starts a second host with no coordination → races on extract dir, sqlite, agentmux-host log file | ✅ second-instance returns / forwards args to existing launcher (parity with Windows) |
| **srv supervision** (restart on crash, orphan reaping) | Launcher Job/process-group | ❌ srv is spawned by host; host crash leaves srv as a zombie until manual SIGKILL | ✅ launcher owns srv; SIGCHLD-driven restart; process-group reaping on launcher exit |
| **Window pool** (pre-warmed CEF windows for instant tear-off) | Launcher reducer | ⚠ pool sagas exist but every `report_*` from the host is a no-op → pool never warms → tear-off uses the cold `CreateWindowTask` path | ✅ pool warms; tear-off uses pooled fast-path (~50ms vs ~400ms cold) |
| **Instance numbering** ("AgentMux 2", "AgentMux 3") | Launcher state | ❌ all instances default to "AgentMux"; no disambiguation in window title or app_id | ✅ launcher hands out monotonic instance numbers (parity with Windows/macOS) |
| **Durable saga coordination** (window create/destroy, pool refill, instance bookkeeping) | Launcher `saga/` + sqlite log | ❌ host runs ad-hoc; no replay on crash, no LSD-3 `--diag sagas` offline forensics | ✅ same saga durability as Windows |
| **Cold-start splash** (200–600 ms) | Launcher splash module | ❌ blank black 200–600 ms before first paint | ✅ brain-logo splash up in <50 ms after AppRun (workstream B) |
| **`launcher_ipc` no-op detection** | Host | Silent — every `report_*` returns Ok with no side effect | Still silent on disconnect, but reaches the launcher on the happy path |

The first six rows are the macOS-spec gap, copy-pasted to Linux: nothing new about the design — A0 just turns on machinery that's already written and tested on Windows.

---

## 3. Workstream A — bundle the launcher as the Linux entry point

### A0 — PRIORITY SLICE: launcher as AppImage entry point

The minimum slice that closes the architectural gap. No splash UI yet (§B is separate). Net effect: process tree becomes `AppRun → launcher → { srv, agentmux-cef }`, matching Windows.

**Code changes:**

1. **`scripts/build-appimage-linux.sh`** — two edits:

   **(a) Stop renaming the host binary.** Today line 84 does `cp dist/cef/agentmux-cef "$APPDIR/usr/bin/agentmux"`. The launcher's `find_cef_binary` (`agentmux-launcher/src/main.rs:1458-1496`) probes these candidates in order: `agentmux-{VERSION}` → `agentmux-*` dir scan (excludes `agentmux-cef`, `agentmux-srv`, `agentmux-launcher`) → `agentmux-cef-{VERSION}` → `agentmux-cef`. A bare `agentmux` (no dash, no version) matches none of them, so a launcher pointed at the current AppImage layout would fall through to the `agentmux-cef` fallback and fail at `main.rs:203`. The simplest fix: keep the host's cargo name `agentmux-cef` so the existing `agentmux-cef` fallback finds it.
   ```bash
   # Before (line 84):
   cp dist/cef/agentmux-cef "$APPDIR/usr/bin/agentmux"
   # After:
   cp dist/cef/agentmux-cef "$APPDIR/usr/bin/agentmux-cef"
   ```
   Also update the `require dist/cef/agentmux-cef` check (line 64) — already matches the source name, no edit there.

   **(b) Add the launcher copy.** Insert immediately after the host-cp:
   ```bash
   cp dist/cef/agentmux-launcher "$APPDIR/usr/bin/agentmux-launcher"
   ```
   And add a `require dist/cef/agentmux-launcher` next to the existing host `require`.

2. **`scripts/linux-apprun.sh`** — exec the launcher instead of the host, in both `run_normally()` and the FUSE-fallback path. AppImage's `$APPIMAGE` invokes `AppRun`, which now execs the launcher; the user's `.desktop` file still points at the AppImage, end-to-end identical from the user's perspective.
   ```bash
   # Before (line 39):
   exec "$this_dir/usr/bin/agentmux" "$@"
   # After:
   exec "$this_dir/usr/bin/agentmux-launcher" "$@"
   ```
   No env-var exports needed — the launcher resolves siblings out of its own `exe_dir` (the flat layout case in `launcher_main`).

3. **`agentmux-launcher/src/main.rs`** — `launcher_main()` already resolves the host + srv from `exe_dir` when there's no `runtime/` subdir (lines 130-138). With change (1a) above (keeping the host at `agentmux-cef`), `find_cef_binary`'s existing `agentmux-cef` fallback returns the right path and no launcher code change is needed for A0.

   **If we ever want the host renamed to bare `agentmux` instead** (a UX preference), the launcher needs a new `find_cef_binary` candidate that probes `exe_dir.join("agentmux")` before any of the existing ones. Recommend deferring that rename — keep the host at `agentmux-cef` in v1 to land A0 with zero launcher code change.

4. **`scripts/install-linux-desktop.sh`** — currently writes `Exec=$APPIMAGE %F` in `agentmux.desktop`. **No change.**

5. **Single-instance pipe naming** — `agentmux-launcher/src/host_pipe/` uses platform-specific naming:
   - Windows: `\\.\pipe\AgentMux-<dir_hash>`
   - macOS/Linux: `$XDG_RUNTIME_DIR/agentmux/launcher-<dir_hash>.sock` (Unix domain socket)

   The macOS impl uses `tokio::net::UnixListener` which works unmodified on Linux. Verify the cfg gate is `cfg(unix)` not `cfg(target_os = "macos")` before A0 ships.

**Workspace changes:** None. The launcher already builds on Linux (`task build:host:linux` does `cargo build --release -p agentmux-launcher`); just `cp` it into the AppImage.

**Risk register for A0:**

| Risk | Likelihood | Mitigation |
|---|---|---|
| Launcher's tokio runtime + the host's main thread fight over signal handling | Low | macOS/Windows already work; Linux is no different — both children run in their own process |
| Existing `extract-once-cache` AppRun logic (decompresses SquashFS to `$HOME/.local/share/agentmux/extracted/<VERSION>/`) re-execs the script from the extract dir — needs to keep working with the new exec target | Medium | Update the `run_normally()` re-exec at the bottom of AppRun to point to `agentmux-launcher` in BOTH the fast path and the FUSE-fallback path |
| Some users have stale `~/.cache/agentmux/launcher-*.sock` from `task dev` Linux runs that would prevent the AppImage launcher from binding | Low | Launcher already cleans stale sockets via `connect → if ECONNREFUSED → unlink → bind` (Phase B.2). Verify on first packaged run. |
| Setting `xdg_toplevel.app_id` for desktop-integration icon — host does this today; launcher process won't (no GUI window in A0) so no app_id collision | Low | host still wins the app_id race because it's the only client with a window in A0 |

### A.2 Verification

```
# build with the §8.5 + §8.6 stack
task build:host && task build:backend && task build:frontend && task bundle
bash scripts/build-appimage-linux.sh ~/Desktop

# launch + verify process tree
~/Desktop/AgentMux_<VERSION>_amd64.AppImage &
sleep 5
pstree -p $(pgrep -f AgentMux_.*.AppImage)
# Expected:
#   AgentMux_.AppImage(N)─┬─agentmux-launcher(N+1)─┬─agentmux-srv-...(N+2)
#                         │                        └─agentmux(N+3)─┬─...zygote
#                         │                                        └─...renderers
```

- [ ] `pstree` shows `launcher → { srv, host }` (not `AppRun → host`)
- [ ] Killing the launcher (`pkill -f agentmux-launcher`) brings down srv and host within 200ms (process-group reap)
- [ ] Launching a second AppImage instance: second `agentmux-launcher` exits within 100ms; the existing launcher's host gets focus / opens a new window per its single-instance recovery policy
- [ ] `~/.local/state/agentmux/sagas/launcher-<dir_hash>.sqlite` shows window/pool events (matches Windows behavior)
- [ ] `--diag sagas` from a second invocation prints the saga log offline (LSD-3 parity)

### A.3 Out of scope for A0

- Splash UI (workstream B)
- Refactoring `launcher_ipc.rs` to surface errors instead of silently no-opping (separate concern, applies to all platforms)
- Single-instance "forward command-line args to existing instance" semantics — A0 is "second instance returns"; arg-forwarding is a follow-up (parity with Windows is the eventual target, but neither macOS nor Linux ship it in v1)

---

## 4. Workstream B — Linux native splash

Mirrors the macOS spec's workstream-B that got folded into A0; here it's separated because the Linux GUI surface is non-trivial (X11/Wayland) and worth a dedicated review pass.

### B.1 Design — X11 via `x11rb`

**Why X11, not Wayland, in v1:**

1. PR #1261 made `--ozone-platform=x11` the default for the CEF host. On every supported compositor (Mutter/GNOME, KWin/Plasma, Sway/wlroots, …) the host is an X11 client via XWayland. A splash that connects to the same X11 display lives in the same translation layer the host already runs in — one less abstraction, one less behavioral surface.
2. `x11rb` is pure-Rust, has no `libxcb`/`libX11` C deps beyond what every distro ships, and produces a ~150 KB additional binary footprint (launcher is otherwise ~2.5 MB). `softbuffer` for the pixel buffer adds ~80 KB.
3. Wayland-native would mean `wayland-client` + `smithay-client-toolkit` + per-compositor protocol quirks (`xdg_shell` borderless override-redirect equivalents are awkward — the splash conceptually wants `wlr_layer_shell`, which Mutter doesn't implement). That's a deep stack of complexity for the long-tail `AGENTMUX_OZONE_PLATFORM=wayland` users.
4. Users who set `AGENTMUX_OZONE_PLATFORM=wayland` are opting out of the splash in v1 (§B.2.4). They get the existing blank cold-start window. Document this in `--help` and the env-var rationale comment in `app.rs`.

### B.2 `agentmux-launcher/src/splash_linux.rs` — module shape

```rust
//! Native Linux startup splash, owned by the **launcher** — the Linux analogue
//! of `splash_mac.rs` and `splash.rs`. Same look (26,26,31 dark backdrop +
//! pulsing brain bitmap + fade-out on first paint), same dismiss protocol as
//! macOS (AGENTMUX_SPLASH_READY_FILE), implemented via X11 / x11rb.
//!
//! Why X11 (not Wayland) — see SPEC §B.1: PR #1261 made x11 ozone the default
//! for the host, so the splash sharing the X11 display is the one-abstraction
//! choice. AGENTMUX_OZONE_PLATFORM=wayland opts out of the splash in v1.

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::Instant;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;

include!(concat!(env!("OUT_DIR"), "/brain_dims.rs"));      // BRAIN_W, BRAIN_H
static BRAIN_RGBA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/brain_rgba.bin"));

const SPLASH_PADDING: i32 = 12;
const SPLASH_SIZE: u16 = (BRAIN_W + SPLASH_PADDING * 2) as u16;
const BG_R: u8 = 0x1A;
const BG_G: u8 = 0x1A;
const BG_B: u8 = 0x1F;

pub struct Splash {
    inner: Arc<SplashInner>,
}

struct SplashInner {
    // X11 connection, window, gc, drawable. All Send via x11rb.
    // Wrapped in Arc so the dismiss-watcher thread can hold a clone.
}

impl Splash {
    /// Create the splash window, paint frame 0, return immediately.
    /// Spawns a thread that drives the pulse animation + watches the
    /// ready-file. On dismiss, fades out (~160 ms) then destroys.
    ///
    /// Returns `None` (silently) if:
    ///   - $DISPLAY is unset (no X server reachable — wlroots-only setup
    ///     without XWayland, or headless launch)
    ///   - `xcb_connect` fails for any reason
    ///   - The host is configured for Wayland-native ozone
    ///     (AGENTMUX_OZONE_PLATFORM=wayland)
    /// In any of those cases the launcher proceeds without a splash;
    /// the user sees the existing blank cold-start window.
    pub fn show() -> Option<Self> { /* ... */ }

    /// Block the calling thread, pumping the X11 event loop, until the
    /// ready-file appears OR a 5-second deadline (defensive — host
    /// crash means no ready file ever, splash shouldn't be a zombie).
    pub fn run_until_dismissed(self) { /* ... */ }
}
```

### B.2.1 Window setup

- `create_window` with `override_redirect=1` (compositor doesn't decorate or position it — same as Win32's `WS_POPUP | WS_EX_LAYERED`)
- `configure_window` to center on the primary screen via `RANDR` extension query (fallback to root window center if RANDR unavailable)
- Shaped via `SHAPE` extension to a rounded-rect 8 px corner radius (macOS does the equivalent via `NSImage` mask; Win32 via `SetWindowRgn`). Acceptable to skip in v1 — square corners are fine for the v1 cut.
- `EWMH` hints: `_NET_WM_WINDOW_TYPE_SPLASH`, `_NET_WM_STATE_ABOVE`, `_NET_WM_STATE_SKIP_TASKBAR`, `_NET_WM_STATE_SKIP_PAGER`. Mutter respects all four; KWin respects all four; xfwm4 respects three (no `_ABOVE`).

### B.2.2 Painting

- Allocate one `Pixmap` the size of the window.
- Frame each tick (60 Hz via `xcb_poll_for_event` with a 16 ms `xcb_wait_for_event` deadline):
  1. Fill backdrop with `BG_R/G/B`.
  2. Blit `BRAIN_RGBA` at `(SPLASH_PADDING, SPLASH_PADDING)` with current pulse-alpha multiplied per-pixel (SW alpha blend — no Cairo/Skia dep needed for one 128×128 bitmap).
  3. `copy_area` Pixmap → Window. Single round-trip; no flicker.
- Pulse: same `0→1` fade-in over 200 ms then 1.1 Hz sine between 0.73 and 1.0 used by Win32/macOS. Animation thread copy-pasted from `splash.rs::pulse_curve` (shared math, platform-specific only at the paint call).

### B.2.3 Dismiss protocol — reuse macOS's

The macOS spec already shipped `AGENTMUX_SPLASH_READY_FILE` (cross-process file-create signal). Reuse verbatim:

1. Launcher creates a unique path under `$XDG_RUNTIME_DIR/agentmux/splash-ready-<pid>` and exports `AGENTMUX_SPLASH_READY_FILE=<path>` to the host child env.
2. CEF host's `client::mod.rs:on_load_end` already does:
   ```rust
   #[cfg(target_os = "macos")]
   { if let Ok(path) = std::env::var("AGENTMUX_SPLASH_READY_FILE") {
         if !path.is_empty() { let _ = std::fs::write(&path, b"ready"); }
   }}
   ```
   **Add `target_os = "linux"` to the cfg gate** — that's a one-line host change. No new IPC, no new mechanism. (Windows stays on the named-event path.)
3. Splash thread polls the file's existence every 16 ms (cheap stat, fits in the 60 Hz tick).
4. On ready, run the fade-out (alpha 1→0 over 160 ms) then `destroy_window` + `disconnect`.

### B.2.4 Wayland-native opt-out

When `AGENTMUX_OZONE_PLATFORM=wayland` is set, the launcher checks the same env var, skips `Splash::show()`, and proceeds straight to `launcher_main()`. Document the trade-off in `app.rs` adjacent to the existing comment block.

### B.3 `build.rs` — brain bitmap embedding

`agentmux-launcher/build.rs` is currently `#[cfg(target_os = "windows")]`-gated end-to-end — it emits `brain_bgra.bin` + `brain_dims.rs` only on Windows. macOS sidesteps build.rs entirely by doing `include_bytes!("../resources/brain.png")` in `splash_mac.rs:37` and letting NSImage decode the PNG at runtime.

Two viable options for Linux:

**Option 1 — extend `build.rs` with a Linux arm.** Add a `#[cfg(target_os = "linux")]` block that decodes the PNG to raw RGBA8 (no pre-multiplication needed — x11rb's `PutImage` accepts straight RGBA on TrueColor depth-24/32 visuals on every modern X server) and emits `brain_rgba.bin` + `brain_dims.rs`. ~30 lines, shares the existing `png` crate dep.

**Option 2 — runtime decode like macOS.** `splash_linux.rs` does `include_bytes!("../resources/brain.png")` and decodes via the `png` crate at module load. Adds ~50 KB to the launcher binary (png + miniz_oxide), no build.rs change.

Recommend **Option 1** for consistency with the Windows path and to keep the splash thread allocation-free at runtime (no PNG decoder running on the cold-start critical path). The `brain_dims.rs` it emits is also reusable verbatim across Windows + Linux — no new code duplication.

### B.4 Cargo dependencies (new)

```toml
[target.'cfg(target_os = "linux")'.dependencies]
x11rb = { version = "0.13", default-features = false, features = ["xinerama", "randr", "shape"] }
```

Footprint: x11rb adds ~150 KB to the stripped launcher binary. No new C deps — pure Rust. No `softbuffer` needed: the splash is one 128×128 RGBA bitmap and a solid color, simple enough to draw straight into a `PutImage` request.

### B.5 Verification

- [ ] Brain logo appears within 50 ms of `AppRun` start (`perf stat` or just human-eye check — the host takes 200–600 ms to first paint, the splash must beat that)
- [ ] Pulse animation runs smoothly under both Mutter (X11) and Mutter (Wayland → XWayland)
- [ ] Splash disappears within 200 ms of CEF host's first frame
- [ ] `AGENTMUX_OZONE_PLATFORM=wayland` skips the splash (no errors, just no UI)
- [ ] Multi-monitor: splash centers on the **primary** screen, not screen 0 (RANDR query)
- [ ] Killing the host before first paint: splash times out at 5 s and exits the launcher cleanly (no orphan)
- [ ] No new linker errors / dlsym fails in `ldd` output of the launcher binary

---

## 5. Sequencing & risks

```
A0 ────────────────────────────────────────────►  ship in PR #N (~50 LOC + AppRun edit)
│
│  unlocks → window pool + saga + single-instance on Linux
│
├──► B  (splash_linux.rs + brain RGBA + host cfg gate)
│        ~400 LOC, can land separately; depends on A0 only for the env-var plumb
│
└──► A1/A2 (full saga IPC parity if anything diverges from Windows)
         likely no-op — Windows behavior matches verbatim
```

**Risks:**

| Risk | Severity | Mitigation |
|---|---|---|
| AppImage extract-once-cache logic was written assuming the host is the AppRun exec target | Low — script is simple | Update both the fast and FUSE-fallback `run_normally` paths in `linux-apprun.sh`; test cold extract + warm cache |
| `dist/bin/agentmux-srv-{version}-linux.x64` filename convention is host-resolved today; launcher resolution needs the same | Low | `srv_spawner.rs` already has the Linux versioned-name probe — verify |
| Wayland-native users (rare today) lose nothing but lose the splash | Acceptable | Documented in §B.2.4 as v1 scope cut; Wayland-native splash is a follow-up if real users emerge |
| `x11rb` Cargo deps not in the stripped-launcher allowlist for CI | Low | Add to `Cargo.toml` workspace allowlist if one exists; no transitive C deps |
| Splash blocks the launcher's tokio runtime startup if synchronous | High if naive | `Splash::show()` returns immediately; the pump loop runs on its own OS thread (mirrors the Windows pattern, NOT the macOS main-thread pattern — Linux doesn't have AppKit's main-thread requirement) |

---

## 6. Out of scope

- **Wayland-native splash** (smithay / wlr_layer_shell). Tracked as a follow-up; user demand is the gating signal.
- **Single-instance arg forwarding** ("open this file in the existing window"). A0 ships "second instance returns gracefully"; arg-forwarding is a separate spec.
- **`launcher_ipc` error surfacing** — replacing silent no-ops with `Result<()>`. Cross-platform concern; not Linux-specific; defer.
- **Custom-rendered splash content** beyond brain + backdrop (e.g. progress text). Win32 and macOS don't have it; Linux doesn't need it for v1.
- **HiDPI scaling of the splash bitmap.** The brain.png at 128×128 looks fine at 1× and 2×; at 3× it's ~42×42 perceived px, still legible. Re-scaling via RANDR DPI is a follow-up.

---

## 7. Open questions

1. **Should the splash detect EWMH `_NET_WM_FULLSCREEN_MONITORS` / multi-monitor primary correctly on KWin?** KWin's primary-screen reporting via RANDR is reliable; xfwm4 sometimes returns screen 0. Acceptable to land on screen-0 fallback for v1 — most users have one monitor.
2. **Should A0 ship before B, or together?** Recommend **A0 ships first** as its own PR. It's the architectural fix that the macOS spec already laid the IPC for. B is purely UI and can land in a second PR without blocking A0's stability benefit.
3. **`scripts/build-appimage-linux.sh` already strips `libcef.so` and the GL/Vulkan libs.** Should the launcher binary be stripped too? Currently it's already release-built and `lto = true` per the workspace `[profile.release]`; explicit `strip` is redundant but doesn't hurt. Mirror what macOS does (it strips both host and launcher).

---

## 8. Decision

**Approve A0 as the next PR** in the §8.5 → §8.6 → §8.7 → **§8.8 (Linux launcher)** sequence laid out in `SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04`. A0 is small, mechanical, and high-leverage: it activates the window-pool / saga / single-instance machinery that the host's `launcher_ipc.rs` has been calling into-the-void for the entire Linux platform lifetime.

B follows when there's bandwidth — it's a self-contained ~400-LOC X11 module + a one-line cfg gate in the host. Not on the critical path, but the visible polish the macOS spec already established as the standard.
