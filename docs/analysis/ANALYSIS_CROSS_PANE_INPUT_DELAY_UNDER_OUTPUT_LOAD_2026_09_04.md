# Analysis & Recommendations: Cross-Pane Input Delay Under Output Load

**Date:** 2026-09-04
**Status:** §5 item 1 (per-pane fair egress dequeue) implemented on
`agent2/fix-cross-pane-input-delay` — see `agentmux-srv/src/server/websocket.rs`'s
`fair_drain_priority`/`priority_pane_key`, unit-tested (TDD), and validated live
against a real running dev build: `tools/tests/bench-term-cross-pane.mjs` against
the patched instance showed pane B's echo latency flat (p50 ~9ms, p95 ~11-14ms)
while pane A sustained a genuine ~54 KB/s (~387 events/sec) flood — no cross-pane
regression. An unpatched-vs-patched side-by-side run was attempted on a second
isolated dev instance to get a directly-comparable "before" number but got stuck
behind heavy build contention from other concurrent agents on the shared build
machine and was abandoned; the live numbers above are from the patched build only.
Items 2-6 below remain unimplemented recommendations.
**Symptom (user-reported):** while one pane (terminal or agent) is actively producing output, typing into a *different* pane feels slow/delayed.

---

## 1. Summary

This is not one bug — it's the combination of two facts that are each individually correct engineering decisions, but compose badly:

1. **Backend egress is multiplexed per-*connection*, not per-*pane*.** Every pane's output shares one ordered FIFO channel and one physical WebSocket write per browser window. A noisy pane's flood of small frames queues ahead of a quiet pane's keystroke-echo frame.
2. **The frontend has exactly one JS main thread.** Any main-thread-blocking work triggered by pane A's data (an expensive render, a big parse, a synchronous DOM write) delays the `keydown` → send path for pane B, because both panes' event handling shares that one thread.

Backend PTY read/write and lock state **are already correctly isolated per pane** — that is not the bottleneck and should not be touched. The bottleneck is entirely on the *fan-out to the browser* and the *shared render thread*, both of which currently have per-connection or global granularity where the fix needs per-pane granularity.

This codebase has already hit and partially fixed adjacent instances of both failure classes (§4) — the fixes below extend the same pattern to the piece that's still missing.

---

## 2. What's already correctly isolated (don't break this)

Verified in `agentmux-srv/src/backend/blockcontroller/shell/`:

- **PTY reads are per-pane**, each a dedicated `spawn_blocking` OS thread (`lifecycle.rs:652-751`, `PTY_READ_BUF_SIZE = 4096` in `pty.rs:12`). Pane A's read never blocks pane B's read.
- **PTY input writes are per-pane**, each with its own **unbounded** mpsc channel (`lifecycle.rs:130-136,753-784`) — a keystroke for pane B reaches its PTY immediately regardless of pane A's volume.
- **Controller state lock is per-pane** (`ShellControllerInner` behind its own `Arc<Mutex<>>`, `controller.rs:46-95`), not a global lock across panes.
- **Incoming WS keystrokes are read with top priority** in a `biased!` select (`server/websocket.rs:142-166`) ahead of any outgoing forwarding, so keystroke *ingestion into the server* is never delayed — only the *echo round-trip back to the browser*.

Any fix should add fairness/isolation on top of this, not restructure it.

---

## 3. Root causes, ranked

### 3.1 (Dominant) One shared FIFO fans out every pane's output on a connection

`agentmux-srv/src/backend/eventbus.rs:104-120` — `EventBus::register_ws` creates **one `priority` mpsc channel per WebSocket connection** (capacity 8192, `eventbus.rs:47`), not one per pane. `EventBusBridge::send_event` (`eventbus.rs:288-316`) routes every non-telemetry event — which includes **all** terminal output/echo for **every pane** in that window — to this single `Lane::Priority` channel, keyed only by `route_id` (the connection), never by pane/block id.

`server/websocket.rs:204-208` (the `Some(event) = priority_rx.recv()` arm) drains this one channel one item at a time and does a **sequential, awaited** `socket.send(...).await` per item (`forward_event`, same file). If pane A is producing hundreds of chunks/sec, pane B's own keystroke-echo frame — generated the instant B's shell echoes the typed character — is appended to the *same* FIFO and must wait its turn behind every one of A's frames already queued, then wait for the single physical socket write to flush. Under sustained flood the channel can fill and B's frame is silently **dropped** (`eventbus.rs:232-243`, "ws egress lane full … dropping event"), not just delayed.

**This is the mechanism that directly matches the reported symptom** — a purely FIFO, connection-wide queue with no per-source fairness.

### 3.2 No coalescing of the priority lane (unlike the background lane)

