# Spec: Portable Build Cleanup

## Problem

`task cef:package:portable` is brittle on Windows and fails silently.
Two independent issues cause it to break:

---

### Issue 1: `scripts/package-cef-portable.sh` fails silently on Windows

**Root cause:** The script uses `grep -ao` on `.exe` binary files to verify the
embedded version string. On Windows (MSYS2/Git Bash), `grep -a` does not handle
binary files correctly — it returns exit code 1 (no match) even when the version
IS present. Combined with `set -euo pipefail`, the script exits silently with no
error message at the version-check step (line 78).

**Evidence:** Running `grep -ao "0.33.17" target/release/agentmux-cef.exe` in
MSYS2 bash returns nothing. The same check via PowerShell `[System.IO.File]
::ReadAllBytes` + ASCII string search finds the version correctly.

**Fix:** Replace `scripts/package-cef-portable.sh` with
`scripts/package-cef-portable.ps1` and update `task cef:package:portable` to
call `pwsh scripts/package-cef-portable.ps1`. The PowerShell script does
everything the bash script does but uses `.NET` byte array search for binary
version verification instead of `grep`.

---

### Issue 2: `cef:build:windows` (`cargo build -p agentmux-cef`) fails with "Access Denied"

**Root cause:** `cef-dll-sys` build script downloads the CEF binary distribution
(~100 MB) on first build for a new hash. On this machine it gets "File I/O error:
Access is denied. (os error 5)" during the download/extraction step.

The crate supports a `CEF_PATH` env var: if it points to a directory containing
a valid `archive.json` and the CEF SDK files (`libcef.lib`, `CMakeLists.txt`,
etc.), the build script skips the download entirely and uses that directory. Once
a successful build has happened, the extraction lives at:

```
target/release/build/cef-dll-sys-{hash}/out/cef_windows_x86_64/
```

Subsequent builds with a new crate hash fail again because they don't know about
the existing extraction.

**Fix:** Auto-detect the cached CEF extraction and set `CEF_PATH` automatically
in `task cef:build:windows` before calling `cargo build`. Detection logic:

```bash
# Find newest cef_windows_x86_64 dir with a valid archive.json
cefPath=$(find target/release/build -name "archive.json" \
  -path "*/cef_windows_x86_64/*" 2>/dev/null \
  | sort | tail -1 | xargs dirname)
```

If found, export `CEF_PATH=$cefPath` before `cargo build`. If not found (first
ever build), proceed without `CEF_PATH` and the download will run normally.

Updated `task cef:build:windows`:

```yaml
cef:build:windows:
    internal: true
    platforms: [windows]
    cmds:
        - cmd: |
            cefPath=$(find target/release/build -name "archive.json" \
              -path "*/cef_windows_x86_64/*" 2>/dev/null \
              | sort | tail -1 | xargs -r dirname)
            if [ -n "$cefPath" ]; then
              echo "Using cached CEF at: $cefPath"
              export CEF_PATH="$(cygpath -w "$cefPath")"
            fi
            cargo build --release -p agentmux-cef
        - cmd: powershell -Command "New-Item -ItemType Directory -Force -Path dist/cef | Out-Null; Copy-Item target/release/agentmux-cef.exe dist/cef/agentmux-cef.exe -Force"
        - echo "✓ Built agentmux-cef for Windows"
```

---

## Deliverables

1. **`scripts/package-cef-portable.ps1`** — PowerShell port of the bash script.
   - Same logic: verify files exist, create directory structure, copy, verify
     embedded version (via byte search), ZIP.
   - Uses `[System.IO.File]::ReadAllBytes` + `-match` for version check instead
     of `grep -ao`.
   - Accepts optional `$OutputDir` parameter (default `$HOME\Desktop`).

2. **`Taskfile.yml` — update `cef:build:windows`** — add CEF cache auto-detect
   before `cargo build`.

3. **`Taskfile.yml` — update `cef:package:portable`** — call
   `pwsh scripts/package-cef-portable.ps1` instead of
   `bash scripts/package-cef-portable.sh`.

4. **`scripts/package-cef-portable.sh`** — keep as fallback for Linux/macOS CI,
   but add a comment noting it is not used on Windows.

## Out of Scope

- Fixing the root cause of the "Access Denied" on the first-ever CEF download
  (likely a machine-specific antivirus or temp-dir policy). The auto-detect
  workaround is sufficient for day-to-day development.
- Changing the portable directory structure or contents.
