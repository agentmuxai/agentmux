# Spec: Versioned Artifact Names — Build Through Task Manager

## Problem

Binary names are inconsistent (`agentmuxsrv-rs`, `wsh`, `agentmux-cef`) and none except wsh include the version in the filename. The entire chain — from `dist/` to the portable ZIP to the running process in Task Manager — uses static names:

| Binary | dist/ name | Portable name | Task Manager |
|--------|-----------|--------------|-------------|
| **agentmux-cef** | `agentmux-cef.exe` | `agentmux-cef.exe` | `agentmux-cef.exe` |
| **agentmuxsrv-rs** | `agentmuxsrv-rs.x64.exe` | `agentmuxsrv-rs.x64.exe` | `agentmuxsrv-rs.x64.exe` |
| **agentmux-launcher** | `agentmux-launcher.exe` | `agentmux.exe` | `agentmux.exe` |
| **wsh** | `wsh-0.33.26-windows.x64.exe` | `wsh.exe` | `wsh.exe` |

**Consequences:**
- Can't tell which version is running in Task Manager when multiple versions are open
- Can't tell if a stale binary slipped into a build
- The packaging script uses a fragile `grep -ao "$VERSION"` heuristic to verify
- Naming is inconsistent: `agentmuxsrv-rs` has a legacy `-rs` suffix, `wsh` doesn't follow the `agentmux-*` pattern

## Goals

1. **Standardize** all binary names to a clean `agentmux-*` family.
2. **Version** every binary filename at every stage: `dist/`, portable `runtime/`, and the running process in Task Manager.

---

## Binary Name Standardization

| Role | Old Crate Dir | Old Binary Name | New Crate Dir | New Binary Name |
|------|-------------|----------------|--------------|----------------|
| Launcher | `agentmux-launcher/` | `agentmux-launcher` | `agentmux-launcher/` | `agentmux` (no change) |
| CEF host | `agentmux-cef/` | `agentmux-cef` | `agentmux-cef/` | `agentmux-cef` (no change) |
| Backend | `agentmuxsrv-rs/` | `agentmuxsrv-rs` | **`agentmux-srv/`** | **`agentmux-srv`** |
| Shell | `wsh-rs/` | `wsh` | **`agentmux-wsh/`** | **`agentmux-wsh`** |

**Rationale:**
- `-rs` suffix on backend was meaningful during Go→Rust migration. Go is fully removed — suffix is noise.
- `wsh` predates the AgentMux rebrand. Bringing it into the `agentmux-*` family makes all processes instantly recognizable in Task Manager and process viewers.

### Scope Decisions

The verification found **442 refs** to `agentmuxsrv-rs` across 102 files and **~150 refs** to `wsh`/`wsh-rs` across 30+ files. Not everything changes. Here's the line:

**CHANGES — binary/crate/build identity:**
- Crate directory names, Cargo.toml package/bin names
- Taskfile build commands and output filenames
- Packaging scripts (portable, MSIX, AppImage)
- Rust binary discovery code (sidecar.rs, launcher)
- Log messages that name the binary (e.g. `[agentmuxsrv-rs] CRASH`)
- Shell integration scripts that invoke the binary
- Benchmark scripts that reference exe names
- WER crash dump registry keys (reference exe name)
- Constants that define the binary name (`WSH_BINARY_NAME`, `REMOTE_FULL_WSH_BIN_PATH`)
- CLI command name (`wsh-rs/src/cli/mod.rs` → `#[command(name = "agentmux-wsh")]`)
- Active docs: `CLAUDE.md`, `README.md`, `BUILD.md`, `CONTRIBUTING.md`
- Test files that invoke the binary by name (50+ `wsh file copy` test scripts)
- Integration test binary reference (`CARGO_BIN_EXE_agentmuxsrv-rs`)
- E2E test comments referencing binary name

**CHANGES — internal Rust identifiers:**
- `wshutil/` module dir → `agentmux_wsh_util/` or keep as `wshutil/` (see decision below)
- `wshrpc.rs` → keep as-is (internal RPC protocol, not user-visible)

**DOES NOT CHANGE — serialized config fields (breaking change to user data):**
- `conn_wsh_enabled`, `conn_wsh_path`, `conn_ask_before_wsh_install` in `wconfig.rs` — these are serialized to `~/.agentmux/config.json` via serde. Renaming would break existing user configs.
- `wsh_enabled`, `wsh_error`, `no_wsh_reason`, `wsh_version` struct fields in `wslconn.rs` — serde-renamed, internal name doesn't affect serialization.
- `META_KEY_CMD_NO_WSH` (`"cmd:nowsh"`) — stored in block metadata.

