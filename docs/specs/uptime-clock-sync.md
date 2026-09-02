# Spec: Robust Multi-Window Uptime Clock Sync

**Status:** Ready to implement
**Date:** 2026-03-17
**Component:** `frontend/app/statusbar/BackendStatus.tsx`

---

## 1. Problem

The uptime timer in the bottom-left status bar can show different values across open windows.

### Root cause

Each window runs an independent `setInterval(() => {...}, 1000)` that starts at an arbitrary
offset from the wall-clock second boundary:

```
Window A setInterval starts at t=47ms  → ticks at :047, 1:047, 2:047 ...
Window B setInterval starts at t=612ms → ticks at :612, 1:612, 2:612 ...
```

Both windows compute `Math.floor((Date.now() - startedAt) / 1000)` at their own tick time.
For up to ~999ms per second, they can display different integer uptime values. Visually:
window A shows `5:23` while window B still shows `5:22`.

Secondary issues:
- `setInterval` accumulates drift over time (GC pauses, JS engine timer clamping)
- Each window independently fetches `getBackendInfo()` — if the backend restarts, windows
  may pick up the new `started_at` at slightly different moments

---

## 2. Fix: Drive uptime from the sysinfo event timestamp

The `sysinfo` WPS event already fires from the Rust backend at a configurable cadence
(default 1 s) and includes a wall-clock timestamp `ts` (ms since epoch). All windows
subscribe to the same WebSocket connection to the same backend, so they receive the
**exact same event** with the **exact same `ts`** value at effectively the same instant.

Replace the `setInterval` with a `waveEventSubscribe` handler:

```typescript
// BackendStatus.tsx — proposed change

// REMOVE:
onMount(() => {
    const iv = setInterval(() => {
        const start = startedAt();
        if (start != null) {
            setUptimeSecs(Math.floor((Date.now() - start) / 1000));
        }
    }, 1000);
    onCleanup(() => clearInterval(iv));
});

// ADD:
onMount(() => {
    const unsubscribe = waveEventSubscribe(
        { eventType: "sysinfo", scope: "global" },
        (event: WaveEvent) => {
            const ts: number | undefined = event.data?.ts;
            const start = startedAt();
            if (ts != null && start != null) {
                setUptimeSecs(Math.floor((ts - start) / 1000));
            }
        }
    );
    onCleanup(unsubscribe);
});
```

**Why this works:**
- The `ts` value is set server-side, so all windows compute `Math.floor((ts - start) / 1000)`
  with the same numerator → always the same integer result
- No independent timers → no phase drift
- Automatically respects the user's `telemetry:interval` setting
- If sysinfo slows down or pauses (e.g. backend under load), the uptime display pauses too —
  this is honest: the display is only as fresh as the backend tick

---

## 3. Fallback: align to wall-clock second boundary (simpler alternative)

If the sysinfo-driven approach adds unwanted coupling, a lighter fix is to align all
`setInterval` timers to the same wall-clock second boundary at startup:

```typescript
onMount(() => {
    let iv: ReturnType<typeof setInterval>;

    const tick = () => {
        const start = startedAt();
        if (start != null) {
            setUptimeSecs(Math.floor((Date.now() - start) / 1000));
        }
    };

    // Delay until the next whole second, then tick every 1000ms
    const delay = 1000 - (Date.now() % 1000);
    const timeout = setTimeout(() => {
        tick();
        iv = setInterval(tick, 1000);
    }, delay);

    onCleanup(() => {
        clearTimeout(timeout);
        clearInterval(iv);
    });
});
```

All windows align to `:000ms` boundaries so they always tick within a few ms of each
other. Simpler, but still subject to long-term `setInterval` drift.

**Recommendation: prefer the sysinfo-driven approach (Section 2).** The fallback is
acceptable if sysinfo subscription proves difficult to wire up in this component.

---

## 4. Files to change

| File | Change |
|------|--------|
| `frontend/app/statusbar/BackendStatus.tsx` | Replace `setInterval` with `waveEventSubscribe` on `sysinfo` event; import `waveEventSubscribe` and `WaveEvent` from store |

No backend changes needed — `sysinfo` already carries `ts`.

---

## 5. Verification

1. Open two windows (`commands::window::open_new_window`)
2. Watch both status bars for 60 seconds — uptime must never differ between windows
3. Open the popover on both windows simultaneously — both must show the same uptime integer
4. Change `telemetry:interval` to 2.0s — uptime should update every 2s on all windows in sync
5. Ensure no regression on single-window builds

---

## 6. Notes

- The sysinfo `ts` is set in `agentmuxsrv-rs/src/backend/sysinfo.rs` as
  `SystemTime::now()` converted to ms — it is the server-side wall clock, not
  the client's `Date.now()`. For uptime purposes this is fine: `startedAt` is
  also a server-side timestamp from `getBackendInfo().started_at`, so the delta
  `ts - startedAt` is entirely server-side and immune to client clock skew.
- WPS event delivery over WebSocket has ~0–5ms jitter between windows on localhost.
  At 1-second granularity this is imperceptible.
