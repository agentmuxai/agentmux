# Phase 14: Delete Electron Code & Update Documentation

> **Status:** SPEC
> **Date:** 2026-02-08
> **Author:** AgentA
> **Priority:** HIGH
> **Target:** 0.18.7
> **Goal:** Remove all Electron code, update docs to reflect Tauri-only architecture

---

## Executive Summary

AgentMux has successfully migrated to Tauri v2. The Electron codebase is no longer needed and is causing confusion for agents working on the project. This phase removes all Electron code and updates documentation to reflect the current Tauri architecture.

**What We're Deleting:**
- ❌ `emain/` directory (entire Electron main process)
- ❌ Electron build configs
- ❌ Electron dependencies
- ❌ Electron-specific scripts
- ❌ Outdated architecture docs

**What We're Updating:**
- ✅ Architecture documentation
- ✅ Build instructions
- ✅ Contributing guidelines
- ✅ README files
- ✅ Agent instruction files (CLAUDE.md)

---

## Audit Results

### Electron Code to Delete

| Path | Type | Size | Description |
|------|------|------|-------------|
| `emain/` | Directory | ~TBD | Entire Electron main process |
| `electron.vite.config.ts` | Config | 6KB | Electron Vite config |
| `electron-builder.config.cjs` | Config | 8KB | Electron Builder config |
| `frontend/types/custom.d.ts` | Types | N/A | ElectronApi types (partial) |

### Dependencies to Remove

**package.json:**
```json
// DELETE from devDependencies
"electron": "^38.1.2",
"electron-builder": "^26.1.0",
"electron-vite": "^4.0.1",

// DELETE from dependencies
"electron-updater": "^6.6",

// DELETE scripts
"dev": "electron-vite dev",
"start": "electron-vite preview",
"build:dev": "electron-vite build --mode development",
"build:prod": "electron-vite build --mode production",
"postinstall": "electron-builder install-app-deps",
```

### Documentation to Update

| File | Issue | Action |
|------|-------|--------|
| `CLAUDE.md` | References Electron architecture | Update to Tauri |
| `BUILD.md` | Electron build instructions | Replace with Tauri |
| `CONTRIBUTING.md` | Mentions Electron dev setup | Update |
| `docs/architecture/agentmux-components.md` | Electron architecture diagram | Rewrite |
| `.roo/rules/overview.md` | Electron references | Update |
| `specs/agentmux-tauri-migration.md` | Migration spec (historical) | Archive or delete |
| `BENCHMARK_REPORT.md` | Compares Electron vs Tauri | Keep (historical data) |

---

## Implementation Plan

### Phase 1: Delete Electron Code (Day 1)

#### 1.1 Delete Electron Main Process

```bash
# Delete entire emain directory
rm -rf emain/

# Verify deletion
git status
```

**Files deleted:**
- `emain/emain.ts` - Main entry point
- `emain/emain-window.ts` - Window management
- `emain/emain-tabview.ts` - Tab management (if still exists)
- `emain/preload.ts` - Preload script
- `emain/menu.ts` - Application menu
- `emain/platform.ts` - Platform utilities
- `emain/authkey.ts` - Auth key injection
- `emain/emain-agentmuxsrv.ts` - Backend spawning
- `emain/updater.ts` - Auto-updater
- `emain/*.ts` - All other Electron files

**Estimated:** ~20-30 files, ~3000-5000 LOC

---

#### 1.2 Delete Electron Build Configs

```bash
# Delete config files
rm electron.vite.config.ts
rm electron-builder.config.cjs

# Keep Tauri configs
# ✅ vite.config.tauri.ts (KEEP)
# ✅ src-tauri/tauri.conf.json (KEEP)
```

---

#### 1.3 Clean Up Frontend Types

```typescript
// frontend/types/custom.d.ts (BEFORE)
declare global {
  interface Window {
    api: ElectronApi;  // ← DELETE THIS
  }
}

interface ElectronApi {
  createTab: (url: string) => void;
  // ... 40+ Electron methods
}

// frontend/types/custom.d.ts (AFTER)
declare global {
  interface Window {
    // Tauri APIs accessed via @tauri-apps/api, not window.api
  }
}

// Or delete file entirely if empty
```

**Files to modify:**
- `frontend/types/custom.d.ts` - Remove ElectronApi interface

---

#### 1.4 Remove Electron Dependencies

```bash
# Remove from package.json
npm uninstall electron electron-builder electron-vite electron-updater

# This will update package.json and package-lock.json
```

