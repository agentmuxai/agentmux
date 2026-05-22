<!-- Captured from GitHub issue #950 — agentmuxai/agentmux -->

## Summary

Three interlocking problems found during the 2026-05-21 terminal input investigation that need to be addressed together:

1. **Terminal opening has no timeout, no error surface, no retry** — any backend delay causes a silent hang
2. **The echo-latency benchmark used internal RPC plumbing directly** — `controllerinput` + seq is the private keyboard wire format; agents must not touch it; flooding it with 220 concurrent fire-and-forget RPCs overflowed the 32-slot reorder buffer and permanently froze those terminals
3. **Agents need a real typing API** — `seq` is an internal mechanism; the agent app API must expose an opaque `term.type` command that delivers input through the same code path as keyboard input, with no seq exposure to callers

Full spec: `docs/specs/SPEC_BULLETPROOF_TERMINALS_2026_05_21.md`

---

## Root Cause: Benchmark Froze User Terminals

`measureStreamThroughput` in `bench-term-echo.mjs` fired 220 `controllerinput` RPCs simultaneously. Multiple Tokio tasks processed them in parallel → out-of-order arrivals → 32-slot reorder buffer overflowed → seqs permanently dropped → `input_seq_next` stuck → terminal frozen.

Evidence from sidecar log (v0.37.6 run):
```
WARN send_input: input reorder buffer full, dropping  block_id="07dfd2db" seq=967
WARN send_input: input reorder buffer full, dropping  block_id="07dfd2db" seq=969
WARN send_input: input reorder buffer full, dropping  block_id="07dfd2db" seq=983
```

After these drops, all input to that block was lost. Opening new terminals in the same session was also affected because `input_seq_next` state from the benchmark leaked into subsequent controller instances (fixed in PR #938 — `start()` now resets seq — but benchmark isolation is still needed).

---

## Proposed: `term.type` Agent App API

A single, stable command for agent-driven terminal input:

```typescript
// Request
{
  command: "term.type",
  data: {
    blockId: string,
    text: string,
    charDelay?: number  // ms between chars, for human-paced input
  }
}

// Response
{ success: true }
// OR
{ error: "block not found" | "controller not running" | ... }
```

**Implementation:** routes to `blockcontroller::send_input(block_id, input, seq=None)` — same as `blockinput` wscommand. No seq machinery involved. Serialized delivery. No reorder buffer. No race conditions.

**Why not `controllerinput` + seq?** That path exists for the TermViewModel keyboard flow: one awaited RPC per keystroke, ordering guarantees needed across WebSocket reordering. Agents sending strings have none of those properties and must not manage seq state. Using `controllerinput` from agent code is wrong in principle and broken in practice.

---

## Proposed: Terminal Opening Guarantees

| Guarantee | Description |
|---|---|
| **G1 — Open must timeout** | No PTY output within 5s → backend sends error event → frontend renders error state + retry button |
| **G2 — Open must be retriable** | `ControllerResync` with `forcerestart: true` kills existing PTY and opens fresh one, idempotent |
| **G3 — Seq resets on new session** | `start()` resets `input_seq_next` and `input_seq_buf` (already in PR #938, must not regress) |
| **G4 — Channel-full never deadlocks** | `try_send` on full channel drops + warns, never blocks mutex holder (already in PR #938) |
| **G5 — PTY spawn errors are surfaced** | Shell binary not found / conpty error → structured error event → frontend shows error, not empty pane |
| **G6 — `term.type` fails cleanly** | Returns `{ error: "controller not running" }` if PTY not started, never silently drops |

---

## Implementation Plan

### Phase 1 — Fix benchmark immediately (unblocks users)
- [ ] Rewrite `measureStreamThroughput` to use `blockinput` wscommand (`seq=None`) for stream characters
- [ ] Verify no "reorder buffer full" log entries after stream benchmark run
- [ ] Verify user terminals remain responsive after benchmark completes

### Phase 2 — Add `term.type` to agent app API
- [ ] Add `term.type` command handler in backend (routes to `send_input(..., None)`)
- [ ] Return structured error if controller not running
- [ ] Rewrite `bench-term-echo.mjs` to use `term.type` exclusively (no raw seq RPCs)
- [ ] Update `docs/specs/app-api-extension.md` with `term.type` documentation

### Phase 3 — Terminal opening reliability
- [ ] 5-second open timeout watchdog in `ShellController::start()`
- [ ] PTY spawn errors propagated as structured frontend events
- [ ] "Terminal failed to start" error state + retry button in `TermView`
- [ ] `ControllerResync` safe to call multiple times

### Phase 4 — Benchmark isolation
- [ ] Benchmark creates dedicated pane via `pane.open`, stores block ID
- [ ] On completion, benchmark calls `pane.close` to clean up
- [ ] No benchmark seq/PTY state survives into user terminal sessions

---

## Never-Fail Contract

A terminal pane satisfies the contract if:

1. Opening never hangs silently — prompt appears within 5s or error state shown
2. Input never silently dropped — channel-full and seq-buffer-full are logged and counted
3. Benchmark runs cannot freeze user terminals — dedicated blocks, cleaned up on exit
4. Session reconnects always work — `input_seq_next` resets on `start()`, session-reset heuristic handles frontend reconnects
5. Agent typing always works — `term.type` uses `seq=None`, bypasses all seq machinery, limited only by channel capacity (32 slots, drains in microseconds)

---

## Related

- PR #938 — seq session reset (fixes `input_seq_next` desync on session boundary, resets on `start()`)
- `tools/tests/bench-term-echo.mjs` — benchmark that needs rewriting
- `docs/specs/SPEC_BULLETPROOF_TERMINALS_2026_05_21.md` — full spec
- `docs/specs/SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md` — existing benchmark spec (to be updated)
