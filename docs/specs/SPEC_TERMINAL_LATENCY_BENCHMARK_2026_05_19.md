# SPEC: Terminal Input Echo-Latency Benchmark

**Status:** Implemented
**Date:** 2026-05-19
**Owner:** AgentY
**Tracking:** PR #926 (writeInFlight fix), `tools/tests/bench-term-echo.mjs`

---

## 1. Purpose

This spec documents the methodology, infrastructure, and tooling for
programmatically benchmarking **terminal input echo latency** — the
wall-clock interval from when a keystroke is sent into a PTY until its
echo appears in the xterm.js viewport.

The benchmark was written to:

1. Prove the `writeInFlight` fix in `termwrap.ts` (PR #926) actually
   reduces P95 echo latency before/after.
2. Give agents and CI a repeatable, non-visual, agent-driveable tool for
   catching future regressions on the terminal render path.
3. Complement CDP-based scroll FPS / heap baselines with a PTY-path-specific
   instrument (`scripts/bench-cdp.mjs` was removed in the scripts cleanup).

---

## 2. What exists

### 2.1 The fix — `termwrap.ts:scheduleRafWrite()`

`frontend/app/view/term/termwrap.ts` schedules xterm.js writes two ways:

- **Fast path** (`data.length ≤ 512 B, rafBuffer empty`): calls
  `terminal.write()` immediately, bypassing the RAF coalescing timer.
  Used for single-keystroke echoes where every millisecond counts.
- **RAF path**: accumulates PTY data into `rafBuffer` and flushes once
  per animation frame. Prevents viewport flicker from Ink's cursor-up
  sequences.

**Root cause of jitter (fixed in PR #926):** the fast-path condition
previously also required `!this.writeInFlight`:

```ts
// BEFORE (sporadic jitter):
if (data.length <= RAF_BYPASS_THRESHOLD
    && this.rafBuffer.length === 0
    && !this.writeInFlight) {   // <─ THIS was the problem

// AFTER (always fast for small data):
if (data.length <= RAF_BYPASS_THRESHOLD
    && this.rafBuffer.length === 0) {
```

When any large PTY batch was in-flight (cursor redraws, command output),
`writeInFlight` was `true`, so keystroke echoes fell through to the RAF
path and waited up to one frame (~16 ms) to render. xterm.js serialises
all `terminal.write()` calls internally, so the guard was unnecessary.

### 2.2 Perf marks (production-safe)

Three `performance.mark()` spans were added to `termwrap.ts`:

| Mark pair | Span | What it measures |
|---|---|---|
| `term-keypress` | `handleTermData()` entry → `sendDataHandler?.()` return | WS send overhead |
| `term-echo-render` | first WS echo received → `terminal.write()` callback | Echo render path |
| `term-raf-write` | `armRaf()` flush start → `terminal.write()` callback | RAF batch cost |

Visible in CEF DevTools Performance tab (Timings row) and the dev-mode
Perf HUD (`Ctrl+Shift+P`). Cost: ~50 ns each. See
`SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md` §A1.

### 2.3 Agent App API (programmatic access)

The backend WebSocket (`ws://<ws_endpoint>/ws`) exposes three operations
used by the benchmark:

#### `pane.open` — create a terminal pane

```json
{
  "wscommand": "rpc",
  "message": {
    "command": "pane.open",
    "reqid": "<uuid>",
    "data": { "view": "term" }
  }
}
```

Response:
```json
{ "reqid": "<same>", "data": { "block_id": "...", "tab_id": "...", "view": "term", "created": true } }
```

#### `eventsub` — subscribe to PTY output events

```json
{
  "wscommand": "rpc",
  "message": {
    "command": "eventsub",
    "reqid": "<uuid>",
    "data": {
      "event": "blockfile",
      "scopes": ["block:<block_id>"],
      "allscopes": false
    }
  }
}
```

PTY output arrives as `eventrecv` messages:
```json
{
  "wscommand": "eventrecv",
  "event": "blockfile",
  "scopes": ["block:<block_id>"],
  "data": {
    "fileop": "append",
    "data64": "<base64-encoded PTY bytes>"
  }
}
```

#### `blockinput` — send keystrokes to PTY

```json
{
  "wscommand": "blockinput",
  "blockid": "<block_id>",
  "inputdata64": "<base64-encoded string>"
}
```

### 2.4 Auth — `authkey.dev`

Dev instances (`task dev`) write `~/.agentmux/dev/<branch>/data/authkey.dev`:

```json
{
  "version": 1,
  "auth_key": "<uuid>",
  "web_endpoint": "127.0.0.1:<port>",
  "ws_endpoint": "127.0.0.1:<port>",
  ...
}
```

WS connection: `ws://<ws_endpoint>/ws?authkey=<auth_key>`
(query-param auth is only permitted on `/ws` — see `server/mod.rs` auth middleware).

Full format documented in `SPEC_TEST_API_ACCESS.md` §5.

### 2.5 Existing benchmark infrastructure

| File | Purpose |
|---|---|
| ~~`scripts/bench-cdp.mjs`~~ | *(removed — scripts cleanup 2026-06-18)* |
| `scripts/benchmarks/` | Startup time + memory + bundle size |
| `tools/tests/authfile.ps1` | PowerShell helper to read `authkey.dev` |
| `tools/tests/pane-focus-stress.ps1` | App API harness via service endpoint |

---

## 3. Best practices for terminal echo-latency benchmarking

### 3.1 Sentinel echo pattern

Never try to detect echoes by scanning for individual characters —
partial writes, line buffering, and concurrent output will cause false
matches. Instead use a unique **sentinel string** per sample:

```
echo __BENCH_42__\r
```

Wait for `__BENCH_42__` to appear in PTY output. The exact sequence is
guaranteed to appear as a coherent unit once the shell processes the
command (shell echo is before execution; `echo` output is another event
but still unique-matchable with the `_N_` counter).

For *keypress* echo latency specifically (not command echo), use
terminal raw-mode bypass: pipe the PTY output and look for the echoed
character byte. The benchmark uses command-echo as a proxy because raw
keypress echo requires disabling PTY line discipline — acceptable for
production benchmarks.

### 3.2 Percentile statistics

Always capture **p50, p95, p99, max** across ≥ 50 samples. P50 tells
you the common-case; P95/P99 reveal the jitter the fix was designed to
eliminate. Discard the first 5 samples as warm-up (JIT, scheduler).

### 3.3 Before/after methodology

Run the benchmark against two instances:

1. **Baseline**: released portable (0.34.0) at `AGENTMUX_LOCAL_URL`.
2. **Fixed**: `task dev` instance with the PR branch applied.

Compare the same percentile columns. A valid "improvement" claim
requires P95 fixed < P95 baseline.

**Caveat**: the running 0.34.0 instance does not write `authkey.dev`
(release builds don't set `AGENTMUX_DEV=1`). Baseline benchmarks must
either use `--auth-key` + `--ws-url` flags (manual) or be run against
a second dev instance of the pre-fix commit.

### 3.4 Scenario types

| Scenario | Description | What it reveals |
|---|---|---|
| **Quiet** | Terminal at shell prompt, no concurrent output | Baseline echo path |
| **Busy** | Concurrent `seq 1 100000` running in background | RAF contention; `writeInFlight` effect |
| **Post-large-batch** | Immediately after a large `cat` output | In-flight drain interaction |

The busy scenario is the one most affected by the `writeInFlight` fix.

### 3.5 Isolation

- Run against the **dev instance** (`task dev`), not the production
  portable, because: (a) you can control the branch, (b) `authkey.dev`
  is available, (c) dev data dir is separate from `~/.agentmux`.
- The benchmark creates its own terminal pane via `pane.open` and cleans
  it up after the run — it never touches existing open panes.
- Do not saturate the PTY with massive output while measuring echo
  latency on a different pane — the event bus broadcasts to all
  subscribers.

---

## 4. Benchmark script — `tools/tests/bench-term-echo.mjs`

### 4.1 Capabilities

- Auto-discovers the dev instance from `authkey.dev` (walks
  `~/.agentmux/dev/*/data/` newest-first, validates `host_pid` alive).
- Creates a fresh terminal pane via `pane.open`, subscribes to PTY
  events via `eventsub`.
- Waits for the shell prompt before sending samples (avoids timing init).
- Sends `echo __BENCH_N__\r` for N = 0…(count−1), measures send-to-echo.
- Reports p50/p95/p99/max in a human-readable table.
- Saves raw JSON results to `--output-file` for before/after comparisons.
- Optionally runs a **busy** scenario: launches `seq 1 50000` in the
  background while measuring.
- Cleans up the pane when done (or on Ctrl+C).

### 4.2 CLI

```
node tools/tests/bench-term-echo.mjs [options]

Options:
  --ws-url <url>       WS endpoint (default: from authkey.dev)
  --auth-key <key>     Auth key   (default: from authkey.dev)
  --count <n>          Samples per scenario (default: 60)
  --warmup <n>         Samples to discard   (default: 5)
  --busy               Run busy-terminal scenario too
  --output-file <path> Save raw JSON results
  --help
```

### 4.3 Sample output

```
AgentMux terminal echo-latency benchmark
Instance: v0.34.0  WS: ws://127.0.0.1:57208/ws

=== Quiet terminal (55 samples after 5 warmup) ===
  p50:  4.2 ms   p95: 18.7 ms   p99: 22.1 ms   max: 24.3 ms

=== Busy terminal (55 samples after 5 warmup) ===
  p50:  4.8 ms   p95: 19.2 ms   p99: 23.4 ms   max: 31.0 ms

Results saved to bench-results-2026-05-19T12-00-00.json
```

### 4.4 Before/after interpretation

With the `writeInFlight` fix:
- **Quiet scenario**: P95 change < 5 ms (fix only helps when in-flight)
- **Busy scenario**: P95 expected to drop ~50–70 % (from ~30 ms → ~10 ms)

---

## 5. How to run

```bash
# Start the dev instance (isolated data dir, separate from running 0.34.0)
task dev

# In another terminal, run the benchmark against the dev instance
node tools/tests/bench-term-echo.mjs --busy --output-file before.json

# Apply the fix (PR #926 already merged to main)
git checkout main && git pull

# Restart dev, run again
task dev
node tools/tests/bench-term-echo.mjs --busy --output-file after.json

# Compare
node -e "
const b = require('./before.json'), a = require('./after.json');
const s = 'busy';
console.log('Quiet P95:', b.quiet.p95.toFixed(1), '->', a.quiet.p95.toFixed(1), 'ms');
console.log('Busy  P95:', b.busy.p95.toFixed(1), '->', a.busy.p95.toFixed(1), 'ms');
"
```

---

## 6. Extending to CI

The benchmark is designed to be CI-driveable. A future CI job can:

1. Start `task dev` in a headless sidecar (or use an existing fixture
   instance written by the CI `task dev` invocation).
2. Wait for `authkey.dev` to appear.
3. Run `node tools/tests/bench-term-echo.mjs --output-file ci-result.json`.
4. Assert P95 busy < 25 ms. Fail the job if regression.

This is left to a follow-up PR once baselines are established.

---

## 7. Cross-references

- `SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md` — overall perf strategy, INP targets
- `SPEC_TEST_API_ACCESS.md` — authkey.dev format, security model
- `tools/tests/README.md` — test harness documentation
- `tools/tests/authfile.ps1` — PowerShell equivalent helper
- ~~`scripts/bench-cdp.mjs`~~ — removed (scripts cleanup 2026-06-18)
- `frontend/app/view/term/termwrap.ts` — the fixed code
- PR #926 — writeInFlight fix + perf marks
