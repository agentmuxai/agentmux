# SPEC: Terminal I/O Has Complete Priority Over Perf Monitoring

**Date:** 2026-06-16
**Status:** Phase 1 implemented (priority/background egress lanes + biased select); Phase 2 implemented (egress-side coalesce to latest-only per event×scope); Phases 3–4 proposed
**Author:** smike (agent)
**Area:** `agentmux-srv` WebSocket egress · sysinfo collector · terminal echo path
**Related history:** PR #926 (`fix(term): remove writeInFlight guard from echo fast path`), `SPEC_TERM_DOUBLE_RAF_TEAROUT_2026_05_30.md`, `docs/terminal-input-latency-report.md`, `SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31.md`

---

## 1. The symptom

After a terminal pane has been open and busy for a long session, typed keystrokes
echo back with a small, periodic stutter — the echo is **"delayed by a tick,"** and
the tick **lines up with the CPU/perf readout refreshing** (the sysinfo widget /
per-core popover updating once a second). On a fresh session it's imperceptible; it
grows with use. This is *not* a regression of the previously-fixed typing-latency
bugs (those were frontend frame-pacing and a write-in-flight guard — see §7). It is a
new, distinct quirk in the **backend egress scheduling**.

**Design intent (non-negotiable):** typing inside the terminal must have **complete
priority over perf monitoring.** Perf telemetry is a best-effort, droppable, 1 Hz
cosmetic readout; a keystroke echo is an interactive, latency-critical signal. When
the two contend, the keystroke wins every time — never the reverse.

---

## 2. Root cause — confirmed in code

### 2.1 Terminal echo and sysinfo share **one** FIFO channel

Every WebSocket connection registers exactly **one** unbounded receiver for *all*
server→client wave events:

`agentmux-srv/src/backend/eventbus.rs:50-65`
```rust
pub fn register_ws(&self, conn_id: &str, tab_id: &str)
    -> tokio::sync::mpsc::UnboundedReceiver<serde_json::Value> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();   // ONE channel for everything
    ...
    rx
}
```

Both of these ride that same channel, FIFO:

- **Terminal echo output** — the PTY read loop appends to the block file and publishes
  a `blockfile` append event (`shell.rs` → `handle_append_block_file` → `broker.publish`).
- **Perf telemetry** — the sysinfo loop publishes a `sysinfo` event (scope `local`)
  **plus one `block:<id>` `blockstats` event per tracked block** every tick
  (`sysinfo.rs:188` and `sysinfo.rs:275`).

All of them funnel through `broker.publish` → `EventBusBridge::send_event` →
`event_bus.send_to_conn` → the connection's single `event_rx`
(`eventbus.rs:100-114`). **There is no separation between interactive terminal data
and droppable telemetry.**

### 2.2 The egress `select!` is **unbiased** — telemetry can win the race

`agentmux-srv/src/server/websocket.rs:171-273`
```rust
loop {
    tokio::select! {
        // (no `biased;` — branches are polled in RANDOM order)
        Some(event) = event_rx.recv() => {          // ← terminal echo AND sysinfo, same branch
            let msg = ...serde_json::to_string(&event)...;
            if socket.send(Message::Text(msg.into())).await.is_err() { break; }
        }
        Some(rpc_msg) = rpc_output_rx.recv() => { ... }   // RPC replies
        msg = socket.recv() => { ... }                    // incoming keystrokes
        _ = ping_interval.tick() => { ... }
    }
}
```

Two compounding problems:

1. **Same branch.** Terminal echo and sysinfo are *indistinguishable* here — both are
   just "an item on `event_rx`." Because the channel is FIFO, if a sysinfo burst was
   enqueued microseconds before the echo, the echo waits behind the entire burst.
2. **Unbiased select.** Even across branches, `tokio::select!` without `biased;`
   polls in a **random** order, so an incoming keystroke (`socket.recv`) and a ready
   telemetry frame (`event_rx`) have equal odds — the runtime may service the
   telemetry write first and make the keystroke wait for a `socket.send().await` to
   complete. (`biased;` is already an accepted pattern in this codebase —
   `subprocess.rs:1185` uses it.)

Each `socket.send(...).await` is a real suspension point. When the loop chooses to
flush a sysinfo/blockstats frame, the next keystroke echo cannot be written until that
send completes — **one frame of head-of-line blocking, paced at exactly the sysinfo
tick.** That is the "delayed by a tick that lines up with the perf info."

### 2.3 Why it **grows with use** ("after long use")

The sysinfo tick's cost and burst size both scale with how much you've done this
session:

- **Per-tick burst grows with open blocks.** The collector emits one `blockstats`
  event *per tracked block* every tick (`sysinfo.rs:236-276`, `broker.publish` inside
  the per-block loop). Open more panes/agents/terminals over a long session → more
  `blockstats` events enqueued ahead of / interleaved with your echo on the shared
  FIFO, every single tick.
