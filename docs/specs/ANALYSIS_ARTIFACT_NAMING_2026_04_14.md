# Artifact Naming Analysis — Remove "cef" from Release Artifact Names
**Date:** 2026-04-14

## Background

When the Tauri host was replaced by CEF, "cef" was added to release artifact names
(`agentmux-cef-*`) to distinguish them from old Tauri artifacts. Tauri is now fully
removed; "cef" is redundant in user-facing artifact names. This analysis covers all
changes needed to rename artifacts back to clean `agentmux-*` names across Windows,
macOS (future), and Linux (future).

---

## Current State

### Windows (active)

There are **two** Windows packaging scripts with inconsistent naming:

| Script | ZIP output | Status |
|--------|-----------|--------|
| `scripts/package-cef-portable.sh` | `agentmux-cef-{version}-x64-portable.zip` | **Has "cef" — needs fix** |
| `scripts/package-portable.ps1` | `agentmux-{version}-x64-portable.zip` | Already clean ✓ |

`cef:package:portable` in Taskfile.yml calls the bash script. The PS1 is called by
`package:portable` (the other task). Both tasks exist; the bash variant is the primary build path.

### macOS / Linux (not yet implemented)

`package:macos` and `package:portable:linux` are stub tasks that print TODO messages.
When implemented, they must use the clean naming pattern from the start.

---

## Changes Required

### 1. `scripts/package-cef-portable.sh` — rename + fix artifact names

**Rename file:** `package-cef-portable.sh` → `package-portable.sh`

**Line-level changes:**

| Lines | Current | New |
|-------|---------|-----|
| 2–5 (comments) | `Package AgentMux CEF as a portable...` | `Package AgentMux as a portable...` |
| 17 | `PORTABLE="$OUTDIR/agentmux-cef-$VERSION-x64-portable"` | `PORTABLE="$OUTDIR/agentmux-$VERSION-x64-portable"` |
| 18 | `ZIPPATH="$OUTDIR/agentmux-cef-$VERSION-x64-portable.zip"` | `ZIPPATH="$OUTDIR/agentmux-$VERSION-x64-portable.zip"` |
| 54 | `agentmux-cef.exe → runtime/agentmux-cef-$VERSION.exe` | `agentmux-cef.exe → runtime/agentmux-$VERSION.exe` |
| 79–84 | Version verification checks `agentmux-cef-$VERSION.exe` | `agentmux-$VERSION.exe` |
| 97 | `ZIP_NAME="agentmux-cef-$VERSION-x64-portable.zip"` | `ZIP_NAME="agentmux-$VERSION-x64-portable.zip"` |
| 160 | `echo "[SUCCESS] CEF Portable v$VERSION"` | `echo "[SUCCESS] Portable v$VERSION"` |

**Note:** The binary inside the runtime dir is copied from `target/release/agentmux-cef.exe`
(the executable filename of the CEF host crate — unchanged). Only the *destination*
versioned filename changes: `agentmux-cef-$VERSION.exe` → `agentmux-$VERSION.exe`.

### 2. `Taskfile.yml` — update script reference

| Line | Current | New |
|------|---------|-----|
| 536 | `bash scripts/package-cef-portable.sh` | `bash scripts/package-portable.sh` |

### 3. `BUILD.md` — update docs

Two places in the portable output structure example:
- Directory: `agentmux-cef-{version}-x64-portable/` → `agentmux-{version}-x64-portable/`
- Versioned binary: `agentmux-cef-{version}.exe` → `agentmux-{version}.exe`

---

## Files That Do NOT Change

| File / Pattern | Reason |
|----------------|--------|
| `agentmux-cef/` workspace directory | Internal Cargo workspace crate name |
| `.bump.json` refs to `agentmux-cef/Cargo.toml` | Workspace path, not artifact |
| Binary `agentmux-cef.exe` (source name in target/) | That is the CEF host crate's binary name |
| Taskfile task names (`cef:build`, `cef:bundle`, `cef:package:portable`) | Internal task API |
| `Cargo.lock` crate names (`agentmux-cef`, `cef`, `cef-dll-sys`) | Auto-generated, dep names |
| `.gitignore` paths inside `agentmux-cef/` | Source tree, not artifacts |
| `scripts/package-portable.ps1` | Already uses clean naming |

---

## Tauri API Packages

`package.json` still lists `@tauri-apps/cli`, `@tauri-apps/api`, and 4 plugins. These
are **actively imported** in the frontend at runtime-detection boundaries:

- `frontend/tauri-bootstrap.ts` — conditionally loaded when `window.__TAURI_INTERNALS__` present
- `frontend/util/tauri-api.ts` — full AppApi shim for Tauri runtime
- `frontend/app/platform/ipc.ts` — dynamic imports of invoke/listen
- `frontend/util/notification.ts` — desktop notifications

The frontend uses a host-abstraction layer (`ipc.ts`) that dispatches to CEF HTTP IPC
or Tauri APIs depending on which runtime is detected. If Tauri is permanently EOL and
no deployment target uses it, these can be removed — but that is a separate cleanup
task and outside the scope of artifact renaming.

---

## Target Artifact Names (Windows)

```
Before:
  agentmux-cef-0.33.163-x64-portable/
  ├── agentmux.exe
  └── runtime/
      ├── agentmux-cef-0.33.163.exe   ← versioned copy of agentmux-cef.exe
      ├── agentmux-srv-0.33.163-windows.x64.exe
      └── libcef.dll, ...
  agentmux-cef-0.33.163-x64-portable.zip

After:
  agentmux-0.33.163-x64-portable/
  ├── agentmux.exe
  └── runtime/
      ├── agentmux-0.33.163.exe       ← same binary, clean name
      ├── agentmux-srv-0.33.163-windows.x64.exe
      └── libcef.dll, ...
  agentmux-0.33.163-x64-portable.zip
```

---

## macOS / Linux (future)

When packaging is implemented for these platforms, use:
- macOS: `agentmux-{version}-macos-{arch}.dmg` (or `.tar.gz` for portable)
- Linux: `agentmux-{version}-linux-x64.AppImage` (or `.tar.gz`)

No "cef" in any platform artifact name.

---

## Implementation Plan

1. Rename `scripts/package-cef-portable.sh` → `scripts/package-portable.sh`, apply all line changes above
2. Update `Taskfile.yml` line 536 to reference new script name
3. Update `BUILD.md` artifact name examples
4. Bump patch version, commit, build to verify
