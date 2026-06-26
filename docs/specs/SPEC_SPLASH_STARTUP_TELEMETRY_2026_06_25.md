# Spec: Splash Screen Startup Telemetry

**Date:** 2026-06-25
**Status:** Draft
**Author:** oozp-0621f
**Related:**
- `docs/specs/SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md` — pruner integrated here as a startup stage
- `agentmux-launcher/src/splash.rs` — current splash implementation
- `agentmux-launcher/src/srv_spawner.rs` — migration subprocess lifecycle

---

## Problem

AgentMux startup can take 1.5–9 seconds depending on data size, CEF cold state,
and pending migrations. During this time the splash shows only:

```
                AgentMux
               v0.49.3
        area54@DESKTOP-XYZ
```

The user has no idea:
- Whether startup is progressing or hung
- Which stage is slow (migrations? CEF? backend?)
- How long it's been running
- What the pruner cleaned up

The splash disappears at first-paint with no summary. If startup took 7 seconds
and three migrations ran, the user never finds out.

---

## Goal

Show a live startup timeline in the splash screen. Every tracked stage displays
as a row with a running clock while active and a final duration when done.
After all stages complete, hold the splash for 3 extra seconds showing a compact
summary, then dismiss normally.

---

## Full Startup Timeline (Reference)

All timings are wall-clock estimates on a typical dev machine (Windows, SSD).
"Always fast" = < 20ms, not worth showing unless it spikes.

```
 Time   Phase  Stage                              Who          Typical
──────────────────────────────────────────────────────────────────────
   0ms   0     Process entry + DLL setup          launcher     < 5ms   (always fast)
   5ms   1     Data dir resolution                launcher     10–30ms (always fast)
  20ms   1     Single-instance pipe bind          launcher     5–50ms  (always fast)
  25ms   1     Job Object creation                launcher     < 1ms   (always fast)
  30ms   1     Splash window shown                launcher     20–50ms (always fast)
  80ms   2     Saga recovery                      launcher     10–100ms ← SHOW
 120ms   2     ► Migrations                       srv          500ms–5s ← SHOW + EXPAND
 ???ms   2     ► Channel pruner                   launcher     50–200ms ← SHOW (new)
 ???ms   2     Backend startup (srv)              launcher     50–200ms ← SHOW
 ???ms   3     CEF + host process                 host         200–600ms ← SHOW
 ???ms   4     Frontend RPC init                  frontend     200–500ms (show via IPC)
 ???ms   4     UI mount                           frontend     100–500ms (show via IPC)
 ???ms   5     ► First paint                      frontend     —        (trigger hold)
+3000ms  5     Summary hold                       splash       3s extra ← NEW
```

**Stages marked ← SHOW** are displayed in the splash. Stages that are always
fast (< 20ms consistently) are omitted from the display to keep it clean.

---

## Startup Event Protocol

A new structured event format is emitted by each stage owner onto a shared
channel (launcher stdout ring / IPC). The migration runner already emits a
subset of this; this spec standardises and extends it.

```jsonc
// Stage begins
{ "event": "startup_stage_begin",
  "stage": "migrations",           // stable string key
  "label": "Migrations",           // display label
  "t": 1234567890123 }             // Unix ms (from launcher epoch)

// Stage ends
{ "event": "startup_stage_end",
  "stage": "migrations",
  "duration_ms": 2341,
  "status": "ok" | "skipped" | "warn" | "error",
  "detail": "13 migrations checked, 2 applied" }  // optional summary line

// Sub-item within a stage (e.g. individual migrations)
{ "event": "startup_sub_begin",
  "stage": "migrations",
  "id": "0009_transcript_backfill",
  "label": "Transcript backfill" }

{ "event": "startup_sub_end",
  "stage": "migrations",
  "id": "0009_transcript_backfill",
  "duration_ms": 2456,
  "status": "ok",
  "detail": "12,847 transcripts indexed" }

// Pruner-specific events (new)
{ "event": "startup_sub_begin",
  "stage": "pruner",
  "id": "scan",
  "label": "Scanning local channels" }

{ "event": "startup_sub_end",
  "stage": "pruner",
  "id": "scan",
  "duration_ms": 34,
  "status": "ok",
  "detail": "7 channels found, 3 dead" }

{ "event": "startup_sub_end",
  "stage": "pruner",
  "id": "delete",
  "duration_ms": 89,
  "status": "ok",
  "detail": "Freed 612 MB (3 dead channels)" }
```

