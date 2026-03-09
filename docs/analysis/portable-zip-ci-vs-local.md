# Analysis: CI vs Local Windows Portable ZIP

**Date:** 2026-03-09
**Author:** AgentX
**Correction:** Initial analysis was inverted — the LOCAL build is correct, the CI build is broken.

---

## TL;DR

The CI-built portable ZIP uses **Tauri target-triple filenames** (`agentmuxsrv-rs-x86_64-pc-windows-msvc.exe`) sourced from `src-tauri/binaries/`. But the app's portable-mode detection code in `sidecar.rs` explicitly looks for **`.x64.exe` suffixed names** (`agentmuxsrv-rs.x64.exe`). CI copies the wrong source directory.

**The local `scripts/package-portable.ps1` is correct.** The CI `tauri-build.yml` portable step needs to match it.

---

## How the App Actually Finds Its Binaries at Runtime

### sidecar.rs — portable mode detection

```rust
// src-tauri/src/sidecar.rs:114-136
let backend_name = "agentmuxsrv-rs";

let portable_path = std::env::current_exe().ok().and_then(|exe_path| {
    let exe_dir = exe_path.parent()?;
    let portable_binary = exe_dir.join(format!("{}.x64.exe", backend_name));
    if portable_binary.exists() {
        Some(portable_binary)
    } else {
        None
    }
});

let sidecar_cmd = if let Some(portable_exe) = portable_path {
    shell.command(portable_exe)          // portable mode: run .x64.exe directly
} else {
    shell.sidecar(backend_name)...       // installer mode: use Tauri sidecar (target triple)
};
```

The app checks for `agentmuxsrv-rs.x64.exe` alongside itself. If found → portable mode (direct spawn). If not → falls back to Tauri's sidecar system (which uses target-triple names). The two naming systems are **mutually exclusive by design**.

### agentmuxsrv-rs — how it finds wsh

```rust
// agentmuxsrv-rs/src/backend/shellintegration.rs:167-196
pub fn find_wsh_binary() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let version = env!("CARGO_PKG_VERSION");

    // Tries versioned name first
    let versioned = exe_dir.join(format!("wsh-{}-windows.x64.exe", version));
    if versioned.exists() { return Some(versioned); }

    // Fallback: plain wsh.exe
    let plain = exe_dir.join("wsh.exe");
    if plain.exists() { return Some(plain); }

    None
}
```

Backend looks for `wsh-<VERSION>-windows.x64.exe` **in its own directory** (same dir as `agentmuxsrv-rs.x64.exe`, which is the ZIP root).

---

## What Each Build Puts in the ZIP

### Local — `scripts/package-portable.ps1` ✅

Sources from `dist/bin/` (arch-suffixed names, pre-Tauri-rename stage):

```
agentmux-0.31.95-x64-portable/
├── agentmux.exe
├── agentmuxsrv-rs.x64.exe          ← sidecar.rs finds this → portable mode ✅
├── bin/
│   └── wsh-0.31.95-windows.x64.exe ← version-isolated install copy ✅
└── README.txt
```

**wsh at root?** — The local script puts wsh only in `bin/`, not at root. But `find_wsh_binary()` looks in the `agentmuxsrv-rs` directory (ZIP root). **This means wsh isn't found at root either in the local build.** However this works because the frontend (`sidecar.rs`) ALSO deploys wsh from resources — the `bin/` copy IS found via a separate path resolution that uses `AGENTMUX_APP_PATH`.

### CI — `tauri-build.yml` PowerShell step ❌

Sources from `src-tauri/binaries/` (Tauri target-triple names, post-rename stage):

```
(flat, no subdirectory)
├── agentmux.exe
├── agentmuxsrv-rs-x86_64-pc-windows-msvc.exe  ← sidecar.rs looks for .x64.exe → NOT FOUND ❌
├── wsh-x86_64-pc-windows-msvc.exe              ← find_wsh_binary looks for versioned name → NOT FOUND ❌
├── bin/
│   └── wsh-0.31.95-windows.x64.exe            ← this part is right ✅
└── schema/                                      ← not needed in portable (loaded from resources)
```

**Result:** `agentmuxsrv-rs.x64.exe` not found → sidecar.rs falls through to Tauri sidecar lookup → also fails because the target-triple .exe is present but Tauri resolves it via its own manifest, not by scanning. **Backend never launches.**

---

## Root Cause

The CI portable step was written to copy `src-tauri/binaries/*` — the directory Tauri uses for **installer bundling**. These files have target-triple names required by the Tauri bundler, not the names required by the app's portable-mode runtime detection.

`dist/bin/` and `src-tauri/binaries/` serve different purposes:

| Directory | Purpose | Naming Convention |
|-----------|---------|-------------------|
| `dist/bin/` | Portable ZIP, backend deployment | `agentmuxsrv-rs.x64.exe`, `wsh-<ver>-windows.x64.exe` |
| `src-tauri/binaries/` | Tauri installer bundling | `agentmuxsrv-rs-x86_64-pc-windows-msvc.exe` |

The CI step grabbed from the wrong one.

---

## Fix

The CI "Create Windows portable ZIP" step needs to source from `dist/bin/` and mirror `scripts/package-portable.ps1`:

```powershell
# WRONG (current CI):
Copy-Item "src-tauri/binaries/*" "$staging/" -Recurse

# CORRECT (matching local script):
Copy-Item "dist/bin/agentmuxsrv-rs.x64.exe" "$staging/"
Copy-Item "dist/bin/wsh-${VERSION}-windows.x64.exe" "$staging/"
New-Item -ItemType Directory -Force -Path "$staging/bin" | Out-Null
Copy-Item "dist/bin/wsh-${VERSION}-windows.x64.exe" "$staging/bin/"
```

Also remove the `schema/` copy — schema is bundled as a Tauri resource and loaded at runtime. Portable mode doesn't need a separate copy.

The filename and subdirectory structure should also match the local convention:
- **Filename:** `AgentMux_<ver>_x64-portable.zip` (keep — matches other release assets)
- **Contents root:** files at ZIP root (no wrapping subdirectory, consistent with other platform installers)

---

## Summary

| | Local (`package-portable.ps1`) | CI (`tauri-build.yml`) |
|--|-------------------------------|------------------------|
| **Binary source** | `dist/bin/` ✅ | `src-tauri/binaries/` ❌ |
| **agentmuxsrv name** | `agentmuxsrv-rs.x64.exe` ✅ | `agentmuxsrv-rs-x86_64-pc-windows-msvc.exe` ❌ |
| **wsh name** | `wsh-<ver>-windows.x64.exe` ✅ | `wsh-x86_64-pc-windows-msvc.exe` ❌ |
| **Backend launches** | ✅ | ❌ |
| **Shell integration** | ✅ | ❌ |
| **Works** | ✅ | ❌ |
