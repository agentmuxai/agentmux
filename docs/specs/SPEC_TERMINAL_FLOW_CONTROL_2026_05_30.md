# SPEC: Terminal flow control (PTY backpressure)

**Status:** Draft — design, pre-implementation
**Date:** 2026-05-30
**Author:** AgentY
**Tracks:** [`SPEC_INPUT_RESPONSIVENESS_*` §5.1](./SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) · [`PLAN_INPUT_RESPONSIVENESS_EXECUTION_*` item 7](./PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md) · [discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161)
**Implements:** the "ACK-based flow control" terminal lever — the genuinely-open xterm.js recommendation (predictive echo is shelved; see the #1161 update).

---

## TL;DR

Under heavy terminal output (`cat` of a large file, build streaming, `yes`), the PTY produces bytes faster than the frontend can render them. Today there is **no backpressure**: the PTY read loop reads a chunk and immediately publishes it to the frontend as fast as it can, then reads again. The renderer and the keystroke loop share the same main-thread budget, so a flood **starves keystrokes** (SPEC §5.1).

Fix: **ACK-based backpressure applied at the PTY read**. The frontend acks how many bytes it has consumed; the backend tracks outstanding (published-but-unacked) bytes and **pauses reading from the PTY master** when that exceeds a high watermark, resuming when an ack drops it below a low watermark. Pausing the PTY read lets the kernel PTY buffer fill, which blocks the child process's `write()` — natural OS backpressure, exactly the [xterm.js flow control](https://xtermjs.org/docs/guides/flowcontrol/) pattern, pushed one layer down to the producer.

**Hard invariant: the pause must be bounded and interruptible.** A client that never acks (disconnect, hang) must degrade to *today's* behavior, never to a frozen terminal or a wedged child process.

---

## 1. Current architecture (verified in `shell.rs`)

The output path is **push-based via the event broker**, not pull-from-store. Verified in `agentmux-srv/src/backend/blockcontroller/shell.rs` (read loop ~L770–796):

```rust
let mut buf = [0u8; PTY_READ_BUF_SIZE];          // 4 KiB
// Reader runs on a dedicated OS thread via spawn_blocking; the PTY
// master is a blocking fd, so a synchronous read loop here is correct
// (portable-pty's reader is not async).
loop {
    match reader.read(&mut buf) {
        Ok(0) => break,                          // EOF — shell exited
        Ok(n) => {
            let data = &buf[..n];
            // Inline delivery: bytes go straight into the block:file
            // append event (data64) via the broker — NO FileStore
            // write-through on the hot path (filestore = None).
            handle_append_block_file(&broker_read, &block_id_read, "term", data, None);
            // update last_pty_output instant …
        }
        Err(_) => break,                         // PTY closed
    }
}
```

```
PTY master ──(read loop, 4 KiB buf, spawn_blocking OS thread)──►
    handle_append_block_file(broker, block_id, "term", data, filestore = None)
        └──► broker publishes a `block:file` append event (bytes inline as data64)
                                                              │
frontend term-rpc ◄── WS `block:file` event ──────────────────┘
        └──► termwrap.handleNewFileSubjectData (decodes data64 @ termwrap.ts:449)
             ──► scheduleRafWrite ──► xterm.write()
```

Key consequences for the design:
- **The backend never blocks on the frontend.** It reads `n` bytes, publishes, and immediately reads again. So backpressure must be introduced *deliberately* in this loop.
- **Delivery is push, and the bytes ride inline** in the `block:file` append event (`data64`) — there is **no FileStore write-through on this hot path** (`filestore = None`). There is therefore **no "FileStore byte cap" safety net on live output**; today an over-producing PTY is simply published as fast as it can be read.
- The ack unit should be a **cumulative consumed byte offset** per block (monotonic), matching the streaming nature of the path.
- The read loop is a **`spawn_blocking` task on a tokio blocking-pool thread** (portable-pty's reader is synchronous). "Pausing" means parking that blocking thread — which must be coordinated with shutdown and must never pin the thread indefinitely (see §3).

---

## 2. Design

### 2.1 Accounting (backend, per block)
- `published: AtomicU64` — total bytes ever published by the read loop (sum of `n`).
- `acked: AtomicU64` — highest cumulative consumed offset the frontend has reported.
- `outstanding = published − acked`.

### 2.2 Pause / resume (in the read loop)
Before reading the next chunk:
- if `outstanding ≥ HIGH_WATERMARK` → **park** (e.g. a `Condvar`, or a `tokio::sync::Notify` the blocking thread blocks on) until **any** of:
  1. an ack lands and `outstanding ≤ LOW_WATERMARK` (hysteresis — avoids thrashing), **or**
  2. the shutdown flag is set (block close / `stop()`), **or**
  3. a **`RESUME_TIMEOUT`** elapses (see §3.2).
- otherwise read normally.

Parking the read loop stops draining the PTY master → kernel PTY ring fills → child `write()` blocks. No data is dropped; the producer is throttled at the source.

### 2.3 Ack channel (frontend → backend)
- New WS message: `termack { blockId, consumedOffset }` where `consumedOffset` is the cumulative byte count the frontend has consumed (decoded + handed to `xterm.write()`).
- Frontend sends one after consuming each `ACK_GRANULARITY` bytes (coalesced — never one ack per append event).
- Backend handler: `acked = max(acked, consumedOffset)`; if it crossed below `LOW_WATERMARK`, wake the read loop.
- Acks are **cumulative + monotonic**, so a lost ack self-heals on the next, and reordered acks are handled by `max`.

### 2.4 Parameters (initial; tune from bench)
| Knob | Initial | Meaning |
|---|---|---|
| `ACK_GRANULARITY` | 256 KiB | frontend acks every N consumed bytes |
| `HIGH_WATERMARK` | 1 MiB | pause reading above this outstanding |
| `LOW_WATERMARK` | 256 KiB | resume reading below this (hysteresis) |
| `RESUME_TIMEOUT` | 2 s | max time the read loop will stay parked waiting for acks |

---

## 3. The never-block contract (deadlock avoidance — the part most likely to break)

1. **Block close / `stop()` must wake a parked read loop.** Set a shutdown flag and wake the park primitive so the loop observes it and exits. Dropping the PTY master unblocks `reader.read`, but a loop *parked before the read* isn't in `read` — it must be woken explicitly.
2. **Frontend disconnect while paused → must not hang forever.** `RESUME_TIMEOUT` is mandatory: on timeout the loop resumes reading. A dead client degrades to today's behavior, never to a frozen pane or a stuck child.
3. **Don't pin the tokio blocking pool.** The loop runs on a `spawn_blocking` thread; an unbounded park holds that pool slot. The `RESUME_TIMEOUT` bound plus shutdown-wake keep the slot from being held forever. Take the count/notify primitives as their own lock; never hold an unrelated controller lock across the park.
4. **Reconnect / session resume.** On reconnect the frontend resubscribes to the `block:file` stream; set `acked` to the frontend's resumed consumed position so `outstanding` is recomputed correctly (don't leave a stale low `acked` that instantly re-pauses).
5. **Agent panes / non-interactive blocks.** Same path; acks come from whoever is consuming. If nothing consumes (headless), `RESUME_TIMEOUT` keeps it flowing.

---

## 4. Touch points (confirmed against the tree)

- **`agentmux-srv/src/backend/blockcontroller/shell.rs`** — read loop (~L770): add the pause check + `published` counter; `stop()`: wake on shutdown.
- **`agentmux-srv/src/server/websocket.rs`** — WS command dispatch: parse `termack`, update `acked`, wake the loop. (There is no `ws/` directory; term WS handling lives here.)
- **`frontend/app/view/term/term-rpc.tsx` / `termwrap.ts`** — accumulate consumed bytes (the `data64` decode point is `termwrap.handleNewFileSubjectData`, ~L449) and send `termack` per `ACK_GRANULARITY`.
- **`handle_append_block_file`** — the inline-delivery entry point the read loop calls; the `published` counter increments alongside it.

---

## 5. Validation

- Extend the load path of the input-latency bench (#1176) / `bench-term-echo.mjs --stream --busy` with a generator: sustained (`yes`), bursty (large `cat` ×N), realistic (`task build:backend` ANSI stream).
- **Gate:** P95 keystroke echo **< 50 ms** under sustained output (SPEC §2), with **no** ordering violations / dropped output.
- **Positive check:** with the pane hidden / frontend not consuming, the child's output rate visibly throttles (proves backpressure reached the producer), and resumes promptly when visible.
- **Safety checks:** killing the frontend mid-flood does not wedge the child or hang `stop()`; closing the block while paused tears down cleanly within `RESUME_TIMEOUT`.

---

## 6. Rollout

Profiling-gated per PLAN item 7: first confirm the problem is real on the bench (P95 echo > 100 ms under load) — if it already stays < 50 ms, the watermarks can ship generous (pure safety net). Land behind a setting (`term:flow_control`, default on once bench-validated). Implementation likely splits backend (accounting + pause/resume + ack handler) from frontend (consumed accounting + ack send).

---

## 7. Why this and not predictive echo

Predictive (mosh-style) local echo is **shelved** (#1161): it can't see the PTY echo mode (would flash plaintext over password prompts), our PTYs are local (the ≤512 B fast path already paints echo same-frame), and it's most fragile under exactly the heavy-output regime this spec governs. Flow control protects the keystroke frame *by construction* (throttle the producer) instead of racing it. Revisit prediction only with remote PTYs + a sidecar-signaled tty/echo mode + RTT telemetry.

---

## Revision note

v2 (2026-05-30): corrected §1 against the real code after reagent review of PR #1197. The original draft wrongly described the read loop as `std::thread::spawn` with a 64 KiB buffer writing to a pull-based FileStore (`append_output` + `publish_data_event`). The actual loop is a `spawn_blocking` task with a `PTY_READ_BUF_SIZE` (4 KiB) buffer doing inline broker delivery via `handle_append_block_file(..., filestore = None)`; bytes ride in the `block:file` append event, not a FileStore the frontend polls. Accounting, reconnect, deadlock, and touch-point sections updated to match; `ws/term.rs` corrected to `server/websocket.rs`.