- **Collection cost grows with total process count.** Pass 1 refreshes **all**
  processes on the machine to populate the PID→parent links
  (`sysinfo.rs:197-201`, `ProcessesToUpdate::All`). The more processes running after a
  long session (spawned agents, shells, build tools), the longer the tick holds and
  the larger the downstream payload — widening the window in which an echo can land
  behind it.

So the stutter is invisible early (1 sysinfo event, ~no blocks, few processes) and
becomes a perceptible periodic tick late (sysinfo + N blockstats, many processes),
exactly matching the report.

### 2.4 Secondary (frontend) contributor

When the **per-core CPU popover is open**, each sysinfo event reconciles an
`<Index>` over every core on the browser main thread
(`CpuCoresPopover.tsx:80-98, 229-286`) — on a high-core machine that competes with
xterm's render on the same thread. This is real but secondary: it only bites with the
popover open and on many-core machines. `SystemStats.tsx` itself is cheap (8 scalar
values). The dominant, always-present cause is the backend shared-channel HOL blocking
in §2.1–§2.3. Frontend mitigation is **Phase 3** (optional).

---

## 3. Design principle

> **Interactive terminal I/O is a priority lane. Perf telemetry is a background lane.
> The background lane may never delay the priority lane — not by a frame, not by a
> byte. Telemetry is droppable; echo is not.**

Concretely:
1. **Separate the lanes.** Terminal echo (and RPC replies / interactive events) must
   not share a FIFO queue with sysinfo/blockstats telemetry.
2. **Bias toward interactivity.** When both lanes are ready, the egress loop drains
   the priority lane to empty before touching the background lane.
3. **Telemetry yields, and may be coalesced or dropped.** A late sysinfo frame is
   worthless — only the latest reading matters. Stale telemetry should be coalesced
   (keep newest, drop superseded) rather than queued.
4. **Optionally, telemetry steps aside while typing.** As the strongest expression of
   "complete priority," the collector may skip/defer a publish for a connection that
   had keystroke activity in the last ~150 ms.

---

## 4. Proposed design

### Phase 1 — Split egress into priority + background lanes (the core fix)

**Goal:** terminal echo and RPC replies never queue behind telemetry, and never lose
a `select!` coin-flip to it.

1. **Two receivers per connection.** Change `EventBus::register_ws` to return a
   `(priority_rx, background_rx)` pair (or a small struct). Internally keep two
   `UnboundedSender`s per `WindowWatchData`.
2. **Classify at enqueue.** In `send_to_conn` / `broadcast_event`, route by event
   type: `sysinfo` and `blockstats` → background lane; everything else (terminal
   `blockfile` appends, `waveobj:update`, config, etc.) → priority lane. Classification
   is a cheap match on `event.event` (constants `EVENT_SYS_INFO`, `EVENT_BLOCK_STATS`
   already exist).
3. **Biased drain in the egress loop** (`websocket.rs:171`):
   ```rust
   loop {
       tokio::select! {
           biased;                                   // poll in order, top-first
           msg = socket.recv() => { ... }            // 1. incoming keystrokes
           Some(rpc_msg) = rpc_output_rx.recv() => { ... }   // 2. RPC replies
           Some(ev) = priority_rx.recv() => { ... }  // 3. terminal echo + interactive
           Some(ev) = background_rx.recv() => { ... }// 4. sysinfo / blockstats (last)
           _ = ping_interval.tick() => { ... }       // 5. ping
       }
   }
   ```
   With `biased;`, a ready keystroke or echo is always serviced before a ready
   telemetry frame. Telemetry is only flushed when the priority lanes are momentarily
   empty.

This alone removes the head-of-line blocking and the lost coin-flip — the two
mechanisms behind the periodic tick.

### Phase 2 — Coalesce / bound the telemetry lane (kill the "grows with use" burst)

Even on its own lane, a fat telemetry burst shouldn't accumulate unbounded behind a
slow client.

1. **Coalesce sysinfo to latest-only.** The background lane should keep only the most
   recent `sysinfo` reading per scope; a new tick supersedes an unsent one. (Either a
   1-slot "latest value" cell drained by the egress loop, or a drop-oldest dedup on
   enqueue keyed by `event` + scope.) A dropped intermediate sysinfo frame is
   invisible to the user — the gauge just shows the newest number.
2. **Same for `blockstats`** — keyed per `block:<id>`.
3. **Lower the default per-tick fan-out cost.** Pass-1 `ProcessesToUpdate::All`
   (`sysinfo.rs:197`) is the per-tick scaling cost. Options (smallest first):
   refresh parent links less often than the publish cadence (e.g. tree topology every
   Nth tick, CPU/mem every tick), or scope pass-1 to the union of tracked trees plus
   their ancestors instead of all processes. **Out of scope to redesign here** — noted
   as a follow-up; Phases 1+2 already remove the *latency*, this only trims CPU.

### Phase 3 — Frontend: telemetry renders off the interactive path (optional)

Only if measurement (§6) still shows a frame cost with the popover open on many-core
machines:
- Gate `CpuCoresPopover` reconciliation behind `requestIdleCallback` / a low-priority
  scheduler so a 1 Hz core-grid update can't land in the same frame as an xterm write.
