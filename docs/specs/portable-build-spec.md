# AgentMux Windows Portable Build — Spec

## Goal

A single command that produces a working portable ZIP on Desktop. No manual steps, no version mismatches, no stale artifacts. Any agent should be able to run it.

## Current Problem

The build requires 5 separate commands in the right order, and agents frequently:
1. Forget to bump version → version check fails in packaging script
2. Build CEF but not backend → missing `wsh` or `agentmuxsrv-rs`
3. Build backend but not CEF → stale CEF binary with wrong version
4. Build frontend with wrong config → missing assets
5. Skip `cef:bundle` → missing `libcef.dll` and resource paks

## The Single Command

```bash
task cef:package:portable
```

This MUST work end-to-end. Currently it only runs the packaging script, assuming everything is pre-built. It needs to be a full pipeline.

## Proposed Taskfile Change

Replace the current `cef:package:portable` task:

```yaml
cef:package:portable:
    desc: Build and package AgentMux CEF as a portable ZIP (Windows).
    platforms: [windows]
    deps:
        - build:frontend:prod    # Vite production build → dist/frontend/
        - build:backend          # agentmuxsrv-rs + wsh → dist/bin/
        - cef:build              # agentmux-cef.exe → dist/cef/ (also builds launcher)
        - cef:bundle             # libcef.dll + paks + locales → dist/cef/
    cmds:
        - cargo build --release -p agentmux-launcher
        - bash scripts/package-cef-portable.sh
```

### Dependency Chain

```
cef:package:portable
├── build:frontend:prod     → dist/frontend/ (index.html, assets/, fonts/)
├── build:backend
│   ├── build:backend:rust  → dist/bin/agentmuxsrv-rs.x64.exe
│   └── build:wsh           → dist/bin/wsh-{version}-windows.x64.exe
├── cef:build               → cargo build --release -p agentmux-cef
├── cef:bundle              → dist/cef/ (libcef.dll, *.pak, locales/)
└── scripts/package-cef-portable.sh
    └── Assembles ~/Desktop/agentmux-cef-{version}-x64-portable/
```

### What the packaging script does

1. Reads version from `package.json`
2. Verifies all required files exist:
   - `target/release/agentmux-cef.exe`
   - `target/release/agentmux-launcher.exe`
   - `dist/cef/libcef.dll`
   - `dist/bin/agentmuxsrv-rs.x64.exe`
   - `dist/bin/wsh-{version}-windows.x64.exe`
   - `dist/frontend/index.html`
3. Creates portable directory structure
4. Verifies version strings in binaries match `package.json`
5. Creates ZIP

### Version String Check

The script `grep`s for the version string in the CEF and sidecar binaries. If they don't match `package.json`, it fails. This catches stale builds.

**Common failure:** Agent bumps version but doesn't rebuild. Fix: the task deps force a rebuild.

## Pre-Requisites (one-time setup)

These must exist on the build machine:

| Tool | Path | Check |
|------|------|-------|
| Rust/Cargo | `~/.cargo/bin/cargo` | `cargo --version` |
| Node.js | system PATH | `node --version` |
| npm | system PATH | `npm --version` |
| CMake | system PATH or VS | `cmake --version` |
| Ninja | `/c/Systems/bin/ninja.exe` | `ninja --version` |
| Visual Studio 2022 | standard install | `cl.exe` via vcvars |
| Task | system PATH | `task --version` |

## Build Times (approximate)

| Step | First Build | Incremental |
|------|------------|-------------|
| Frontend (Vite) | 15s | 5s |
| Backend (Rust) | 2-3min | 15-30s |
| CEF host (Rust) | 35s | 35s (CEF SDK rebuild) |
| CEF bundle (copy DLLs) | 5s | 5s |
| Launcher (Rust) | 5s | 2s |
| Packaging script | 10s | 10s |
| **Total** | **~4min** | **~1.5min** |

## Agent Instructions

For any agent building a portable release:

```bash
# 1. Bump version (REQUIRED before every build)
bump patch -m "description" --commit

# 2. Build and package (single command)
task cef:package:portable

# 3. Test
# Launch ~/Desktop/agentmux-cef-{version}-x64-portable/agentmux.exe
```

If `task cef:package:portable` fails:
- **"not found" error**: Run the missing build step manually (`task build:backend`, `task cef:build`, etc.)
- **Version mismatch**: You bumped but didn't rebuild. Run `task cef:package:portable` again (deps rebuild)
- **CMake/Ninja error**: Verify `cmake --version` and `ninja --version` work

## What NOT To Do

- **Don't copy binaries manually** (`cp target/release/agentmux-cef.exe ...`) — use the packaging script
- **Don't skip the version bump** — the version check will fail
- **Don't run individual build commands** unless diagnosing a failure — use the single task
- **Don't kill running AgentMux instances before building** — they're isolated (separate dirs)
