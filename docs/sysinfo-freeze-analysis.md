# Sysinfo Pane Freeze — Root Cause Analysis

**Date:** 2026-03-08
**Status:** Identified, not yet fixed

## Symptoms

- Sysinfo pane stops updating after some time
- Opening a second sysinfo pane makes the frozen one "catch up"
- Freezes at random/non-deterministic times

## Data Flow Overview

```
Backend (Rust)                          Frontend (TypeScript)
─────────────────                       ─────────────────────
sysinfo.rs                              wps.ts
  └─ broker.publish(WaveEvent)            └─ waveEventSubjects Map
       │                                       │
wps.rs (Broker)                           handleWaveEvent()
  └─ route to "ws-main"                    └─ iterate all handlers
       │                                       │
eventbus.rs → websocket.rs              sysinfo.tsx
  └─ broadcast to WS clients              └─ addContinuousData(dataItem)
                                               │
                                           React re-render via Jotai atom
```

## Root Causes (ranked by likelihood)

### 1. Bug in `waveEventUnsubscribe` — early return drops handlers (HIGH)

**File:** `frontend/app/store/wps.ts:84-105`

When unsubscribing multiple handlers, if *any* handler isn't found in the map,
the function `return`s early instead of `continue`ing, leaving remaining
unsubscriptions unprocessed:

```typescript
// Line 88-93 — both early returns should be "continue"
if (subjects == null) {
    return;  // BUG: should be "continue"
}
const idx = subjects.findIndex((s) => s.id === unsubscribe.id);
if (idx === -1) {
    return;  // BUG: should be "continue"
}
```

This causes orphaned handlers to accumulate in the `waveEventSubjects` map.
These stale handlers hold closures pointing to old Jotai atom setters that no
longer trigger React renders. When a sysinfo event arrives, the stale handler
consumes it silently — the live handler may or may not also fire depending on
map iteration order and whether the stale handler's scope still matches.

### 2. Stale closure in sysinfo subscription handler (HIGH)

**File:** `frontend/app/view/sysinfo/sysinfo.tsx:379-405`

The `useEffect` subscribes to `"sysinfo"` events with a handler that closes
over `addContinuousData` (a Jotai `useSetAtom` return value). When React
re-renders and `addContinuousData` changes identity, the effect re-runs:
unsubscribe old → subscribe new.

Due to bug #1, the old handler may not be properly removed, leaving a stale
handler that consumes events but writes to a dead atom setter reference.

### 3. Why opening a second pane fixes it

When a second sysinfo pane mounts, it calls
`waveEventSubscribe("sysinfo", scope)` which triggers
`updateWaveEventSub("sysinfo")`. This sends a fresh `eventsub` RPC to the
backend, re-aggregating all scopes. The backend broker (`wps.rs:153-177`)
calls `unsubscribe_nolock` then re-subscribes with the new scope list for
route `"ws-main"`. This effectively refreshes the subscription state. Since
the frontend event dispatch (`wps.ts:126-144`) iterates *all* handlers in the
map, both old and new handlers start receiving events again — and the new
handler has a live atom setter.

### 4. Single route ID "ws-main" on backend (MEDIUM — amplifier)

**File:** `agentmuxsrv-rs/src/server/websocket.rs:441`

All frontend subscriptions use a hardcoded `"ws-main"` route ID. Every
`eventsub` call first *unsubscribes* the old `"ws-main"` entry for that event
type, then re-subscribes. There's a brief window where events can be lost
during this swap. With high-frequency sysinfo events (especially at 0.1–0.2s
intervals), this race becomes more likely.

## Recommended Fixes

### Fix 1 — Critical: `return` → `continue` in `waveEventUnsubscribe`

```typescript
// frontend/app/store/wps.ts lines 88-93
function waveEventUnsubscribe(...unsubscribes: WaveEventUnsubscribe[]) {
    const eventTypeSet = new Set<string>();
    for (const unsubscribe of unsubscribes) {
        let subjects = waveEventSubjects.get(unsubscribe.eventType);
        if (subjects == null) {
            continue;  // was: return
        }
        const idx = subjects.findIndex((s) => s.id === unsubscribe.id);
        if (idx === -1) {
            continue;  // was: return
        }
        subjects.splice(idx, 1);
        if (subjects.length === 0) {
            waveEventSubjects.delete(unsubscribe.eventType);
        }
        eventTypeSet.add(unsubscribe.eventType);
    }
    for (const eventType of eventTypeSet) {
        updateWaveEventSub(eventType);
    }
}
```

### Fix 2 — Defensive: Stable callback ref in `SysinfoView`

Avoid frequent unsubscribe/resubscribe cycles by using a ref:

```typescript
// frontend/app/view/sysinfo/sysinfo.tsx
const addContinuousData = jotai.useSetAtom(model.addContinuousDataAtom);
const addRef = React.useRef(addContinuousData);
addRef.current = addContinuousData;

React.useEffect(() => {
    const unsubFn = waveEventSubscribe({
        eventType: "sysinfo",
        scope: connName,
        handler: (event) => {
            // ... validation ...
            addRef.current(dataItem);  // always calls latest setter
        },
    });
    return () => unsubFn();
}, [connName]);  // removed addContinuousData from deps
```

### Fix 3 — Backend hardening: Per-connection route IDs

Replace the hardcoded `"ws-main"` with a unique ID per WebSocket connection to
prevent subscription interference across multiple windows.

## Priority

Fix #1 alone is likely sufficient to resolve the freezing. Fix #2 reduces
unnecessary unsubscribe/resubscribe churn. Fix #3 is a longer-term improvement
for multi-window correctness.
