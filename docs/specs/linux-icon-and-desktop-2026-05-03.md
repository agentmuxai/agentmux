# Linux Taskbar Icon & Desktop Registration

**Date:** 2026-05-03
**Status:** Spec / Proposal
**Repo state:** main @ `6a8727c4`, AgentMux v0.33.591
**Author:** AgentC

---

## Problem

The AgentMux icon shown in the GNOME taskbar/dock (and Activities overview, alt-tab switcher, app launcher) is a generic settings cog instead of the AgentMux logo (stacked rectangles). This affects:

1. **`task dev`** — the development binary launches with no icon registration; whatever stale `.desktop` exists in `~/.local/share/applications/` is what GNOME uses (currently broken, see below).
2. **Production / portable bundle runs** (`dist/cef/agentmux-cef`) — same as `task dev`. There is no install step.
3. **AppImage runs** — there is no current AppImage build (`task package:linux` is `[TODO]`). The legacy AppRun script (`scripts/linux-apprun.sh`) references files that no longer exist in the repo (`AgentMux.desktop`, `AgentMux.png`).

The goal is: **the AgentMux logo appears as the taskbar/dock/launcher icon on Linux for all three run modes, both immediately on first launch and after a reboot, with no manual user action.**

---

## TL;DR

- Source-of-truth icon (`assets/agentmux-logo.png`, 1200×1200) is correct and committed; the **failure is in the registration plumbing** (`.desktop` file and icon-theme install).
- The currently-installed `~/.local/share/applications/agentmux.desktop` was written by a v0.33.42 AppImage and has `Icon=AgentMux` (capital). The installed icon file is `agentmux.png` (lowercase). Linux icon-theme lookup is case-sensitive → no match → cog fallback.
- The registration template (`AgentMux.desktop`) and the multi-size icon source were deleted alongside `scripts/build-appimage.sh` during the Tauri→CEF migration. They need to be re-introduced as committed artifacts.
- Fix surfaces: a committed `.desktop` template, a committed icon source-set (or a generation script), a small `scripts/install-linux-desktop.sh` helper, and Taskfile wiring to invoke it from `task dev`, the (future) `task package:linux`, and `scripts/linux-apprun.sh`.
- Estimated change: 3–4 new files (`.desktop` template, install script, possibly pre-generated icon sizes), ~80 LOC; plus 2–3 Taskfile.yml edits.

---

## Discovery — what's actually broken

### Currently-installed state on this dev machine

| Location | Contents | Issue |
|---|---|---|
| `~/.local/share/applications/agentmux.desktop` | `Icon=AgentMux`, `Exec=/home/snowbark/Desktop/AgentMux_0.33.42_amd64.AppImage`, `StartupWMClass=agentmux-cef` | `Icon=` case mismatch (icon file is `agentmux.png` lowercase); `Exec=` points to a deleted file; `StartupWMClass` doesn't match Wayland app_id (which is `agentmux` per `CLAUDE.md` memory). |
| `~/.local/share/applications/ai.agentmux.app.v0-32-{5,6}.desktop` | `Icon=agentmux`, `StartupWMClass=agentmux` | The OLD format that **was working**. Got regressed somewhere between v0.32.6 and v0.33.42. |
| `~/.local/share/icons/hicolor/{32,128,256}x{32,128,256}/apps/agentmux.png` | Identical bytes to `assets/agentmux-logo.png` (md5 `5725fa6a…`) | **Icon files are correct** — the AgentMux logo is sitting on disk in the right place; only the `.desktop`'s `Icon=` field doesn't reference it correctly. |

### Repo state — what's missing

| Expected | Found | Action |
|---|---|---|
| `agentmux.desktop` template | (none) | **Must commit.** |
| Multi-size icon set or generator | only `assets/agentmux-logo.png` (single 1200×1200), `frontend/logos/agentmux-logo.svg` | Either commit pre-generated PNGs, or commit a generator script using `convert`/`rsvg-convert`. |
| `scripts/build-appimage.sh` | **Deleted** (visible in `git log --diff-filter=D`) | Out of scope (see "Future work"). |
| Linux registration in `Taskfile.yml` | `package:linux` is `[TODO]`; `dev:serve` does not register `.desktop` | Add `install:linux:desktop` task; wire into `dev:serve` and (future) `package:linux`. |
| `set_app_id` / `--class=` in agentmux-cef Rust | **None.** No `set_app_id`, `wm_class`, `set_class`, or `set_program_class` in `agentmux-cef/src/`. | Need to verify what app_id CEF actually emits (see "Open question 1" below). |