**package.json changes:**
```diff
  "devDependencies": {
-   "electron": "^38.1.2",
-   "electron-builder": "^26.1.0",
-   "electron-vite": "^4.0.1",
    "@vitejs/plugin-react-swc": "4.2.2",
    // ... other deps
  },
  "dependencies": {
-   "electron-updater": "^6.6",
    "color": "^4.2.3",
    // ... other deps
  }
```

---

#### 1.5 Update package.json Scripts

```diff
  "scripts": {
-   "dev": "electron-vite dev",
-   "start": "electron-vite preview",
-   "build:dev": "electron-vite build --mode development",
-   "build:prod": "electron-vite build --mode production",
-   "postinstall": "electron-builder install-app-deps",
+   "dev": "tauri dev",
+   "build": "tauri build",
    "test": "vitest",
    "coverage": "vitest run --coverage",
-   "tauri:dev": "tauri dev",
-   "tauri:build": "tauri build"
  }
```

**Rationale:** Remove "tauri:" prefix since Tauri is now the only build system.

---

### Phase 2: Update Documentation (Day 2)

#### 2.1 Update CLAUDE.md (Agent Instructions)

```markdown
# CLAUDE.md (CURRENT - OUTDATED)

## Architecture

- **AgentMux.exe** = Electron app (UI)
- **agentmuxsrv** = Go backend (auto-spawned)
- **wsh** = Shell integration

## Development

task dev          # Hot reload - USE THIS FOR DEVELOPMENT
```

```markdown
# CLAUDE.md (NEW - UPDATED)

## Architecture

AgentMux is a **Tauri v2** desktop application with:
- **agentmux.exe** = Tauri app (Rust + single webview)
- **agentmuxsrv** = Go backend sidecar (auto-spawned)
- **wsh** = Shell integration binary

## Development

task dev          # Tauri hot reload - USE THIS FOR DEVELOPMENT
task build        # Production Tauri build
task build:backend # Rebuild Go binaries (agentmuxsrv, wsh)
```

**Files to update:**
- `CLAUDE.md` - Main agent instruction file

---

#### 2.2 Update BUILD.md

```markdown
# BUILD.md (BEFORE - Electron)

## Prerequisites

- Node.js v22+
- Go 1.23+
- Electron 38+

## Build Commands

npm run build:prod    # Build Electron app
npm run package       # Create installer
```

```markdown
# BUILD.md (AFTER - Tauri)

## Prerequisites

- Node.js v22+
- Go 1.23+
- Rust 1.77+ (for Tauri)
- Zig 0.13+ (for CGO cross-compilation)

## Build Commands

### Development
task dev              # Hot reload (frontend + backend)
task build:backend    # Rebuild Go binaries only

### Production
task build            # Create Tauri production build
# Outputs to: src-tauri/target/release/bundle/

### Release
./bump-version.sh patch --message "Description"
task build:backend    # Rebuild with new version
task build            # Package installer
```

**Files to update:**
- `BUILD.md`

---

#### 2.3 Rewrite Architecture Documentation

