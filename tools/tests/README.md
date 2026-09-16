# Tools/tests

Ad-hoc test harnesses for behaviour that's hard to unit-test.

All scripts here target a running **dev** agentmux-cef instance and
discover it via `authkey.dev`, the file that debug builds write to
their data dir at startup. See
[`docs/specs/SPEC_TEST_API_ACCESS.md`](../../docs/specs/SPEC_TEST_API_ACCESS.md)
§5–§6 for the file format and security model. Release builds do not
write the file, so these scripts only work against `task dev`.

## `authfile.ps1`

Helper module sourced by the other scripts. Exports:

- `Get-AgentMuxAuthFile` — finds the newest `authkey.dev` under
  `%APPDATA%\ai.agentmux.cef.*`, validates the recorded `host_pid`
  is alive, and returns the parsed JSON as a PSCustomObject.
- `Invoke-AgentMuxService` — POSTs to `/agentmux/service` with the
  auth key. Returns the response `data` field, throws on error.
- `Get-AgentMuxHostLogPath` — resolves the host log file for the
  instance described by an authfile object.

```powershell
. .\tools\tests\authfile.ps1
$auth = Get-AgentMuxAuthFile
$client = Invoke-AgentMuxService -Auth $auth -Service client -Method GetClientData
```

## `bench-term-echo.mjs`

Node.js benchmark for **terminal input echo latency** — the interval from
sending a command into a PTY via the App API until its echo appears in
the PTY output stream. Uses the sentinel-echo pattern to avoid false
matches from concurrent output.

```bash
# Basic run (quiet terminal, 60 samples)
node tools/tests/bench-term-echo.mjs

# With busy-terminal scenario and results file
node tools/tests/bench-term-echo.mjs --busy --output-file results.json

# Manual target (production instance)
node tools/tests/bench-term-echo.mjs --ws-url ws://127.0.0.1:PORT/ws --auth-key KEY
```

Reports p50/p95/p99/max in ms. Save `--output-file` before and after a
fix to compare distributions. Full spec at
[`docs/specs/SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md`](../../docs/specs/SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md).

## `bench-agent-keystroke.mjs`

Node.js benchmark for **agent composer keystroke latency** — the
synchronous cost of the `<textarea>` `onInput` handler plus the
RAF-coalesced scroll callback. Counterpart to `bench-term-echo.mjs`,
but for the agent pane.

CDP-driven (not App-API-driven) — connects to CEF's debug port
(9223 dev / 9222 release), dispatches synthetic keystrokes via
`Input.dispatchKeyEvent`, and reads `agent-keystroke:*` and
`agent-input-raf-cb:*` perf measures via `performance.getEntriesByType`.

**Prereq:** an agent pane must be open before running (the bench finds
the textarea via `document.querySelector('.agent-input')`). The bench
does NOT submit and clears the textarea on exit — zero token cost.

```bash
# Basic run (200 keystrokes, 50 ms apart)
node tools/tests/bench-agent-keystroke.mjs

# Custom thresholds + results file
node tools/tests/bench-agent-keystroke.mjs --count 500 --p95-threshold-ms 30 --output-file results.json

# Different CDP port (release build)
node tools/tests/bench-agent-keystroke.mjs --cdp-port 9222
```

Reports p50/p95/p99/max for `agent-keystroke` (handler cost) and
`agent-input-raf-cb` (RAF callback cost). Exits non-zero if keystroke
P95 exceeds `--p95-threshold-ms` (default 50 ms — the spec's "snappy"
internal target). Limitation: dispatches DOM-level events; does not
exercise the OS keyboard pipeline.

Spec: [`docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md`](../../docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) §7.2.

## `pane-load.mjs`

On-demand PTY output load, for reproducing **cross-pane input lag by hand**.
Run it in one pane, type in another while it floods, and see whether your
keystrokes stutter. It measures nothing about the other pane — your fingers
are the instrument — it just produces load heavy and varied enough to provoke
the symptom, for a window long enough to type in.

```bash
node tools/tests/pane-load.mjs                       # mixed, 10s
node tools/tests/pane-load.mjs --mode paint --secs 15
node tools/tests/pane-load.mjs --mode spinner --throttle-ms 1
node tools/tests/pane-load.mjs --help
```

**Recipe:** open two panes → run this in A → move your cursor to B during the
countdown → type continuously for the whole flood → note stutter, dropped
characters, or lag between keypress and echo.