`websocket.rs:236-243` explicitly coalesces the *background* (sysinfo/blockstats telemetry) lane to bound bursts (`coalesce_background`, `websocket.rs:321-341`) — but the priority lane has **no equivalent**. Every one of pane A's 4 KiB PTY-read chunks (`pty.rs:12`) produces its own broker publish and its own WS frame, maximizing how many frames pane B's echo must queue behind. This is the same fix pattern already applied to telemetry, just not yet applied to terminal output itself.

### 3.3 No PTY-side backpressure — the producer is unbounded

The read loop in `lifecycle.rs` reads a chunk and publishes it, then immediately reads again, with **no flow control**. This is a known, previously-scoped gap: `docs/specs/SPEC_TERMINAL_FLOW_CONTROL_2026_05_30.md` designs exactly this (ACK-based pause/resume of the PTY read once outstanding-unacked bytes cross a high watermark) but is explicitly **"Draft — design, pre-implementation."** Confirmed still unimplemented — no `termack`, `HIGH_WATERMARK`, or `LOW_WATERMARK` symbols exist anywhere in the current tree. `PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md` §Item 7 gated this work behind a profiling step ("if P95 keystroke echo under heavy load > 100 ms → implement") that was never run — there's no bench result under `docs/perf/`. In other words: the safety net this repo already designed for exactly this symptom was never built because nobody had reproduced/measured it yet. The user's report is that missing evidence.

### 3.4 General risk class: a blocking call inside an async task stalls the whole connection's egress

