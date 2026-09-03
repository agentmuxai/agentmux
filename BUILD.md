# Building AgentMux

These instructions cover setting up dependencies and building AgentMux from source on Windows, macOS, and Linux.

**Architecture:** AgentMux is a **Chromium-based desktop app** with a **100% Rust backend**.

---

## Prerequisites

### Required Tools

| Tool | Version | Purpose |
|------|---------|---------|
| **Node.js** | v24 LTS | Frontend build (SolidJS/Vite) |
| **Rust** | 1.77+ | Backend (agentmux-srv) + Host (agentmux-cef) |
| **Task** | Latest | Build orchestration |
| **CMake** | 3.20+ | CEF native build (cef-dll-sys) |
| **Ninja** | 1.10+ | CEF native build (cef-dll-sys) |

> **Note:** Go and Zig are no longer required. The backend is 100% Rust since v0.31.0.

### Platform-Specific Setup

#### Windows

1. **Install Rust** (includes cargo):
   ```powershell
   # Download from https://rustup.rs/
   rustup-init.exe
   ```

2. **Install Visual Studio Build Tools** (required by Rust):
   - Download: https://visualstudio.microsoft.com/visual-cpp-build-tools/
   - Install: "Desktop development with C++"

3. No WebView2 install is needed — AgentMux embeds Chromium via CEF and bundles its own runtime.

#### macOS

1. **Install Xcode Command Line Tools**:
   ```bash
   xcode-select --install
   ```

2. **Install Rust**:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

#### Linux

AgentMux embeds Chromium via CEF — no system WebKitGTK or WebView2 required.

1. **Install build tools** (Debian/Ubuntu):
   ```bash
   sudo apt install cmake ninja-build build-essential curl wget file libssl-dev git zip \
     libwayland-dev libxkbcommon-dev libgtk-3-dev \
     libglib2.0-dev libpango1.0-dev libcairo2-dev \
     libgdk-pixbuf2.0-dev libatk1.0-dev
   ```
   CMake and Ninja are required by `cef-dll-sys`, which builds CEF's C wrapper at compile time.
   The remaining packages are CEF/GTK runtime dev headers — see `.github/workflows/build-linux.yml`
   for the list this repo's own CI installs; keep this list in sync with it.

2. **Install Rust**:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

> **Note — `libcef.so`:** Running on Linux requires a compatible `libcef.so`. `task package:linux` downloads the pre-built binary automatically. If you need to rebuild libcef from source (for CEF patches), see [`docs/cef-build/build-patched-libcef.md`](docs/cef-build/build-patched-libcef.md).

### Install Task

Task is the primary build orchestrator:

```bash
# macOS
brew install go-task/tap/go-task

# Linux
sudo snap install task --classic

# Windows (PowerShell)
winget install Task.Task
```

See full instructions: https://taskfile.dev/installation/

---

## Clone the Repository

```bash
git clone https://github.com/agentmuxai/agentmux.git
cd agentmux
```

---

## Install Dependencies

First-time setup after cloning:

```bash
npm install
```

If you have build issues later, run `npm install` again to refresh dependencies.

`npm install` also runs the `prepare` script which installs the project's git
hooks (`.githooks/pre-commit`) by setting `core.hooksPath`. The pre-commit
hook runs `git diff --check` to block commits that contain unresolved merge
conflict markers or whitespace errors in staged changes. To bypass for one
commit (e.g. authoring prose that legitimately contains `<<<<<<< HEAD` as
example text), use `git commit --no-verify`. See spec
`docs/specs/SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md` §5 Phase 0
for rationale.

---

## Build Commands

### Development (Hot Reload)

**This is the recommended way to run AgentMux during development:**

```bash
task dev
```

Features:
- Frontend hot reload (Solid Refresh via Vite)
- DevTools available (Ctrl+Shift+I)

Rust changes are **not** auto-rebuilt — run `task build:backend` (or `task build:host`) and restart `task dev`, see below.

**Important:** Always use `task dev` for development.

---

### Backend Rebuild

If you modify Rust backend code (`agentmux-srv/src/`):

```bash
# Rebuild Rust binary
task build:backend

# Then restart dev server
task dev
```

This rebuilds:
- `dist/bin/agentmux-srv-{version}-{platform}.{arch}.exe` (backend server)

---

### Production Build

Create a local portable build (Windows):