**The mode that reproduces is the finding, not a detail.** Each leans on a
different mechanism, so narrowing tells you where to look:

| mode | shape | implicates |
|---|---|---|
| `text` | bulk bytes, moderate writes | WS egress, FileStore write-through, xterm raw parse |
| `paint` | bulk escape sequences | xterm parser + renderer; real repaint work per frame |
| `spinner` | many tiny writes | per-write overhead: WS frames, event dispatch, `PTY_COALESCE_WINDOW` |
| `mixed` | rotates all three | default — find out *if* it reproduces, then narrow |

**Why not just `yes`:** `bench-term-cross-pane.mjs` floods with `yes` — a
stream of `y
`, maximal bytes and near-zero terminal work, which xterm parses
almost for free. Output that actually makes a UI feel bad is escape-sequence
heavy (cursor jumps, colour churn, in-place redraws). A repro built on `yes`
can be perfectly green while the thing users complain about is untouched.

**Two defaults worth knowing**, both there because the naive version is a
footgun:

- **`--max-mb 64`** is a safety valve, not a tuning knob. A pane's terminal
  output is written through to the FileStore for scrollback persistence, so an
  unbounded flood is an unbounded write to your disk.
- **Auto-pacing** spreads that budget across the requested duration
  (64 MB / 10 s ≈ 6.4 MB/s, sustained). Without it these modes hit the cap in
  a fraction of a second and the typing window disappears before you can move
  your hands. `--throttle-ms` or `--unpaced` opts out.

**Scope — this is the xterm path only.** It loads a *terminal* surface: a
Terminal pane, or an agent pane's shell drawer. The agent pane's own
conversation rendering (markdown/SolidJS, `MarkdownBlock`/`ChunkList`) is a
different code path that this cannot touch. To load *that*, have an agent emit
a large amount of text — e.g. ask it to `cat` a big file so the output streams
into the conversation. Less controllable, but it is the only way to exercise
that renderer, and the original cross-pane report was about an *agent* pane
producing output — so ruling one in does not rule the other out.

## `term-keyrepeat-hiccups.mjs`

CDP-driven diagnostic for the **"not silky" stutter felt while HOLDING a key**
in a terminal pane (sustained key-repeat) — a distinct failure mode from
single-keystroke echo latency (`bench-term-echo.mjs`) and the agent composer
(`bench-agent-keystroke.mjs`).

Attaches an in-page monitor to **every** shell window over CDP (9223 dev /
9222 release) and, while you hold a key, records per window:

- **rAF frame intervals** → dropped/janky frames (>20 / >33 / >50 / >100 ms)
- **`longtask`** entries → main-thread JS blocking ≥ 50 ms
- **perf measures** → `term-keypress` / `term-echo-render` / `term-raf-write`
- **focus + active-element sampling** and an **xterm-rows `MutationObserver`** —
  these *prove* the terminal was focused and actually echoing in the measured
  window, so a run is marked **VALID** only when the keystrokes truly landed (and
  rendered) there. A mis-focused capture can't masquerade as data.

It auto-selects the active terminal window and classifies the hiccup as
**main-thread JS** (schedulable/fixable), **xterm write cost**, or
**compositor/GPU stall** (render-path, not scheduler).

```bash
# focus a terminal pane, run this, then HOLD a key for the whole countdown
node tools/tests/term-keyrepeat-hiccups.mjs                       # 18 s, port 9223
node tools/tests/term-keyrepeat-hiccups.mjs --secs 30 --json out.json
node tools/tests/term-keyrepeat-hiccups.mjs --cdp-port 9222       # release build
```

Limitation: `longtask` only fires for blocks ≥ 50 ms, so a 17–49 ms main-thread
task can drop a frame without being counted. For definitive frame-by-frame
attribution, follow up with a CDP `Tracing` capture.

Spec: [`docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md`](../../docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) · Umbrella: [discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161).

## `pane-focus-smoke.ps1`

Minimum-viable harness sanity check: reads the auth file and calls
`client.GetClientData`. Use this to confirm the harness↔backend path
is wired correctly before debugging the longer stress test.

```powershell
pwsh tools/tests/pane-focus-smoke.ps1
```

Exit 0 = ready to run pane-focus-stress.ps1. Exit 1 = no dev
instance, stale auth file, or auth/route mismatch.

## `three-pane-layout.ps1`

