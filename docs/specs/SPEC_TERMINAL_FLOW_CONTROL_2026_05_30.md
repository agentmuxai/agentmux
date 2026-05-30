# SPEC: Terminal flow control (PTY backpressure)

**Status:** Draft — design, pre-implementation
**Date:** 2026-05-30
**Author:** AgentY
**Tracks:** [`SPEC_INPUT_RESPONSIVENESS_*` §5.1](./SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) · [`PLAN_INPUT_RESPONSIVENESS_EXECUTION_*` item 7](./PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md) · [discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161)
**Implements:** the "ACK-based flow control" terminal lever — the genuinely-open xterm.js recommendation (predictive echo is shelved; see the #1161 update).

---

## TL;DR

Under heavy terminal output (`cat` of a large file, build streaming, `yes`), the PTY produces bytes faster than the frontend can render them. Today there is **no backpressure**: the read loop appends to the scrollback FileStore and fires "data available" events as fast as it can read, and the only safety net is the FileStore byte cap. The renderer and the keystroke loop share the same main-thread budget, so a flood **starves keystrokes** (SPEC §5.1).

Fix: **ACK-based backpressure applied at the PTY read**, adapted to AgentMux's pull-based delivery model. The frontend acks how many bytes it has consumed; the backend tracks outstanding (appended-but-unacked) bytes and **pauses reading from the PTY master** when that exceeds a high watermark, resuming when an ack drops it below a low watermark. Pausing the PTY read lets the kernel PTY buffer fill, which blocks the child process's `write()` — natural OS backpressure, exactly the [xterm.js flow control](https://xtermjs.org/docs/guides/flowcontrol/) pattern, pushed one layer down to the producer.

**Hard invariant: the pause must be bounded and interruptible.** A client that never acks (disconnect, hang) must degrade to *today's* behavior (cap-bounded output), never to a frozen terminal or a wedged child process.

---

## 1. Current architecture (verified in `shell.rs`)

The output path is **pull-based**, not push:

```
PTY master ──(read loop, 64 KiB buf)──► append_output(FileStore)      // scrollback, byte-capped
                                    └──► publish_data_event(WPS, n)    // "n bytes available"
                                                                  │
frontend term-rpc ◄── WPS event ──────────────────────────────────┘
        └──► pulls bytes from FileStore API ──► termwrap ──► xterm.write()
```

Confirmed in `agentmux-srv/src/backend/blockcontroller/shell.rs` (PTY read loop, ~L456–487):

```rust
let read_handle = std::thread::spawn(move || {
    let mut buf = [0u8; 65536];                 // 64 KiB read buffer
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,                      // EOF
            Ok(n) => {
                let data = &buf[..n];
                Self::append_output(&store_for_read, &block_id, data);   // FileStore
                Self::publish_data_event(&wps_for_read, &block_id, n);   // WPS event → frontend pulls
                // last_pty_output = now
            }
            Err(_) => break,
        }
    }
    Self::handle_read_loop_exit(...);
});
```

Key consequences for the design:
- The backend **never blocks on the frontend** — it appends + notifies and immediately reads again. So backpressure must be introduced *deliberately* in this loop.
- Delivery is **pull**, so the natural ack unit is a **cumulative consumed byte offset** into the per-block stream, not per-message acks.
- The FileStore **byte cap** is the existing (and remaining) ultimate floor; flow control engages *before* the cap so we get backpressure instead of silent truncation.
- The read loop is a dedicated **`std::thread`** (portable-pty's reader is synchronous), so pausing means parking that thread — must be coordinated with `stop()`/join and must never hold the controller's inner lock while parked.

---

## 2. Design

### 2.1 Accounting (backend, per block)
- `appended: AtomicU64` — total bytes ever appended by the read loop.
- `acked: AtomicU64` — highest cumulative consumed offset the frontend has reported.
- `outstanding = appended − acked`.

### 2.2 Pause / resume (in the read loop)
Before reading the next chunk:
- if `outstanding ≥ HIGH_WATERMARK` → **park** on a `Condvar` (or `tokio::Notify` bridged to the sync thread) until **any** of:
  1. an ack lands and `outstanding ≤ LOW_WATERMARK` (hysteresis — avoids thrashing), **or**
  2. the shutdown flag is set (block close / `stop()`), **or**
  3. a **`RESUME_TIMEOUT`** elapses (see §3.2).
- otherwise read normally.

Parking the read loop stops draining the PTY master → kernel PTY ring fills → child `write()` blocks. No data is dropped; the producer is throttled at the source.

### 2.3 Ack channel (frontend → backend)
- New WS message: `termack { blockId, consumedOffset }` where `consumedOffset` is the cumulative byte count the frontend has pulled-and-written.
- Frontend sends one after consuming each `ACK_GRANULARITY` bytes (coalesced — never one ack per WPS event).
- Backend handler: `acked = max(acked, consumedOffset)`; if it crossed below `LOW_WATERMARK`, notify the read loop.
- Acks are **cumulative + monotonic**, so a lost ack self-heals on the next, and reordered acks are handled by `max`.

### 2.4 Parameters (initial; tune from bench)
| Knob | Initial | Meaning |
|---|---|---|
| `ACK_GRANULARITY` | 256 KiB | frontend acks every N consumed bytes |
| `HIGH_WATERMARK` | 1 MiB | pause reading above this outstanding |
| `LOW_WATERMARK` | 256 KiB | resume reading below this (hysteresis) |
| `RESUME_TIMEOUT` | 2 s | max time the read loop will stay parked waiting for acks |

Watermarks are bytes of *outstanding*, comfortably under the FileStore cap.

---

## 3. The never-block contract (deadlock avoidance — the part most likely to break)

1. **Block close / `stop()` must wake a parked read loop.** Set a shutdown flag and `notify`; the park predicate checks it so `read_handle.join()` can't hang. (Also: dropping the PTY master already unblocks `reader.read`, but a *parked* loop isn't in `read` — it's in the park; it must be woken explicitly.)
2. **Frontend disconnect while paused → must not hang forever.** `RESUME_TIMEOUT` is mandatory: on timeout the loop resumes reading and falls back to cap-bounded behavior. A dead client degrades to today, never to a frozen pane or a stuck child.
3. **Never park while holding the controller inner `Mutex`.** Take the count/notify primitives as their own lock; release the inner lock before parking. (Lock-ordering audit required at impl.)
4. **Reconnect / session resume.** The frontend re-syncs from the FileStore on reconnect (existing mechanism). On resume, set `acked` to the frontend's resumed read position so outstanding is recomputed correctly (don't leave a stale low `acked` that instantly re-pauses).
5. **Agent panes / non-interactive blocks.** Same path; acks still come from whoever is pulling. If nothing pulls (headless), `RESUME_TIMEOUT` keeps it flowing.

---

## 4. Touch points (confirm exact lines at implementation)

- **`agentmux-srv/src/backend/blockcontroller/shell.rs`** — read loop: add the pause check, the `appended` counter increment, the park primitive; `stop()`: wake on shutdown.
- **WS term handler** (PLAN cites `agentmux-srv/src/ws/term.rs` — verify) — parse `termack`, update `acked`, notify.
- **`frontend/app/view/term/term-rpc.tsx`** — after pulling+writing, accumulate consumed bytes; send `termack` per `ACK_GRANULARITY`.
- **`frontend/app/view/term/termwrap.ts`** — only if "consumed" is best measured at the xterm `write()` callback rather than at pull time.
- **FileStore API** — need a stable cumulative offset for "appended" and "consumed"; reuse the existing scrollback offset if present.

> These are scoped from the verified read-loop architecture; exact signatures/line numbers to be pinned when implementing (the editor tooling was returning padded reads during this design pass, so line-level edits were deferred to a clean session).

---

## 5. Validation

- Extend the load path of the input-latency bench (#1176) with a generator: sustained (`yes`), bursty (large `cat` ×N), realistic (`task build:backend` ANSI stream).
- **Gate:** P95 keystroke echo **< 50 ms** under sustained output (SPEC §2), with **no** "reorder buffer full"/drop warnings.
- **Positive check:** with the pane hidden / frontend not pulling, the child's output rate visibly throttles (proves backpressure reached the producer), and resumes promptly when visible.
- **Safety checks:** killing the frontend mid-flood does not wedge the child or hang `stop()`; closing the block while paused joins cleanly within `RESUME_TIMEOUT`.

---

## 6. Rollout

Profiling-gated per PLAN item 7: first confirm the problem is real on the bench (P95 echo > 100 ms under load) — if it already stays < 50 ms, the watermarks can ship generous (pure safety net). Land behind a setting (`term:flow_control`, default on once bench-validated). Implementation likely splits backend (accounting + pause/resume + ack handler) from frontend (consumed accounting + ack send).

---

## 7. Why this and not predictive echo

Predictive (mosh-style) local echo is **shelved** (#1161): it can't see the PTY echo mode (would flash plaintext over password prompts), our PTYs are local (the ≤512 B fast path already paints echo same-frame), and it's most fragile under exactly the heavy-output regime this spec governs. Flow control protects the keystroke frame *by construction* (throttle the producer) instead of racing it. Revisit prediction only with remote PTYs + a sidecar-signaled tty/echo mode + RTT telemetry.