### Why this breaks the icon — exact cause-and-effect

1. Frontend launches → CEF emits `xdg_toplevel.set_app_id("agentmux")` (per `CLAUDE.md` memory; needs verification — see Open question 1).
2. GNOME Shell looks for `agentmux.desktop` in `~/.local/share/applications/` and `/usr/share/applications/` — finds the stale 0.33.42 install.
3. GNOME reads `Icon=AgentMux` from the `.desktop`.
4. GNOME consults the icon theme (`hicolor`) for an icon named `AgentMux` — **not found** (the file on disk is `agentmux.png`, lowercase; freedesktop icon names are case-sensitive).
5. GNOME falls back to its generic application icon → user sees a cog.

The fix is therefore **two correct strings** (`.desktop` basename matching app_id, `Icon=` matching the installed icon name) plus **the install actually happening**.

---

## Design

### Files to commit

#### `assets/linux/agentmux.desktop` (new template)

```desktop
[Desktop Entry]
Type=Application
Name=AgentMux
Comment=AgentMux — AI-native terminal multiplexer
Exec=__EXEC__ %F
Icon=agentmux
Categories=Development;TerminalEmulator;
StartupWMClass=agentmux
StartupNotify=true
Terminal=false
```

- Filename basename `agentmux` matches the expected Wayland app_id and X11 wm_class.
- `Icon=agentmux` matches the installed PNG basename.
- `Exec=__EXEC__ %F` is a placeholder substituted at install time (per-mode: AppImage path, dev binary path, or installed binary path). `%F` lets the user open files with AgentMux.
- `StartupWMClass=agentmux` covers the X11 fallback path.
- `StartupNotify=true` reduces "no icon during startup" flicker on GNOME.

#### `assets/linux/icons/hicolor/<size>x<size>/apps/agentmux.png` (new, pre-generated)

Sizes: `16, 32, 48, 64, 128, 256, 512`. Pre-generate from `assets/agentmux-logo.png` with `convert assets/agentmux-logo.png -resize <N>x<N> assets/linux/icons/hicolor/<N>x<N>/apps/agentmux.png`. Committing pre-generated PNGs (rather than generating at install time) keeps the install script dependency-free and survives systems without ImageMagick.

Also commit `assets/linux/icons/hicolor/scalable/apps/agentmux.svg` (copy of `frontend/logos/agentmux-logo.svg`) so HiDPI displays get a vector source.

#### `scripts/install-linux-desktop.sh` (new)

```bash
#!/usr/bin/env bash
# Install agentmux.desktop + hicolor icon files for the current user.
# Idempotent: re-runs are safe; updates Exec= / Icon= if changed.
#
# Usage: bash scripts/install-linux-desktop.sh <exec-path>
#   <exec-path> — absolute path to the binary OR AppImage that the .desktop's
#                 Exec= line should point to.
#
# Removes any pre-existing /home/USER/.local/share/applications/agentmux.desktop
# that has a stale absolute Exec= path or wrong-case Icon= field.

set -euo pipefail
EXEC_PATH="${1:?usage: $0 <exec-path>}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APPS_DIR="$HOME/.local/share/applications"
ICONS_ROOT="$HOME/.local/share/icons/hicolor"

mkdir -p "$APPS_DIR"

# 1. Install icons
for size in 16 32 48 64 128 256 512; do
    dst="$ICONS_ROOT/${size}x${size}/apps/agentmux.png"
    src="$REPO_ROOT/assets/linux/icons/hicolor/${size}x${size}/apps/agentmux.png"
    if [ -f "$src" ]; then
        mkdir -p "$(dirname "$dst")"
        cp -f "$src" "$dst"
    fi
done
svg_src="$REPO_ROOT/assets/linux/icons/hicolor/scalable/apps/agentmux.svg"
if [ -f "$svg_src" ]; then
    mkdir -p "$ICONS_ROOT/scalable/apps"
    cp -f "$svg_src" "$ICONS_ROOT/scalable/apps/agentmux.svg"
fi

# 2. Render .desktop (substitute Exec=)
desktop="$APPS_DIR/agentmux.desktop"
sed "s|__EXEC__|$EXEC_PATH|" \
    "$REPO_ROOT/assets/linux/agentmux.desktop" \
    > "$desktop.tmp" && mv "$desktop.tmp" "$desktop"
chmod 644 "$desktop"

# 3. Refresh caches (best-effort; tools may not exist on minimal systems)
update-desktop-database "$APPS_DIR" 2>/dev/null || true
gtk-update-icon-cache -f "$ICONS_ROOT" 2>/dev/null || true

echo "✓ Installed $desktop with Exec=$EXEC_PATH"
```

