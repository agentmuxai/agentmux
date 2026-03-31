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
| `task dev` | **Development** (CEF host + Vite hot reload) | Yes - hot reload |
| `task cef:package:portable` | **Portable release builds** | No |
| `task dev:tauri` | [DEPRECATED] Tauri dev mode | Yes - hot reload |

**Note:** The Tauri host is deprecated. All development uses the CEF host.

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
- `task build:backend` - Rust sidecar binaries (agentmuxsrv-rs + wsh-rs)
- `task build:frontend` - Frontend only
- `task test` - Run tests
- `task clean` - Clean artifacts

**npm Users:** Can use `npm run <command>` - it delegates to Task.

### After Code Changes

- **TypeScript/SolidJS** - Auto-reloads in `task dev`
- **Rust backend** - `task build:backend` then restart `task dev`
- **Test package** - `task cef:package:portable` then extract ZIP

### Architecture

AgentMux uses a **CEF (Chromium Embedded Framework)** host with a **100% Rust backend**:

- **agentmux-cef** = CEF host app (Rust, IPC bridge, window management, bundled Chromium)
- **agentmux-launcher** = 325 KB launcher exe (sets DLL path, spawns CEF host from `runtime/`)
- **agentmuxsrv-rs** = Rust backend sidecar (auto-spawned, don't run manually)
- **wsh** = Rust shell integration binary (wsh-rs crate, must be versioned correctly)

**Important:** The Tauri host (`src-tauri/`) is deprecated. CEF is the primary host. All Go and Electron code has been removed.

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

### Terminal rendering issues on Linux
**DO NOT enable WebGL as the default renderer on Linux.**
WebKitGTK's WebGL2 implementation has systemic rendering issues — the texture atlas
doesn't redraw after control sequences (`\x08` backspace, `ESC[K` erase-in-line).
This is a WebKitGTK upstream bug (Tauri #6559, WebKit Bug 228268), not an xterm.js bug.
Fix: use the DOM renderer on Linux (default when no renderer addon is loaded).
WebGL is used on macOS/Windows. Users can opt into WebGL on Linux via
`term:disablewebgl=false` if their GPU/driver supports it.
This has regressed multiple times — the Linux check must stay.

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

### CRITICAL: Never Kill AgentMux by Image Name
- **NEVER** use `taskkill //im agentmux-cef.exe` or `taskkill //im agentmuxsrv-rs.x64.exe`
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
