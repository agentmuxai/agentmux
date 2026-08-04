# Spec: Telemetry Sampling Interval Setting

**Status:** Proposed
**Date:** 2026-03-08

## Problem

The sysinfo telemetry loop (`sysinfo.rs`) hardcodes a 1-second sampling interval. Users who want higher-resolution monitoring (e.g. catching CPU spikes) or lower resource usage (longer interval) have no way to configure this.

## Solution

Add a `telemetry:interval` setting (in seconds, float) that controls the sysinfo sampling rate. Range: 0.1s to 2.0s, default 1.0s. Live-reloadable via the settings file watcher.

## Setting

| Key | Type | Default | Min | Max |
|-----|------|---------|-----|-----|
| `telemetry:interval` | float | 1.0 | 0.1 | 2.0 |

Example in settings.json:
```jsonc
// "telemetry:interval": 1.0,   // sampling interval in seconds (0.1 - 2.0)
```

## Implementation

### Backend (`agentmuxsrv-rs`)

1. **`wconfig.rs`** — Add `telemetry_interval` field to `SettingsType`:
   ```rust
   #[serde(rename = "telemetry:interval", default, skip_serializing_if = "is_zero_f64")]
   pub telemetry_interval: f64,
   ```

2. **`sysinfo.rs`** — Accept an `Arc<AppState>` (or a watch channel) instead of just the broker. On each loop iteration, read the current interval from the config and adjust the tokio interval. Clamp to [0.1, 2.0].

   The loop currently uses `tokio::time::interval(Duration::from_secs(1))`. Change to:
   - Read `telemetry:interval` from config on each tick
   - Use `tokio::time::sleep(duration)` instead of `interval` (sleep-based loop adapts immediately to config changes, while `interval` requires `reset()`)

3. **`main.rs`** — Pass `AppState` (or a config watch channel) to `run_sysinfo_loop`.

### Frontend

4. **`gotypes.d.ts`** — Add `"telemetry:interval"?: number` to `SettingsType`.

5. **`schema/settings.json`** — Add `"telemetry:interval": { "type": "number" }`.

6. **`platform.rs`** (settings template) — Add commented-out default:
   ```
   // "telemetry:interval":       1.0,
   ```

### Config change propagation

The sysinfo loop runs in the backend sidecar (`agentmuxsrv-rs`), not the Tauri frontend. Config changes flow:

1. User edits settings.json
2. File watcher in `agentmuxsrv-rs` detects change, re-reads config
3. Sysinfo loop reads new interval on next tick (no restart needed)

The loop needs access to the current config. Options:
- **A)** Pass `Arc<AppState>` and read `app_state.config.read()` each tick
- **B)** Use a `tokio::sync::watch` channel that broadcasts config changes

Option A is simpler and sufficient — the config read is cheap (RwLock, no I/O).

## Frontend display note

The sysinfo view (`sysinfo.tsx`) uses gap detection at 2000ms (line 174: `if (timeDiff > 2000)`). At intervals > 1s, this threshold may need adjusting to `interval * 2.5` to avoid false gap detection. At intervals < 1s, no change needed.

## Files Changed

| File | Change |
|------|--------|
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Add `telemetry_interval` field |
| `agentmuxsrv-rs/src/backend/sysinfo.rs` | Read interval from config, use sleep-based loop |
| `agentmuxsrv-rs/src/main.rs` | Pass AppState to sysinfo loop |
| `frontend/types/gotypes.d.ts` | Add type |
| `schema/settings.json` | Add schema entry |
| `src-tauri/src/commands/platform.rs` | Add to settings template |