#### `Taskfile.yml` wiring

Add a new internal task and call it from `dev:serve`:

```yaml
install:linux:desktop:
    internal: true
    platforms: [linux]
    desc: Install ~/.local/share/applications/agentmux.desktop + hicolor icons.
    cmds:
        - bash scripts/install-linux-desktop.sh "{{.ROOT_DIR}}/dist/cef/agentmux-cef"

dev:serve:
    # ... existing ...
    cmds:
        - task: install:linux:desktop  # NEW — runs once before launch (idempotent)
        - task: build:host
        # ... rest unchanged ...
```

For the `task package:linux` future work (and the existing `scripts/linux-apprun.sh`), the same script is invoked with the AppImage path: `bash scripts/install-linux-desktop.sh "$APPIMAGE"` — replacing the inline icon/desktop logic in `linux-apprun.sh:23-39`.

### Per-run-mode `Exec=` resolution

| Run mode | `Exec=` should be | Set by |
|---|---|---|
| `task dev` | `<repo>/dist/cef-dev/agentmux-cef` (the dev session copy created by `dev:serve`) | `task dev:serve` runs `install:linux:desktop` with that path |
| Direct portable run | `<wherever-bundle-lives>/agentmux-cef` | User runs `bash scripts/install-linux-desktop.sh <path>` once after extracting the bundle |
| AppImage | `$APPIMAGE` env var (set by AppImage runtime) | `scripts/linux-apprun.sh` calls the installer on first run with `$APPIMAGE` |

Each mode writes to the **same** `~/.local/share/applications/agentmux.desktop`, so only the most-recently-launched mode "owns" the launcher entry. This is intentional — at any given moment, the user only has one preferred way to launch AgentMux. (The alternative — distinct `.desktop` per mode — multiplies the GNOME Shell entries and confuses the user.)

### Icon refresh

GNOME Shell reads the icon cache lazily. After `gtk-update-icon-cache` runs, **already-running shell processes may keep showing the old icon** until log-out/log-in. The install script does its best (cache refresh) but the spec acknowledges: a one-time log-out is needed for users upgrading from a stale install. New `task dev` runs after the fix won't need it.

---

## Open questions / verification steps

### 1. What is the actual `xdg_toplevel.app_id` agentmux-cef emits?

`CLAUDE.md` memory asserts it's `"agentmux"`. The repo has no `set_app_id` / `set_class` call in `agentmux-cef/src/`. CEF on Linux/Ozone defaults to the basename of `argv[0]`, which is `agentmux-cef`, **not** `agentmux`. So either:

- (a) The launcher renames/symlinks the binary to `agentmux` before exec (need to check `scripts/linux-apprun.sh:41` — it execs `usr/bin/agentmux`, suggesting the AppImage stage does install the binary as `agentmux`, not `agentmux-cef`), **or**
- (b) The Wayland app_id is actually `agentmux-cef` and `CLAUDE.md` memory is wrong (or describes the AppImage path only), **or**
- (c) CEF passes a `--class=agentmux` switch internally for some reason.