**DOES NOT CHANGE — URI scheme:**
- `wsh://` connection scheme in `connparse.rs` (`SCHEME_WSH`) — this is a user-facing protocol identifier. Changing it is a separate breaking change, not part of this spec.

**DOES NOT CHANGE — internal module names (low value, high churn):**
- `wshutil/`, `wshrpc.rs` module names — internal to `agentmux-srv`, not user-visible. Renaming is optional cosmetic churn.
- `wavebase.rs`, `wavefileutil.rs` — legacy names from Wave era, separate cleanup.

**DOES NOT CHANGE — deprecated/historical:**
- `src-tauri/` (deprecated Tauri host — 13+ refs in sidecar.rs, binary.rs, etc.)
- `docs/HANDOFF-*.md`, `docs/retros/`, `specs/archive/`, `VERSION_HISTORY.md`
- `docs/CRASH-DUMP-ANALYSIS-*.md` (historical forensics)

---

### Crate Rename: `agentmuxsrv-rs` → `agentmux-srv`

**Directory:** `mv agentmuxsrv-rs/ agentmux-srv/`

**`agentmux-srv/Cargo.toml`:**
```toml
[package]
name = "agentmux-srv"

[[bin]]
name = "agentmux-srv"
path = "src/main.rs"
```

**Verified file list (442 refs total, ~60 files in scope after excluding deprecated/historical):**

| File | Refs | What to change |
|------|------|---------------|
| `Cargo.toml` (workspace) | 1 | `members` list |
| `Cargo.lock` | 1 | Auto-regenerated by `cargo generate-lockfile` |
| `agentmux-srv/Cargo.toml` | 2 | `name`, `[[bin]] name` |
| `agentmux-srv/src/config.rs` | 1 | `#[command(name = "agentmux-srv")]` |
| `agentmux-srv/tests/integration_test.rs` | 3 | `CARGO_BIN_EXE_agentmux-srv`, spawn/log messages |
| `agentmux-srv/src/backend/reactive/registry.rs` | 1 | Doc comment |
| `.bump.json` | 2 | Target file path, git add path |
| `Taskfile.yml` | 26 | `cargo build -p`, copy commands, task names/descs, all 3 platforms |
| `agentmux-cef/src/sidecar.rs` | 11 | `backend_name`, all log strings, resolve paths, doc comments |
| `agentmux-cef/src/state.rs` | 2 | Doc comments |
| `agentmux-cef/src/commands/backend.rs` | 1 | Doc comment |
| `scripts/package-cef-portable.sh` | 3 | Binary filename, verify, copy |
| `scripts/package-portable.ps1` | 6 | Binary filename, copy, size check |
| `scripts/package-msix.ps1` | 1 | Binary source mapping |
| `scripts/bench-full.sh` | 3 | taskkill pattern, process list |
| `scripts/bench-compare.sh` | 4 | taskkill pattern, process list |
| `scripts/bench-memory-scaling.sh` | 3 | Process pattern, taskkill |
| `scripts/benchmark-startup.ps1` | 2 | Output text |
| `scripts/build-appimage.sh` | 1 | Comment |
| `tools/enable-crash-dumps.reg` | 2 | WER registry key path (exe name) |
| `wsh-rs/src/rpc/mod.rs` | 2 | Doc comments |
| `e2e/widget-click.test.ts` | 1 | Comment |
| `CLAUDE.md` | 5 | Architecture, build, task kill warning |
| `README.md` | 3 | Architecture diagram, build tasks |
| `BUILD.md` | 8 | Build guide, binary layout, log paths |
| `CONTRIBUTING.md` | 3 | Directory tree, architecture section |

**Active spec/analysis docs to update:**
- `docs/specs/sidecar-isolation.md` (15 refs)
- `docs/specs/sidecar-isolation-impl-plan.md` (11 refs)
- `docs/specs/sidecar-modularization-impl-plan.md` (10 refs)
- `docs/analysis/portable-zip-ci-vs-local.md` (14 refs)
- `docs/specs/portable-build-spec.md` (4 refs)
- `docs/macos-signing.md` (2 refs)
- `docs/exe-return-codes.md` (references)
- `specs/SPEC_BACKEND_LIFECYCLE.md` (11 refs)
- `specs/lan-awareness-and-embedded-jekt-api.md` (17 refs)
- `specs/CLEANUP_LEGACY_REMNANTS.md` (3 refs)
- `docs/specs/dead-code-strip.md`, `docs/specs/codex-gemini-cli-integration.md`, and others