**Transport:**
- Migration events: already emitted on `agentmux-srv migrate` stdout — extend
  to use this format (currently uses a slightly different schema; normalise)
- Launcher-owned stages (saga, pruner, srv spawn): emitted to an in-process
  `StartupEventSink` struct that the splash reads
- Frontend stages (RPC init, UI mount): emitted via the launcher IPC pipe
  (`startup_stage_begin` / `startup_stage_end` messages) so the splash (owned
  by the launcher) can display them

---

## Stages to Display

### S1 — Saga Recovery
```
Saga recovery     ▶ 23ms
```
- **Owner:** launcher
- **Begin:** before `run_saga_recovery()`
- **End:** after saga vacuum completes
- **Detail on end:** `"N sagas recovered, N pruned"` (or `"clean"` if none)
- **Typical:** 10–100ms; spikes to 500ms+ after a crash with many pending sagas

### S2 — Migrations
```
Migrations        ▶ 2.3s
  0000 Bootstrap           3ms ✓
  0001 Legacy data dir   147ms ✓
  0009 Transcript backfill ▶ 2.1s...
```
- **Owner:** srv subprocess (`agentmux-srv migrate`)
- **Begin:** when launcher spawns the migrate subprocess
- **End:** when subprocess exits
- **Sub-items:** one per migration (shown only if it ran; skipped migrations are omitted)
- **Detail on end:** `"13 checked, 2 applied (2.3s)"` or `"13 checked, all current (8ms)"`
- **Typical:** 8ms (all current) to 5s (transcript backfill on large history)

### S3 — Channel Pruner
```
Channel pruner    ▶ 89ms
  Scanned 7 channels       34ms ✓
  Freed 612 MB (3 dead)    55ms ✓
```
- **Owner:** launcher (new `pruner.rs`)
- **Begin:** when pruner task starts (concurrent with migrations)
- **End:** when pruner completes
- **Detail on end:** `"Freed N MB (N channels)"` or `"Nothing to prune"` or `"N live old instances"`
- **Typical:** 50–200ms depending on number of local channels
- **Note:** If live old instances found, append to detail: `"⚠ 2 old instances still running (0.48.1)"`

### S4 — Backend Startup
```
Backend startup   ▶ 134ms
```
- **Owner:** launcher (waiting for `AGENTMUXSRV-ESTART` on srv stderr)
- **Begin:** when `spawn_srv()` is called
- **End:** when the `AGENTMUXSRV-ESTART` signal is received
- **Detail on end:** `"srv ready on port XXXXX"`
- **Typical:** 50–200ms

### S5 — CEF Init
```
CEF init          ▶ 312ms
```
- **Owner:** host process (`agentmux-cef`)
- **Begin:** emitted via launcher IPC when host connects
- **End:** emitted when host signals `AGENTMUX_CEF_READY` (after `CefInitialize` + first window created)
- **Detail on end:** `"GPU: ANGLE/D3D11"` or `"GPU: software (SwiftShader)"`
- **Typical:** 200–600ms

### S6 — Frontend Init
```
Frontend init     ▶ 487ms
```
- **Owner:** frontend (app-init.ts)
- **Begin:** emitted when `initApp()` starts
- **End:** emitted when `scheduleRevealLift()` fires (just before body becomes visible)
- **Detail on end:** `"N panes mounted, N tabs"`
- **Typical:** 200–1000ms

---

## Splash Screen Rendering

### Current State

The splash is a native window (GDI on Windows, Cocoa on macOS, GTK/X11 on Linux)
showing static branding text + a footer line. The launcher calls
`update_splash_text(msg)` for simple string updates.

### New Layout

