# Report: renderer/GPU-helper CPU during active agent output — RCA and fix

**Date:** 2026-09-11
**Trigger:** repo owner, live: *"figure out why the agentmux helper takes up
so much CPU. we can feel the slowdown."*
**Status:** Active — RCA complete, fix implemented and unit-verified (§6-7). Live production CPU comparison deliberately deferred — see §7.
**Method:** Live measurement on the affected machine (this is not doc
archaeology or a synthetic repro) — `ps`/`top` for real-time CPU, macOS
`sample` for a stack-level profile of the hot process, `muxspect` for live
pane state, then a direct read of the current PTY read path on `main`.

---

## 1. Symptom

`AgentMux Helper (Renderer)` and `AgentMux Helper (GPU)` — CEF helper
processes belonging to a long-running installed AgentMux window (channel
`local-clare06219-734214c7f-20260906t141543`, up 4+ days) — were observed
consuming sustained, user-perceptible CPU. Confirmed real via four
consecutive live `top -pid` reads, ~1s apart:

```
PID    COMMAND          %CPU
39396  AgentMux Helper  0.0
39396  AgentMux Helper  27.7
39396  AgentMux Helper  18.3
39396  AgentMux Helper  18.4

39391  AgentMux Helper  0.0
39391  AgentMux Helper  17.8
39391  AgentMux Helper  16.0
39391  AgentMux Helper  16.4
```

Bursty — roughly once-per-second peaks, not a flat 100%-of-one-core spin —
but real and sustained over the whole observation window, not a one-off
transient.

## 2. What the CPU is actually doing (profiled, not guessed)

`sample`d PID 39396 (the renderer) for 15 seconds at 1ms resolution. The
"sort by top of stack" breakdown — which function each thread was inside at
the moment of each sample — was overwhelmingly:

| Frame | Samples (of ~253k total) |
|---|---|
| `mach_msg2_trap` | 176,807 |
| `kevent64` | 50,192 |
| `__workq_kernreturn` | 12,719 |
| `read` | 12,697 |
| every actual "doing work" frame (memmove, memset, CEF-internal) | single/double digits each |

This is the signature of **many small, frequent wakeups and IPC round-trips**
— threads constantly blocking on `mach_msg`/`kevent`/`read` and getting woken
again — not a single hot function spinning. If this were a stuck computation
or a busy-loop bug, one or a few frames would dominate the "doing work"
column instead. They don't; that column is nearly empty by comparison to the
wakeup frames.

## 3. What was live on this window at the time

`muxspect list` against this exact instance showed two agent panes, **both
`turn_active: true`** at the moment of investigation — one of them this very
investigating session. This machine has run multiple concurrent, long-lived
agent sessions (confirmed separately via `ps`: at least three resumed
`claude` processes) producing continuous, high-volume terminal/build output
(cargo builds, git operations, test suites, log tails) for hours in this
window.

## 4. Root cause

`agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs`'s PTY output
read loop (the thread that owns each pane's PTY):

```rust
let mut buf = [0u8; PTY_READ_BUF_SIZE];   // PTY_READ_BUF_SIZE = 4096, pty.rs:12
loop {
    match reader.read(&mut buf) {
        Ok(0) => break,
        Ok(n) => {
            // ... OSC extraction ...
            handle_append_block_file(broker, &block_id_read, "term", chunk, ...);
            //  ^ file-store write AND live WebSocket broadcast to the
            //    renderer, unconditionally, on EVERY read() return.
            for ev in &osc_events { wps::publish_block_activity(...); }
            if let Some(ref mut t) = translator {
                accumulate_and_translate(...);   // agent-pane JSON-stream path
            }
        }
        Err(_) => break,
    }
}
```

**There is no batching or coalescing between consecutive reads.** `read()`
returns as soon as the OS has *any* data ready, up to 4KB — it does not wait
to fill the buffer. A fast producer (exactly what `cargo build`, `git log`,
or a verbose test run is) causes the OS to deliver output across many
separate `read()` returns in quick succession, and **every single one**
triggers the full downstream pipeline: a disk write, a live WebSocket
broadcast to the renderer, OSC extraction, and — for agent panes — a
JSON-stream-translation pass.

Each of those broadcasts is individually cheap. The *rate* is not: a burst
of output that could be delivered as one or two renderer updates instead
generates dozens of small ones, each carrying its own IPC round-trip
(browser process → renderer, `mach_msg` under the hood) and its own paint /
xterm.js write. That matches §2's profile exactly — CPU dominated by wakeup
and IPC-wait frames, not by any single expensive operation — and matches §3:
it only shows up as *user-perceptible* when a pane is actively streaming
fast, which two panes on this exact window were doing at the time.

This is not a stuck process, a leak, or an infinite loop. It is the
aggregate cost of firing one full render update per 4KB PTY chunk instead of
coalescing a burst into fewer, larger ones — paid continuously by every
agent/terminal pane that is actively producing fast output, which on a
machine running several long, verbose agent sessions concurrently is close
to constant.

## 5. What this report does NOT claim

No direct browser-side (DevTools/CEF) profile correlating individual
WebSocket message timestamps to the observed CPU bursts was taken — this
agent's shell has no screen access on this machine (established earlier this
session; see memory `macos-agent-shell-verification-limits`). §2–§4 are
strong, convergent circumstantial evidence — a live measurement, a stack
profile, live pane state, and a direct code read that all point the same
way — not a single smoking-gun trace. A human with DevTools access could
tighten this further by watching the WS message rate during a live `cargo
build` in an agent pane against the same CPU counters.