**Action before implementation:** verify the running app's app_id with `dotool` (already in memory) running `wl-info` or via a one-shot Rust call to `Window::application_id()` if CEF exposes it. If app_id is `agentmux-cef`, either:

- Rename the dev binary to `agentmux` in `dev:serve`'s copy step (mirroring the AppImage convention), or
- Pass `--class=agentmux` on the agentmux-cef command line, or
- Use `agentmux-cef.desktop` as the basename and update `Icon=` accordingly.

Whichever choice the verification points to, the spec's plumbing is unaffected — only the basename in `agentmux.desktop` and the install script's destination filename change.

### 2. Should we also commit `agentmux.desktop` to `/usr/share/applications/` for system-wide installs?

Out of scope here. The user-local install (`~/.local/share/applications/`) is sufficient for `task dev` and AppImage runs. A `.deb` / `.rpm` packaging story (which would write to `/usr/share/`) is separate future work.

### 3. Multiple sub-windows showing the same icon

Each browser/window CEF creates inherits the same app_id (Wayland convention is one app_id per process / per `xdg_toplevel`). Sub-windows opened via the status bar should automatically get the same icon. **Implication: no per-window icon override is needed.** Confirm visually after the fix.

### 4. macOS / Windows considerations

This spec is Linux-only. macOS uses `.app` bundle's `Info.plist` + `Icon.icns` (separate work). Windows uses `agentmux-cef/resources/win/agentmux.ico` (already in repo) — no changes needed here.

---

## Implementation steps

