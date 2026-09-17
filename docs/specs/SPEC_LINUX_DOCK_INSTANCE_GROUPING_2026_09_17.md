# SPEC: Linux Dock/Taskbar Instance Grouping Fix

**Date:** 2026-09-17
**Status:** Implemented
**Area:** `agentmux-cef/src/app/window_settings.rs` · `scripts/install-linux-desktop.sh` ·
`scripts/linux-apprun.sh` · `scripts/stage-linux-runtime.sh` · `scripts/build-appimage-linux.sh` ·
`scripts/build-deb-linux.sh` · `scripts/build-rpm-linux.sh` · `assets/linux/agentmux.desktop`

---

## 1. Problem

Two independently-running AgentMux instances on Linux — different channels, different
versions, each already fully isolated at the data-dir/socket level per
`SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` — showed up as a single dock
entry with "2 windows" instead of two separate entries. This is purely a desktop-shell
grouping/icon problem; the app's own multi-instance isolation was never at fault (each
process's `InstancePanel`/`launcher_event_bridge` only ever sees its own windows).

## 2. Root cause

Two independent, compounding bugs:

1. **Static app_id.** `write_linux_window_properties()` (`window_settings.rs`) wrote a
   single hardcoded `b"agentmux"` into `wayland_app_id`/`wm_class_class`/`wm_class_name`
   for every window, regardless of build channel or version. GNOME/KWin/sway group
   dock/taskbar entries purely by this string, so every AgentMux process — however
   isolated internally — was indistinguishable to the window manager.

2. **Single shared `.desktop` file.** Even if two instances' windows had been
   distinguishable, `scripts/install-linux-desktop.sh` always installed/overwrote the
   same `~/.local/share/applications/agentmux.desktop`, with a fixed `Exec=` line. The
   last instance to start up would silently repoint the *other* instance's launcher
   entry at itself.

Neither bug is new — the app_id was introduced in PR #669 (2026-05-03) purely to get a
taskbar icon showing at all, before per-channel/per-version parallel instances were a
supported scenario (`SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md`, a month
later). Neither was revisited when that isolation work landed.

(macOS already computes a per-`(channel, version)` `CFBundleIdentifier` —
`scripts/package-macos.sh`, `agentmux-cef/src/macos_compat.rs` — per
`SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md`. This spec brings Linux to parity with
that scheme. Windows has an analogous unfixed `AppUserModelID` gap
(`agentmux-cef/src/lib.rs`) — out of scope here, tracked separately.)

## 3. Design

### 3.1 App id scheme

`agentmux-<channel>-<version>`, matching macOS's `<channel>.<version>` granularity —
computed once per process and cached (`window_settings.rs::linux_app_id()`):

- `channel` resolves the same way `agentmux_common::DataPaths` resolves it for
  `Installed`/`Portable` modes: `AGENTMUX_CHANNEL` env override if set, else the
  compile-time `AGENTMUX_BUILD_CHANNEL_DEFAULT` (default `"stable"`).
- `version` is `CARGO_PKG_VERSION`.

This is deliberately the same resolution order already used for data-dir isolation —
no new channel concept, just reusing the existing one as the window-manager identity
too.

### 3.2 Desktop file per app_id

`install-linux-desktop.sh` now takes an optional second argument, the app_id, and:

- Installs `~/.local/share/applications/<app_id>.desktop` (was always `agentmux.desktop`).
- Substitutes `StartupWMClass=<app_id>` into the template (was a static
  `StartupWMClass=agentmux`; `assets/linux/agentmux.desktop` now carries a
  `__WMCLASS__` placeholder alongside the existing `__EXEC__` one).
- Leaves `Icon=agentmux` and `Name=AgentMux` unchanged — icons are identical across
  builds, and the visible label doesn't need to change, only the WM-matching key and
  filename (so two instances' desktop entries don't overwrite each other).

`scripts/linux-apprun.sh` computes the app_id to pass by reading two new build-time
markers staged by `scripts/stage-linux-runtime.sh` alongside the existing `VERSION`
marker: `usr/share/agentmux/CHANNEL` (written from `AGENTMUX_BUILD_CHANNEL_DEFAULT`,
default `stable`), combined with an `AGENTMUX_CHANNEL` env override check — the same
two-input resolution as the Rust side, so the two can never disagree.

`.deb`/`.rpm` packaging (`build-deb-linux.sh`/`build-rpm-linux.sh`) has no channel
concept and always builds against the `stable` fallback, so their static
`/usr/share/applications/agentmux.desktop` substitutes `agentmux-stable-<version>`
for `StartupWMClass` at build time. The AppImage's own internal top-level
`.desktop` (read by some desktop-integration tools directly, not just the
runtime-installed copy) does the equivalent substitution in
`build-appimage-linux.sh` using the same `AGENTMUX_BUILD_CHANNEL_DEFAULT`/`VERSION`
values baked into that specific build.

### 3.3 Non-goals

- Pruning accumulated per-app_id desktop files/icons over time — same open follow-up
  as the existing per-build data-dir accumulation (CLAUDE.md, "Data isolation is
  per-BUILD for local builds").
- Windows `AppUserModelID` — real, same-shaped bug, tracked separately.

## 4. Files changed

| File | Change |
|---|---|
| `agentmux-cef/src/app/window_settings.rs` | `linux_app_id()` replaces the static `b"agentmux"` |
| `assets/linux/agentmux.desktop` | `StartupWMClass=agentmux` → `StartupWMClass=__WMCLASS__` placeholder |
| `scripts/install-linux-desktop.sh` | Accepts `[app-id]`; installs `<app_id>.desktop`, substitutes `__WMCLASS__` |
| `scripts/linux-apprun.sh` | Resolves the app_id from `CHANNEL`/`VERSION` markers + `AGENTMUX_CHANNEL`, passes it to the installer |
| `scripts/stage-linux-runtime.sh` | Writes the new `usr/share/agentmux/CHANNEL` marker |
| `scripts/build-appimage-linux.sh` | Substitutes `__WMCLASS__` in the AppImage's own internal top-level `.desktop` |
| `scripts/build-deb-linux.sh`, `scripts/build-rpm-linux.sh` | Substitute `__WMCLASS__` with the `stable` fallback app_id |

## 5. Acceptance

- Two AppImages of different channels and/or versions running simultaneously produce
  two distinct `~/.local/share/applications/*.desktop` files and two distinct dock
  entries.
- A single instance's icon/taskbar behavior is unchanged from before (still shows the
  AgentMux icon and label, still launches via the same `Exec=` mechanism).
- `.deb`/`.rpm`/AppImage-internal desktop files never ship a literal `__WMCLASS__`.