```bash
task package                         # portable build, per-build data dir
task package -- --fresh              # no-op — every build is already isolated (kept for muscle memory)
task package -- ~/Desktop/staging    # alternate output dir
```

Output: `~/Desktop/agentmux-<version>+g<sha>[.dirty].<stamp>-x64-portable/` and `.zip`

`task package` is for **local** builds. It does **not** bump the version and does **not** touch git — the artifact carries an ephemeral build *label* (the part after `+` is semver build metadata, ignored for precedence), not a new release version. Every build bakes a unique per-build channel (`local-<branch>-<hash>-<build-id>`), so each build is its own AgentMux instance with its own data dir and cef-cache — a running instance never blocks the next build, and two builds never collide on disk. Agents and auth are global and carry over across builds; only pane layout and memories start fresh per build. `--fresh` is redundant given per-build isolation and is a no-op. The committed version moves only through `task release` (changesets, below). Full rationale: [docs/specs/SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md](./docs/specs/SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md) and [docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md](./docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md).

---

## Version Management (Changesets — RFC #857 Phase 2)

**Feature PRs do not bump the version.** Add a changeset instead:

```bash
task changeset -- patch "fix(scope): short description"
# Allowed bump types: patch | minor | major
```

This creates `.changesets/<id>.md`. Commit it with your code. Version bumps live in dedicated **release PRs** which consume all pending changesets at once:

```bash
task release         # processes .changesets/, bumps version, updates VERSION_HISTORY
git commit -m "chore: release v<X.Y.Z>"
git push -u origin agenta/release-vX.Y.Z
```

See `.changesets/README.md` and `docs/specs/SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md` for rationale.

---

## Development Workflow

### Typical Development Session

```bash
# 1. Pull latest changes
git checkout main
git pull origin main

# 2. Create feature branch
git checkout -b agenta/feature-name

# 3. Start dev server
task dev

# 4. Make changes to code
# - Frontend (frontend/): Auto-reloads
# - Rust backend (agentmux-srv/src/): Run `task build:backend`, restart dev
# - Host (agentmux-cef/src/): Run `task build:host`, restart dev

# 5. Test changes in running app

# 6. Add a changeset (describes your change for the release log)
task changeset -- patch "fix(area): short description"
# or: task changeset -- minor "feat(area): description"

# 7. Commit and push
git add -p
git commit -m "feat: description"
git push -u origin agenta/feature-name

# 8. Create PR
gh pr create --title "Feature" --body "Description"
```

---

## Architecture

### Build Output

After building, you'll have:

#### Windows (portable ZIP)

```
~/Desktop/agentmux-{version}+g{sha}.{stamp}-x64-portable/
├── agentmux.exe                             # Launcher (entry point)
└── runtime/
    ├── agentmux-{version}.exe               # CEF host
    ├── agentmux-srv-{version}-windows.x64.exe  # Backend
    ├── libcef.dll                           # CEF runtime
    └── frontend/                            # Bundled web UI
```

#### Linux (AppImage)

```
~/Desktop/AgentMux_{version}+g{sha}.{stamp}_amd64.AppImage
└── usr/bin/
    ├── agentmux-launcher    # AppImage entry point (supervises srv + host)
    ├── agentmux-cef         # CEF host binary
    ├── agentmux-srv-{version}-linux.x64  # Backend
    ├── libcef.so            # Chromium runtime
    ├── libEGL.so / libGLESv2.so          # GPU abstraction
    ├── *.pak / icudtl.dat / …            # Chromium resources
    └── frontend/            # Bundled web UI
```

On first launch the AppImage extracts itself to `~/.local/share/agentmux/extracted/<version>/` for faster subsequent starts (~1 s vs ~3 s cold from FUSE).

### Component Sizes

| Component | Windows | Linux |
|-----------|---------|-------|
| Launcher | ~325 KB | ~325 KB |
| CEF host | ~8 MB | ~17 MB |
| Backend (agentmux-srv) | ~4 MB | ~4 MB |
| CEF runtime (libcef + paks) | ~160 MB | ~620 MB |
| **Portable build** | ~156 MB ZIP | ~220 MB AppImage |

---

## Debugging

### Frontend Logs

Open Chrome DevTools in the app:
- **Windows/Linux:** `Ctrl+Shift+I`
- **macOS:** `Cmd+Option+I`

Logs appear in the Console tab.