---

### Crate Rename: `wsh-rs` → `agentmux-wsh`

**Directory:** `mv wsh-rs/ agentmux-wsh/`

**`agentmux-wsh/Cargo.toml`:**
```toml
[package]
name = "agentmux-wsh"

[[bin]]
name = "agentmux-wsh"
path = "src/main.rs"
```

**Verified file list (~150 refs total, ~30 files in scope):**

| File | Refs | What to change |
|------|------|---------------|
| `Cargo.toml` (workspace) | 1 | `members` list |
| `Cargo.lock` | — | Auto-regenerated |
| `agentmux-wsh/Cargo.toml` | 2 | `name`, `[[bin]] name` |
| `agentmux-wsh/src/cli/mod.rs` | 1 | `#[command(name = "agentmux-wsh")]` |
| `agentmux-wsh/src/cli/conn.rs` | 3 | Error messages referencing "wsh-rs" |
| `agentmux-wsh/src/cli/file.rs` | 2 | Error messages referencing "wsh-rs" |
| `agentmux-wsh/src/cli/info.rs` | 3 | Version output, error messages |
| `agentmux-wsh/src/cli/view.rs` | 4 | Not-yet-implemented error messages |
| `.bump.json` | 2 | Target file path, git add path |
| `Taskfile.yml` | 26 | `cargo build -p`, copy commands, task names, all 3 platforms, `dev:installwsh` |
| `agentmux-cef/src/sidecar.rs` | 12 | `deploy_wsh()`, source filename, all log strings |
| `agentmux-srv/src/backend/shellintegration.rs` | 8 | `find_wsh_binary()`, versioned path construction |
| `agentmux-srv/src/backend/shellintegration/bash.sh` | 1 | PATH comment + binary invocation |
| `agentmux-srv/src/backend/shellintegration/zsh.sh` | 1 | PATH comment + binary invocation |
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | 3 | Env var setting wsh path |
| `agentmux-srv/src/backend/remote/mod.rs` | 1 | `WSH_BINARY_NAME` constant → `"agentmux-wsh"` |
| `agentmux-srv/src/backend/wavebase.rs` | 1 | `REMOTE_FULL_WSH_BIN_PATH` → `"~/.agentmux/bin/agentmux-wsh"` |
| `scripts/package-cef-portable.sh` | 3 | Binary filename, verify, copy |
| `frontend/types/gotypes.d.ts` | 1 | `"wsh"` in union type → `"agentmux-wsh"` |
| `tests/copytests/cases/test*.sh` | 50+ | `wsh file copy` → `agentmux-wsh file copy` |
| `CLAUDE.md` | 3 | Architecture, build |
| `README.md` | 2 | Architecture, build |
| `BUILD.md` | 5 | Build guide, binary refs |
| `CONTRIBUTING.md` | 3 | Directory tree |

**Does NOT change (serialized / protocol):**

| File | What | Why |
|------|------|-----|
| `agentmux-srv/src/backend/remote/connparse.rs` | `SCHEME_WSH = "wsh"`, `wsh://` URI scheme | User-facing protocol — separate breaking change |
| `agentmux-srv/src/backend/wconfig.rs` | `conn_wsh_enabled`, `conn_wsh_path`, `conn_ask_before_wsh_install` | Serialized config fields — breaks user data |
| `agentmux-srv/src/backend/wslconn.rs` | `wsh_enabled`, `wsh_error` struct fields | Serde-renamed, internal only |
| `agentmux-srv/src/backend/blockcontroller/mod.rs` | `META_KEY_CMD_NO_WSH = "cmd:nowsh"` | Stored in block metadata |
| `agentmux-srv/src/backend/wshutil/` | Module dir and `wshrpc.rs` | Internal module names, cosmetic |
| `agentmux-srv/src/backend/remote/mod.rs` | `WSL_DOMAIN_SOCKET_PATH = "/var/run/wsh.sock"` | Socket path on remote hosts |

---

## Naming Convention

### The Four Binaries

```
agentmux         — launcher (top-level user entry point)
agentmux-cef     — CEF host (Chromium runtime)
agentmux-srv     — backend sidecar (WebSocket server, PTY, etc.)
agentmux-wsh     — shell integration CLI
```