```
┌─────────────────────────────────────────────────────┐
│                                                     │
│                    AgentMux                         │
│                   v0.49.3                           │
│                                                     │
│  Saga recovery          23ms                        │
│  Migrations           2,341ms                       │
│    0001 Legacy data dir    147ms ✓                  │
│    0009 Transcript backfill 2,187ms ✓               │
│  Channel pruner           89ms  freed 612 MB        │
│  Backend startup         134ms                      │
│  CEF init                312ms  ANGLE/D3D11         │
│  Frontend init     ▶ 0:00.4                         │  ← running clock
│                                                     │
│  ──────────────────────────────────────────────── │
│  area54@DESKTOP-XYZ · 2026-06-25 · 3.2s total      │
└─────────────────────────────────────────────────────┘
```

**Formatting rules:**
- Stage row: `  {label:<24} {duration_ms or running_clock>8}  {detail}`
- Sub-item row (indented 2 more spaces): `    {id_short:<22} {duration_ms>6}ms {status_icon}`
- Running clock format: `▶ 0:00.0` (tenths of seconds, reset to 0 on each stage begin)
- Completed duration: right-aligned ms value (`2,341ms`) or seconds (`2.3s` if > 999ms)
- Status icons: `✓` (ok), `⚠` (warn), `✗` (error), `` (skipped/not shown)
- Sub-items only shown if the migration/pruner actually ran (skip "all current" migrations)
- Max sub-items visible: 6 (scroll not needed for typical case; truncate with `  … N more`)
- Splash height: auto-size to content (Windows: resize native window; min 220px, max 480px)

### Running Clock

A timer task in the launcher updates the splash every 100ms while any stage is
active. The clock shows elapsed time for the currently-running stage:

```
Frontend init     ▶ 0:00.4
```

When the stage ends, the clock is replaced with the final duration:

```
Frontend init          487ms
```

---

## Summary Hold (3 extra seconds)

When the frontend fires `scheduleRevealLift()` (first-paint ready), instead of
immediately dismissing the splash, the launcher:

1. Replaces the running clock with the final duration
2. Adds a total line in the footer: `{user}@{host} · {date} · {total}s total`
3. **Holds for 3,000ms** (configurable via `AGENTMUX_SPLASH_HOLD_MS` env var)
4. Then fades out (existing fade mechanism)

The 3-second hold uses the existing splash dismiss flow — the frontend's
`fadeOutStartupSplash()` IPC call is held until the launcher sends the dismiss
signal after the hold expires.

**Special cases:**
- If total startup was < 500ms (all stages fast, no migrations): hold only 1s
  (user barely saw the splash; no need to dwell)
- If an error occurred (e.g. migration failed, srv failed to start): hold until
  user clicks Dismiss (splash gains a `[OK]` button in that case)
- `AGENTMUX_SPLASH_HOLD_MS=0` disables the hold (CI/test mode)

---

## Integration Points

### Launcher changes

**`agentmux-launcher/src/startup_events.rs`** (new)

```rust
pub struct StartupEventSink {
    sender: tokio::sync::broadcast::Sender<StartupEvent>,
}

pub struct StartupEvent {
    pub kind: StartupEventKind,  // Begin | End | SubBegin | SubEnd
    pub stage: &'static str,
    pub label: &'static str,
    pub id: Option<&'static str>,
    pub t: u64,                  // Unix ms
    pub duration_ms: Option<u64>,
    pub status: Option<StartupStatus>,
    pub detail: Option<String>,
}
```

**`agentmux-launcher/src/main.rs`** — instrument existing stages:

```rust
// Before saga recovery
sink.begin("saga", "Saga recovery");
run_saga_recovery(...).await;
let n_recovered = ...;
sink.end("saga", StartupStatus::Ok,
    Some(format!("{n_recovered} recovered")));

// Before migration subprocess
sink.begin("migrations", "Migrations");
// ... spawn migrate subprocess, forward stdout as sub-events
sink.end("migrations", StartupStatus::Ok, Some(migration_summary));

// Before srv spawn (concurrent with pruner)
sink.begin("pruner", "Channel pruner");
tokio::spawn(pruner::run(sink.clone(), current_channel, channels_dir));

sink.begin("backend", "Backend startup");
let srv = spawn_srv(...).await;
sink.end("backend", StartupStatus::Ok,
    Some(format!("ready on port {}", srv.port)));
```

