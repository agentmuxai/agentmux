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

`.deb`/`.rpm` packaging (`build-deb-linux.sh`/`build-rpm-linux.sh`) and the
AppImage's own internal top-level `.desktop` (read by some desktop-integration
tools directly, not just the runtime-installed copy, `build-appimage-linux.sh`)
all substitute `StartupWMClass` at build time from
`${AGENTMUX_BUILD_CHANNEL_DEFAULT:-stable}` and `VERSION` — the same two inputs
`window_settings.rs::linux_app_id()` resolves at compile time. This matters
because `task package:linux:deb`/`:rpm` (like `task package:linux`) export
`AGENTMUX_BUILD_CHANNEL_DEFAULT` to a real per-build `local-*` channel before
compiling (`package-linux.sh`) — only the `package:release:linux:*` variants
(`RELEASE_CHANNEL=stable`) actually bake `stable`. An earlier revision of this
paragraph assumed `.deb`/`.rpm` builds never set the channel env var and
hardcoded `agentmux-stable-<version>` unconditionally; ReAgent caught that
this desyncs `StartupWMClass` from the real app_id for a local `.deb`/`.rpm`
build, reintroducing the exact bug this spec fixes for that one packaging
path (PR #3323).

### 3.3 `task dev` / `task dev:standalone`

These invoke `install-linux-desktop.sh` directly (bypassing `linux-apprun.sh`
entirely — there's no AppImage, no staged `CHANNEL`/`VERSION` markers), and no
packaging script sets `AGENTMUX_BUILD_CHANNEL_DEFAULT` before `task dev`'s build
steps compile the binary. `linux_app_id()` itself never consults `RuntimeMode` —
it only checks `AGENTMUX_CHANNEL`, else the compile-time `BUILD_CHANNEL_DEFAULT`
(`agentmux-common/src/runtime_mode.rs`'s dev-mode auto-detection, which *does*
walk up from the exe path to find `.git` and hash the canonicalized clone root to
produce `dev-<branch>[-<clone-id>]`, is what the app's *data-dir* resolution
uses — a separate code path `linux_app_id()` doesn't call at all). So absent a
fix, a `task dev` binary would advertise the generic `agentmux-stable-<version>`
(the same string a real installed `stable` release advertises) rather than
anything dev/branch-specific — exactly the kind of collision this whole spec
exists to prevent, just reached a different way for `task dev`. Rather than
teaching `linux_app_id()` to replicate the data-dir side's git-branch/clone-hash
detection just for this one launch path, `Taskfile.yml`'s Linux
`dev:serve`/`dev:standalone:serve` blocks compute a simpler `dev-<branch-slug>`
string themselves and `export AGENTMUX_CHANNEL` with it before the later
launcher/host exec in the same script — `linux_app_id()` already checks that env
var first (same override every packaging path uses), so setting it here makes
both sides agree by construction rather than by parallel re-implementation. This
is scoped to the WM app_id only: dev mode's own data-dir resolution intentionally
ignores `AGENTMUX_CHANNEL` (`agentmux-common/src/data_paths.rs`), so it doesn't
change which data dir a dev instance uses. Caught by ReAgent's second review pass
on PR #3323 (a third pass then caught this paragraph itself incorrectly
describing `linux_app_id()` as consulting `RuntimeMode`) — the first
version of this PR only wired up the packaging-script paths (3.1/3.2) and left
`task dev` mismatched, which would have broken icon-matching for the single most
common development workflow.

### 3.4 Non-goals

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
| `scripts/build-deb-linux.sh`, `scripts/build-rpm-linux.sh` | Substitute `__WMCLASS__` from `${AGENTMUX_BUILD_CHANNEL_DEFAULT:-stable}` (the same fallback the compiled binary itself uses) |
| `Taskfile.yml` | `dev:serve`/`dev:standalone:serve` (Linux): export `AGENTMUX_CHANNEL=dev-<branch>` and pass the matching app_id to the installer |

## 5. Acceptance

- Two AppImages of different channels and/or versions running simultaneously produce
  two distinct `~/.local/share/applications/*.desktop` files and two distinct dock
  entries.
- A single instance's icon/taskbar behavior is unchanged from before (still shows the
  AgentMux icon and label, still launches via the same `Exec=` mechanism).
- `task dev` on Linux installs a `.desktop` whose `StartupWMClass` matches the
  running dev binary's actual `wayland_app_id`/`WM_CLASS`.
- `.deb`/`.rpm`/AppImage-internal desktop files never ship a literal `__WMCLASS__`.
