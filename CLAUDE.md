# AgentMux Development Guide

## Repository

- **Name:** AgentMux
- **GitHub:** https://github.com/agentmuxai/agentmux
- **Type:** CEF desktop application (Tauri host is deprecated)
- **Build System:** Task (Taskfile.yml)

---

## Development Workflow

### Commands

| Command | Use When | Auto-Updates? |
|---------|----------|---------------|
| `task dev` | **Development** (CEF host + Vite hot reload) | Yes - hot reload |
| `task cef:package:portable` | **Portable release builds** | No |
**Note:** The Tauri host (`src-tauri/`) is deprecated. All development uses the CEF host. Do not reference Tauri APIs, Tauri identifiers, or Tauri commands in new code.

### Build System

**Primary:** Task (Taskfile.yml)
- All builds go through `task <command>`
- npm scripts are thin wrappers that delegate to Task
- Run `task --list` to see all available commands

**Common Tasks:**
- `task dev` - Development mode (CEF + Vite)
- `task cef:build` - Build CEF host binary
- `task cef:bundle` - Bundle CEF runtime DLLs
- `task cef:package:portable` - Portable ZIP
- `task build:backend` - Rust sidecar binaries (agentmux-srv + agentmux-wsh)
- `task build:frontend` - Frontend only
- `task test` - Run tests
- `task clean` - Clean artifacts

**npm Users:** Can use `npm run <command>` - it delegates to Task.

### Build Prerequisites

CMake and Ninja are required for `cef-dll-sys` (builds CEF's C wrapper). Both must be on PATH.

| Platform | CMake | Ninja |
|----------|-------|-------|
| **Windows** | Ships with Visual Studio | Copy from VS: `cp "/c/Program Files/Microsoft Visual Studio/*/Community/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja/ninja.exe" /c/Systems/bin/` |
| **macOS** | `brew install cmake` | `brew install ninja` |
| **Linux** | `apt install cmake` | `apt install ninja-build` |

On this dev machine, Ninja is at `/c/Systems/bin/ninja.exe` (copied from VS 2022). If `cargo build` fails with "CMake was unable to find a build program corresponding to Ninja", verify `ninja --version` works.

### After Code Changes

- **TypeScript/SolidJS** - Auto-reloads in `task dev`
- **Rust backend** - `task build:backend` then restart `task dev`
- **Test package** - `task cef:package:portable` then extract ZIP

### Architecture

AgentMux uses a **CEF (Chromium Embedded Framework)** host with a **100% Rust backend**:

- **agentmux-cef** = CEF host app (Rust, IPC bridge, window management, bundled Chromium)
- **agentmux-launcher** = 325 KB launcher exe (sets DLL path, spawns CEF host from `runtime/`)
- **agentmux-srv** = Rust backend sidecar (auto-spawned, don't run manually)
- **agentmux-wsh** = Rust shell integration binary (must be versioned correctly)

**Important:** The Tauri host (`src-tauri/`) is deprecated and must not be used. CEF is the only active host. All Go and Electron code has been removed.

### Multiple Instances Run in Parallel

AgentMux is designed to run multiple instances simultaneously — different versions, dev + portable, or multiple portable copies. Each instance is fully isolated:

- **Separate CEF data dirs:** Each instance uses its own CEF user data directory based on version, so browser state, cookies, and caches never collide.
- **Separate backend sidecars:** Each instance spawns its own `agentmux-srv` on a dynamic port. No port conflicts.
- **Separate binaries:** Portable instances run from their own extracted folder. `task dev` copies to `dist/cef-dev/`. Nothing is shared.
- **Dev mode isolation:** `AGENTMUX_DEV=1` → data dir `~/.agentmux-dev` (separate from `~/.agentmux`).

This means:
- You can test v0.33.14 while v0.33.13 is still running.
- `task dev` is always safe alongside a running portable instance.
- **NEVER kill by image name** (`taskkill //im agentmux-cef.exe`) — it kills ALL instances. Always kill by PID.

### Widgets

Widgets are defined in `agentmux-srv/src/config/widgets.json`. These are the **only** widget types — do not invent or reference widgets that don't exist here.

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

This updates: `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `Cargo.lock`, `src-tauri/tauri.conf.json`, `agentmux-srv/Cargo.toml`, `agentmux-wsh/Cargo.toml`, `VERSION_HISTORY.md`

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

### Tauri Version Management (deprecated — legacy only)

Tauri code in `src-tauri/` is deprecated. These scripts exist only if legacy Tauri files need updating:
`./scripts/verify-tauri-versions.sh`, `./scripts/update-tauri.sh`. Do not use for new work.

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

### Frontend (TypeScript/SolidJS)
```bash
npm run build:dev    # Development build
npm run build:prod   # Production build
```

### Package Release (CEF)
```bash
task cef:build              # Build CEF host binary
task cef:bundle             # Bundle CEF runtime DLLs
task cef:package:portable   # Portable ZIP (Windows)
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

### Terminal rendering issues on Linux (Tauri-era, may not apply to CEF)
**DO NOT enable WebGL as the default renderer on Linux.**
WebKitGTK's WebGL2 had systemic rendering issues under the old Tauri host.
CEF bundles its own Chromium so this may be resolved, but the Linux check should
stay until verified on CEF Linux builds.

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
The Wayland `xdg_toplevel.app_id` is `"agentmux"` (the binary name). GNOME matches
the running window to `agentmux.desktop` only. Only `agentmux.desktop` is needed.

### CRITICAL: Never Kill AgentMux by Image Name
- **NEVER** use `taskkill //im agentmux-cef.exe` or `taskkill //im agentmux-srv.exe`
- Multiple AgentMux instances (portable, dev, different versions) share the same binary names
- Killing by image name kills ALL instances, including the one you are running inside of
- **Always kill by PID:** `taskkill /PID <pid> /F`
- If you need to find the PID: `tasklist | grep agentmux` then kill the specific PID
- `task dev` handles its own lifecycle — you should NEVER need to manually kill AgentMux processes

### Port Conflicts
- Dev server port: 5173 (Vite) + backend port (varies)
- Check: `netstat -ano | grep :5173`
- Kill: `taskkill /PID <pid> /F` (Windows)

---

## Reference

- **Project Docs:** `./README.md`, `./VERSION_HISTORY.md`
- **Build Guide:** `./BUILD.md`
