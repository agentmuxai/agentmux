# Polling Landscape Analysis — AgentMux

**Date:** 2026-06-23  
**Scope:** All timer-based repeating work in frontend (TS/SolidJS) and backend (Rust/tokio)  
**Question:** Is a unified polling framework worth building?

---

## TL;DR

38 polling sites total. No catastrophic leaks, but the 1-second clock-tick pattern is independently re-invented 10+ times across components. The highest-ROI improvement is a **shared `useTick(ms)` primitive** that collapses those into one interval with N subscribers — low effort, meaningful reduction in timer overhead and code duplication. A full "polling framework" is not warranted: the backend is already well-structured and most frontend polls are correctly scoped to component lifecycle via `createEffect`/`onCleanup`.

---

## Inventory — Frontend (32 sites)

### Category A: Clock ticks (UI only, no network) — 1 000 ms

These components independently own a 1-second interval to drive elapsed-time or relative-time displays. They share zero state with each other.

| # | File | Purpose |
|---|------|---------|
| 1 | `AgentFooter.tsx:56` | Elapsed timer while agent is working |
| 2 | `AgentFooter.tsx:70` | Same component, second interval (phrase rotation at 30 000 ms listed separately) |
| 3 | `AgentComposerStrip.tsx:125` | Elapsed timer while composing |
| 4 | `ActivityRow.tsx:47` | Elapsed timer for running shell/cron activities |
| 5 | `ActivityDock.tsx:80` | Fade-out tick for expiring terminal rows |
| 6 | `ToolBlock.tsx:48` | Elapsed timer for running tool invocations |
| 7 | `AgentInstallModal.tsx:119` | Elapsed time during provider install (250 ms — faster tick) |
| 8 | `PersistentShellBlock.tsx:50` | Elapsed timer for running shell sessions |
| 9 | `diag-panel.tsx:122` | Relative-age strings on audit records |
| 10 | `warden.tsx:173` | Clock column refresh (Host section) |
| 11 | `warden.tsx:327` | Clock column refresh (Internet section) |
| 12 | `useAgentFailure.ts:108` | Auto-retry countdown display |
| 13 | `usenotification.tsx:77` | Notification expiration filter |

**All 13 are properly cleaned up** (`clearInterval` in `onCleanup`/`createEffect` cleanup). They're all conditional on component visibility or active state.

### Category B: Data refresh (network) — 2 000–5 000 ms

| # | File | Interval | Endpoint / Purpose |
|---|------|----------|--------------------|
| 14 | `warden.tsx:170` | 5 000 ms | `/agentmux/reactive/agents` — agent list |
| 15 | `warden.tsx:323` | 5 000 ms | Network/internet data (Warden) |
| 16 | `OAuthConnectPanel.tsx:93` | 2 000 ms | OAuth flow status polling |
| 17 | `sysinfo-view.tsx:83` | 2 000 ms | Throttles SVG chart rebuild (data pushed via WPS, chart updates capped at 0.5 Hz) |

### Category C: Perf / metrics — 1 000 ms

| # | File | Purpose |
|---|------|---------|
| 18 | `hud.tsx:78` | `perfStore.snapshot()` — only while HUD is visible |
| 19 | `agent-pane-perf-section.tsx:60` | `agentPerfStore.snapshot()` — no visibility gate |

**Note:** #19 runs whenever the component is mounted, even if the user isn't looking at it. Low cost, but a visibility gate would be consistent.

### Category D: Minutes tick — 60 000 ms

| # | File | Purpose |
|---|------|---------|
| 20 | `termViewModel.ts:39` | `nowMinute` signal — drives runtime labels. Stored on `globalThis.__nowMinuteInterval` with dedup check to avoid multiple instances. |
| 21 | `MyAgentsList.tsx:154` | Relative timestamps ("5m ago") on session list |

**Note:** #20 uses `globalThis` as a singleton guard — a manual workaround for what a shared primitive would give you automatically.

### Category E: Phrase rotation — 30 000 ms

| # | File | Purpose |
|---|------|---------|
| 22 | `AgentFooter.tsx:56` | Rotates "thinking" phrases while loading |