### In `dist/` (build artifacts)
```
dist/bin/agentmux-srv-{VERSION}-{platform}.{arch}[.exe]
dist/bin/agentmux-wsh-{VERSION}-{platform}.{arch}[.exe]
dist/cef/agentmux-cef-{VERSION}-{platform}.{arch}[.exe]
dist/cef/agentmux-launcher-{VERSION}-{platform}.{arch}[.exe]
```

### In portable (what the user runs — and what Task Manager shows)
```
agentmux-cef-{VERSION}-x64-portable/
  agentmux-{VERSION}.exe                          → Task Manager: agentmux-0.33.27.exe
  runtime/
    agentmux-cef-{VERSION}.exe                    → Task Manager: agentmux-cef-0.33.27.exe
    agentmux-srv-{VERSION}.exe                    → Task Manager: agentmux-srv-0.33.27.exe
    agentmux-wsh-{VERSION}.exe                    → Task Manager: agentmux-wsh-0.33.27.exe
    libcef.dll, chrome_elf.dll, ...               (CEF runtime — unchanged)
    locales/en-US.pak
    frontend/index.html, ...
```

### In `dist/cef-dev/` (dev mode)
Same versioned names. `cef:dev:serve` copies from `dist/cef/` and preserves the versioned name.

### Windows examples (v0.33.27)
```
dist/bin/agentmux-srv-0.33.27-windows.x64.exe
dist/bin/agentmux-wsh-0.33.27-windows.x64.exe
dist/cef/agentmux-cef-0.33.27-windows.x64.exe
dist/cef/agentmux-launcher-0.33.27-windows.x64.exe
```

### macOS / Linux examples
```
dist/bin/agentmux-srv-0.33.27-darwin.arm64
dist/bin/agentmux-wsh-0.33.27-darwin.arm64
dist/cef/agentmux-cef-0.33.27-darwin.arm64
dist/cef/agentmux-launcher-0.33.27-darwin.arm64

dist/bin/agentmux-srv-0.33.27-linux.x64
dist/bin/agentmux-wsh-0.33.27-linux.x64
dist/cef/agentmux-cef-0.33.27-linux.x64
dist/cef/agentmux-launcher-0.33.27-linux.x64
```

---

## Changes Required

### 1. Taskfile.yml — Build Tasks

**`build:backend:rust:{platform}`** — new crate name + versioned output.

Current (Windows):
```yaml
- cargo build --release -p agentmuxsrv-rs
- Copy-Item target/release/agentmuxsrv-rs.exe dist/bin/agentmuxsrv-rs.x64.exe
```
New:
```yaml
- cargo build --release -p agentmux-srv
- Remove-Item dist/bin/agentmux-srv-*-windows.x64.exe -Force -ErrorAction SilentlyContinue
- Copy-Item target/release/agentmux-srv.exe dist/bin/agentmux-srv-{{.VERSION}}-windows.x64.exe
```

Same pattern for darwin (`.arm64`) and linux (`.x64`).

**`build:wsh:{platform}`** — new crate name + versioned output.

Current (Windows):
```yaml
- cargo build --release -p wsh-rs
- Copy-Item target/release/wsh.exe dist/bin/wsh-{{.VERSION}}-windows.x64.exe
```
New:
```yaml
- cargo build --release -p agentmux-wsh
- Remove-Item dist/bin/agentmux-wsh-*-windows.x64.exe -Force -ErrorAction SilentlyContinue
- Copy-Item target/release/agentmux-wsh.exe dist/bin/agentmux-wsh-{{.VERSION}}-windows.x64.exe
```

**`cef:build:{platform}`** — versioned output.

Current (Windows):
```yaml
- Copy-Item target/release/agentmux-cef.exe dist/cef/agentmux-cef.exe
```
New:
```yaml
- Remove-Item dist/cef/agentmux-cef-*-windows.x64.exe -Force -ErrorAction SilentlyContinue
- Copy-Item target/release/agentmux-cef.exe dist/cef/agentmux-cef-{{.VERSION}}-windows.x64.exe
```

**New task: `cef:build:launcher:{platform}`**

