# SPEC: Keystroke Ordering Guarantee

**Date:** 2026-05-20  
**Status:** Open — pending fix  
**Observed on:** v0.34.0+ (all platforms)  
**Symptom:** Characters typed in order appear in the terminal out of order  
**Severity:** High — data-corrupting under sustained fast typing

---

## Summary

A structural race in the RPC dispatch layer allows consecutive `controllerinput`
messages to reach the per-block input channel in a different order than the user
typed them. The result is transposed, dropped, or duplicated characters in the
terminal — not a rendering glitch, but actual PTY input corruption.

The fix does not require any performance improvement. A sequence-number guard on
`CommandBlockInputData` and a per-block reorder buffer eliminates the race with
microsecond-level overhead.

---

## Keystroke Path (current)

```
keypress
  │
  ▼ (xterm.js onData — synchronous, one event per key)
handleTermData()  [termwrap.ts]
  │
  ▼ (synchronous)
sendDataToController()  [termViewModel.ts]
  │
  ▼ (synchronous, direct WS.send())
WS frame: { wscommand:"rpc", message:{ command:"controllerinput", ... } }
  │
  ▼ (TCP — guaranteed in-order delivery)
WS receive loop  [websocket.rs — single Tokio task]
  │
  ▼ engine.handle_message(rpc_msg)
tokio::spawn(async { engine.handle_request(msg).await })  ← ORDER LOST HERE
  │
  ▼ (Tokio thread pool — tasks may execute in any order)
handle_request → send_input → inner.lock() → try_send(input_tx)
  │
  ▼ (mpsc channel — FIFO *within* the order items were inserted)
input loop: input_rx.recv() → writer.write_all()  [PTY stdin — FIFO, correct]
```

---

## Root Cause

`engine.handle_message()` (engine.rs ~line 269) spawns a new Tokio task for every
incoming RPC command:

```rust
pub fn handle_message(self: &Arc<Self>, msg: RpcMessage) {
    let engine = self.clone();
    tokio::spawn(async move {
        engine.handle_request(msg).await;   // ← independent task per message
    });
}
```

The WS receive loop dispatches messages in TCP arrival order. But once spawned,
each task is scheduled independently by Tokio's work-stealing executor. On a
multi-core machine, five tasks spawned in order 1→2→3→4→5 may acquire the
`ShellController.inner` lock and call `try_send()` in order 1→3→2→5→4.

The per-block MPSC channel then delivers 1→3→2→5→4 to PTY stdin.

**No sequence numbers exist** in `RpcMessage`, `CommandBlockInputData`, or any
WS framing struct to detect or correct this reordering.

---

## Why It Appears During Long Sessions

Tokio's work-stealing becomes more aggressive under load (more tasks in flight,
more CPU contention). A fresh session with low memory and few background tasks
rarely triggers the race. A long session with many open panes, background agents,
and increased GC/scheduling pressure causes frequent out-of-order execution.

---

## Ordering Guarantees at Each Layer

| Layer | Ordered? | Why |
|-------|----------|-----|
| xterm.js `onData` | ✅ Yes | Synchronous JS event dispatch |
| Frontend `WS.send()` | ✅ Yes | Direct call, no queue |
| TCP delivery | ✅ Yes | TCP stream guarantee |
| WS receive loop | ✅ Yes | Single-task loop, messages processed one at a time |
| `tokio::spawn` dispatch | ❌ **No** | Independent tasks, work-stealing scheduler |
| `inner.lock()` + `try_send()` | ❌ **No** | Race between concurrent async tasks |
| MPSC channel → PTY stdin | ✅ Yes | Single consumer, FIFO within insertion order |

The ordering is lost between WS receive and the MPSC channel insertion.

---

## Proposed Fix — Sequence Numbers + Per-Block Reorder Buffer

### Why not just handle `controllerinput` synchronously?

Handling it inline in the WS receive loop (instead of spawning a task) would
preserve order, but the receive loop is shared by all RPC commands. A slow
`controllerinput` (e.g. waiting on a full MPSC channel) would stall all WS
processing for all blocks. The sequence-number approach is safer and gives
additional diagnostic value.

### Changes

#### 1. Add `seq` to `CommandBlockInputData` (`rpc_types.rs`)

```rust
pub struct CommandBlockInputData {
    pub blockid: String,
    pub inputdata64: String,
    pub signame: String,
    pub termsize: Option<serde_json::Value>,
    pub seq: Option<u64>,   // ← NEW: monotonically increasing per-block counter
}
```

Default `None` preserves backward compatibility — inputs with no seq are
accepted unconditionally (existing clients continue to work).

#### 2. Per-block sequence state in `ShellControllerInner`