### Category F: WebSocket keepalive — 5 000 ms

| # | File | Purpose |
|---|------|---------|
| 23 | `ws.ts:62` | Ping heartbeat. No cleanup (cleared only on socket close). Module-level, expected to run for app lifetime. |

### Category G: Stream / health watchdog — 5 000 ms

| # | File | Purpose |
|---|------|---------|
| 24 | `useAgentStream.ts:605` | Detects stuck agent streams. Cleaned up in `onCleanup`. |

### Category H: Layout init polling — 10–200 ms (one-shot)

These run briefly at mount time to detect when an async-rendered child is available. They stop themselves once the target is found.

| # | File | Interval | Purpose |
|---|------|----------|---------|
| 25 | `TileLayout.win32.tsx:519` | 100 ms | Wait for header element to mount |
| 26 | `TileLayout.linux.tsx:431` | 100 ms | Same |
| 27 | `TileLayout.darwin.tsx:433` | 100 ms | Same |
| 28 | `browser-view.tsx:325` | 200 ms | Sync native browser pane rect |
| 29 | `DragOverlay.tsx:136` | 10 ms | Wait for window label async signal |
| 30 | `app-init.ts:490` | 50 ms | Wait for `window.api` bridge (5s timeout) |

All stop once their condition is met. These are structural patterns for async DOM/signal availability — not really "periodic polling" in the steady-state sense.

### Category I: Functional misc — various

| # | File | Interval | Purpose |
|---|------|----------|---------|
| 31 | `whisperVoiceEngine.ts:347` | 100 ms | Audio level meter during recording |
| 32 | `wos.ts:298` | 30 000 ms | WaveObject cache cleanup. **No cleanup handler. Runs for app lifetime.** |

---

## Inventory — Backend (6 sites)

All use `tokio::time::interval` in `loop { interval.tick().await }`. Pattern is consistent; no structural issues.

| # | File | Interval | Purpose | Exits? |
|---|------|----------|---------|--------|
| 33 | `sysinfo.rs:137` | 0.2–2.0s (configurable) | CPU/mem/net/disk metrics → WPS broker | No (per-connection lifetime) |
| 34 | `blockcontroller/core.rs:106` | 5 000 ms | Health watchdog for active agent turn | Yes — when turn ends |
| 35 | `server/websocket.rs:124` | 10 000 ms | WebSocket ping | Yes — on disconnect |
| 36 | `process_tracker/registry.rs:195` | 2 000 ms | New child process detection + metrics | No (host lifetime) |
| 37 | `storage/filestore/core.rs:707` | Configurable | Flush in-memory file cache to disk | No (store lifetime) |
| 38 | `blockcontroller/watchdog.rs:26` | 60 000 ms | Max-runtime + idle-output timeout enforcement | No (host lifetime) |

Backend pattern is clean and idiomatic Rust. No action needed.

---

## Issues Found

### 1. The 1-second clock tick is re-invented 13 times

Every component that shows elapsed time creates its own `setInterval(fn, 1000)`. At peak load (multiple agent panes open, warden visible, diag panel open) there could be 15–20 independent 1-second intervals all firing within the same JS event loop turn. Each is cheap individually, but they're hidden cost with no visibility.

### 2. `wos.ts:298` has no cleanup

The WaveObject cache cleanup interval at `wos.ts:298` is module-level with no `clearInterval` path. This is intentional (cache lives for app lifetime) but makes it invisible to any future polling audit.

### 3. `termViewModel.ts:39` uses `globalThis` as a manual singleton

The minute-tick interval stores itself on `globalThis.__nowMinuteInterval` to prevent double-registration. This is a hand-rolled deduplication mechanism for exactly the problem a shared primitive would solve.

### 4. `agent-pane-perf-section.tsx:60` has no visibility gate

Polls perf store every second regardless of whether the perf section is expanded/visible. Low cost but inconsistent with the pattern used in `hud.tsx` (which does gate on `visible()`).

### 5. Three `TileLayout` platform files have identical header-polling code

`win32.tsx`, `linux.tsx`, and `darwin.tsx` each contain the same 100 ms header-polling block. This is a code duplication issue independent of the polling framework question — it should live in a shared util.