### Backend Logs

Use the `muxlog` helper (shipped in every AgentMux terminal) — it discovers and
renders NDJSON logs across every running instance (shared dir, each `task dev`
branch under `~/.agentmux/dev/<branch>/`, and per-build channels), defaulting
to the most-recently-active one:

```bash
muxlog srv          # tail the active sidecar (agentmux-srv) log, follow
muxlog ls            # list every instance's logs first if several are running
```

Full reference: [docs/MUXLOG.md](docs/MUXLOG.md).

### Host Logs

Rust host logs appear in the terminal where you ran `task dev`.

---

## Troubleshooting

### Issue: Backend binary not found (ENOENT)

**Cause:** Backend binary not built or wrong version.

**Fix:**
```bash
# Rebuild Rust backend
task build:backend

# Verify binaries exist
ls -lh dist/bin/agentmux-srv-*
```

### Issue: Build fails with linker errors

**Cause:** Missing Rust toolchain or system libraries.

**Fix (Windows):**
```powershell
# Install Visual Studio Build Tools
# https://visualstudio.microsoft.com/visual-cpp-build-tools/
```

**Fix (Linux):**
```bash
sudo apt install cmake ninja-build build-essential libssl-dev
```

### Issue: Frontend not loading in dev mode

**Cause:** Vite dev server failed to start, or port conflict.

**Fix:**
```bash
# Check if port 5173 is in use
netstat -ano | grep :5173

# Clear and reinstall
rm -rf node_modules package-lock.json
npm install
task dev
```

### Issue: Schema directory missing after clean

**Cause:** `task clean` wipes `dist/schema/` but it's needed for the build.

**Fix:**
```bash
task copy:schema
# or manually:
cp -r schema dist/schema
```

This is handled automatically in the normal build pipeline.

---

## CI/CD

### GitHub Actions

Automated builds run on push to `main`:

- **Windows:** NSIS installer (.exe) + portable ZIP
- **macOS:** DMG installer (.dmg)
- **Linux:** DEB package (.deb), AppImage

Artifacts are uploaded to GitHub Releases on tagged commits.

### Local Release Build

```bash
# 1. Add a changeset
task changeset -- patch "fix(scope): description"

# 2. Rebuild Rust binaries
task build:backend

# 3. Build portable package
task package

# 4. Test portable build
# Extract and run from ~/Desktop/agentmux-{version}-x64-portable/

# 5. Tag and push
git push origin main --tags
```

---

## Cross-Platform Notes

### Windows

- Uses **NSIS** for installers
- CEF bundles its own Chromium runtime — no WebView2 required
- No CGO / no Zig required (pure Rust)

### macOS

- Uses **DMG** for distribution
- WKWebView built-in (no WebView2 needed)
- Code signing required for distribution (not dev)
- Universal binary supported (x64 + ARM64)

### Linux

- **AppImage** (universal) and **DEB** (Debian/Ubuntu) produced by CI.
- CEF bundles Chromium — no system WebKitGTK required.
- Default display: **XWayland** (`--ozone-platform=x11`). Set `AGENTMUX_OZONE_PLATFORM=wayland` for native Wayland (experimental).
- Window drag and right-click on title-bar require the patched `libcef.so` (included in all release AppImages). See `docs/cef-build/build-patched-libcef.md`.

---

## Advanced

### Build Backend for Specific Platform

```bash
# Rust cross-compilation
cargo build --release --target x86_64-pc-windows-msvc
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-unknown-linux-gnu
```

### Build Frontend Only

```bash
# Development build
npm run build:dev

# Production build
npm run build:prod
```

---

## Resources

- **Task Configuration:** [Taskfile.yml](Taskfile.yml)
- **Architecture Docs:** [docs/architecture/](docs/architecture/)
- **Contributing:** [CONTRIBUTING.md](CONTRIBUTING.md)
- **Version History:** [VERSION_HISTORY.md](VERSION_HISTORY.md)

---

## Quick Reference

| Task | Command |
|------|---------|
| **Development** | `task dev` |
| **Rebuild Rust backend** | `task build:backend` |
| **Build host** | `task build:host` |
| **Bundle runtime** | `task bundle` |
| **Portable ZIP** | `task package` |
| **Add changeset** | `task changeset -- patch "description"` |
| **Run tests** | `npm test` |
| **Verify versions** | `bump verify` |
