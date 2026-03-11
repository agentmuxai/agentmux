# Sysinfo CPU History Length Setting

## Overview

Add a `telemetry:numpoints` setting to `settings.json` that controls how many data points the sysinfo CPU chart displays. This directly controls how far back in time the chart shows.

**History duration = numPoints × intervalSecs**

| numPoints | interval | Duration |
|-----------|----------|----------|
| 120 (default) | 1.0s | 2 minutes |
| 300 | 1.0s | 5 minutes |
| 600 | 1.0s | 10 minutes |
| 120 | 0.5s | 1 minute |

## Current State

| Component | Current Behavior |
|-----------|-----------------|
| Backend persist buffer | Hardcoded `PERSIST_COUNT = 1024` in `wps.rs` |
| Frontend default points | Hardcoded `DefaultNumPoints = 120` in `sysinfo-model.ts` |
| Initial history load | Requests `maxitems: 120` via `EventReadHistoryCommand` |
| Chart X-axis domain | `maxX - targetLen * intervalSecs * 1000` |
| Existing settings | `telemetry:enabled` (bool), `telemetry:interval` (0.2–2.0s) |

The `DefaultNumPoints` constant is used in the view model to size the data array and passed as `targetLen` to the chart component. The backend can store up to 1024 events but the frontend only ever requests 120.

## Changes

### 1. Settings Schema

**File:** `schema/settings.json`

Add entry after `telemetry:interval`:

```json
"telemetry:numpoints": {
    "type": "integer",
    "minimum": 30,
    "maximum": 1024,
    "default": 120,
    "description": "Number of data points to display in sysinfo charts. Higher values show more history. Duration = numpoints × interval."
}
```

**Constraints:**
- Min 30 (below this the chart is too sparse to be useful)
- Max 1024 (backend `PERSIST_COUNT` limit — requesting more than stored is pointless)
- Default 120 (preserves current behavior)

**User's settings.json** — the setting lives in the telemetry section:

```json
    // -- Telemetry --
    // "telemetry:enabled":        true,
    "telemetry:interval":       0.2,
    "telemetry:numpoints":      300,
```

### 2. Backend Config Struct

**File:** `agentmuxsrv-rs/src/backend/wconfig.rs`

Add field to `SettingsType`:

```rust
pub telemetry_numpoints: Option<i64>,
```

Map from `"telemetry:numpoints"` in the settings deserializer. Default to 120 if absent.

### 3. Frontend View Model

**File:** `frontend/app/view/sysinfo/sysinfo-model.ts`

- Replace hardcoded `DefaultNumPoints = 120` with a derived atom that reads from `fullConfigAtom.settings["telemetry:numpoints"]`
- Clamp to [30, 1024], fallback to 120
- Use this value for:
  - `loadInitialData()` → `maxitems` parameter
  - `targetLen` passed to `SingleLinePlot`
  - Data array trimming in `addContinuousDataAtom`

### 4. Frontend Plot Component

**File:** `frontend/app/view/sysinfo/sysinfo-plot.tsx`

No changes needed — `targetLen` is already a prop passed from the view model.

### 5. Reactivity

When the user changes `telemetry:numpoints` in settings.json:
- The config watcher fires an update
- The numpoints atom recomputes
- `loadInitialData()` re-fetches with the new `maxitems` count
- The chart re-renders with the new time window

This matches how `telemetry:interval` changes are already handled.

## Files to Modify

| File | Change |
|------|--------|
| `schema/settings.json` | Add `telemetry:numpoints` schema entry |
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Add `telemetry_numpoints` field |
| `frontend/app/view/sysinfo/sysinfo-model.ts` | Read setting, replace hardcoded 120 |

## Testing

- Default behavior (no setting): chart shows 120 points (2 min at 1s interval) — unchanged
- Set `"telemetry:numpoints": 300`: chart shows 5 minutes of history
- Set `"telemetry:numpoints": 1024`: chart shows max ~17 minutes at 1s interval
- Set below 30 or above 1024: clamped to bounds
- Change setting while sysinfo is open: chart reloads with new history length
