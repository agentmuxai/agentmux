# Implementation Plan: Version-Stamped Sidecar Isolation

**Status:** Ready to implement
**Author:** AgentA
**Date:** 2026-03-21
**Depends on:** [sidecar-isolation.md](sidecar-isolation.md)

---

## Goal

Every component of AgentMux is version-stamped and isolated, so multiple versions can run in parallel on Windows without build conflicts or binary lock contention.

```
Before:  src-tauri/binaries/agentmuxsrv-rs-x86_64-pc-windows-msvc.exe  ← shared, locked
After:   {app_data_dir}/sidecar/agentmuxsrv-rs.exe                      ← per-version copy
```

---

## Consistent Versioning Pattern (All Components)

| Component | Current | After This PR |
|-----------|---------|--------------|
| `agentmux.exe` | Versioned via Tauri identifier | ✅ No change needed |
| WebView2 UDF | Versioned via Tauri identifier | ✅ No change needed |
| Auth dirs | `{app_data_dir}/auth/{provider}/` | ✅ Already done |
| `agentmuxsrv-rs` | Fixed name in `src-tauri/binaries/` | ✅ Copied to `{app_data_dir}/sidecar/` |
| `wsh` | Fixed name in `src-tauri/binaries/` | ✅ Copied to `{app_data_dir}/sidecar/` |

---

## Implementation Steps

### Step 1 — Add `copy_sidecar_to_data_dir` in `src-tauri/src/sidecar.rs`

New function that runs at startup in `setup()`:

```rust
/// Copy a bundled sidecar to the version-isolated app data dir.
///
/// Tauri's externalBin places sidecars at src-tauri/binaries/{name}-{triple},
/// which is a fixed path shared across all versions. On Windows, a running
/// sidecar holds a read lock that blocks tauri-build during builds of new versions.
///
/// Fix: copy to {app_data_dir}/sidecar/{name}(.exe) on first launch, run from there.
/// The app data dir is already version-isolated via the Tauri identifier.
/// The copy is skipped if the destination already exists and has the same size+mtime.
pub fn ensure_versioned_sidecar(
    app: &tauri::AppHandle,
    sidecar_name: &str,   // e.g. "agentmuxsrv-rs"
) -> Result<PathBuf, String> {
    let data_dir = app.path().app_data_dir()
        .map_err(|e| format!("data dir error: {e}"))?;
    let sidecar_dir = data_dir.join("sidecar");
    std::fs::create_dir_all(&sidecar_dir)
        .map_err(|e| format!("create sidecar dir: {e}"))?;

    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    let dest = sidecar_dir.join(format!("{}{}", sidecar_name, exe_suffix));

    // Find the source binary in the resource dir (Tauri places externalBin there)
    let triple = std::env::consts::ARCH; // simplified — use full triple from build env
    let src_name = format!("{}-{}{}", sidecar_name, TARGET_TRIPLE, exe_suffix);
    let src = app.path().resource_dir()
        .map_err(|e| format!("resource dir: {e}"))?
        .join(&src_name);

    // Skip copy if dest exists and is up-to-date (same size + mtime)
    if dest.exists() {
        let src_meta = std::fs::metadata(&src).ok();
        let dst_meta = std::fs::metadata(&dest).ok();
        if let (Some(s), Some(d)) = (src_meta, dst_meta) {
            if s.len() == d.len() && s.modified().ok() == d.modified().ok() {
                return Ok(dest);
            }
        }
    }

    std::fs::copy(&src, &dest)
        .map_err(|e| format!("copy {} → {}: {}", src.display(), dest.display(), e))?;

    // Set executable bit on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).ok();
    }

    tracing::info!(
        src = %src.display(),
        dest = %dest.display(),
        "[sidecar] copied to versioned data dir"
    );

    Ok(dest)
}
```

The `TARGET_TRIPLE` is a compile-time constant set in `build.rs`:
```rust
// build.rs
println!("cargo:rustc-env=TARGET_TRIPLE={}", std::env::var("TARGET").unwrap());
```

