# Sysinfo Tick Irregularity — Deep Analysis

**Date:** 2026-03-08
**Version:** 0.31.81

## Reported Issues

1. **`telemetry:interval` of `2` doesn't work, but `1.9` does**
2. **Unequal tick intervals:** at 0.5s, 3 fast ticks then 1 slow tick

---

## Issue 1: Integer `2` Rejected

### Root Cause: `val <= 0.0` guard + JSON integer edge case — **NOT the cause**

The backend clamping logic is correct:

```rust
fn get_interval_secs(config_watcher: &ConfigWatcher) -> f64 {
    let val = config_watcher.get_settings().telemetry_interval;
    if val <= 0.0 {
        return DEFAULT_INTERVAL_SECS;  // 1.0
    }
    val.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS)  // clamp(0.1, 2.0)
}
```

`f64::clamp(2.0, 0.1, 2.0)` returns `2.0` — this is correct. Serde also
correctly deserializes JSON integer `2` as `f64` value `2.0`.

### Actual Cause: Frontend `prevLastTs` gap detection

In `sysinfo.tsx:392-395`:

```typescript
const prevData = globalStore.get(model.dataAtom);
const prevLastTs = prevData[prevData.length - 1]?.ts ?? 0;
if (dataItem.ts - prevLastTs > 2000) {   // <-- HARDCODED 2000ms threshold
    model.loadInitialData();              // <-- full reload, resets loading=true
} else {
    addContinuousDataRef.current(dataItem);
}
```

When `telemetry:interval` is `2.0` (2000ms), the actual elapsed time between
events is always **slightly more** than 2000ms due to:
- `tokio::time::sleep` scheduling jitter
- CPU/memory/network refresh time (~5-50ms)
- Event serialization + WebSocket delivery latency (~1-10ms)

So `dataItem.ts - prevLastTs` is typically 2010-2060ms, which **exceeds the
2000ms threshold**, triggering `loadInitialData()` on *every single tick*.

`loadInitialData()` sets `loadingAtom = true`, and the event handler
short-circuits when loading:

```typescript
const loading = globalStore.get(model.loadingAtom);
if (loading) return;  // <-- drops events during reload
```

This creates a loop: tick arrives → gap > 2000ms → reload → next tick arrives
during reload → dropped → reload finishes → next tick arrives → gap > 2000ms
again → reload again... The chart never updates continuously.

At `1.9s`, the gap is ~1910-1960ms which stays under 2000ms, so it works fine.

### Fix

The gap threshold should be **dynamic**, based on the configured interval:

```typescript
// Use 2x the configured interval (or 2000ms minimum) as the gap threshold
const configInterval = globalStore.get(atoms.fullConfigAtom)?.settings?.["telemetry:interval"] ?? 1.0;
const gapThreshold = Math.max(2000, configInterval * 1000 * 2);
if (dataItem.ts - prevLastTs > gapThreshold) {
```

Or simpler: change the hardcoded `2000` to `3000` since the max interval is
2.0s and a 3s gap clearly indicates a real disconnection, not normal jitter.

---

## Issue 2: Unequal Tick Intervals (3 fast + 1 slow)

### Root Cause: `sysinfo` crate's `MINIMUM_CPU_UPDATE_INTERVAL`

The `sysinfo` crate (v0.34) documents that `refresh_cpu_usage()` requires a
**minimum interval of ~200ms** between calls to produce accurate readings.
Internally, the crate tracks timestamps and returns cached/zero values if
called too frequently.

However, this is NOT what causes the visual stutter at 0.5s. The actual cause
is the **synchronous blocking nature of the refresh calls**:

```rust
loop {
    let interval_secs = get_interval_secs(&config_watcher);
    tokio::time::sleep(Duration::from_secs_f64(interval_secs)).await;

    // These are SYNCHRONOUS and block the async executor:
    sys.refresh_cpu_usage();   // ~1-5ms typical, but can spike to 50-200ms
    sys.refresh_memory();      // ~1-5ms typical
    networks.refresh(true);    // ~1-10ms, but can spike on Windows

    // ... serialize + publish
}
```

The loop uses **sleep-then-work**, not **interval-based timing**. Each tick's
actual period is: `configured_interval + refresh_time + publish_time`.

On Windows, `refresh_cpu_usage()` periodically takes longer (50-200ms) due to:
- WMI/PDH counter collection variability
- Kernel scheduling of the performance counter reads
- Antivirus real-time scanning interference

At 0.5s configured interval, the pattern looks like:

| Tick | Sleep | Refresh | Total | Visual Gap |
|------|-------|---------|-------|------------|
| 1    | 500ms | 5ms     | 505ms | ~0.5s      |
| 2    | 500ms | 5ms     | 505ms | ~0.5s      |
| 3    | 500ms | 5ms     | 505ms | ~0.5s      |
| 4    | 500ms | 150ms   | 650ms | ~0.65s (slow tick) |

The 4th tick takes 30% longer, which is visually noticeable as a "stutter".

### Additional Factor: Frontend X-Axis Domain

The chart's x-axis domain is hardcoded to assume 1-second intervals:

```typescript
// sysinfo.tsx:523
let minX = maxX - targetLen * 1000;  // <-- hardcoded 1000ms per point
```

And the data cutoff in `addContinuousDataAtom`:

```typescript
// sysinfo.tsx:192
const cutoffTs = latestItemTs - 1000 * targetLen;  // <-- hardcoded 1000ms
```

At 0.5s intervals, data accumulates 2x faster than the window expects, so the
chart shows only the most recent half of the time window, and older points get
cut off prematurely. This doesn't cause the stutter but means the chart isn't
correctly scaled for non-1s intervals.

### Fix

1. **Use `tokio::time::interval` instead of `sleep`** to maintain steady tick
   rate regardless of refresh duration:

   ```rust
   let mut interval = tokio::time::interval(Duration::from_secs_f64(interval_secs));
   interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
   loop {
       interval.tick().await;
       // ... refresh and publish
       // Re-check config and reset interval if changed
   }
   ```

2. **Run refresh in `spawn_blocking`** to avoid blocking the tokio executor:

   ```rust
   let (cpu_data, mem_data, net_data) = tokio::task::spawn_blocking(move || {
       sys.refresh_cpu_usage();
       sys.refresh_memory();
       networks.refresh(true);
       // collect data
   }).await?;
   ```

3. **Make frontend cutoff interval-aware:**

   ```typescript
   const configInterval = globalStore.get(atoms.fullConfigAtom)
       ?.settings?.["telemetry:interval"] ?? 1.0;
   const cutoffTs = latestItemTs - configInterval * 1000 * targetLen;
   ```

   And for the x-axis domain:

   ```typescript
   let minX = maxX - targetLen * configInterval * 1000;
   ```

---

## Summary of All Issues

| Issue | Cause | Severity | Fix |
|-------|-------|----------|-----|
| `2` doesn't work | Hardcoded `> 2000` gap threshold | High | Make threshold dynamic or increase to 3000 |
| Unequal ticks | sleep-then-work timing + Windows refresh spikes | Medium | Use `tokio::time::interval` + `spawn_blocking` |
| Chart x-axis wrong | Hardcoded `1000ms` per point assumption | Low | Read interval from config |

## Affected Files

- `agentmuxsrv-rs/src/backend/sysinfo.rs` — tick loop timing
- `frontend/app/view/sysinfo/sysinfo.tsx` — gap threshold, x-axis domain, data cutoff