Not the current cause (already fixed for its one known instance), but the same class of bug that would reproduce this exact symptom if reintroduced. `agentmux-srv/src/backend/sysinfo.rs` used to call `sys.refresh_processes_specifics(...)` synchronously inside an async Tokio task; on a host with many child processes the `/proc` scan took 5-20ms and occupied a Tokio worker thread, which is exactly enough to starve the WS egress loop at 1 Hz (fixed in commit `0f34704a8`, "eliminate 1Hz typing jerk from sysinfo blocking Tokio workers," #1782 — wrapped in `tokio::task::block_in_place`, plus the priority/background lane split that #3.1/#3.2 above build on). **Any future periodic/global task added to the server (new telemetry, a new scan, a new broadcast) that does synchronous I/O or CPU work without `block_in_place`/`spawn_blocking` reintroduces this for every pane on the connection, not just its own.**

### 3.5 Frontend: shared main thread renders all panes

JS is single-threaded; a main-thread-blocking task triggered by pane A's data delays pane B's `keydown` handling regardless of how well-isolated the backend is. This codebase already found and partially fixed one instance:

`docs/analysis/ANALYSIS_AGENT_PANE_TYPING_LATENCY_2026_05_30.md` — a streaming agent response used to re-parse + re-syntax-highlight + rebuild the **entire** markdown document from scratch on every streamed frame (`frontend/app/element/markdown.tsx`'s `renderedMarkdown` memo), an O(n²) cost over a turn that produced escalating long-tasks (measured 52ms → 1754ms) and starved keystroke RAFs for *any* other input on the page. Confirmed fixed in current code: `frontend/app/view/agent/components/MarkdownBlock.tsx:30,62-78` now throttles commits to at most one per 90ms (`STREAM_RENDER_MS`) and skips syntax highlighting on intermediate commits, applying a full highlighted render only once the stream settles.

This fix **bounds frequency**, not total cost — a very long streaming message still does a full-document parse+highlight on each 90ms commit and once on settle, so a sufficiently large single message can still produce a multi-hundred-ms main-thread block, just less often than every frame. The rejected alternative (`§5-6` of that analysis, "per-block incremental render") would have bounded *cost* instead of *frequency* and remains the more complete fix if this resurfaces for very long messages.

The terminal pane (`frontend/app/view/term/termwrap.ts`) writes PTY output straight into `xterm.js` (`doTerminalWrite`, `termwrap.ts:600-656`) and relies on xterm's own internal `RenderDebouncer` (one paint per animation frame) as the sole coalescer — a prior double-RAF layer was deliberately removed (`SPEC_TERM_DOUBLE_RAF_TEAROUT_2026_05_30`). This is fine for paint *frequency*, but `terminal.write()` still does real parsing/buffer work proportional to bytes written on the calling thread; a sustained high-throughput producer (`cat` of a large file, a build log) can still consume main-thread time slices that a different pane's keystroke handler needs, independent of anything the backend does. This is the frontend-side counterpart to §3.3: no flow control exists to slow the producer down when the consumer (the render thread) can't keep up.

---

## 4. Why this wasn't caught earlier

The repo has strong, specific contracts for *within-surface* input latency (`SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md`: ≤16ms keystroke handlers, defer-what-can-wait, no layout reads on the input path) and has fixed several concrete regressions against those rules. All of that work is scoped to **one pane's own output not delaying its own input** (echo latency, textarea reflow). None of it is scoped to **pane A's output not delaying pane B's input** — a cross-pane fairness problem that doesn't show up in a single-pane benchmark (`tools/tests/bench-term-echo.mjs` drives one terminal, not two). That's a gap in bench coverage, not a gap in engineering care.

---

## 5. Recommended fixes, ranked by impact/effort

| # | Fix | Addresses | Effort | Notes |
|---|---|---|---|---|
| 1 | **Per-pane fairness on the priority lane**: round-robin or weighted-fair dequeue across pending panes instead of one FIFO, before the single `socket.send()`. E.g. key queued events by `block_id`/pane, and drain one-per-pane-per-tick instead of oldest-first-globally. | §3.1 (dominant cause) | Medium | Keep the single WS connection (no protocol change) — this is purely a backend scheduling change in `websocket.rs`'s drain loop / `eventbus.rs`'s queue structure. |
| 2 | **Coalesce consecutive same-pane output frames** the way `coalesce_background` already does for telemetry, before they hit the wire — collapse N queued chunks for the *same* pane into one WS frame when the lane is backed up. | §3.2 | Low-Medium | Mirrors existing, proven code (`websocket.rs:321-341`). Must preserve byte ordering within a pane. |
| 3 | **Implement the already-designed ACK-based PTY flow control** (`SPEC_TERMINAL_FLOW_CONTROL_2026_05_30.md`) — pause the PTY read loop for an over-producing pane once unacked bytes cross a high watermark. This throttles the producer at the OS level, which also naturally caps how much any one pane can contribute to the shared queue in §3.1. | §3.3 | Medium-High | The design and touch-points are already written; this report is the missing profiling evidence the plan asked for before greenlighting it. |
| 4 | **Codify the async-blocking rule** from the sysinfo incident as a lint/review checklist item: any new periodic or on-demand server-side task that does sync I/O, `/proc` scans, filesystem walks, or CPU-heavy work inside a `tokio::spawn`'d future must wrap it in `block_in_place`/`spawn_blocking`. | §3.4 (regression prevention) | Low | Process fix, not code — add to `CLAUDE.md` or a `docs/specs` contract like the terminal input-priority one, since it already bit this exact symptom once. |
| 5 | **Extend the bench harness to a 2-pane scenario**: one pane running a heavy producer (`yes`, large `cat`, streaming build output), a second pane's keystroke-echo latency measured concurrently. Add this as a CI gate alongside the existing single-pane `bench-term-echo.mjs`. | Prevents regression of all of the above | Low | This is the missing coverage identified in §4 — a single-pane bench cannot detect a cross-pane fairness bug by construction. |
| 6 | **(If §5's bench shows it's still needed) revisit the streaming-markdown fix** from "bound frequency" (current, 90ms throttle) to "bound cost" (the previously-designed but reverted per-block incremental render, §3.5) for very long single messages. | §3.5, residual | Medium | Only pursue if the 2-pane bench shows agent-pane streaming, not terminal output, is the practical trigger — don't re-attempt the rejected split-at-blank-lines approach; any retry needs to solve the list/paragraph-spacing breakage that got it rejected the first time. |

Items 1, 2, and 5 are the "do these now" set: they directly target the confirmed dominant cause (§3.1/§3.2), are backend-only, and item 5 gives objective before/after numbers for all the others. Item 3 is larger but has zero remaining design risk — the spec is already written and reviewed. Item 4 is process, not code, and costs nothing to adopt immediately.

---

## 6. Validation plan

1. Land item 5 (2-pane bench) first, run it against current `main` to get a baseline P95 keystroke-echo latency in pane B while pane A streams `yes`/a large `cat`/a build log. This confirms the symptom quantitatively and gives a number to beat.
2. Implement item 1 (per-pane fair dequeue) and re-run the same bench — expect P95 to drop close to the single-pane baseline regardless of pane A's load.
3. Implement item 2 (same-pane coalescing) and re-run — expect further reduction in frame count and tail latency under extreme floods.
4. Only pursue item 3 (PTY flow control) if items 1+2 don't bring P95 under the existing internal "snappy" target (`SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md` §2: P50 ≤ 25ms, P95 ≤ 50ms) — the spec's own rollout section already says to ship it "profiling-gated," and items 1+2 may be sufficient on their own.
5. Confirm no ordering violations (a pane's bytes must still arrive in-order) and no regression to the existing telemetry-starvation fix (§3.4) — background lane must still stay servable during a priority-lane flood, per the existing comment in `websocket.rs:212-234`.