### Step 2 — Modify `src-tauri/src/sidecar.rs` spawn logic

Currently the sidecar is spawned using Tauri's built-in `app.shell().sidecar("agentmuxsrv-rs")`. Change to spawn from the versioned copy:

```rust
// Before:
let (_, child) = app.shell()
    .sidecar("agentmuxsrv-rs")
    .unwrap()
    .spawn()
    .unwrap();

// After:
let sidecar_path = ensure_versioned_sidecar(&app, "agentmuxsrv-rs")
    .expect("failed to prepare versioned sidecar");

let child = std::process::Command::new(&sidecar_path)
    .args(&[...])  // same args as before
    .spawn()
    .expect("failed to spawn agentmuxsrv-rs");
```

> **Note:** Bypassing Tauri's `.sidecar()` means losing the auto-injected `TAURI_*` env vars. These must be passed manually (or we keep using `.sidecar()` with a custom path — check Tauri 2.x API for `Command::new_sidecar_at_path()`).

### Step 3 — Apply same pattern to `wsh`

`wsh` is short-lived (called per-terminal open), but the `src-tauri/binaries/wsh-{triple}` naming still causes build conflicts. Copy it the same way:

```rust
let wsh_path = ensure_versioned_sidecar(&app, "wsh")?;
// Store in app state for use by shell controller
```

The shell integration code in `agentmuxsrv-rs` currently resolves `wsh` by looking in `dist/bin/wsh-{version}-{platform}.x64.exe`. This can be updated to also accept the versioned sidecar path passed via an env var set by the Tauri host at startup.

### Step 4 — Update `CLAUDE.md` build notes

Add to `CLAUDE.md` "Common Issues":
```markdown
### Sidecar build conflict (Windows — fixed in vX.Y.Z)
Building a new version while the previous version's sidecar is running caused
`PermissionDenied` in tauri-build. Fixed by copying sidecars to the version-isolated
app data dir at startup. For versions before the fix: kill orphaned `agentmuxsrv-rs.exe`
(not `.x64.exe`) processes before building.
```

### Step 5 — Remove workaround from CLAUDE.md after ship

Once this is on main and stable, remove the "kill orphaned sidecar" workaround note.

---

## Tauri 2.x API Reference

Tauri 2.x `tauri-plugin-shell` exposes:
- `app.shell().sidecar("name")` — resolves by name from `externalBin`
- Does NOT support custom paths directly

Options:
1. Keep using `.sidecar()` for the resource resolution, but intercept and re-spawn from the copy
2. Use `tauri::process::Command` directly with the versioned path (need to pass `TAURI_*` env manually)
3. Check if Tauri 2.x supports `Command::new_sidecar_at_path()` (not confirmed — verify in source)

**Recommended:** Option 1 — use `.sidecar()` to resolve the resource path, extract it, copy to data dir, then re-spawn from there using `std::process::Command`.

---

## File Changes Summary

| File | Change |
|------|--------|
| `src-tauri/build.rs` | Emit `TARGET_TRIPLE` compile env var |
| `src-tauri/src/sidecar.rs` | Add `ensure_versioned_sidecar()`, update spawn to use versioned copy |
| `src-tauri/src/lib.rs` | Call `ensure_versioned_sidecar` for both sidecars in `setup()` |
| `agentmuxsrv-rs/src/backend/shell.rs` | Accept wsh path from env var set by Tauri host |
| `CLAUDE.md` | Add build note, remove after ship |

---

## Testing Checklist

- [ ] Build v0.32.N while v0.32.N-1 is running — no `PermissionDenied`
- [ ] `task dev` while release build is running — no conflict
- [ ] Sidecar copies to `{app_data_dir}/sidecar/` on first launch
- [ ] Second launch skips copy (mtime/size match)
- [ ] Shell integration (`wsh`) still works after path change
- [ ] Linux/macOS: no regression (copy still happens, executable bit set)
- [ ] Windows portable ZIP: sidecar resolves correctly from bundled resources

---

## Priority

**High.** Blocks parallel version development workflow on Windows.
Implement immediately after current auth isolation PR is on main.
