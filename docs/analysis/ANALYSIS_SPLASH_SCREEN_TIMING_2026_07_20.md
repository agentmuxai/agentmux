# macOS Splash Screen — Countup Timing & the Total-vs-Items Gap

**Date:** 2026-07-20
**Scope:** `agentmux-launcher/src/splash_mac.rs` (macOS-native splash renderer) +
`agentmux-launcher/src/startup_events.rs` (event model) + emitters in
`agentmux-launcher/src/srv_spawner.rs`, `agentmux-launcher/src/supervisor/unix.rs`,
`agentmux-cef/src/lib.rs`.

**Two questions asked:** (1) why do some splash lines show a live countup while
others just snap to their final value, and (2) why is there a visible gap
between the sum of the per-item numbers and the "total" row at the bottom.

Note on scope: `splash.rs` at the repo root of `agentmux-launcher/src/` is the
**Windows** splash (`#![cfg(target_os = "windows")]`) — a structurally similar
but entirely separate, non-shared implementation from `splash_mac.rs` (the
duplication is called out as a known follow-up in `splash_mac.rs:79-84`).
`splash_text.rs` is also Windows-only — macOS renders every line via retained
`NSTextField`s instead (`splash_mac.rs:1034-1045`). Everything below is macOS
(`splash_mac.rs`) specifically.

---

## 1. What drives a redraw

`Splash::run_until_dismissed` (`splash_mac.rs:656-754`) runs a loop with two
distinct phases:

- **Before the host signals first paint** (`ready_at` is still `None`): the
  loop runs unconditionally on a fixed cadence — `pump_app_events(0.016)`
  (16ms AppKit event wait, line 668) followed by
  `std::thread::sleep(Duration::from_millis(8))` (line 740) — roughly an
  8–24ms tick. `update_stage_fields(...)` is called **every tick**, whether or
  not a new event arrived: `if changed || ready_at.is_none() { ... }`
  (lines 713-718). So pre-ready, this is genuinely timer-driven.
- **After ready** (host signaled, or the 10s `DISMISS_TIMEOUT` fired): the
  condition collapses to event-driven only — the display only updates again
  when a new `StartupEvent` shows up (`changed`). One forced final refresh
  fires exactly at the ready transition (line 707, `changed = true; // force
  one final refresh showing the "total:" row`) to freeze the display with the
  total visible.

Events are drained non-blocking every tick:
`while let Ok(ev) = self.startup_rx.try_recv() { apply_event(...); changed = true; }`
(lines 672-676).

---

## 2. Why some lines countup live and others just flash to a final value

**It is not "only the currently-active row animates."** `flatten_rows`
(`splash_mac.rs:218-281`) recomputes *every* row with `done == None` on every
single tick, all at once — there's no concept of a single "active" item.

The actual mechanism:
- `apply_event` pushes a new row on `StageBegin`/`SubBegin`
  (`done: None`, `started_at: Instant::now()`) and only flips `done` when the
  matching `StageEnd`/`SubEnd` arrives (lines 144-178).
- Both Begin and End travel over the same unbounded `mpsc` channel
  (`startup_events.rs:56-65`) and get drained together in one `try_recv` loop
  per tick.
- **If a step's real work finishes fast enough that both its Begin and End
  land in the channel before the splash's next drain, `apply_event` processes
  them back-to-back within one tick** — the row is created *already done*.
  No "running" frame is ever rendered for it, so it just appears with its
  final value.
- If the Begin→End gap is longer than roughly one tick (~8–24ms), at least
  one intermediate `format_running(started_at)` frame gets drawn before the
  End arrives — that's the live countup.

**There is no minimum-duration threshold constant anywhere in the crate for
this.** It's a pure emergent effect of tick cadence vs. item speed, not a
deliberate design choice. The `--splash-selftest` fixture (`main.rs:218-245`)
demonstrates exactly this — it inserts real `std::thread::sleep`s (80ms, 40ms,
1500ms, etc.) between paired begin/end calls specifically so the countup is
visible in the demo; without an artificial delay, fast steps just wouldn't
show one.

**Related edge case:** a row whose End event never arrives before `ready_at`
fires gets frozen forever at whatever value the one forced final refresh
produced — stuck in the "in progress" color, never turns green, never updates
again (post-ready refreshes only happen on `changed`, and nothing will mark it
done after that point).

---

## 3. How each per-item duration is computed

Two independent numbers exist per row, and the splash only ever shows one:

- **Splash-local `started_at: Instant`** (lines 129, 136) — stamped when the
  splash thread *processes* the Begin event (i.e. at whatever tick actually
  drains it), used only to drive the live counter while `done` is `None`.
- **Reported `duration_ms: u64`** — carried as data inside the `StageEnd`/
  `SubEnd` event itself (`startup_events.rs:30-35, 43-50`), computed
  independently by whatever code emits it, not by the splash:
  - "migrations" stage: `srv_spawner.rs:105` (`Instant::now()`) through
    `srv_spawner.rs:187` (`t.elapsed()`).
  - "host" stage: `supervisor/unix.rs:363, 368`.
  - Individual sub-migrations: self-reported in each migration binary's own
    stdout JSON, parsed at `srv_spawner.rs:155-163, 220, 234`.
  - CEF-side stages (`dlopen`, `cef_init`): computed by the host process and
    forwarded cross-process over IPC
    (`agentmux-cef/src/lib.rs:577-578, 883-958` →
    `Command::ReportStartupStageBegin/End` →
    `agentmux-launcher/src/ipc/server.rs:527-555`).