This report also does not claim every renderer/GPU CPU cost on this machine
traces to this one path — only that it is a real, evidenced, and structural
contributor, consistent with everything measured.

## 6. Fix — implemented

`agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs`: the PTY
read loop's blocking `read()` semantics are completely untouched (this file
supports both Unix and Windows PTYs, and that boundary is not something to
risk getting subtly platform-wrong). Instead, the loop now sends each
already-OSC-cleaned chunk through an `mpsc::unbounded_channel` to a new,
standalone async fn, `run_pty_output_flusher`, which:

- waits for the first chunk of a new batch, then opportunistically drains
  any more that are already queued (or arrive within `PTY_COALESCE_WINDOW`
  = 20ms) into the same batch;
- stops early, regardless of the time window, once the batch reaches
  `PTY_COALESCE_MAX_BYTES` (64 × the 4KB single-read size = 256KiB) — bounds
  both broadcast latency and memory during a sustained fast burst (`yes`
  piped into a shell pane);
- flushes the whole batch through one call to `flush_pty_batch` (the file
  write, WebSocket broadcast, OSC-activity publish, and agent-translation
  step that used to run once per raw read, now once per batch);
- flushes any final partial batch when the channel closes (PTY EOF/error)
  rather than dropping it.

Content is unaffected — same bytes, same order, just batched into fewer
calls; scrollback persistence, OSC title extraction, and the agent
JSON-stream translator all see identical input to before.

## 7. Verification

**Done:**

- `cargo test -p agentmux-srv`: 3280 passed, 0 failed — no regression in
  the existing PTY/lifecycle/scrollback/OSC/translation test coverage.
- Five new tests directly exercise the new logic (not just the surrounding
  code), against a real in-memory `FileStore` and a real `wps::Broker` with
  a recording `WpsClient` so broadcast *count* — not just final content —
  is observable:
  - `flush_pty_batch_writes_and_broadcasts_one_call` / `..._is_a_no_op_for_empty_input`
    — the single-batch flush primitive in isolation.
  - `rapid_chunks_coalesce_into_far_fewer_broadcasts_with_no_data_loss` —
    50 chunks sent back-to-back (deterministic, not timing-dependent: every
    chunk is queued before the flusher is ever polled) collapsed into
    **exactly 1 broadcast**, verified stable across 8 repeated local runs
    before asserting on the exact count rather than a loose bound. Final
    content, and the concatenation of every individual broadcast payload in
    delivery order, both reconstruct the exact original byte stream.
  - `byte_cap_forces_a_flush_before_the_channel_closes` — 80 × 4KB chunks
    (over the 256KiB cap) forced ≥2 broadcasts rather than growing one
    batch unboundedly, with no data loss.
  - `no_broker_is_a_clean_no_op` — mirrors the read loop's own
    `broker.is_some()` gate.

**Deliberately not done, and why:** a live before/after CPU comparison
against the actual production instance (§1-3's evidence) would mean
building and swapping in this binary on the shared AgentMux window this
investigation was run from — restarting it, which tears down every open
pane, including the sessions that were actively `turn_active: true` at the
time of the original measurement. That is a real, visible, disruptive
action on shared state, not something to do unasked mid-investigation.
Left for the repo owner to trigger deliberately (a `task package:macos`
build + manual swap, or simply picking this fix up in the next normal
release) rather than done silently here.