```yaml
cef:build:launcher:
    desc: Build the agentmux-launcher binary for the current platform.
    cmds:
        - task: cef:build:launcher:{{OS}}

cef:build:launcher:windows:
    internal: true
    platforms: [windows]
    cmds:
        - cargo build --release -p agentmux-launcher
        - powershell -Command "Remove-Item dist/cef/agentmux-launcher-*-windows.x64.exe -Force -ErrorAction SilentlyContinue; Copy-Item target/release/agentmux-launcher.exe dist/cef/agentmux-launcher-{{.VERSION}}-windows.x64.exe -Force"
        - echo "✓ Built agentmux-launcher-{{.VERSION}} for Windows"
```

### 2. `scripts/package-cef-portable.sh` — Versioned Runtime Names

```bash
VERSION=$(node -p "require('./package.json').version")

CEF_BIN="dist/cef/agentmux-cef-${VERSION}-windows.x64.exe"
SRV_BIN="dist/bin/agentmux-srv-${VERSION}-windows.x64.exe"
WSH_BIN="dist/bin/agentmux-wsh-${VERSION}-windows.x64.exe"
LAUNCHER_BIN="dist/cef/agentmux-launcher-${VERSION}-windows.x64.exe"

# Verify ALL versioned binaries exist — hard fail if any missing
for f in "$CEF_BIN" "$SRV_BIN" "$WSH_BIN" "$LAUNCHER_BIN" dist/cef/libcef.dll dist/frontend/index.html; do
    if [ ! -f "$f" ]; then
        echo "ERROR: $f not found — version mismatch or build skipped" >&2
        exit 1
    fi
done

# Copy with VERSIONED names (visible in Task Manager)
cp "$LAUNCHER_BIN" "$PORTABLE/agentmux-${VERSION}.exe"
cp "$CEF_BIN"      "$PORTABLE/runtime/agentmux-cef-${VERSION}.exe"
cp "$SRV_BIN"      "$PORTABLE/runtime/agentmux-srv-${VERSION}.exe"
cp "$WSH_BIN"      "$PORTABLE/runtime/agentmux-wsh-${VERSION}.exe"
```

Remove the `grep -ao "$VERSION"` verification — versioned filenames in `dist/` are proof enough.

### 3. `agentmux-launcher/src/main.rs` — Discover Versioned CEF Binary

The launcher currently hardcodes `"agentmux-cef.exe"`. It needs to find the versioned binary at runtime.

**Strategy:** Glob for `agentmux-cef-*.exe` in `runtime/`. There should be exactly one.

```rust
fn find_versioned_binary(dir: &std::path::Path, prefix: &str) -> Option<std::path::PathBuf> {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(prefix) && name_str.ends_with(ext) {
                return Some(entry.path());
            }
        }
    }
    None
}

fn main() {
    // ... existing DLL setup ...

    let real_exe = find_versioned_binary(&runtime_dir, "agentmux-cef-").unwrap_or_else(|| {
        eprintln!(
            "FATAL: No agentmux-cef-VERSION{} found in: {}\nContents:",
            if cfg!(target_os = "windows") { ".exe" } else { "" },
            runtime_dir.display()
        );
        // List what IS in the directory to aid debugging
        if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
            for entry in entries.flatten() {
                eprintln!("  {}", entry.file_name().to_string_lossy());
            }
        }
        std::process::exit(1);
    });
    // ... rest unchanged ...
}
```

### 4. `agentmux-cef/src/sidecar.rs` — Discover Versioned Backend Binary

`resolve_backend_binary()` (line 267) currently checks for `agentmuxsrv-rs.x64.exe`. Update to `agentmux-srv` with versioned discovery:

```rust
fn resolve_backend_binary(
    backend_name: &str,    // "agentmux-srv"
    exe_suffix: &str,
) -> Result<std::path::PathBuf, String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe: {}", e))?;
    let exe_dir = exe_path.parent().unwrap();
    let version = env!("CARGO_PKG_VERSION");

    // Versioned path — the ONLY supported layout
    // Portable: agentmux-srv-{version}.exe (same dir as CEF host)
    let versioned = exe_dir.join(format!("{}-{}{}", backend_name, version, exe_suffix));
    if versioned.exists() {
        tracing::info!("Using {} at: {:?}", backend_name, versioned);
        return Ok(versioned);
    }

    // Dev mode: unversioned agentmux-srv(.exe) adjacent to host (cargo puts it here)
    let dev = exe_dir.join(format!("{}{}", backend_name, exe_suffix));
    if dev.exists() {
        tracing::info!("Using dev-mode {} at: {:?}", backend_name, dev);
        return Ok(dev);
    }

    Err(format!(
        "FATAL: Backend binary not found.\n  Expected: {:?}\n  Dev mode: {:?}\n  Searched in: {:?}",
        versioned, dev, exe_dir
    ))
}
```