```rust
pub struct ShellControllerInner {
    // ...existing fields...
    pub input_seq_next: u64,          // next expected sequence number
    pub input_seq_buf: BTreeMap<u64, BlockInputUnion>,  // reorder buffer
}
```

#### 3. Sequence enforcement in `send_input()` (`shell.rs`)

```rust
fn send_input(&self, input: BlockInputUnion, seq: Option<u64>) -> Result<(), String> {
    let mut inner = self.inner.lock().unwrap();
    let tx = inner.input_tx.as_ref()
        .ok_or_else(|| "controller is not running".to_string())?;

    match seq {
        None => {
            // No seq — legacy client, send immediately
            tx.try_send(input).map_err(|e| format!("send_input: {e}"))
        }
        Some(s) => {
            let next = inner.input_seq_next;
            if s == next {
                // In-order: send now, then drain any buffered successors
                tx.try_send(input).map_err(|e| format!("send_input: {e}"))?;
                inner.input_seq_next = next + 1;
                while let Some(buffered) = inner.input_seq_buf.remove(&inner.input_seq_next) {
                    tx.try_send(buffered).map_err(|e| format!("send_input drain: {e}"))?;
                    inner.input_seq_next += 1;
                }
                Ok(())
            } else if s > next {
                // Early arrival: buffer it
                inner.input_seq_buf.insert(s, input);
                Ok(())
            } else {
                // Duplicate (seq < next): discard
                tracing::warn!(block_id = %self.block_id, seq, next, "duplicate input seq, discarding");
                Ok(())
            }
        }
    }
}
```

#### 4. Frontend: increment and attach `seq` in `sendDataToController()`

```typescript
// termViewModel.ts
private inputSeq = 0;

sendDataToController(data: string) {
    const b64data = stringToBase64(data);
    RpcApi.ControllerInputCommand(TabRpcClient, {
        blockid: this.blockId,
        inputdata64: b64data,
        seq: this.inputSeq++,
    });
}
```

The counter is per-`TermViewModel` instance (= per terminal pane). It resets
when the pane is closed and a new one opened.

---

## Reorder Buffer Sizing & Safety

The buffer holds at most `N - 1` entries where `N` is the window of concurrent
Tokio tasks in flight for a single block. In practice this is 2–5 during fast
typing. A hard cap of **32 entries** is reasonable (matching `SHELL_INPUT_CH_SIZE`);
entries beyond the cap are dropped with a warning.

The buffer is bounded in memory: each entry is at most a few hundred bytes
(keystrokes are short). 32 entries × 512 bytes = 16 KB worst case per block.

---

## Diagnostic Additions

### 1. Log out-of-order arrival (INFO level)

```rust
tracing::info!(block_id = %self.block_id, arrived = s, expected = next,
    "out-of-order input: buffering");
```

This makes the bug visible in production logs without user impact.

### 2. Expose `input_reorder_count` in `BlockControllerRuntimeStatus`

```rust
pub input_reorder_count: u64,  // total out-of-order arrivals since start
```

Agents and the benchmark can query this to measure how often reordering occurs
under load — turning a silent corruption into a measurable metric.

### 3. Benchmark extension: `--seq-test` mode

Add a mode to `bench-term-echo.mjs` that sends a known character sequence
(`abcde...z` × N) and checks if the echo comes back in the same order. Any
transposition is reported as a reorder event. This gives a reproducible test
for the fix and a regression guard for CI.

---

## Before/After Verification

```bash
# Before fix: run seq-test under load
node tools/tests/bench-term-echo.mjs --seq-test --busy

# After fix: same command — reorder_count should be 0
node tools/tests/bench-term-echo.mjs --seq-test --busy
```

Expected output (post-fix):
```
=== Sequence integrity test (100 rounds, busy terminal) ===
  Reorder events: 0 / 100
  PASS
```

---

## Non-Goals

- **Not** changing the `tokio::spawn` model for all RPC commands — only
  `controllerinput` ordering matters for correctness; other commands are
  idempotent or order-insensitive.
- **Not** switching to a synchronous input path — sequence numbers give the
  same correctness guarantee without stalling the WS receive loop.
- **Not** fixing the RT info race (Root Cause 2 in SPEC_DEAD_TERMINAL_PANE) —
  that is a separate issue.

---

## Related

- `SPEC_DEAD_TERMINAL_PANE_2026_05_20.md` — PTY spawn failure (different issue)
- `SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md` — benchmark infrastructure
- `agentmux-srv/src/backend/rpc/engine.rs` — `handle_message()` spawn site
- `agentmux-srv/src/backend/blockcontroller/shell.rs` — `send_input()`, input loop
- `frontend/app/view/term/termViewModel.ts` — `sendDataToController()`