---

## Framework Recommendation

### What a "unified polling framework" could mean

**Option A — Shared `useTick(ms)` primitive (recommended)**  
A SolidJS primitive that returns a reactive signal incrementing at a given interval, sharing one underlying `setInterval` per distinct period across all subscribers. Consumers call `useTick(1000)` instead of `setInterval(fn, 1000)` in a `createEffect`. Lifecycle is handled automatically by the reactive graph.

```typescript
// Shared singleton map: interval_ms → { count: Signal<number>, refCount: number, timerId: number }
const tickers = new Map<number, { tick: Accessor<number>; refCount: number; id: number }>();

export function useTick(ms: number): Accessor<number> {
    if (!tickers.has(ms)) {
        const [tick, setTick] = createSignal(0);
        const id = setInterval(() => setTick(n => n + 1), ms);
        tickers.set(ms, { tick, refCount: 0, id });
    }
    const entry = tickers.get(ms)!;
    entry.refCount++;
    onCleanup(() => {
        entry.refCount--;
        if (entry.refCount === 0) {
            clearInterval(entry.id);
            tickers.delete(ms);
        }
    });
    return entry.tick;
}
```

Usage in a component:
```typescript
// Before (each component):
const [elapsed, setElapsed] = createSignal(0);
createEffect(() => {
    if (!props.loading) return;
    const id = setInterval(() => setElapsed(s => s + 1), 1000);
    onCleanup(() => clearInterval(id));
});

// After:
const tick = useTick(1000);
const elapsed = () => props.loading ? tick() : 0; // derives from shared tick
```

**Benefit:** 13 independent 1-second intervals → 1. Eliminates `globalThis.__nowMinuteInterval` hack. Works naturally with SolidJS reactivity.  
**Cost:** ~50 lines of code, low risk, pure addition.

---

**Option B — Polling registry / devtools panel**  
A lightweight registry where each poll registers `{ name, intervalMs, purpose }` on start and deregisters on cleanup. No behavior change — pure observability. Valuable for debugging "what is the app doing right now?" but not worth the annotation overhead unless debugging becomes a regular problem.

**Verdict:** Don't build this unless you find a polling bug that would have been caught by it.

---

**Option C — Full centralized scheduler (overkill)**  
A single scheduler that owns all timers, supports pause/resume, priority, batching, and visibility-gating. This would require every polling site to migrate. The existing SolidJS `createEffect`/`onCleanup` pattern already handles lifecycle correctly. A central scheduler adds indirection without proportionate benefit given that most polls are already conditional.

**Verdict:** Not recommended.

---

## Recommended Actions (priority order)

| Priority | Action | Effort | Benefit |
|----------|--------|--------|---------|
| P1 | Build `useTick(ms)` in `frontend/app/hook/useTick.ts` | ~50 LOC | Eliminates 13 duplicate 1s intervals; fixes `globalThis` hack |
| P2 | Migrate all 1 000 ms clock-tick sites to `useTick(1000)` | ~2h | Reduction to 1 interval; visible in devtools |
| P2 | Migrate 60 000 ms sites (`termViewModel`, `MyAgentsList`) to `useTick(60000)` | ~30 min | Removes `globalThis` guard |
| P3 | Add visibility gate to `agent-pane-perf-section.tsx` | ~5 min | Consistency with `hud.tsx` |
| P3 | Extract TileLayout header-polling to shared util | ~30 min | Removes triplication |
| P4 | Document `wos.ts:298` lifetime intent with a comment | ~2 min | Future audit clarity |

The backend does not need a framework. Its existing `tokio::time::interval` + loop pattern is idiomatic, consistent, and well-isolated.

---

## Summary

| Dimension | Frontend | Backend |
|-----------|----------|---------|
| Total polling sites | 32 | 6 |
| Always-running (no conditional gate) | 5 | 4 (lifetime of host) |
| Properly cleaned up | 27/32 | N/A (loop exits or runs forever by design) |
| Most common interval | 1 000 ms (13 sites) | varies |
| Biggest duplication | 1s clock tick (13×) | none |
| Framework verdict | `useTick` primitive only | No change needed |