Call site:
```rust
let backend_name = "agentmux-srv";
```

### 5. `agentmux-cef/src/sidecar.rs` `deploy_wsh()` — Versioned agentmux-wsh

Currently looks for `wsh.exe` adjacent to the CEF host. Update to `agentmux-wsh-{version}.exe`:

```rust
fn deploy_wsh(app_path: &std::path::Path) {
    let version = env!("CARGO_PKG_VERSION");
    let wsh_src_name = if cfg!(windows) {
        format!("agentmux-wsh-{}.exe", version)
    } else {
        format!("agentmux-wsh-{}", version)
    };
    let bundled_wsh = app_path.join(&wsh_src_name);
    if !bundled_wsh.exists() {
        tracing::error!(
            "FATAL: Bundled agentmux-wsh not found at: {}",
            bundled_wsh.display()
        );
        return;  // Non-fatal for the app — wsh is optional for shell integration
    }
    // Deploy to ~/.agentmux/bin/agentmux-wsh
    // ...
}
```

### 6. `agentmux-srv/src/backend/shellintegration.rs` — wsh Binary Name

Shell integration scripts and the deploy logic reference `wsh` as the binary name. Update to `agentmux-wsh`:

```rust
// Was: "wsh"
let wsh_binary = "agentmux-wsh";
```

Shell scripts (`bash.sh`, `zsh.sh`, `fish.fish`, `pwsh.ps1`) that invoke `wsh` should invoke `agentmux-wsh` instead.

### 7. `cef:dev:serve` (Taskfile.yml line ~830)

Dev runner launch line changes:

```bash
# Find the versioned CEF binary
CEF_EXE=$(ls "$DEV_DIR"/agentmux-cef-*.exe 2>/dev/null | head -1)
if [ -z "$CEF_EXE" ]; then
    echo "❌ No versioned agentmux-cef binary in $DEV_DIR"
    exit 1
fi
cd "$DEV_DIR" && LD_LIBRARY_PATH=. "$CEF_EXE" --url=http://localhost:5173
```

### 8. `README.txt` in Portable ZIP

Generated dynamically since the launcher filename changes per version:

```bash
cat > "$PORTABLE/README.txt" <<EOF
AgentMux v$VERSION - Portable Edition

Quick Start:
  1. Extract this folder anywhere
  2. Run agentmux-${VERSION}.exe

Requirements:
  - Windows 10/11 x64
  - No installation needed
  - No admin rights required
EOF
```

### 9. `.bump.json`

```json
{
  "targets": [
    { "file": "agentmux-srv/Cargo.toml", "type": "toml", "path": "package.version" },
    { "file": "agentmux-wsh/Cargo.toml", "type": "toml", "path": "package.version" },
    { "file": "agentmux-cef/Cargo.toml", "type": "toml", "path": "package.version" },
    { "file": "agentmux-launcher/Cargo.toml", "type": "toml", "path": "package.version" }
  ],
  "git": {
    "add": [
      "agentmux-srv/Cargo.toml",
      "agentmux-wsh/Cargo.toml",
      "agentmux-cef/Cargo.toml",
      "agentmux-launcher/Cargo.toml"
    ]
  }
}
```

(Plus existing `package.json`, `Cargo.lock`, `VERSION_HISTORY.md`, `src-tauri/*` entries.)

### 10. `Cargo.toml` (workspace root)

```toml
members = ["src-tauri", "agentmux-srv", "agentmux-wsh", "agentmux-cef", "agentmux-launcher"]
```

---

## Task Manager Result

After this change, running two versions simultaneously shows:

| Process | v0.33.26 | v0.33.27 |
|---------|----------|----------|
| Launcher | `agentmux-0.33.26.exe` | `agentmux-0.33.27.exe` |
| CEF host | `agentmux-cef-0.33.26.exe` | `agentmux-cef-0.33.27.exe` |
| Backend | `agentmux-srv-0.33.26.exe` | `agentmux-srv-0.33.27.exe` |
| Shell | `agentmux-wsh-0.33.26.exe` | `agentmux-wsh-0.33.27.exe` |

All four processes clearly identified by name and version.

---

## Complete Verified File Change List

