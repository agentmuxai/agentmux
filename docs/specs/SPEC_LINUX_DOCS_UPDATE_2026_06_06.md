# SPEC — Linux documentation catch-up

**Date:** 2026-06-06
**Author:** AgentU
**Status:** Ready to implement
**Scope:** `BUILD.md`, `README.md`, new `docs/linux.md`
**Motivated by:** ~4 major Linux milestones (CEF migration, AppImage launcher, Unix-socket IPC, X11 ozone default, native window drag) shipped since the Linux docs were last updated. Docs still describe the old WebKitGTK/Tauri architecture.

---

## 0. Audit summary — what is wrong today

### BUILD.md

| Location | Wrong | Correct |
|---|---|---|
| Line 53 — Linux prerequisites | `apt install zip libwebkit2gtk-4.1-dev … libayatana-appindicator3-dev librsvg2-dev` | CEF build: `cmake ninja-build zip build-essential` + CEF's own `install-build-deps.sh`. `libwebkit2gtk` is gone — CEF bundles Chromium. |
| Lines 314–316 — "Fix (Linux)" troubleshooting | "missing libwebkit2gtk" fix | Obsolete. CEF has no WebKitGTK dep. Remove. |
| Lines 397–400 — Platform notes Linux | "Uses DEB and AppImage. WebKitGTK required." | CEF runtime bundled in AppImage; no system-level WebKitGTK. Remove that line. |
| Lines 228–253 — Build output tree + component sizes | Windows-only (`dist/cef-dev/`, `.exe`) | Add Linux AppImage tree; update component sizes. |
| Dev workflow step 6 | `./bump-version.sh patch` | `task changeset -- patch "..."` (changesets RFC #857). |

### README.md

| Location | Wrong / missing |
|---|---|
| §Build commands | `task package:linux` correct, but missing context on CEF build prerequisites. |
| Nowhere | X11 ozone default (XWayland), opt-out to native Wayland via `AGENTMUX_OZONE_PLATFORM=wayland`. |
| Nowhere | Launcher is the AppImage entry point (A0, #1286). The host is no longer `exec`'d directly. |
| Nowhere | Linux limitations: no native splash yet; transparency not yet working (CEF Wayland transparency under investigation). |
| Nowhere | `patched-libcef` requirement: window drag, right-click-on-title-bar, and eventual transparency require the `agentmux/7778-drag-rightclick-and-transparency` build of `libcef.so`. Pre-built binaries ship with it; dev builds use `task build:host:linux`. |

### docs/cef-build/build-patched-libcef.md

Out of date — still references CEF 146 (`agentmux/7680-drag-rightclick-and-transparency`) and the old a5af/cef fork. Current state:
- CEF 148 (Chromium 148)
- Branch `agentmuxai/cef@agentmux/7778-drag-rightclick-and-transparency`
- Key fix: `BeginWindowDrag` annotation corrected to `added=14800` (was `added=NEXT`; the NEXT tag caused CppToC type-tag mismatch → silent drag no-op)
- Binding: `AgentU-asaf/cef-rs@agentmux/148-begin-window-drag` (patched `cef-dll-sys`)

---

## 1. BUILD.md changes

### 1.1 Replace Linux prerequisites (lines 49–67)

Replace:
```markdown
#### Linux

1. **Install dependencies** (Debian/Ubuntu):
   ```bash
   sudo apt install zip libwebkit2gtk-4.1-dev \
     build-essential curl wget file libssl-dev \
     libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
   ```
```

With:
```markdown
#### Linux

AgentMux embeds Chromium via CEF — no system WebView2 or WebKitGTK required.

1. **Install build tools** (Debian/Ubuntu):
   ```bash
   sudo apt install cmake ninja-build build-essential curl wget file libssl-dev git zip
   ```
   CMake and Ninja are required by `cef-dll-sys` (builds CEF's C wrapper at compile time).

2. **Install Rust**:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

> **Note — `libcef.so`:** Running on Linux requires a compatible `libcef.so`. `task package:linux` builds the AppImage with the pre-built binary from the CI artifact store. If you need to rebuild libcef for CEF patches (window drag, transparency), see [`docs/cef-build/build-patched-libcef.md`](docs/cef-build/build-patched-libcef.md).
```

### 1.2 Add Linux build output tree to §Architecture

After the Windows `dist/cef-dev/` tree, add:

```markdown
#### Linux (AppImage)

```
~/Desktop/
└── AgentMux_<version>+g<sha>[.dirty].<stamp>_amd64.AppImage   # Self-contained AppImage
    └── usr/bin/
        ├── agentmux-launcher   # AppImage entry point (since A0, #1286)
        ├── agentmux            # CEF host
        ├── agentmux-srv-*.x64  # Rust async backend
        ├── libcef.so           # Chromium runtime (~613 MB stripped)
        └── *.pak / icudtl.dat / …  # Chromium resources
```

The launcher (`agentmux-launcher`) is the AppImage entry point. `AppRun` → `linux-apprun.sh` execs the launcher; the launcher spawns `agentmux-srv` and `agentmux` (the CEF host). On Linux, the launcher provides PR_SET_PDEATHSIG–based process supervision; the full Window/saga coordinator (A1, #1288) drives the same reducer as Windows via Unix-socket IPC.
```

### 1.3 Update component sizes table

Add a Linux row or note:

```markdown
| Component | Windows | Linux |
|-----------|---------|-------|
| Launcher | ~325 KB | ~325 KB |
| Host (agentmux / agentmux-cef) | ~8 MB | ~17 MB (includes `custom-protocol` feature) |
| Backend (agentmux-srv) | ~4 MB | ~4 MB |
| CEF runtime (libcef + paks) | ~160 MB | ~620 MB (libcef.so ~613 MB stripped) |
| **AppImage / ZIP** | ~156 MB | ~220 MB |
```

### 1.4 Remove stale "Fix (Linux)" section

Lines 314–316 reference `libwebkit2gtk-4.1-dev` as the fix for a rendering issue. This no longer applies. Delete the paragraph. If there's a replacement for common CEF startup issues, see §Known issues below.

### 1.5 Update Linux platform notes

Replace (lines 397–400):
```markdown
### Linux

- Uses **DEB** (Debian/Ubuntu) and **AppImage** (universal)
- WebKitGTK required: `libwebkit2gtk-4.1-dev`
```

With:
```markdown
### Linux

- **AppImage** (universal) and **DEB** (Debian/Ubuntu) produced by CI.
- CEF bundles Chromium — no system WebView2/WebKitGTK required.
- Default display backend: **X11 via XWayland** (`--ozone-platform=x11`). Override with `AGENTMUX_OZONE_PLATFORM=wayland` for native Wayland (experimental).
- Window drag, right-click on title-bar, and future transparency features require the patched `libcef.so` (bundled in all release AppImages). See `docs/cef-build/build-patched-libcef.md`.
```

### 1.6 Update dev workflow step 6 (version bump)

Replace:
```bash
./bump-version.sh patch --message "Description of change"
```

With:
```bash
task changeset -- patch "fix(area): description"
# or: task changeset -- minor "feat(area): description"
```

---

## 2. README.md additions

### 2.1 Linux-specific callout in §Getting started / §Build

Add after the `task package:linux` entry:

```markdown
**Linux prerequisites:** `cmake ninja-build build-essential` (for the CEF C wrapper). See [BUILD.md](BUILD.md#linux) for full instructions.
```

### 2.2 Linux limitations section

Add a new `### Linux` subsection under §Platform notes or §Known limitations:

```markdown
### Linux

- **Display:** Runs on XWayland by default (`--ozone-platform=x11`). Set `AGENTMUX_OZONE_PLATFORM=wayland` for native Wayland.
- **Splash screen:** Not yet implemented on Linux (Windows and macOS have native splash screens).
- **Window transparency:** Under active development. The CEF Wayland transparency root cause is identified (views::SolidBackground / kColorPrimaryBackground); a fix is blocked on Mutter surface visibility behavior without an opaque base pixel.
- **Window drag:** Requires the patched `libcef.so`. All release AppImages include it. Dev builds: `task build:host:linux` uses `--features patched-libcef`.
```

---

## 3. Update docs/cef-build/build-patched-libcef.md

The doc currently describes a CEF 146 build on the `a5af/cef` fork. Update:

**§What this libcef contains:** Same two items (BeginWindowDrag + transparency), but note:
- BeginWindowDrag annotation is `added=14800` (was `added=14600` on the 146 branch, briefly `added=NEXT` during the 148 port — the NEXT tag caused `CefWindow_14800_CppToC::Get()` type-tag mismatch, silently dropping all drags).

**§Repos/branches (replace entirely):**
```markdown
- **CEF source:** `agentmuxai/cef@agentmux/7778-drag-rightclick-and-transparency` (base: Chromium 148 / CEF 7778)
- **Rust binding:** `AgentU-asaf/cef-rs@agentmux/148-begin-window-drag` — adds `begin_window_drag` field to `_cef_window_t` in the linux_x86_64 binding
- **Workspace patch:** `[patch.crates-io] cef-dll-sys = { git = "…AgentU-asaf/cef-rs", rev = "515b3ac5…" }` in `Cargo.toml`
```

**§HEAD:** Remove old `5ab41b6` SHA; note current HEAD is `47e94db7a` ("Change BeginWindowDrag annotation from added=NEXT to added=14800").

**§Build steps:** CEF 148 build command is the same (`automate-git.py --branch 7778 …`); update the branch name.

---

## 4. New doc: docs/linux.md

Create a Linux-specific operator guide covering what isn't in BUILD.md or README.md. Content:

```markdown
# Linux — operator guide

## AppImage structure

The AppImage entry point is the **launcher** (since v0.42.x, A0). When you `chmod +x AgentMux_*.AppImage && ./AgentMux_*.AppImage`:

1. `AppRun` → `usr/bin/linux-apprun.sh`
2. `linux-apprun.sh` → `exec usr/bin/agentmux-launcher`
3. Launcher spawns `agentmux-srv` (backend) and `agentmux` (CEF host) as a supervised process group
4. Launcher registers a Unix-socket IPC server; host connects and reports window lifecycle events
5. Host opens the main window and loads the frontend from the embedded `dist/frontend`

## Display server

By default, the app uses **XWayland** (`--ozone-platform=x11`) under any Wayland compositor (Mutter, KWin, etc.). This maximises compat and avoids frame-stall regressions seen with the `wayland` platform on some GPU configurations.

Set `AGENTMUX_OZONE_PLATFORM=wayland` in the environment to use native Wayland (`xdg_toplevel`). This is experimental and not tested against all compositors.

## Window drag

Title-bar drag and floating-pane header drag use `CefWindow::BeginWindowDrag()` — a native AgentMux patch that dispatches `xdg_toplevel.move` (Wayland) or `_NET_WM_MOVERESIZE` (X11/XWayland). The AgentMux-forked `libcef.so` must be present (all release builds include it). Dev builds compile with `--features patched-libcef` automatically via `task build:host:linux`.

## Log access

```bash
muxlog host          # tail the CEF host log ([fe] lines = frontend)
muxlog srv           # tail the backend sidecar log
muxlog host cat      # full host log
cat "$AGENTMUX_LOG_DIR/agentmux-launcher.log"   # launcher log
```

`$AGENTMUX_LOG_DIR` = `~/.agentmux/logs/` for stable channel, `~/.agentmux-dev/logs/` for dev builds.

## Launcher IPC diagnostics

The launcher runs the full reducer + saga coordinator (same as Windows, since v0.42.x A1). Diagnostic output:

```bash
./AgentMux_*.AppImage --diag sagas   # dump the saga journal (cross-platform)
# --diag wrr and --diag srv are Windows-only (Phase 7 will add Unix socket parity)
```

## Remote debugging

The CEF host starts a remote debugger automatically on port 9222 (release) or 9223 (`task dev`).
Open `chrome://inspect` in a Chromium browser and add `localhost:9222` (or `9223`) as a target.

## Single-instance enforcement

The launcher enforces single-instance per (data-dir, version) pair via a Unix domain socket at `$XDG_RUNTIME_DIR/agentmux/<hash16>.sock` (fallback: `/tmp/agentmux-<uid>/<hash16>.sock`). Opening a second instance sends an `open_new_window` command to the running launcher and exits.

## Known Linux limitations (as of v0.42.x)

| Feature | Status |
|---|---|
| Splash screen | Not yet implemented (Windows + macOS have it) |
| Window transparency | Under investigation — CEF Wayland surface visibility without opaque base pixel |
| Native Wayland (non-XWayland) | Experimental; set `AGENTMUX_OZONE_PLATFORM=wayland` |
| Linux deb package | Produced by CI builder (`agentmuxai/agentmux-builder`) only |
| macOS-style owned-window floaters (`transient-for` + `destroy-with-parent`) | Phase B, not yet implemented |
```

---

## 5. Implementation order

1. `BUILD.md` — fix the three active wrong sections (prerequisites, Linux platform notes, version workflow)
2. `README.md` — add Linux prerequisites callout + limitations section
3. `docs/cef-build/build-patched-libcef.md` — update CEF 146 → 148 and fix the annotation history
4. `docs/linux.md` — create new operator guide (content in §4 above, adjust prose as needed)

Items 1–2 are the highest visibility (public-facing). Items 3–4 are internal / maintainer-facing.
