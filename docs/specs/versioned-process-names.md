# Versioned Process Names Spec

**Date:** 2026-04-04
**Status:** Draft
**Problem:** When agentmux crashes, WER dumps and Event Viewer show `agentmux-cef.exe` with no version. Task Manager shows the same generic name. With multiple versions on disk, it's impossible to tell which version crashed.

---

## How Windows Resolves Process Names

| Tool / Column | Source | Example |
|---|---|---|
| Task Manager → Processes tab "Name" | PE `FileDescription` | "AgentMux CEF v0.33.43" |
| Task Manager → Details tab "Image name" | Exe filename on disk | `agentmux-cef-0.33.43.exe` |
| Task Manager → Details tab "Description" | PE `FileDescription` | "AgentMux CEF v0.33.43" |
| WER crash dump filename | Exe filename on disk | `agentmux-cef-0.33.43.exe.12345.dmp` |
| Event Viewer "Faulting application name" | Exe filename on disk | `agentmux-cef-0.33.43.exe` |
| Event Viewer "Faulting application version" | PE `FileVersion` | `0.33.43` |
| Resource Monitor "Image" | Exe filename on disk | `agentmux-cef-0.33.43.exe` |
| Process Explorer "Description" | PE `FileDescription` | "AgentMux CEF v0.33.43" |
| PowerShell `Get-Process` ProcessName | Exe filename minus `.exe` | `agentmux-cef-0.33.43` |

**Key insight:** Two independent mechanisms must both carry the version:
1. **PE VERSIONINFO resource** — controls Task Manager friendly name
2. **Filename on disk** — controls crash dumps, Event Viewer, Resource Monitor

---

## Current State (Broken)

### PE Resources (`build.rs`)

Both `agentmux-cef/build.rs` and `agentmux-launcher/build.rs` only set the icon. The `winres` crate auto-fills defaults from Cargo.toml:

| Field | Current value (auto from Cargo.toml) |
|---|---|
| FileDescription | `"agentmux-cef"` (from `package.name`) |
| ProductName | `"agentmux-cef"` (from `package.name`) |
| FileVersion | `"0.33.43"` (from `package.version`) — correct but not visible in name |
| ProductVersion | `"0.33.43"` — correct |

**Problem:** `FileDescription` has no version number, so Task Manager shows "agentmux-cef".

### Filename on Disk

| Binary | Filename in portable | Versioned? |
|---|---|---|
| agentmux-cef | `runtime/agentmux-cef.exe` | NO |
| agentmux-srv | `runtime/agentmux-srv-0.33.43-windows.x64.exe` | YES |
| wsh | `runtime/wsh.exe` | NO |
| launcher | `agentmux.exe` | NO (by design) |

**Problem:** `agentmux-cef.exe` is the main process that shows in Task Manager and crash dumps, but has no version in its filename.

---

## Fix

### 1. PE VERSIONINFO — Embed version in FileDescription

Update `agentmux-cef/build.rs`:

```rust
#[cfg(target_os = "windows")]
{
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut res = winres::WindowsResource::new();
    res.set("FileDescription", &format!("AgentMux CEF v{}", version));
    res.set("ProductName", "AgentMux");
    res.set("CompanyName", "AgentMux");
    res.set("InternalName", "agentmux-cef");
    // Icon
    let icon_path = std::path::Path::new("resources/win/agentmux.ico");
    if icon_path.exists() {
        res.set_icon(icon_path.to_str().unwrap());
    }
    res.compile().expect("winres");
}
```

Same pattern for `agentmux-launcher/build.rs`:
```rust
res.set("FileDescription", &format!("AgentMux v{}", version));
```

And for `agentmux-srv` if it has a build.rs.

**Result:** Task Manager shows "AgentMux CEF v0.33.43" in the Name column.

### 2. Filename on Disk — Rename in packaging

Update `scripts/package-cef-portable.sh` to version the CEF binary:

```bash
# Before:
cp target/release/agentmux-cef.exe "$PORTABLE/runtime/"

# After:
cp target/release/agentmux-cef.exe "$PORTABLE/runtime/agentmux-cef-$VERSION.exe"
```

Update `agentmux-launcher/src/main.rs` to find the versioned binary:

```rust
// Try versioned binary first (e.g., agentmux-cef-0.33.43.exe)
// Fall back to plain name for dev mode
let versioned = format!("agentmux-cef-{}.exe", env!("CARGO_PKG_VERSION"));
let real_exe = if runtime_dir.join(&versioned).exists() {
    runtime_dir.join(&versioned)
} else {
    runtime_dir.join("agentmux-cef.exe")
};
```

**Result:** WER dumps show `agentmux-cef-0.33.43.exe.12345.dmp`.

### 3. CEF Subprocesses

CEF spawns GPU/renderer/utility processes using the same executable. Since the main binary is now `agentmux-cef-0.33.43.exe`, all subprocesses automatically inherit the versioned name. No extra work needed — `browser_subprocess_path` in CEF settings already points to `current_exe()`.

### 4. Launcher stays unversioned

The launcher is the user-facing entry point (`agentmux.exe`). It should NOT have a version in its filename — users double-click it. But its PE `FileDescription` should show the version so Task Manager is clear:

| Process in Task Manager | Shows as |
|---|---|
| `agentmux.exe` (launcher) | "AgentMux v0.33.43" |
| `agentmux-cef-0.33.43.exe` (browser) | "AgentMux CEF v0.33.43" |
| `agentmux-cef-0.33.43.exe` (gpu) | "AgentMux CEF v0.33.43" |
| `agentmux-cef-0.33.43.exe` (renderer) | "AgentMux CEF v0.33.43" |
| `agentmux-srv-0.33.43-windows.x64.exe` | (already versioned) |

### 5. MUI Cache Invalidation

Windows caches `FileDescription` in the MUI Cache registry (`HKCU\Software\Classes\Local Settings\MuiCache`). After updating PE resources, the cache entry for the old exe path must be invalidated. This happens automatically when the filename changes (new path = new cache entry). For the launcher (`agentmux.exe`) which keeps its name, the cache updates when the file's modification timestamp changes (i.e., on rebuild).

---

## Files to Change

| File | Change |
|---|---|
| `agentmux-cef/build.rs` | Set `FileDescription` with version |
| `agentmux-launcher/build.rs` | Set `FileDescription` with version |
| `agentmux-launcher/src/main.rs` | Glob for `agentmux-cef-*.exe` in runtime/ |
| `scripts/package-cef-portable.sh` | Rename agentmux-cef.exe to versioned name |

---

## Verification

After build + package:

```powershell
# Check PE resources
(Get-Item runtime\agentmux-cef-0.33.43.exe).VersionInfo | Format-List FileDescription, ProductName, FileVersion

# Should show:
#   FileDescription : AgentMux CEF v0.33.43
#   ProductName     : AgentMux
#   FileVersion     : 0.33.43
```

In Task Manager → Processes tab, should show "AgentMux CEF v0.33.43" instead of "agentmux-cef".