**`agentmux-launcher/src/splash.rs`** — subscribe to `StartupEventSink`; on each
event, re-render the stage list. Timer task fires every 100ms to update running
clocks.

### Migration runner changes

**`agentmux-srv/src/migrations/runner.rs`** — normalise stdout JSON to the new
`startup_sub_begin` / `startup_sub_end` format. Keep backward compat: launcher
accepts both old and new formats during a transition window.

### Host (CEF) changes

**`agentmux-cef/src/launcher_ipc.rs`** — after `CefInitialize()` completes, send:
```json
{ "event": "startup_stage_end", "stage": "cef",
  "duration_ms": 312, "status": "ok", "detail": "ANGLE/D3D11" }
```

### Frontend changes

**`frontend/app-init.ts`** — at `initApp()` start and at `scheduleRevealLift()`:
```ts
// at start of initApp():
sendStartupEvent({ event: "startup_stage_begin", stage: "frontend", label: "Frontend init" });

// just before scheduleRevealLift fires:
sendStartupEvent({ event: "startup_stage_end", stage: "frontend",
    duration_ms: Date.now() - frontendStartMs, status: "ok",
    detail: `${tabCount} tabs, ${paneCount} panes` });
```

`sendStartupEvent` calls `window.api.sendLauncherMsg(payload)` (existing IPC
bridge into the launcher).

---

## Pruner Integration

The pruner (see `SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md`) runs as a background
Tokio task during Phase 2, concurrent with migrations. It emits its own
`startup_sub_begin` / `startup_sub_end` events into the same `StartupEventSink`,
so its progress appears as a stage row in the splash:

```
Channel pruner         89ms  freed 612 MB
  Scanned 7 channels      34ms ✓
  Freed 3 dead channels   55ms ✓  ← or "Nothing to prune"
```

If live old instances are found (pipes still alive):

```
Channel pruner         89ms  ⚠ 2 old instances (0.48.1)
```

The live-instance warning also feeds Phase 2 of the pruner spec (the frontend
notification banner), surfaced via the same data carried in the pruner's
`startup_stage_end` detail field.

---

## Error Handling

| Scenario | Splash behaviour |
|---|---|
| Migration fails | Stage row shows `✗ 89ms  migration 0009 failed: {reason}`; hold splash until user clicks `[OK]`; launcher exits with error |
| Backend fails to start (30s timeout) | `✗ Backend startup  30,000ms  timeout waiting for srv`; same hold + exit |
| Pruner errors | `⚠ Channel pruner  23ms  scan failed: {reason}`; startup continues (non-fatal) |
| Frontend init error | `✗ Frontend init  — `; CEF has its own error display by this point |
| Stage takes > 10s | Add `⚠` prefix to running clock: `▶ ⚠ 0:12.3` (user can see something is stuck) |

---

## Implementation Order

1. **`startup_events.rs`** — define `StartupEventSink` + event types (no splash changes yet)
2. **Instrument launcher stages** — emit events from main.rs (saga, backend, pruner)
3. **Normalise migration JSON** — align srv's stdout to the new format
4. **Splash renderer** — subscribe to sink; re-render stage list + running clock timer
5. **Summary hold** — implement 3s hold in splash dismiss path
6. **CEF IPC event** — emit `cef` stage end from host after `CefInitialize`
7. **Frontend events** — send `frontend` stage begin/end via `sendLauncherMsg`
8. **Pruner integration** — wire pruner to emit into `StartupEventSink`

Steps 1–5 ship together (the useful core); 6–8 add detail progressively.

---

## Success Metrics

- User can see during a slow startup exactly which stage is slow
- After first paint, splash holds 3s showing the full timeline
- Pruner results (freed MB, live old instances) visible in the splash summary
- Zero regression to startup performance (event emission is async, sink is
  non-blocking)
- `AGENTMUX_SPLASH_HOLD_MS=0` passes in CI without visible splash
