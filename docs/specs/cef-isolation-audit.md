# CEF Version Isolation Audit

**Status:** Bug confirmed — CEF does not fully isolate versions like Tauri does
**Version:** 0.32.111
**Severity:** High — settings bleed between versions, breaks isolation contract

---

## How Tauri Does It (Correct — Full Isolation)

Each Tauri version gets its **own top-level directory** keyed by the Tauri identifier:

```
AppData/Roaming/
  ai.agentmux.app.v0-31-100/    ← version 0.31.100
    settings.json                 fully isolated
    instances/v0.31.100/db/
    instances/v0.31.100/logs/
  ai.agentmux.app.v0-31-101/    ← version 0.31.101
    settings.json                 fully isolated
    instances/v0.31.101/db/
  ai.agentmux.app.dev/          ← dev mode
    settings.json
    instances/...
```

Everything — settings, DB, logs — is per-version. No cross-version bleed.

## How CEF Does It (Bug — Partial Isolation)

CEF uses a single root with versioned subdirectories:

```
AppData/Roaming/
  ai.agentmux.cef/              ← single root for ALL CEF versions
    settings.json                 ← SHARED across v0.32.110, v0.32.111, etc.
    instances/
      v0.32.110/db/
      v0.32.110/logs/
      v0.32.111/db/              ← data is isolated
      v0.32.111/logs/
```

Data (DB, logs) is version-isolated via `instances/v{VERSION}/`, but
`settings.json` is shared at the root. The `get_data_dir` and `get_config_dir`
IPC commands also return the unversioned root path to the frontend.

## The Fix

Match Tauri's pattern — give each CEF version its own top-level directory:

```
AppData/Roaming/
  ai.agentmux.cef.v0-32-110/    ← version 0.32.110
    settings.json
    db/
    logs/
  ai.agentmux.cef.v0-32-111/    ← version 0.32.111
    settings.json
    db/
    logs/
  ai.agentmux.cef.dev/          ← dev mode
    settings.json
    db/
    logs/
```

### Changes Required

**1. `sidecar.rs` — compute version-specific root dir**

Replace:
```rust
let data_dir = dirs::data_dir()...join("ai.agentmux.cef");
let config_dir = dirs::config_dir()...join("ai.agentmux.cef");
let version_instance_id = format!("v{}", current_version);
let version_data_home = data_dir.join("instances").join(&version_instance_id);
```

With:
```rust
let version_slug = format!("v{}", current_version.replace('.', "-"));
let is_dev = cfg!(debug_assertions);
let dir_name = if is_dev {
    "ai.agentmux.cef.dev".to_string()
} else {
    format!("ai.agentmux.cef.{}", version_slug)
};
let data_dir = dirs::data_dir()...join(&dir_name);
let config_dir = dirs::config_dir()...join(&dir_name);
// No more instances/ subdirectory — everything at root
```

**2. `commands/platform.rs` — return version-specific paths**

`get_data_dir()` and `get_config_dir()` should return the version-specific
root, not the shared `ai.agentmux.cef/` root. Store in `AppState` during
sidecar init.

**3. `AGENTMUX_SETTINGS_DIR` — point to version-specific root**

Currently points to `ai.agentmux.cef/` (shared). Change to point to the
version-specific directory so settings are isolated.

**4. Env vars passed to sidecar**

All of these should use the version-specific root:
- `AGENTMUX_DATA_HOME` → `ai.agentmux.cef.v0-32-111/`
- `AGENTMUX_CONFIG_HOME` → `ai.agentmux.cef.v0-32-111/`
- `AGENTMUX_SETTINGS_DIR` → `ai.agentmux.cef.v0-32-111/`
- `--wavedata` → `ai.agentmux.cef.v0-32-111/`
- `--instance` → `v0.32.111`

### Coexistence

Both builds already coexist — different directory prefixes:
- Tauri: `ai.agentmux.app.v0-31-*`
- CEF: `ai.agentmux.cef.v0-32-*` (after fix)
- CEF dev: `ai.agentmux.cef.dev`

Multiple CEF versions can run simultaneously since each has its own
data directory, DB, and settings.

### Migration

No migration needed — old `ai.agentmux.cef/` directory can be left in place
or cleaned up manually. New versions will create their own directories.

---

## Summary

| Aspect | Tauri (current) | CEF (current) | CEF (after fix) |
|--------|----------------|---------------|-----------------|
| Dir pattern | `ai.agentmux.app.v0-31-NNN/` | `ai.agentmux.cef/` (shared) | `ai.agentmux.cef.v0-32-NNN/` |
| Settings | Per-version | Shared | Per-version |
| DB | Per-version | Per-version (in instances/) | Per-version |
| Logs | Per-version | Per-version (in instances/) | Per-version |
| `get_data_dir` returns | Version-specific | Root (bug) | Version-specific |
