# cef-dll-sys Build Failure Report — 2026-03-30

**Error:** `cef-dll-sys v146.2.0+146.0.9` build script fails with `Error: File I/O error: Access is denied. (os error 5)`
**Affects:** Both `cargo check` and `cargo build` (debug and release profiles)
**Reproducible:** Yes, persists after `rm -rf target/*/build/cef-dll-sys-*`

---

## Root Cause Analysis

### What cef-dll-sys does at build time

The `cef-dll-sys` crate's `build.rs` performs these steps:

1. **Check for `CEF_PATH` env var** — if set, validates an `archive.json` file exists at that path and skips download
2. **If no `CEF_PATH`:** Downloads a ~155 MB `.tar.bz2` archive from Spotify CDN (`cef-builds.spotifycdn.com`)
3. **Extracts** the tarball into `$OUT_DIR/<os>-<arch>/` (e.g., `target/release/build/cef-dll-sys-<hash>/out/cef_windows_x86_64/`)
4. **Moves** `Release/`, `Resources/`, `CMakeLists.txt`, `cmake/`, `include/`, `libcef_dll/` into the final `cef_windows_x86_64/` directory
5. **Runs cmake** (via Ninja) to build `libcef_dll_wrapper` static library
6. **Emits** `cargo::rustc-link-search` and `cargo::rustc-link-lib` directives

### Why "Access is denied" happens

Each cargo build gets a **unique hash** for the build script output directory. When the `cef-dll-sys` version or features change (or after `cargo clean`), Cargo assigns a **new hash**, creating a fresh `out/` directory. The build script then needs to re-download and re-extract.

**The access denied occurs during step 3 (extraction).** The `tar::Archive::new(decoder).unpack(&location)` call at `download-cef/src/lib.rs:489` extracts files that may collide with or reference paths locked by:

1. **Windows Search Indexer** — `SearchProtocolHost.exe` scans new files in `target/` and holds read locks
2. **Windows Defender / Antivirus** — scans newly created DLLs (`libcef.dll`, `chrome_elf.dll`) during extraction
3. **Previous partial extraction** — if a build was interrupted, leftover files with wrong ACLs remain

**Evidence:** The previous successful build hash (`fa579e07993fc188`) has a complete extraction. The new hash (`450daa7637e2dfde`) has no `out/` directory at all — the build script fails before creating any output.

### The deeper problem: CEF_PATH doesn't work either

When `CEF_PATH` is set to the previous build's extraction:
```
CEF_PATH=target/release/build/cef-dll-sys-fa579e07993fc188/out/cef_windows_x86_64
```
The build fails with `Error: File I/O error: The system cannot find the file specified. (os error 2)` because:
- `check_archive_json()` looks for `<cef_path>/archive.json`
- The `write_archive_json()` function writes this file after extraction
- The previous extraction only has `vk_swiftshader_icd.json` — no `archive.json`
- **The previous build likely used an older version of download-cef that didn't write `archive.json`**, or it was stripped during the `task cef:bundle` packaging step

---

## Isolation Problem

This connects to the user's requirement that **each AgentMux instance must be 100% isolated**:

### Current isolation gaps

| Resource | Isolated? | Issue |
|----------|-----------|-------|
| **Build output (`target/`)** | No | Single shared `target/` dir. Version bumps invalidate cef-dll-sys hash, triggering re-extraction of 155MB archive |
| **CEF runtime DLLs** | Partial | `task cef:bundle` copies DLLs to portable dir, but build still extracts fresh copy each time |
| **Cargo registry cache** | No | `~/.cargo/registry/` shared across all projects |
| **cmake build cache** | No | `target/*/build/cef-dll-sys-*/out/build/` wiped on hash change |
| **Portable instances** | Yes | Each portable ZIP has own `runtime/` with full CEF bundle |
| **User data dirs** | Yes | Versioned: `~/.agentmux/ai.agentmux.cef.v0-33-N/` |

### The rebuild trap

Every `bump patch` changes `Cargo.toml` versions, which:
1. Changes the crate version → new fingerprint hash
2. New hash → new `OUT_DIR` → cef-dll-sys re-downloads 155MB
3. Download + extract takes ~2-5 minutes
4. Windows file scanning can cause "access denied" during extraction
5. **If extraction fails, there's no fallback** — the build is dead until manually fixed