- Leave `SystemStats` (the always-visible cheap readout) as-is.

### Phase 4 — "Step aside while typing" (optional, strongest guarantee)

The most literal reading of "complete priority": the sysinfo collector skips a publish
to a connection that typed in the last ~150 ms. Requires the egress side to expose a
per-connection `last_input_at` the collector can read. **Deliberately deferred** —
Phases 1+2 should fully resolve the symptom without coupling the collector to input
state, and this adds a grace-timer-like heuristic the project generally avoids (cf.
the predictive-echo "no wall-clock timers" rule). Documented as the fallback if a
residual tick survives Phases 1–2.

---

## 5. Why not the simpler knobs

- **Just slow the sysinfo interval (2 s) / "pause when focused."** Treats the symptom,
  not the cause — the HOL blocking still happens, just less often, and it degrades the
  feature the user explicitly wants to keep. Rejected as the primary fix.
- **Just add `biased;` (no lane split).** Necessary but insufficient: terminal echo
  and sysinfo share the *same* `event_rx` branch (§2.1), so biasing across branches
  doesn't separate them — a sysinfo frame already ahead of the echo in that one FIFO
  still blocks it. The lane split is what makes `biased;` effective for echo.
- **Bound the channel / drop on backpressure.** The input channel is intentionally
  unbounded to avoid the paste-truncation bug; we keep echo lossless and instead make
  *telemetry* the thing that coalesces/drops (Phase 2). Correct lane to shed load.

---

## 6. Verification

**Reproduce first (so we can prove the fix):**
- Long-session repro: open several agent + terminal panes, let many processes run,
  hold a key down in a terminal, and watch for a ~1 Hz hitch in the echo that
  coincides with the sysinfo readout updating.
- Instrument with the existing perf marks: `term-echo-render`
  (`termwrap.ts`, WPS-arrival → xterm write callback). Record the Performance
  timeline while typing continuously for ~30 s; look for a recurring ~1 Hz outlier in
  the `term-echo-render` histogram and confirm it disappears after Phase 1.
- Optional backend trace: log egress-branch selection + queue depth; confirm echo
  frames stop being preceded by telemetry frames on the same lane.

**Automated:**
- Unit test `EventBus` lane classification: a `sysinfo`/`blockstats` event lands on
  `background_rx`; a `blockfile`/`waveobj:update` event lands on `priority_rx`.
- Unit test the coalescer: enqueuing two `sysinfo` frames for the same scope before a
  drain yields only the newest.
- `cargo check -p agentmux-srv` + existing eventbus/websocket tests green.

**Acceptance:**
- With many panes open and processes running, continuous typing shows **no** periodic
  echo hitch correlated with the sysinfo tick (the `term-echo-render` ~1 Hz outlier is
  gone).
- Sysinfo gauges still update at their configured cadence (telemetry not broken, just
  deprioritized/coalesced).

---

## 7. Relationship to prior terminal-latency work (not a regression)

This is a *new* layer of the problem; the prior fixes stand:

| Prior work | Layer it fixed | Why it doesn't cover this |
|---|---|---|
| PR #926 — remove `writeInFlight` guard | Frontend echo fast-path stall | Frontend write gating; unrelated to backend egress scheduling |
| `SPEC_TERM_DOUBLE_RAF_TEAROUT_2026_05_30` | Frontend double-rAF frame beat | Removed a second frame gate; this is server→client queueing |
| `docs/terminal-input-latency-report.md` | xterm write serialization | Per-write ordering; not cross-event-type priority |
| `SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31` | Hides round-trip via local echo | Predicts the echo; doesn't fix the authoritative echo being delayed by telemetry |

Predictive local echo, where enabled, *masks* this for printable characters — but the
authoritative stream (and anything prediction can't predict: control sequences, program
output, cursor moves) still rides the contended channel. Fixing the channel benefits
every byte, predicted or not.

---

## 8. Scope / files touched

**Phase 1 (core):**
- `agentmux-srv/src/backend/eventbus.rs` — two-lane `register_ws`; classify in
  `send_to_conn` / `broadcast_event`.
- `agentmux-srv/src/server/websocket.rs` — consume two receivers; `biased;` select
  with priority order input → RPC → priority events → background events → ping.
- Caller of `register_ws` is only `websocket.rs:127` (verified) — small blast radius.

**Phase 2:** `eventbus.rs` (coalescing background lane); optional `sysinfo.rs` pass-1
scoping (follow-up).

**Phase 3 (optional):** `frontend/app/statusbar/CpuCoresPopover.tsx`.

**Phase 4 (deferred):** per-connection `last_input_at` shared with `sysinfo.rs`.

---

## 9. Recommendation

Ship **Phase 1** first — it directly removes both mechanisms (shared FIFO + lost
coin-flip) behind the reported tick and is a contained, testable change with a tiny
blast radius. Measure with `term-echo-render`. Add **Phase 2** coalescing to keep the
fix robust as sessions grow. Treat Phases 3–4 as conditional, driven by whether any
residual hitch survives measurement.
