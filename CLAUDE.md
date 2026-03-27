# AgentMux Development Guide

## Repository

- **Name:** AgentMux
- **GitHub:** https://github.com/agentmuxai/agentmux
- **Type:** Tauri v2 terminal application
- **Build System:** Task (Taskfile.yml)

---

## Development Workflow

### Commands

| Command | Use When | Auto-Updates? |
|---------|----------|---------------|
| `task dev` | **Development** (normal work) | Yes - hot reload |
| `task start` | Standalone testing (rare) | No |
| `task package` | **Final release builds ONLY** | No |

**Note:** Never launch from `make/` during development - it's stale.

### Build System

**Primary:** Task (Taskfile.yml)
- All builds go through `task <command>`
- npm scripts are thin wrappers that delegate to Task
- Run `task --list` to see all available commands

**Common Tasks:**
- `task dev` - Development mode
- `task package` - Production installer
- `task package:portable` - Portable ZIP
- `task build:backend` - Rust binaries (agentmuxsrv-rs + wsh-rs)
- `task build:frontend` - Frontend only
- `task test` - Run tests
- `task clean` - Clean artifacts

**npm Users:** Can use `npm run <command>` - it delegates to Task.

### After Code Changes

- **TypeScript/React** - Auto-reloads in `task dev`
- **Rust backend** - `task build:backend` then restart `task dev`
- **Test package** - `task package` then extract/install artifact

### Architecture

AgentMux is built on **Tauri v2** with a **100% Rust backend**:

- **agentmux** = Tauri app (Rust + single webview)
- **agentmuxsrv-rs** = Rust backend sidecar (auto-spawned, don't run manually)
- **wsh** = Rust shell integration binary (wsh-rs crate, must be versioned correctly)

**Important:** All Go and Electron code has been removed. Only Rust + Tauri is supported.

### Widgets

Widgets are defined in `agentmuxsrv-rs/src/config/widgets.json`. These are the **only** widget types — do not invent or reference widgets that don't exist here.

| Widget Key | View | Label | Opens in Pane? |
|------------|------|-------|----------------|
| `defwidget@agent` | `agent` | agent | Yes |
| `defwidget@forge` | `forge` | forge | Yes |
| `defwidget@identity` | `identity` | identity | Yes |
| `defwidget@swarm` | `swarm` | swarm | Yes (hidden by default) |
| `defwidget@terminal` | `term` | terminal | Yes |
| `defwidget@sysinfo` | `sysinfo` | sysinfo | Yes |
| `defwidget@help` | `help` | help | Yes |
| `defwidget@settings` | `settings` | settings | No — opens external editor |
| `defwidget@devtools` | `devtools` | devtools | No — toggles browser inspector |

---

## Version Management

**CRITICAL:** Always use `@a5af/bump-cli` - never manually edit version numbers.

### Install bump-cli (one-time)

```bash
# Configure npm for @a5af GitHub Packages (requires GITHUB_TOKEN with read:packages)
echo "@a5af:registry=https://npm.pkg.github.com" >> ~/.npmrc
echo "//npm.pkg.github.com/:_authToken=${GITHUB_TOKEN}" >> ~/.npmrc
npm install -g @a5af/bump-cli
```

### Mandatory Workflow

**Step 1: Bump version** (updates ALL files automatically via `.bump.json`)
```bash
bump patch -m "Description"
# OR: bump minor / bump major / bump 1.2.3
```

This updates: `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `Cargo.lock`, `src-tauri/tauri.conf.json`, `agentmuxsrv-rs/Cargo.toml`, `wsh-rs/Cargo.toml`, `VERSION_HISTORY.md`

**Step 2: Verify consistency**
```bash
bump verify
```

**Step 3: Rebuild binaries**
```bash
task build:backend
```

**Step 4: Commit and push**
```bash
# bump --commit stages and commits all version files automatically:
bump patch -m "Description" --commit
git push origin <branch>
```

### Tauri Version Management

Tauri versions MUST be synchronized across all packages to prevent build failures.

**Verify before building:**
```bash
./scripts/verify-tauri-versions.sh
```

**Update Tauri:**
```bash
./scripts/update-tauri.sh 2.11.0           # Core packages only
./scripts/update-tauri.sh 2.11.0 --plugins  # Core + plugins
```

**Version Pinning:** package.json uses exact versions (no `^`), Cargo.toml uses `=MAJOR.MINOR` range.

---

## Git Workflow

```bash
# Create feature branch
git checkout -b feature-name

# Make changes, commit
git commit -m "feat: description"

# Push to remote
git push -u origin feature-name

# Create PR via GitHub
gh pr create --title "Feature" --body "Description"
```

---

## Testing

```bash
npm test                       # Run all tests
npm test -- app.e2e.test.ts    # Run e2e tests
npm run coverage               # Generate coverage
```

---

## Build System

### Backend (Rust)
```bash
task build:backend        # All Rust binaries
task build:backend:rust   # Backend server only
task build:wsh            # wsh only
```

### Frontend (TypeScript/React)
```bash
npm run build:dev    # Development build
npm run build:prod   # Production build
```

### Package Release
```bash
task package             # Distributable package
task package:portable    # Portable ZIP (Windows)
```

---

## Common Issues

### wsh binary not found
Version mismatch between package.json and built binaries.
```bash
task build:backend          # Rebuild
ls -lh dist/bin/wsh-*       # Verify
```

### Title bar shows wrong version
Ensure `frontend/wave.ts` uses `getApi().getAboutModalDetails().version`

### Build Fails After Clean
`dist/schema/` is wiped by `task clean` but automatically recreated by the
`copy:schema` dependency in `dev`, `start`, `quickdev`, and `package` tasks.

### Backspace broken in terminal on Linux (WebGL renderer)
**DO NOT remove the Linux Canvas renderer override in `termwrap.ts:loadRendererAddon`.**
xterm.js's WebGL renderer does not correctly handle control sequences (`\x08` backspace,
`ESC[K` erase-in-line) on WebKitGTK — the PTY round-trip is correct but WebGL fails to
display the result. Fix: force Canvas renderer on Linux (`PLATFORM === PlatformLinux`
check at the top of `loadRendererAddon`). WebGL is still used on macOS/Windows.
This has regressed multiple times — the check must stay.

### AppImage shows cog/gear icon instead of app icon
`appimagetool` creates `.DirIcon` inside the AppImage as an **absolute symlink** to the
build machine's AppDir path. The symlink is broken on any other machine, so Nautilus falls
back to a generic icon.

**Fix** (already applied in `Taskfile.yml` package task): the `.DirIcon` symlink is replaced
with a real file copy of `AgentMux.png` before `appimagetool` runs. If the icon regresses,
verify with:
```bash
./AgentMux_*.AppImage --appimage-extract .DirIcon
ls -la squashfs-root/.DirIcon   # must be a regular file, not a symlink
```
Also clear Nautilus's thumbnail cache if the old icon was cached: `rm -rf ~/.cache/thumbnails/`

### Wayland app_id and desktop file matching
The Wayland `xdg_toplevel.app_id` is `"agentmux"` (the binary name), **not** the Tauri
`identifier` field from `tauri.conf.json`. GNOME matches the running window to
`agentmux.desktop` only. The old code incorrectly registered a versioned
`ai.agentmux.app.vX-Y-Z.desktop` which was never matched. Only `agentmux.desktop` is needed.

### Port Conflicts
- Dev server port: 1420 (Vite) + backend port (varies)
- Check: `netstat -ano | grep :1420`
- Kill: `taskkill /PID <pid> /F` (Windows)

---

## Reference

- **Project Docs:** `./README.md`, `./VERSION_HISTORY.md`
- **Build Guide:** `./BUILD.md`