---

## Proposed Fixes

### Fix 1: Stable CEF extraction path (recommended)

Set `CEF_PATH` to a **persistent, shared location** outside of `target/`:

```bash
# One-time setup: extract CEF to a stable location
mkdir -p /c/Systems/cef-runtime
# Copy from a successful build:
cp -r target/release/build/cef-dll-sys-fa579e07993fc188/out/cef_windows_x86_64 /c/Systems/cef-runtime/cef_146.0.9_windows_x86_64

# Generate missing archive.json:
echo '{"name":"cef_binary_146.0.9+g3ca6a87+chromium-146.0.7680.165_windows64_minimal.tar.bz2"}' \
  > /c/Systems/cef-runtime/cef_146.0.9_windows_x86_64/archive.json

# Set in environment (add to .bashrc or settings.json env):
export CEF_PATH="C:/Systems/cef-runtime/cef_146.0.9_windows_x86_64"
```

**Pros:** No re-download on version bumps. Instant builds.
**Cons:** Must manually update when CEF version changes (rare — tied to cef-dll-sys crate version).

### Fix 2: Windows Defender exclusion

```powershell
Add-MpPreference -ExclusionPath "C:\Systems\agentmux\target"
```

Prevents Defender from scanning during extraction. Reduces "access denied" frequency but doesn't eliminate the re-download problem.

### Fix 3: Pre-build script in Taskfile

Add a `task prebuild` that:
1. Checks if `CEF_PATH` is set and valid
2. If not, checks if any existing `target/*/build/cef-dll-sys-*/out/cef_windows_x86_64/` exists
3. Copies it to a stable location and sets `CEF_PATH`
4. Generates `archive.json` if missing

### Fix 4: Workspace-level CEF_PATH in `.cargo/config.toml`

```toml
[env]
CEF_PATH = "C:/Systems/cef-runtime/cef_146.0.9_windows_x86_64"
```

Persists across all cargo invocations without needing shell env setup.

---

## Immediate Recovery Steps

To unblock the current build:

```bash
# 1. Create stable CEF dir
mkdir -p /c/Systems/cef-runtime

# 2. Copy existing extraction
cp -r /c/Systems/agentmux/target/release/build/cef-dll-sys-fa579e07993fc188/out/cef_windows_x86_64 \
      /c/Systems/cef-runtime/cef_146.0.9_windows_x86_64

# 3. Create archive.json (required by check_archive_json)
echo '{"name":"cef_binary_146.0.9+g3ca6a87+chromium-146.0.7680.165_windows64_minimal.tar.bz2"}' \
  > /c/Systems/cef-runtime/cef_146.0.9_windows_x86_64/archive.json

# 4. Set env and rebuild
export CEF_PATH="C:/Systems/cef-runtime/cef_146.0.9_windows_x86_64"
cargo check -p agentmux-cef --release

# 5. If it works, persist in .cargo/config.toml
echo '[env]' >> /c/Systems/agentmux/.cargo/config.toml
echo 'CEF_PATH = "C:/Systems/cef-runtime/cef_146.0.9_windows_x86_64"' >> /c/Systems/agentmux/.cargo/config.toml
```

---

## Build Script Source Reference

- **cef-dll-sys build.rs:** `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cef-dll-sys-146.2.0+146.0.9/build.rs`
- **download-cef lib.rs:** `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/download-cef-2.3.1/src/lib.rs`
- Key functions:
  - `build.rs:29` — `if !fs::exists(&cef_dir)?` decides download vs reuse
  - `build.rs:35` — `version.download_archive(&out_dir, false)` — 155MB download
  - `build.rs:37` — `extract_target_archive()` — bz2 extraction (where access denied hits)
  - `build.rs:56` — `cmake::Config::new(&cef_dir)` — builds libcef_dll_wrapper via Ninja
  - `lib.rs:489` — `tar::Archive::new(decoder).unpack(&location)` — actual file I/O failure point
  - `lib.rs:108` — `File::open(archive_json_path(location))` — why CEF_PATH fails without archive.json