**Total scope: ~600 line changes across ~90 files** (excluding deprecated `src-tauri/` and historical docs).

### Crate renames (directory + Cargo)
| Old | New |
|-----|-----|
| `agentmuxsrv-rs/` | `agentmux-srv/` |
| `wsh-rs/` | `agentmux-wsh/` |
| `Cargo.toml` workspace `members` | Both renames |
| `Cargo.lock` | Auto-regenerated |

### Config / build / packaging
| File | Ref Count | What Changes |
|------|-----------|-------------|
| `.bump.json` | 4 | Both crate target paths + git add paths |
| `Taskfile.yml` | ~52 | All `build:backend:rust:*`, `build:wsh:*`, `cef:build:*`, new `cef:build:launcher:*`, `cef:dev:serve`, `tauri:copy-sidecars`, `dev:installwsh` — versioned output names |
| `scripts/package-cef-portable.sh` | 6 | Both binary refs — new names + versioned src/dest |
| `scripts/package-portable.ps1` | 6 | Backend binary refs |
| `scripts/package-msix.ps1` | 1 | Binary source mapping |
| `scripts/bench-full.sh` | 3 | Process patterns, taskkill — **edit only, never execute during migration** |
| `scripts/bench-compare.sh` | 4 | Process patterns, taskkill — **edit only, never execute during migration** |
| `scripts/bench-memory-scaling.sh` | 3 | Process patterns, taskkill — **edit only, never execute during migration** |
| `scripts/benchmark-startup.ps1` | 2 | Output text |
| `scripts/build-appimage.sh` | 1 | Comment |
| `tools/enable-crash-dumps.reg` | 2 | WER registry key → `agentmux-srv.exe` |

### Rust source — CEF host
| File | Ref Count | What Changes |
|------|-----------|-------------|
| `agentmux-cef/src/sidecar.rs` | 23 | `backend_name`, `resolve_backend_binary()` (hard fail), `deploy_wsh()` (hard fail), all log strings |
| `agentmux-cef/src/state.rs` | 2 | Doc comments |
| `agentmux-cef/src/commands/backend.rs` | 1 | Doc comment |
| `agentmux-launcher/src/main.rs` | 1 | Versioned CEF binary discovery (hard fail) |

### Rust source — backend (agentmux-srv)
| File | Ref Count | What Changes |
|------|-----------|-------------|
| `agentmux-srv/Cargo.toml` | 2 | Package name, bin name |
| `agentmux-srv/src/config.rs` | 1 | `#[command(name = "agentmux-srv")]` |
| `agentmux-srv/tests/integration_test.rs` | 3 | `CARGO_BIN_EXE_`, log messages |
| `agentmux-srv/src/backend/reactive/registry.rs` | 1 | Doc comment |
| `agentmux-srv/src/backend/shellintegration.rs` | 8 | `find_wsh_binary()`, binary name, paths |
| `agentmux-srv/src/backend/shellintegration/bash.sh` | 1 | Binary invocation |
| `agentmux-srv/src/backend/shellintegration/zsh.sh` | 1 | Binary invocation |
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | 3 | Env var setting wsh path |
| `agentmux-srv/src/backend/remote/mod.rs` | 1 | `WSH_BINARY_NAME` → `"agentmux-wsh"` |
| `agentmux-srv/src/backend/wavebase.rs` | 1 | `REMOTE_FULL_WSH_BIN_PATH` → `"~/.agentmux/bin/agentmux-wsh"` |

### Rust source — shell CLI (agentmux-wsh)
| File | Ref Count | What Changes |
|------|-----------|-------------|
| `agentmux-wsh/Cargo.toml` | 2 | Package name, bin name |
| `agentmux-wsh/src/cli/mod.rs` | 1 | `#[command(name = "agentmux-wsh")]` |
| `agentmux-wsh/src/cli/conn.rs` | 3 | Error messages |
| `agentmux-wsh/src/cli/file.rs` | 2 | Error messages |
| `agentmux-wsh/src/cli/info.rs` | 3 | Version output |
| `agentmux-wsh/src/cli/view.rs` | 4 | Error messages |
| `agentmux-wsh/src/rpc/mod.rs` | 2 | Doc comments referencing backend name |

### Frontend
| File | Ref Count | What Changes |
|------|-----------|-------------|
| `frontend/types/gotypes.d.ts` | 1 | `"wsh"` in union type → `"agentmux-wsh"` |

