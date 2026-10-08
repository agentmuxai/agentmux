# REPORT — Performance indicators: from top-of-window banners to the status bar dot and its panel

**Date:** 2026-10-08
**Author:** Camper@narko
**Trigger:** Operator: the performance checks that appear at the top of the window, edge to edge, and
come and go, should move off the top. The status bar's backend dot turns yellow while there is a problem
(no chip on the status bar itself); the notices appear as chips at the top of the panel that opens from
the uptime; and they still come and go as the instance recovers. Reassess the architecture behind them too.
**Status:** implemented (#4487) — plan agreed with the operator (§6). §3 is the design, §5 the work.

## 1. What exists today

### 1.1 The three signals

| Kind | What it measures | Producer | Levels and thresholds | Sent when |
|---|---|---|---|---|
| `ram` | free physical memory | CEF host, `memory_heartbeat.rs` → `memory_pressure.rs` | normal / warn / critical, debounced in the host | on every transition, **and every heartbeat while not normal** |
| `pagefile` | free commit charge (page file), plus whether Windows can grow it | same heartbeat | same | same, with disk context when it could be read |
| `backend` | srv's responsiveness: the launcher's rolling average of health-probe round trips (`srv_liveness::LatencyTracker`, 6 probes) | launcher supervisor → `NotifySrvLatency` over the launcher pipe → CEF host | warn ≥ 1.0 s (clears < 0.6 s), critical ≥ 2.5 s (back to warn < 1.8 s); a missed probe counts as the full timeout | **on transitions only** |

Specs: `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` §5.F, `SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07.md`,
`docs/analysis/ANALYSIS_SRV_HTTP_STALL_IO_DRIVER_STARVATION_2026_09_26.md` §8.2.

### 1.2 Transport

All three ride one CEF event, `memory-pressure`, emitted to every top-level window
(`crates/cef/src/ui_tasks/window.rs`: `EmitMemoryPressureTask`, `EmitSrvLatencyTask`). The payload is
`{ kind, level, … }` with per-kind extras (`phys_free_mb`; `commit_free_mb`, `system_managed`,
`disk_free_pct`; `avg_ms`). The event has no replay: a window only knows what it hears after it subscribes.

### 1.3 Display

`frontend/app/notification/memory-pressure-banner.tsx` is mounted three times in `app.tsx`, once per kind,
above the workspace in every window except floating-pane windows. Each instance:

- subscribes to `memory-pressure` itself and keeps its own `level`, `payload` and `dismissedAt`;
- renders a full-width strip (amber for warn, red-tinted for critical) with a sentence of advice and ×;
- hides on dismiss until the level escalates or a new episode starts (normal, then up again).

The status bar (`frontend/app/statusbar/StatusBar.tsx`) starts with `BackendStatus`: a dot coloured by
srv's connection state (accent running, yellow spinning connecting, red crashed) and the uptime. Clicking
it opens `BackendStatusPanel` (status, PID, uptime, endpoint, pending migrations, GPU, crash details).
Nothing in the status bar knows about the three signals.

## 2. What is wrong with it

1. **The banners move the layout.** Each one is a block above the workspace, so appearing and clearing
   pushes every pane down and back up. Terminals reflow, native-view panes reposition, and the backend
   signal in particular flaps under build load. A signal meant to say "things are slow" makes things jump.
2. **The state is private to each banner.** Three components each hold their own copy, so nothing else
   (the status bar dot, the panel) can read it. Moving the display means moving the state first.
3. **`backend` has no catch-up.** RAM and page file re-assert every heartbeat while not normal, so a
   window opened or reloaded mid-episode catches up within one tick. The latency signal is sent only on a
   change, so a new window shows nothing until the next transition, possibly for the whole episode.
4. **The name is wrong.** The backend's latency travels as `memory-pressure`.
5. **Dismiss is per window and per kind**, and is needed only because the banner takes room. A yellow dot
   takes none.
6. **The copy is written for a strip**: one long sentence per level. A notice needs a two-word heading, and
   the panel can hold the full sentence and the numbers.

## 3. Proposal

### 3.1 One health-signal store per window

New `frontend/app/store/health-signals.ts`:

```ts
type HealthKind = "ram" | "pagefile" | "backend";
type HealthLevel = "normal" | "warn" | "critical";
interface HealthSignal {
    kind: HealthKind;
    level: Exclude<HealthLevel, "normal">;
    since: number;          // ms epoch, when this episode began
    payload: HealthPayload; // the per-kind extras, as today
}
// active signals, worst first; empty when all is normal
export const healthSignals: Accessor<HealthSignal[]>;
export const worstHealthLevel: Accessor<HealthLevel>;
```

- One listener, installed once per window at app start, feeds a pure reducer
  (`applyHealthEvent(state, payload, now)`): a non-normal level adds or updates the kind (keeping
  `since` across warn ↔ critical), and normal removes it.
- The wording moves out of the banner into `health-signals-text.ts`: a short **label** per kind for the
  panel's notice chips ("Low RAM", "Low page file", "Slow backend") and today's **message** functions (`messageFor`,
  `pagefileGuidance`, `backendMessage`), unchanged.

### 3.2 A window catches up on open, for every kind

The CEF host keeps the last payload per kind in `AppState` (it already sees every RAM and page-file level,
and every `NotifySrvLatency`). A window asks for them when it starts, through a new host API
`getHealthSignals()`, then follows the push events. The host stamps each episode with its start
(`since_ms`), so every window shows the same duration. The per-tick re-send of RAM and page file stays:
catch-up no longer needs it, but it keeps their numbers and the page-file guidance current.

Rejected alternative: make the launcher re-send the latency level every probe. That spreads the fix across
two processes and a pipe to cover what is really a missing snapshot.

### 3.3 Rename the event

`memory-pressure` → `health-signal`, same payload. The CEF host and the frontend ship in one build, so
the rename needs no compatibility period.

### 3.4 The status bar: the dot only

- While srv is running, the backend dot is the warning colour (yellow) whenever any signal is active,
  warn or critical, and the accent colour otherwise. Connecting (yellow, spinning) and crashed (red) are
  unchanged and take precedence, so red on the dot keeps meaning "srv is down".
- **No chip or text is added to the status bar.** The uptime stays as it is.
- The dot's tooltip names the problems ("Backend status: low RAM, slow backend. Click for details"), so
  hovering says what is wrong without opening the panel.
- A visually hidden `aria-live="polite"` region announces a new signal, so a screen reader still hears
  what the banner used to say.

### 3.5 The uptime panel: notice chips at the top

When signals are active, the first thing in `BackendStatusPanel`, above the Status row, is one notice per
signal, worst first, stacked the way the banners stack at the top of the main window today. Each notice is
a chip-styled card: amber for warn, red-tinted for critical, with the icon and label as its heading,
today's full message below (the RAM advice, page-file guidance or the backend average), and how long it
has lasted ("for 4 min"). The notices come and go live while the panel is open. With no active signals
there are none and the panel is as it is now.

### 3.6 Coming and going

Signals clear exactly as the banners do: when the producer reports normal. The thresholds and hysteresis
stay in the producers (§1.1), so a notice doesn't flicker more than the banner did. When the last signal
clears, its notice leaves the panel and the dot returns to the accent colour. Nothing lingers after
recovery: no "recovered" note.

### 3.7 No dismiss

The × and its sticky-per-severity logic go. A yellow dot costs the user no space, so there is nothing to
dismiss; the indicator stays exactly as long as the problem does.

### 3.8 Removed

`MemoryPressureBanner`, its stylesheet and its three mounts in `app.tsx`. The pure helpers and their tests
move to the new text module.

### 3.9 Unchanged

- Floating-pane windows show no indicator, as today: they have neither a status bar nor the banners.
- The producers, their thresholds, and the memory-pressure actions in the host (evicting idle pool windows
  under page-file pressure) are untouched.

## 4. Later, on the same model

The panel already shows other things that are really health signals: GPU running in software, the
terminal on the DOM renderer, pending migrations. Once the store exists they can become kinds too and get a
notice when they matter. That is out of scope here.

## 5. Work

One PR is enough; listed in the order to build it.

1. **Store and wording** — `store/health-signals.ts` (reducer, accessors, one listener) and
   `health-signals-text.ts` (labels, the moved message functions). Unit tests for the reducer: add, update,
   escalate keeping `since`, clear, order by severity.
2. **Host snapshot and rename** — `AppState` keeps the last payload per kind; `getHealthSignals()` in the
   host API; `health-signal` replaces `memory-pressure` in `window.rs`, `launcher_ipc`, and the frontend;
   `since_ms` on each episode.
3. **Status bar** — the dot's colour from `worstHealthLevel`, its tooltip naming the active signals, and
   the hidden live region.
4. **Panel** — the notice chips at the top of `BackendStatusPanel`.
5. **Remove the banners**, and update the specs in §1.1 to point at this design.
6. **Verify in a `task dev` build**, with each signal driven from DevTools:
   `window.dispatchEvent(new CustomEvent('agentmux-event', { detail: { event: 'health-signal', payload: { kind: 'backend', level: 'warn', avg_ms: 1800 } } }))`.
   Check the dot and its tooltip, the panel notices, live clearing with the panel open, a second window
   opened mid-episode, and that no pane moves when a signal appears.

## 6. Decisions (operator, 2026-10-08)

1. **No chip on the status bar.** The backend dot alone turns yellow; the notices appear as chips at the
   top of the uptime panel.
2. **A critical signal keeps the dot yellow**; only its notice in the panel is red-tinted. Red on the dot
   still means srv is down.
3. **No "recovered" note.** A notice disappears when its signal clears.