```markdown
# docs/architecture/agentmux-components.md (NEW)

# AgentMux Architecture (Tauri v2)

## System Overview

```
┌─────────────────────────────────────────────────────────┐
│                 Tauri Rust Frontend                      │
│  src-tauri/src/                                          │
│  ├── main.rs          - Entry point                      │
│  ├── commands/        - Tauri commands (#[tauri::command])│
│  ├── sidecar.rs       - agentmuxsrv lifecycle            │
│  ├── state.rs         - App state (Mutex)               │
│  ├── menu.rs          - Native menus                     │
│  ├── tray.rs          - System tray                      │
│  └── backend/         - Rust backend port (Agent3)      │
└──────────────────┬────────────────────────────────────┘
                   │ Tauri sidecar (stdio)
                   ▼
┌─────────────────────────────────────────────────────────┐
│              Go Backend (agentmuxsrv)                     │
│  cmd/server/         - Main server entry                │
│  pkg/web/            - HTTP + WebSocket server          │
│  pkg/wstore/         - SQLite data store                │
│  pkg/blockcontroller/- Terminal lifecycle               │
│  pkg/shellexec/      - PTY + shell execution            │
│  pkg/waveai/         - AI integration                   │
└──────────────────┬────────────────────────────────────┘
                   │ WebSocket + HTTP (localhost)
                   ▼
┌─────────────────────────────────────────────────────────┐
│              React Frontend (Vite)                       │
│  frontend/wave.ts         - App bootstrap               │
│  frontend/app/view/       - React components            │
│  frontend/app/store/      - State (Jotai atoms)         │
│  frontend/app/aipanel/    - AI chat panel               │
└─────────────────────────────────────────────────────────┘
```

## Key Components

### Tauri Rust Layer
- **Single webview per window** (no multi-WebContentsView)
- **Native OS integration** (menus, tray, notifications)
- **Sidecar management** (spawn/monitor agentmuxsrv)
- **IPC bridge** (Tauri commands for frontend ↔ Rust)

### Go Backend (agentmuxsrv)
- **Terminal management** (PTY, shell execution)
- **Data persistence** (SQLite via wstore)
- **WebSocket server** (real-time frontend ↔ backend)
- **AI integration** (Anthropic, OpenAI, etc.)

### React Frontend
- **Single-pane UI** (tabs/workspaces removed in Phase 13)
- **Terminal rendering** (xterm.js)
- **AI chat panel** (built-in AI assistant)
- **State management** (Jotai atoms)

## Communication Flows

### 1. Frontend → Backend (Go)
```
React component
  → fetch('http://localhost:PORT/api/...')
  → Go HTTP handler
  → Response
```

### 2. Frontend ↔ Backend (WebSocket)
```
React
  → WebSocket.send(RpcMessage)
  → Go WebSocket handler
  → WebSocket.send(RpcResponse)
  → React callback
```

### 3. Frontend → Tauri (Rust)
```
React
  → import { invoke } from '@tauri-apps/api/core'
  → invoke('command_name', { args })
  → Rust #[tauri::command] handler
  → Return value
```

### 4. Tauri → Backend (Sidecar)
```
Tauri
  → Command::new_sidecar("agentmuxsrv")
  → Spawn process with stdio
  → Monitor stderr for "WAVESRV-ESTART"
  → Parse endpoints (WS, HTTP)
  → Pass to frontend
```

## Build Pipeline

### Development (Hot Reload)
```
task dev
  ├─ cargo watch (Rust hot reload)
  ├─ vite dev server (React HMR)
  └─ agentmuxsrv (auto-restart on crash)
```

### Production
```
task build
  ├─ vite build (React → dist/)
  ├─ cargo build --release (Rust)
  ├─ Embed frontend in binary
  ├─ Bundle sidecars (agentmuxsrv, wsh)
  └─ Create installer (NSIS, DMG, DEB, AppImage)
```

## File Structure

```
agentmux/
├── src-tauri/              # Tauri Rust app
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands/
│   │   ├── sidecar.rs
│   │   └── backend/        # Rust backend port (Agent3)
│   ├── Cargo.toml
│   └── tauri.conf.json
├── frontend/               # React app
│   ├── app/
│   │   ├── view/           # Components
│   │   ├── store/          # State
│   │   └── aipanel/        # AI chat
│   └── wave.ts
├── cmd/                    # Go binaries
│   ├── server/             # agentmuxsrv
│   └── wsh/                # wsh
├── pkg/                    # Go packages
│   ├── web/
│   ├── wstore/
│   ├── blockcontroller/
│   └── waveai/
└── dist/                   # Build outputs
    ├── bin/                # Go binaries
    └── frontend/           # Vite output
```

## Migration Status

- ✅ Electron removed (Phase 14)
- ✅ Tauri v2 stable
- ✅ Single-pane UI (tabs/workspaces removed temporarily)
- ✅ Go backend unchanged (sidecar model)
- 🚧 Rust backend port in progress (Agent3, Phase 15+)

---

**Last Updated:** 2026-02-08
```

**Files to create/update:**
- `docs/architecture/agentmux-components.md` - Complete rewrite

---

#### 2.4 Update README.md

```diff
  # AgentMux

- AgentMux is an AI-native terminal built on Electron and Go.
+ AgentMux is an AI-native terminal built on Tauri v2 and Go.

  ## Features

- - Multi-tab terminal
- - Workspace management
+ - Fast, lightweight terminal (Tauri-powered)
  - AI chat integration
  - File previews
  - Cross-platform (Windows, macOS, Linux)

  ## Development

- npm run dev     # Start Electron dev mode
+ task dev        # Start Tauri dev mode (hot reload)
```

**Files to update:**
- `README.md`

---

#### 2.5 Update Contributing Guidelines

```markdown
# CONTRIBUTING.md (UPDATE)

## Development Setup

### Prerequisites
- Node.js v22+
- Go 1.23+
- Rust 1.77+ (for Tauri)
- Zig 0.13+ (for CGO cross-compilation on Windows)

### Getting Started

1. Clone the repository
   ```bash
   git clone https://github.com/agentmuxhq/agentmux.git
   cd agentmux
   ```

2. Install dependencies
   ```bash
   npm install
   ```

3. Start development server
   ```bash
   task dev
   ```

### Architecture

AgentMux uses **Tauri v2** (not Electron) with:
- **Rust** frontend (Tauri, single webview)
- **Go** backend (agentmuxsrv sidecar)
- **React** UI (Vite + TypeScript)

See [docs/architecture/agentmux-components.md](docs/architecture/agentmux-components.md) for details.

### Making Changes

- **Frontend (React):** Edit `frontend/app/`, hot reload enabled
- **Tauri (Rust):** Edit `src-tauri/src/`, restart `task dev`
- **Backend (Go):** Edit `pkg/`, run `task build:backend`, restart

### Testing

```bash
npm test              # Run all tests
npm run coverage      # Generate coverage report
```
```

**Files to update:**
- `CONTRIBUTING.md`

---

#### 2.6 Archive or Delete Migration Spec

**Option A: Archive**
```bash
mkdir -p docs/archive/
mv specs/agentmux-tauri-migration.md docs/archive/
git add docs/archive/agentmux-tauri-migration.md
git commit -m "docs: archive Electron→Tauri migration spec (historical)"
```

**Option B: Delete**
```bash
rm specs/agentmux-tauri-migration.md
git commit -m "docs: remove outdated migration spec"
```

**Recommendation:** Archive (Option A) - keeps historical context.

---

#### 2.7 Update .roo/rules/overview.md (AI Agent Rules)

```diff
  # AgentMux Codebase Overview

  ## Architecture

- AgentMux is a desktop application built with **Electron** and **Go**.
+ AgentMux is a desktop application built with **Tauri v2** and **Go**.

  ## Key Directories

- ├── emain/              # Electron main process (TypeScript)
+ ├── src-tauri/          # Tauri Rust app
  ├── frontend/          # React frontend (TypeScript)
  ├── cmd/               # Go binaries (server, wsh)
  ├── pkg/               # Go packages
  └── dist/              # Build outputs
```

**Files to update:**
- `.roo/rules/overview.md`

---

### Phase 3: Clean Up Taskfile (Day 3)

#### 3.1 Remove Electron Build Tasks

```yaml
# Taskfile.yml (BEFORE)

tasks:
  electron:dev:
    desc: Start Electron development mode
    cmds:
      - npm run dev

  electron:build:
    desc: Build Electron app
    cmds:
      - npm run build:prod
```

```yaml
# Taskfile.yml (AFTER)

tasks:
  dev:
    desc: Start Tauri development mode (hot reload)
    cmds:
      - npm run tauri dev

  build:
    desc: Build Tauri production app
    cmds:
      - npm run tauri build
```

**Files to update:**
- `Taskfile.yml` - Remove Electron tasks, rename Tauri tasks

---

### Phase 4: Update GitHub Workflows (Day 4)

#### 4.1 Update CI/CD Pipeline

```yaml
# .github/workflows/build.yml (UPDATE)

name: Build AgentMux

on:
  push:
    branches: [main]

jobs:
  build-tauri:  # Renamed from build-electron
    strategy:
      matrix:
        platform: [windows-latest, macos-latest, ubuntu-latest]
    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
      - uses: dtolnay/rust-toolchain@stable  # Add Rust
      - name: Build Tauri app
        run: |
          npm install
          npm run tauri build
```

**Files to update:**
- `.github/workflows/*.yml` - Replace Electron with Tauri

---

### Phase 5: Clean Up Unused Files (Day 5)

#### 5.1 Find and Remove Stale References

```bash
# Find any remaining Electron references
grep -r "electron" --include="*.ts" --include="*.tsx" --exclude-dir=node_modules frontend/

# Check for window.api usage (Electron IPC)
grep -r "window\.api" --include="*.ts" --include="*.tsx" --exclude-dir=node_modules frontend/

# Find emain imports
grep -r "from.*emain" --include="*.ts" --include="*.tsx" --exclude-dir=node_modules frontend/
```

**Manual review:** Fix any remaining references found.

---

## Testing Strategy

### Pre-Deletion Checks

✅ **Verify Tauri build works:**
```bash
task build
# Check: src-tauri/target/release/bundle/ contains installer
```

✅ **Verify dev mode works:**
```bash
task dev
# Check: App launches, terminal works, AI panel works
```

✅ **Run test suite:**
```bash
npm test
# Check: All tests pass (no Electron test failures)
```

### Post-Deletion Validation

✅ **Build still works:**
```bash
npm install  # Re-install without Electron
task build
```

✅ **No broken imports:**
```bash
npm run type-check  # TypeScript compilation
```

✅ **Documentation accurate:**
- Read updated docs
- Follow dev setup instructions
- Verify accuracy

---

## Rollback Plan

**If deletion causes issues:**

1. **Revert Git Commits**
   ```bash
   git log --oneline -10  # Find deletion commit
   git revert <commit-hash>
   git push origin main
   ```

2. **Restore package.json**
   ```bash
   npm install electron electron-builder electron-vite
   ```

3. **Restore emain/ directory**
   ```bash
   git checkout <pre-deletion-commit> -- emain/
   ```

**Risk:** Very low - Tauri is already working, Electron code is unused.

---

## Success Criteria

### Must Have (P0)

- ✅ `emain/` directory deleted
- ✅ Electron dependencies removed from package.json
- ✅ Electron config files deleted
- ✅ CLAUDE.md updated with Tauri architecture
- ✅ BUILD.md updated with Tauri instructions
- ✅ Architecture docs rewritten for Tauri
- ✅ Tauri build still works
- ✅ Tauri dev mode still works

### Should Have (P1)

- ✅ All specs updated (no Electron references)
- ✅ Contributing guidelines updated
- ✅ CI/CD pipelines updated
- ✅ Taskfile cleaned up

### Nice to Have (P2)

- ✅ Migration spec archived
- ✅ Historical benchmark data preserved
- ✅ Agent rule files updated

---

## Timeline

| Phase | Task | Duration |
|-------|------|----------|
| **Phase 1** | Delete Electron code | 1 day |
| **Phase 2** | Update documentation | 1 day |
| **Phase 3** | Clean up Taskfile | 1 day |
| **Phase 4** | Update GitHub workflows | 1 day |
| **Phase 5** | Clean up stale references | 1 day |
| **Total** | | **5 days** |

**Target:** Complete before Phase 13 (tabs/workspaces removal)

---

## Metrics

### Code Deletion (Expected)

| Category | Files Deleted | LOC Removed |
|----------|---------------|-------------|
| **Electron main** | ~20-30 files | ~3000-5000 LOC |
| **Config files** | 2 files | ~15 LOC |
| **Types** | Partial | ~100 LOC |
| **Docs (outdated)** | ~5 files | ~1000 LOC |
| **Total** | **~30 files** | **~4000-6000 LOC** |

### Dependency Reduction

| Metric | Before | After | Reduction |
|--------|--------|-------|-----------|
| **node_modules size** | ~800MB | ~600MB | 25% |
| **package.json deps** | ~80 | ~76 | 4 removed |
| **Install time** | ~45s | ~35s | 22% |

---

## Communication Plan

### Commit Message Template

```
chore: remove Electron code, update docs to Tauri-only

AgentMux has fully migrated to Tauri v2. This commit removes all
Electron code and updates documentation to prevent agent confusion.

Changes:
- Delete emain/ directory (Electron main process)
- Remove Electron dependencies (electron, electron-builder, electron-vite)
- Update CLAUDE.md, BUILD.md, CONTRIBUTING.md for Tauri
- Rewrite architecture documentation
- Update scripts in package.json
- Archive migration spec

BREAKING CHANGE: Electron build system removed. Use Tauri only.

Refs: #<PR-number>
Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>
```

---

## Appendix: Files Checklist

### Files to DELETE

```
✅ emain/                                 # Entire directory
✅ electron.vite.config.ts
✅ electron-builder.config.cjs
```

### Files to UPDATE (Major)

```
✅ package.json                           # Remove deps, update scripts
✅ CLAUDE.md                              # Update architecture
✅ BUILD.md                               # Replace build instructions
✅ docs/architecture/agentmux-components.md # Rewrite
✅ CONTRIBUTING.md                        # Update dev setup
✅ .roo/rules/overview.md                 # Update agent rules
```

### Files to UPDATE (Minor)

```
✅ README.md                              # Update description
✅ Taskfile.yml                           # Remove Electron tasks
✅ frontend/types/custom.d.ts             # Remove ElectronApi
✅ .github/workflows/*.yml                # Update CI/CD
```

### Files to ARCHIVE

```
✅ specs/agentmux-tauri-migration.md → docs/archive/
```

### Files to KEEP (Historical)

```
✅ BENCHMARK_REPORT.md                    # Electron vs Tauri comparison
✅ BENCHMARK_RESULTS_FINAL.md             # Historical data
```

---

**END OF SPEC**