### Tests
| File | Ref Count | What Changes |
|------|-----------|-------------|
| `tests/copytests/cases/test*.sh` | 50+ files | `wsh file copy` → `agentmux-wsh file copy` |
| `e2e/widget-click.test.ts` | 1 | Comment |

### Documentation (active)
| File | Ref Count | What Changes |
|------|-----------|-------------|
| `CLAUDE.md` | 8 | Architecture, build commands, task kill warning, widgets path |
| `README.md` | 5 | Architecture diagram, build tasks |
| `BUILD.md` | 13 | Build guide, binary layout, log paths |
| `CONTRIBUTING.md` | 6 | Directory tree, architecture sections |
| `docs/specs/sidecar-isolation.md` | 15 | All backend refs |
| `docs/specs/sidecar-isolation-impl-plan.md` | 11 | All backend refs |
| `docs/specs/sidecar-modularization-impl-plan.md` | 10 | All backend refs |
| `docs/analysis/portable-zip-ci-vs-local.md` | 14 | Binary paths |
| `docs/specs/portable-build-spec.md` | 4 | Binary refs |
| `docs/macos-signing.md` | 4 | Signing paths for both binaries |
| `docs/exe-return-codes.md` | 1+ | Section header |
| `specs/SPEC_BACKEND_LIFECYCLE.md` | 11 | Backend refs |
| `specs/lan-awareness-and-embedded-jekt-api.md` | 17 | Backend + wsh refs |
| `specs/CLEANUP_LEGACY_REMNANTS.md` | 3 | Cargo.toml refs |
| + ~10 more spec/analysis docs | various | References to old names |

### Explicitly NOT changed
| File / Category | Why |
|----------------|-----|
| `src-tauri/**` (30+ refs) | Deprecated host — dead code |
| `docs/HANDOFF-*.md` | Historical session records |
| `docs/retros/`, `specs/archive/` | Historical |
| `VERSION_HISTORY.md` | Accurate past entries |
| `docs/CRASH-DUMP-ANALYSIS-*.md` | Historical forensics |
| `agentmux-srv/src/backend/wconfig.rs` config fields | Serialized to user config |
| `agentmux-srv/src/backend/wslconn.rs` struct fields | Serde-renamed, internal |
| `agentmux-srv/src/backend/remote/connparse.rs` `wsh://` | Protocol scheme — separate change |
| `agentmux-srv/src/backend/blockcontroller/mod.rs` `"cmd:nowsh"` | Stored in block metadata |
| `agentmux-srv/src/backend/wshutil/` module dir | Internal module name, cosmetic |
| `agentmux-srv/src/backend/remote/mod.rs` `wsh.sock` path | Socket path on remote hosts |

---

## No Fallbacks — Hard Fail

There are **no** backward compatibility fallbacks. If the versioned binary isn't found, the app crashes with a clear error listing what it expected and what it found. This is intentional:

- Stale/mismatched binaries are a real source of bugs (the original problem that prompted this spec)
- Silent fallback hides version mismatches — the exact thing we're trying to eliminate
- A hard crash with a descriptive error is infinitely easier to debug than "it works but the output is mangled"
- All binaries are built together in one `task` invocation — there's no valid scenario where only some are versioned

**If it breaks, you'll know immediately and know exactly why.**

---

## Migration Plan

> ⚠️ **Do not execute benchmark or bench-compare scripts during migration.** They contain `taskkill //im agentmuxsrv-rs.x64.exe` which kills by image name and will kill the running instance. Edit them, don't run them.

1. **Rename crate directories** (`agentmuxsrv-rs/` → `agentmux-srv/`, `wsh-rs/` → `agentmux-wsh/`)
2. **Update Cargo.toml** (workspace members, package names, bin names)
3. **Update `.bump.json`** (target paths, git add paths)
4. **Update Taskfile.yml** (build tasks, crate names, versioned output)
5. **Update Rust discovery code** (sidecar.rs, launcher main.rs) — hard fail, no fallbacks
6. **Update packaging scripts** (versioned source + destination names) — edit only, don't run bench scripts
7. **Update shell integration scripts** (`wsh` → `agentmux-wsh`) and test scripts (50+ `wsh file copy`)
8. **Update active docs** (CLAUDE.md, README.md, BUILD.md, CONTRIBUTING.md)
9. **Delete old artifacts** from `dist/bin/` and `dist/cef/` (40+ stale binaries, unversioned copies)
10. **Bump version, full build, verify** Task Manager shows all four versioned names
