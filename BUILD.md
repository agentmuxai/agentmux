# Building AgentMux

These instructions cover setting up dependencies and building AgentMux from source on Windows, macOS, and Linux.

**Architecture:** AgentMux is a **Chromium-based desktop app** with a **100% Rust backend**.

---

## Prerequisites

### Required Tools

| Tool | Version | Purpose |
|------|---------|---------|
| **Node.js** | v22 LTS | Frontend build (React/Vite) |
| **Rust** | 1.77+ | Backend (agentmux-srv) + Host (agentmux-cef) |
| **Task** | Latest | Build orchestration |

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

3. **WebView2** is pre-installed on Windows 10/11. If missing, it will auto-install on first launch.

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

1. **Install dependencies** (Debian/Ubuntu):
   ```bash
   sudo apt install zip libwebkit2gtk-4.1-dev \
     build-essential curl wget file libssl-dev \
     libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
   ```

2. **Install Rust**:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

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

---

## Build Commands

### Development (Hot Reload)

**This is the recommended way to run AgentMux during development:**

```bash
task dev
```

Features:
- Frontend hot reload (React HMR via Vite)
- Auto-rebuild on Rust changes
- DevTools available (Ctrl+Shift+I)

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

Create a portable build (Windows):

```bash
task package
```

Output: `~/Desktop/agentmux-{version}-x64-portable/` and `.zip`

---

## Version Management

**Before releasing, ensure version consistency across all files:**

```bash
# Bump version (updates package.json, Cargo.toml, etc.)
./bump-version.sh patch --message "Your change description"

# Verify consistency
bump verify

# Push with tags
git push origin <branch> --tags
```

**Critical:** Always use `bump-version.sh` — never manually edit version numbers.

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

# 6. Bump version
./bump-version.sh patch --message "Description of change"

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

```
dist/bin/
└── agentmux-srv-{version}-windows.x64.exe       # Rust backend (sidecar)

target/release/
├── agentmux-cef.exe                              # Host
└── agentmux-launcher.exe                         # Portable launcher

~/Desktop/
└── agentmux-{version}-x64-portable/             # Portable build
    ├── agentmux.exe                             # Launcher
    └── runtime/
        ├── agentmux-{version}.exe               # Host
        ├── agentmux-srv-{version}-windows.x64.exe  # Backend
        ├── libcef.dll                           # CEF runtime
        └── frontend/                            # Web UI
```

### Component Sizes (v0.31.0+)

| Component | Size | Purpose |
|-----------|------|---------|
| `agentmux.exe` (launcher) | ~325 KB | Portable launcher |
| `agentmux-cef.exe` | ~8 MB | Host (Rust + Chromium) |
| `agentmux-srv.exe` | ~4 MB | Rust async backend server |
| CEF runtime (libcef + paks) | ~160 MB | Bundled Chromium |
| **Portable ZIP** | ~156 MB | Compressed with CEF bundle |

---

## Debugging

### Frontend Logs

Open Chrome DevTools in the app:
- **Windows/Linux:** `Ctrl+Shift+I`
- **macOS:** `Cmd+Option+I`

Logs appear in the Console tab.

### Backend Logs

Rust backend logs (agentmux-srv):
- **Development:** `~/.agentmux-dev/agentmux.log`
- **Production:** `~/.agentmux/agentmux.log`

View logs in real-time:

```bash
# Development
tail -f ~/.agentmux-dev/agentmux.log

# Production
tail -f ~/.agentmux/agentmux.log
```

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
sudo apt install libwebkit2gtk-4.1-dev build-essential libssl-dev
```

### Issue: Frontend not loading in dev mode

**Cause:** Vite dev server failed to start, or port conflict.

**Fix:**
```bash
# Check if port 1420 is in use
netstat -ano | grep :1420

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
# 1. Bump version
./bump-version.sh patch --message "v0.31.x release"

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
- WebView2 runtime required (auto-installs if missing)
- No CGO / no Zig required (pure Rust)

### macOS

- Uses **DMG** for distribution
- WKWebView built-in (no WebView2 needed)
- Code signing required for distribution (not dev)
- Universal binary supported (x64 + ARM64)

### Linux

- Uses **DEB** (Debian/Ubuntu) and **AppImage** (universal)
- WebKitGTK required: `libwebkit2gtk-4.1-dev`

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
| **Bump version** | `./bump-version.sh patch` |
| **Run tests** | `npm test` |
| **Verify versions** | `bump verify` |
