# AgentMux Development Guide

## Repository

- **Name:** AgentMux
- **GitHub:** https://github.com/agentmuxai/agentmux
- **Type:** Desktop application (Chromium-based)
- **Build System:** Task (Taskfile.yml)

---

## Development Workflow

### Commands

| Command | Use When | Auto-Updates? |
|---------|----------|---------------|
| `task dev` | **Development** (Vite hot reload, launcher-in-loop on Windows) | Yes - hot reload |
| `task dev:local` | **Dev with ephemeral version bump** — same as `task dev` but temporarily bumps `package.json`/`Cargo.toml` for this session and restores on Ctrl+C. Use when you want the dev build to advertise a unique version (so you can tell which merge it corresponds to) and when you need to force cargo's incremental cache to recompile after a workspace-version-affecting change. No git mutation. | Yes - hot reload |
| `task dev:standalone` | Debug the no-launcher fallback path (host invoked directly, Phase B features bypassed) | Yes - hot reload |
| `task package` | **Portable builds.** Auto-bumps the patch version (committed) FIRST, every run — so every portable lands on a unique, monotonic version and two builds never collide on version or data dir. The only portable-build command. | No |
| `task package:local` | DEPRECATED — alias of `task package`. The old ephemeral-bump variant kept no memory of the last build number, so it reused the same version every run. | No |

On Windows, `task dev` builds a production-parallel layout in `dist/cef-dev/` (launcher at root, host + DLLs + srv in `runtime/`) and invokes `agentmux-launcher.exe` — so the Job Object, single-instance pipe, saga coordinator, splash, and launcher-spawned srv paths are exercised in dev exactly as in package builds. On Linux/macOS, `task dev` still invokes the host directly (Phase 7 cross-platform parity will integrate the launcher). See `docs/specs/SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13.md`.

**Build versioning — every portable gets a unique version, automatically:** `task package` runs a committed `bump patch` as its mandatory first step, so a portable can *never* be built at a stale or duplicate version — the build system enforces it, not the operator's memory. The committed version IS the durable monotonic counter; each portable advances it for real. CI and the release flow do not call `task package`, so the auto-bump never fights `task release`. This replaced the old `package:local` ephemeral bump, which recomputed `committed-version + 1` every run and so reused the same number forever.

`task dev:local` still does an ephemeral bump — useful only for forcing cargo's incremental cache to recompile after a workspace-version change. Plain `task dev` does not bump and does not need to: the dev data dir is keyed on the git branch (`~/.agentmux/dev/<branch>/`), not the version, so dev instances are already isolated and the version label there is cosmetic.

### Build System

**Primary:** Task (Taskfile.yml)
- All builds go through `task <command>`
- npm scripts are thin wrappers that delegate to Task
- Run `task --list` to see all available commands

**Common Tasks:**
- `task dev` - Development mode (Vite + host)
- `task package` - Portable ZIP (Windows)
- `task build:host` - Build host binary
- `task bundle` - Bundle runtime DLLs
- `task build:backend` - Rust sidecar binary (agentmux-srv)
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
- **Test package** - `task package` then extract ZIP

### Architecture

AgentMux is a **100% Rust** desktop app with a **Chromium-based UI**:

- **agentmux-cef** = Host app (Rust, IPC bridge, window management, bundled Chromium)
- **agentmux-launcher** = launcher exe — owns Job Object J0, named-pipe IPC, single-instance enforcement, saga coordinator, splash, and srv lifecycle; spawns host from `runtime/`. Exercised by `task dev` on Windows (production-parallel layout).
- **agentmux-srv** = Rust backend sidecar (auto-spawned, don't run manually)
- **agentmux-common** = Shared utilities used by all the above

**Note:** There is only one host. Tauri, Go, and Electron code has been removed.

### Multiple Instances Run in Parallel

AgentMux is designed to run multiple instances simultaneously — different versions, dev + portable, or multiple portable copies. Each instance is fully isolated:

- **Separate data dirs:** Each instance uses its own user data directory based on version, so browser state, cookies, and caches never collide.
- **Separate backend sidecars:** Each instance spawns its own `agentmux-srv` on a dynamic port. No port conflicts.
- **Separate binaries:** Portable instances run from their own extracted folder. `task dev` copies to `dist/cef-dev/`. Nothing is shared.
- **Dev mode isolation:** `AGENTMUX_DEV=1` → data dir `~/.agentmux-dev` (separate from `~/.agentmux`).

This means:
- You can test v0.33.14 while v0.33.13 is still running.
- `task dev` is always safe alongside a running portable instance.
- **NEVER kill by image name** (`taskkill //im agentmux-cef.exe`) — it kills ALL instances. Always kill by PID.

### Widgets

Widgets are defined in `agentmux-srv/src/config/widgets.json`. These are the **only** widget types — do not invent or reference widgets that don't exist here.

The widget bar's visibility logic is in `frontend/app/window/action-widgets.tsx`: pinned widgets (`"display:pinned": true`) appear directly in the bar; everything else lives in the **More** dropdown. Both tiers are user-facing. By default every surfaced widget is pinned. Their text labels collapse to icon-only automatically when the title bar is too narrow (and the manual `widget:icononly` setting can force icon-only at any width).

| Widget Key | View | Label | Tier |
|------------|------|-------|------|
| `defwidget@agent` | `agent` | Agent | Pinned |
| `defwidget@browser` | `browser` | Browser | Pinned |
| `defwidget@terminal` | `term` | Terminal | Pinned |
| `defwidget@sysinfo` | `sysinfo` | Sysinfo | Pinned |
| `defwidget@editor` | `editor` | Editor | Pinned |
| `defwidget@drone` | `drone` | Drone | Pinned |
| `defwidget@help` | `help` | Help | Pinned |
| `defwidget@swarm` | `swarm` | Swarm | Pinned |
| `defwidget@warden` | `warden` | Warden | Pinned |

### Not widgets

These views exist in the codebase but are **not** widget-bar entries — do not describe them as widgets to users:

| Surface | How it's reached |
|---|---|
| **Identity** | Tab inside an Agent pane (cog → settings panel → Identity tab). The `view: "identity"` registration and `IdentityPaneViewModel` exist for `pane.open` RPC and right-click menu paths; no widget-bar entry. |
| **Memory** | Tab inside an Agent pane (cog → settings panel → Memory tab). Same shape as Identity — view registered for programmatic access only. Replaces the old Forge concept. The `block.tsx` migration shim still redirects `view: "forge"` blocks to `view: "agent"` for backward compatibility. |
| **Settings** | Hamburger menu (≡) in the top tab bar → Settings. Opens `settings.json` in the user's default editor. |
| **DevTools** | Hamburger menu (≡) in the top tab bar → Dev Tools. Toggles Chromium DevTools — does not open a pane. Was a `defwidget@devtools` widget-bar entry until PR #936. |
| **Subagent** | Spawned by clicking a sub-agent in the Swarm pane's overview. Not a top-level pane type the user opens directly. |

---

## Log Access

All logs land in `~/.agentmux/logs/` (`$AGENTMUX_LOG_DIR` in terminals). Pointer files resolve the current filename.

| What | Command |
|------|---------|
| Tail host log | `muxlog host` |
| Tail sidecar log | `muxlog srv` |
| Frontend logs | `muxlog host '\[fe\]'` |
| Memory heartbeat | `muxlog host mem_heartbeat` |
| Full host log | `muxlog host cat` |
| Launcher log | `cat "$AGENTMUX_LOG_DIR/agentmux-launcher.log"` |

Works identically across `task dev`, portable, and install builds. Logs auto-rotate daily and are retained for 7 days.

---

## Version Management

**As of RFC #857 Phase 2, feature PRs use the changesets workflow — do NOT run `bump patch` in feature PRs.** Version bumps happen in dedicated release PRs that consume pending changesets.

### Feature PR workflow

Add a changeset describing your change:

```bash
task changeset -- patch "fix(auth): short description"
# OR: task changeset -- minor "feat(...): description"
# OR: task changeset -- major "breaking change description"
```

This creates `.changesets/<unix-ts>-<slug>.md`. Commit it alongside your code changes. **Do not bump `package.json` or any Cargo.toml** — the release step owns that.

The conflict surface is now zero version files per feature PR: agents committing in parallel get unique filenames automatically.

### Release PR workflow (separate, periodic)

```bash
task release            # consume all .changesets/, bump, update history (no commit yet)
git diff --staged       # review what would land
git commit -m "chore: release v<X.Y.Z>"
git push -u origin agenta/release-vX.Y.Z
```

The release script picks the highest bump type across pending changesets (major > minor > patch), runs `scripts/bump-wrapper.sh`, appends to `VERSION_HISTORY.md`, and deletes the consumed changesets.

### Background

`@a5af/bump-cli` is still installed and used internally by the release script. The `.bump.json` config now targets only the workspace root (`Cargo.toml` + `package.json` + lockfiles) thanks to Phase 1's workspace-version-inheritance — see `docs/specs/SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md`.

If you absolutely need to manually bump (e.g. rebuilding tooling locally), `bump patch -m "..." --commit` still works — but **don't push it in a feature PR**.

### Release consistency invariant (reagent gate)

In every commit, these MUST all equal the same version:

- `VERSION_HISTORY.md`'s top `## X.Y.Z` section
- `package.json.version`
- `Cargo.toml [workspace.package].version`
- `Cargo.lock`'s workspace-member versions (e.g. `agentmux-cef`)
- `package-lock.json`'s root `version`

**reagent: when reviewing any PR that touches `VERSION_HISTORY.md`, `package.json`, or `Cargo.toml`, check this invariant and return `CHANGES_REQUESTED` on a mismatch.** A `chore: release vX.Y.Z` PR whose `VERSION_HISTORY` head ≠ `package.json` version is an automatic block.

This invariant is also enforced deterministically by `scripts/release.sh` (re-reads all five locations after the bump and fails loudly if any disagrees). reagent is the safety net for PRs that don't come from `task release`.

History: `docs/retro/retro-release-version-desync-2026-05-22.md` — PR #964 silently shipped 0.38.0 with `package.json` stranded at 0.37.2 because bump-cli skipped the file and nothing checked.

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
task build:backend        # Backend server (agentmux-srv)
task build:backend:rust   # Same (explicit platform target)
```

### Frontend (TypeScript/SolidJS)
```bash
npm run build:dev    # Development build
npm run build:prod   # Production build
```

### Package Release
```bash
task build:host     # Build host binary
task bundle         # Bundle runtime DLLs
task package        # Portable ZIP (Windows)
```

---

## Common Issues

### Title bar shows wrong version
Ensure `frontend/app-init.ts` uses `getApi().getAboutModalDetails().version`

### Build Fails After Clean
`dist/schema/` is wiped by `task clean` but automatically recreated by the
`copy:schema` dependency in `dev`, `start`, `quickdev`, and `package` tasks.


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
- **NEVER** use `taskkill //im agentmux-cef.exe` or `taskkill //im agentmux-srv.x64.exe`
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