Once `apply_event` stores the reported value in `done`, `format_ms(ms)`
renders it verbatim forever — the splash never recomputes or reconciles it
against its own clock.

---

## 4. How the "total" is computed

A **separate wall-clock stopwatch**, unrelated to any item's duration:

```rust
let start = Instant::now();               // splash_mac.rs:657, at run_until_dismissed entry
...
if ready_at.is_none() && (self.ready_file.exists() || start.elapsed() > DISMISS_TIMEOUT) {
    ...
    total_ms = start.elapsed().as_millis() as u64;   // splash_mac.rs:695-700
}
```

`start` begins right after the splash window is built (`Splash::show`, called
from `main.rs:131`) — i.e. before the "prep" stage even begins. It stops the
instant the host's ready-file appears (first paint detected) or the 10s
timeout fires. This `total_ms` is threaded into
`flatten_rows(stages, total_ms)` and rendered as the `"total: {}"` row
(lines 269-278). **It has zero arithmetic relationship to any stage/sub
`duration_ms` value** — it's purely `Instant::now() - start` at the moment
readiness is detected.

---

## 5. Why the gap exists, concretely

The splash **never sums the item durations itself** — `flatten_rows` /
`draw_stages` don't compute a "sum of items" anywhere. That comparison only
exists if someone manually adds up the displayed per-row numbers and compares
against the separately-displayed total. Once you do that, the gap comes from
several genuinely additive sources:

1. **Real elapsed time that no stage/sub covers at all.** The "host" stage is
   explicitly scoped to process-spawn latency only, not full first paint — the
   code says so directly:
   > `supervisor/unix.rs:352-361`: "'host' stage currently covers process-spawn
   > latency only (begin → spawn_host_unix returning a live Child), not full
   > first-paint... Extending this stage to span to first-paint is a
   > follow-up."

   Even after `cef_init` ends (`agentmux-cef/src/lib.rs:953-958`, right after
   `cef::initialize()` returns), there's still browser-window creation,
   frontend navigation, and full page load before `on_load_end` writes the
   ready-file that stops `total_ms`
   (`agentmux-cef/src/client/navigation.rs:293-315`). None of
   dlopen-end→cef_init-begin or cef_init-end→on_load_end is wrapped in any
   stage/sub event — invisible to "sum of items," fully counted in "total."

2. **Idle time between stages isn't attributed to anything.** `total_ms` runs
   continuously from before "prep" starts; any gap between one stage's End and
   the next stage's Begin (process-spawn latency, IPC round-trips, scheduler
   delay) accrues to the wall-clock total but to no row. The self-test fixture
   makes this explicit with genuine sleeps *between* items (e.g. a 100ms gap
   between `stage_end("prep",...)` and `stage_begin("migrations",...)`,
   `main.rs:227-238`) — dead time with no owning row, by construction.

3. **A stage's own reported duration already exceeds the sum of its own
   subs**, for legitimate reasons — the same pattern repeats one level down.
   In `run_migrate` (`srv_spawner.rs:99-213`), the "migrations" stage's
   `duration_ms` (line 187) spans process spawn through process wait,
   including subprocess spawn overhead and stdout-parsing latency between
   sub-migrations — none of which is attributed to any individual
   sub-migration's own self-reported duration.

4. **Unsynchronized clock sources.** The splash's own `Instant`s (`started_at`,
   the total's `start`) and each emitter's own timers are independent
   monotonic clocks running in different threads/processes. Nothing
   reconciles them against each other; they're just displayed side by side.

No double-counting or overlap is happening — the "gap" is real elapsed time
that's genuinely uncovered by any instrumented stage, not an arithmetic bug.

---

## 6. Related constants

| Constant | Value | Location | Effect |
|---|---|---|---|
| `DISMISS_TIMEOUT` | 10s | `splash_mac.rs:72` | Safety cap forcing `ready_at`/`total_ms` even if the host never signals readiness |
| Tick cadence | ~8–24ms (16ms event wait + 8ms sleep, both literals, no named constant) | `splash_mac.rs:668, 740` | The de facto "poll interval" — determines whether a fast item's Begin/End land in the same drain batch (→ no countup) or different ones (→ countup) |
| `AGENTMUX_SPLASH_HOLD_MS` | env, default 2000ms | `splash_mac.rs:701-704` | Post-ready hold before fade-out; capped to `min(1000)` if `total_ms < 500` (line 705) |
| `FADE_OUT` | 0.16s | `splash_mac.rs:73` | Fade-out duration after the hold |

**No minimum-duration-to-animate threshold exists anywhere in the crate** —
confirmed by grep. The animate/no-animate split is a pure side effect of tick
cadence vs. item speed, not a deliberate design decision.

---

## 7. If we want to change the behavior

Not implemented here — investigation only, per the ask. But worth noting for
a follow-up:

- **To make every item show at least one countup frame:** insert a minimum
  artificial delay before processing an End event whose paired Begin landed in
  the same tick (cheap, but adds latency to fast steps purely for cosmetics),
  or decouple rendering from the drain loop — poll/redraw on a fixed timer
  independent of `try_recv`, so a fast item's "running" state gets at least
  one paint even if its End is already queued behind it in the channel.
- **To close the total-vs-items gap** (if the goal is for them to visibly
  reconcile, not just to explain it): would need genuinely new instrumentation
  — e.g. a stage spanning cef_init-end → on_load_end (extending the "host"
  stage's documented scope per the `supervisor/unix.rs:352-361` comment), and
  an explicit "idle"/"other" row that gets whatever `total_ms` minus sum-of-
  stages leaves over, rather than leaving that time unlabeled.