Helper module (dot-source only — not a standalone script). Exports:

- `New-AgentMuxTestTab` — creates a fresh tab in the active
  workspace via `workspace.CreateTab` and returns `{tabid,
  workspaceid}`.
- `New-AgentMuxThreePaneLayout` — calls `object.CreateBlock` three
  times (browser, terminal, browser) targeting the test tab via
  `uicontext.activetabid`, then pushes three `pendingbackendactions`
  (`insert` + two `splithorizontal`) onto the tab's LayoutState via
  `object.UpdateObject`. The frontend's `LayoutModel` drains those
  actions and reduces them client-side (see
  [`frontend/layout/lib/layoutPersistence.ts`](../../frontend/layout/lib/layoutPersistence.ts)).
- `Remove-AgentMuxTestTab` — closes the test tab via
  `workspace.CloseTab`. Safe to call on an already-closed tab.

## `layout-smoke.ps1`

Proves the three-pane helper works end-to-end. Creates the layout,
asserts the tab has three blocks referenced and a `rootnode` on the
LayoutState, then cleans up. Use after `pane-focus-smoke.ps1` passes
and before touching `-CreateLayout`.

```powershell
pwsh tools/tests/layout-smoke.ps1             # create + verify + cleanup
pwsh tools/tests/layout-smoke.ps1 -KeepTab    # leave the tab around for inspection
```

## `pane-focus-stress.ps1`

Drives the 4-round pane-focus stress workload from
[`docs/specs/SPEC_PANE_FOCUS_STRESS_TEST.md`](../../docs/specs/SPEC_PANE_FOCUS_STRESS_TEST.md).
Asserts log-side invariants (Chrome_RenderWidgetHostHWND located,
no key events leaked to pane HWNDs during main-focus steps, etc.).
Does not attempt to read Chromium DOM values — UIA doesn't expose
them — so the log is the ground truth.

### Run (recommended: `-CreateLayout`)

```powershell
pwsh tools/tests/pane-focus-stress.ps1 -CreateLayout
```

With `-CreateLayout` the harness:

1. Reads `authkey.dev` and finds the dev instance.
2. Repositions the window to `(50, 50, 1300, 900)`.
3. Calls `New-AgentMuxTestTab` + `New-AgentMuxThreePaneLayout` to
   build a fresh `P1 | T | P2` tab programmatically (see the helper
   docs above). Both browsers are navigated to google.com.
4. Auto-computes click coordinates from the known window geometry —
   `pane-focus-stress.targets.json` is not needed.
5. Runs the 4 stress rounds.
6. Closes the test tab via `Remove-AgentMuxTestTab` in `finally`,
   even if a round failed mid-way.

### Run (manual layout)

If you've set up the layout yourself and want to test on it:

1. `task dev` in the repo root. Wait for the window to appear.
2. Set up a 3-pane layout manually — right-click the initial block →
   **Replace With…** → **browser** → P1. Right-click P1 → **Split
   Right** → terminal block T. Right-click T → **Split Right** →
   another browser → P2. Navigate both browsers to google.com.
3. Measure pixel coordinates for P1 search + address, P2 search +
   address, terminal prompt; copy
   `pane-focus-stress.targets.json.example` to
   `pane-focus-stress.targets.json` and fill them in.
4. Run:

   ```powershell
   pwsh tools/tests/pane-focus-stress.ps1
   ```

With `-SkipAuthFile` the harness falls back to image-name discovery,
which can target the wrong process if multiple `agentmux-cef`
instances are running.

Pass: exits 0 with `PASS (24/24 steps)`.

Fail: exits 1, writes a full per-step failure report to
`$env:TEMP/pane-focus-stress-<ts>.log` with the log delta around
each failed step.

### Limits

- **Auto-computed coordinates are approximate.** `-CreateLayout` uses
  a static geometry model based on the fixed (50, 50, 1300, 900)
  window — they should land inside the target elements, but a
  fractional-DPI display or a frontend chrome-height change could
  miss. Override with `pane-focus-stress.targets.json` if that
  happens. (Better long-term: have the harness ask the frontend for
  each block's rect via a new service method.)
- **Pixel coordinates** still break if you resize the window mid-run.
- **Host-log only** (no sidecar log). If a failure looks like it
  lives in `agentmux-srv`, add a `-SrvLogPath` parameter — kept out
  for now to keep the harness narrow.