1. **Verify app_id** (Open question 1). Run agentmux, capture `xdg_toplevel.set_app_id` value via a Wayland tracer (`WAYLAND_DEBUG=1 ./agentmux-cef 2>&1 | grep set_app_id`) or by reading what CEF passes. Update spec if app_id ≠ "agentmux".
2. **Generate the size set** of icons from `assets/agentmux-logo.png` using `convert` for sizes [16, 32, 48, 64, 128, 256, 512]. Commit under `assets/linux/icons/hicolor/...`. Also copy `frontend/logos/agentmux-logo.svg` to `assets/linux/icons/hicolor/scalable/apps/agentmux.svg`.
3. **Commit the `.desktop` template** at `assets/linux/agentmux.desktop`.
4. **Add `scripts/install-linux-desktop.sh`** (and `chmod +x`).
5. **Refactor `scripts/linux-apprun.sh`** to call the new installer instead of the inline cp/sed (keeps a single source of truth).
6. **Wire `Taskfile.yml`** — add `install:linux:desktop` task, invoke from `dev:serve`.
7. **Migration cleanup of stale state** — the install script's `mv`/overwrite handles the existing broken `agentmux.desktop`. The `ai.agentmux.app.v0-32-{5,6}.desktop` files on this dev machine are orphans pointing to deleted AppImages; they're harmless (don't match app_id) but should be hand-removed by the user (out of repo scope).
8. **Test plan** (next section).

---

## Test plan

For each of the three run modes, on a Linux/Wayland (GNOME) desktop:

- [ ] Verify the installed `~/.local/share/applications/agentmux.desktop` has `Icon=agentmux` (lowercase), `Exec=` matching the run mode's binary path.
- [ ] Verify `~/.local/share/icons/hicolor/256x256/apps/agentmux.png` exists and md5 matches the source `assets/linux/icons/hicolor/256x256/apps/agentmux.png`.
- [ ] Launch agentmux. The taskbar/dock entry shows the AgentMux logo (stacked rectangles), not a cog.
- [ ] Open the Activities overview / app launcher — AgentMux entry shows the logo.
- [ ] Alt-tab through windows — AgentMux thumbnail shows the logo.
- [ ] Right-click the dock icon → "Show details" or similar shows the right Name/Comment.
- [ ] Open a sub-window via the status bar — the sub-window also shows the logo (no per-window override needed; same app_id).
- [ ] Reboot. Re-launch. Icon still shows the logo (not regressed by missed cache refresh).
- [ ] Wipe `~/.local/share/applications/agentmux.desktop` and `~/.local/share/icons/hicolor/256x256/apps/agentmux.png`. Run `task dev` once. Both files reappear; icon shows correctly.

---

## Risks / non-goals

- **GNOME Shell icon-cache TTL:** users upgrading from a broken install may need to log out/in once for GNOME to pick up the new `Icon=` field. Document this in the PR. Acceptable trade-off.
- **No system-wide install** (out of scope; would require root and packaging story).
- **No AppImage build flow** is being re-introduced here — `task package:linux` stays `[TODO]`. The spec only ensures the installer **would work** when called from a future AppImage AppRun.
- **Wayland app_id discrepancy** (Open question 1) is the one item that could change the spec's filenames; it must be resolved before implementation.

---

## File-by-file change summary

**New (committed):**
- `assets/linux/agentmux.desktop` (~12 lines)
- `assets/linux/icons/hicolor/{16,32,48,64,128,256,512}x<N>/apps/agentmux.png` (8 PNG files, generated from `assets/agentmux-logo.png`)
- `assets/linux/icons/hicolor/scalable/apps/agentmux.svg` (copy of `frontend/logos/agentmux-logo.svg`)
- `scripts/install-linux-desktop.sh` (~30 lines)

**Edited:**
- `scripts/linux-apprun.sh` — replace lines 22-40 with a single call to `install-linux-desktop.sh "$APPIMAGE"`.
- `Taskfile.yml` — add `install:linux:desktop` task; call from `dev:serve` once before `build:host`.

**No changes to:**
- frontend / TS / SCSS.
- macOS / Windows packaging.

---

## Postscript — what changed during implementation (2026-05-03)

Three things differed from the original plan:

### 1. App_id was empty, not "agentmux" — required Rust changes

The CLAUDE.md memory note was wrong. `WAYLAND_DEBUG=1` showed `xdg_toplevel.set_app_id("")` — CEF emits an empty string by default because no `WindowDelegate::GetLinuxWindowProperties` was implemented. We added one in `agentmux-cef/src/app.rs`.

But it didn't work the obvious way: cef 146.7.0's `From<CefStringUtf16> for _cef_string_utf16_t` impl (`registry/.../cef-146.7.0+146.0.12/src/string.rs`) silently drops `Clear` variants — the kind `CefString::from("agentmux")` produces. The trait method would set `props.wayland_app_id = CefString::from("agentmux")`, the WrapParamRef writeback would zero it, and CEF would still emit `set_app_id("")`.

**Workaround** (`install_linux_window_properties_override` in `app.rs`): after each `WindowDelegate::new(...)`, override the `get_linux_window_properties` C function pointer in the delegate's struct with our own `extern "C"` shim that uses raw FFI (`cef::sys::cef_string_utf8_to_utf16`) to write directly to the C struct. Bypasses the broken Rust wrapper entirely.

### 2. Build script for the AppImage was added

The spec said `task package:linux` would stay `[TODO]`. The user wanted an AppImage build — added `scripts/build-appimage-linux.sh` and wired it into `Taskfile.yml`. Layout uses `usr/bin/agentmux` (renamed from `agentmux-cef`) with libcef.so + paks colocated; AppRun sets `LD_LIBRARY_PATH` so the dynamic linker finds libcef.so. `.DirIcon` is a real file copy (not a symlink) per the existing CLAUDE.md AppImage memory.

### 3. install-linux-desktop.sh layout-detect

Original spec assumed REPO_ROOT-relative asset paths. Inside an AppImage the script lives at `AppDir/install-linux-desktop.sh` and assets at `AppDir/assets/linux/...` — no `..` parent with an `assets/` dir. Updated the script to try both `<script_dir>/assets/linux` (AppImage layout) and `<script_dir>/../assets/linux` (dev tree) before bailing.

### 4. Brain vs stacked-rectangles icon

`assets/agentmux-logo.png` was the brain logo; the user's intended app icon is the stacked-rectangles in `assets/favicon-300x300.png`. All brain assets were renamed to `*-brain-alternate*` so future agents pick the right source. The hicolor icon set was regenerated from `assets/favicon-300x300.png`.
