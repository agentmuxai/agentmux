# PLAN — Live test requirements: idle hung-renderer detection

**Status:** open — needs an agent on an isolated test box
**Date:** 2026-09-24
**Author:** Agent3
**Design under test:** `docs/specs/SPEC_IDLE_HUNG_RENDERER_DETECTION_2026_09_23.md`
**Builds on:** #3594 (merged; `agentmux-cef/src/client/unresponsive.rs`)
**Code under test:** branch `agent3/spike-idle-hang-probe` (spike, not for merge; `agentmux-cef/src/client/liveness.rs`)

## 1. Why this needs an isolated box

Every assumption in the spec is about how Chromium's hang monitor reacts to
input. Real human input on the same desktop makes the results meaningless.
On the operator's machine, the one run of the key test (V3) was contaminated:
the operator activated, minimized, restored, clicked and then closed the
window mid-run. The hang report arrived before the injected stimulus, so it
proves nothing.

**The test box must have:**

- Windows 10/11 with an **interactive, unlocked desktop session**. Not a
  service session and not a disconnected RDP session. Chromium does not run
  the hang monitor for a hidden widget, so the window must really be on
  screen. A VM with a console session that is logged in and stays unlocked
  is ideal.
- **No human input** for the duration of each run, and no other app that
  moves the cursor or injects input.
- A toolchain able to run `task dev` for this repo. See `BUILD.md`, and
  launch via `scripts/dev-agent.cmd`.
- Node 22+ (it has a built-in `WebSocket`, used by the probe scripts below).
- Optional, for §5 V8: a second monitor or an easy way to fully cover one
  window with another.

macOS/Linux runs are welcome but secondary. Windows is where the 09-22 freeze
happened.

## 2. Build and launch

```bash
git fetch origin && git switch agent3/spike-idle-hang-probe
# Launch with a NORMAL window. A minimized launch makes the AgentMux window
# start minimized, and then nothing below works.
AGENTMUX_SPIKE_POKE_MODE=keyup ./scripts/dev-agent.cmd TITLE=hang-test
```

- **CDP port:** read it from the renderer process command line
  (`--remote-debugging-port=NNNN`). Don't assume 9223.
- **Main window target:** `curl http://127.0.0.1:<port>/json/list`, then take
  the `page` whose URL has no `windowLabel=`/`pool=`. That is the main window.
- **Host log:** `~/.agentmux/dev/<branch-slug>/<clone-id>/logs/agentmux-host-*.log*`.
  Every event below is a JSON line with `"target":"crash"` and a `kind` field.

Environment knobs (spike only):

| Variable | Default | Effect |
|---|---|---|
| `AGENTMUX_SPIKE_POKE_MODE` | `keyup` | `keyup` = F24 key-up only; `keydown` = raw key-down + key-up |
| `AGENTMUX_SPIKE_SILENCE_SECS` | `25` | Seconds of probe silence before a poke. `0` = poke every healthy window about every 10 s (V4) |

## 3. Instrumentation (required on every run)

Record all of the following for the whole run. Without them a result is not
accepted:

1. **Host log `crash` events**, with timestamps: `liveness_probe_started`,
   `renderer_probe_silent`, `renderer_poked` (+`mode`),
   `renderer_probe_alive_again`, `renderer_unresponsive` (+`reports`),
   `renderer_hung_terminated`, `renderer_terminated` (+`reason`),
   `renderer_responsive_again`.
2. **Proof of no outside input:** sample `GetCursorPos`, the window rect,
   `IsIconic` and `GetForegroundWindow` every 250 ms, and count the samples
   where the cursor was inside the window or the window was in the
   foreground. The host log's own `[wrr] callback event=0x3/0x16/0x17`
   lines (foreground / minimize start / minimize end) on the main HWND must
   be absent during the measured interval.
3. **Renderer liveness at the end:** `Runtime.evaluate("location.href")` over
   CDP, with a 5 s timeout. A `data:` URL means the recovery page loaded.
4. **Keep the debugger detached.** Chromium suppresses hang reports while a
   CDP client is attached. Connect only to arm a hang or to read the result,
   never while measuring.

Probe scripts (Node, `node x.mjs <port> <targetIdPrefix>`):

```js
// hang.mjs — hang the page's main thread, then DETACH
const [port, idp] = process.argv.slice(2);
const t = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.id.startsWith(idp));
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise(r => ws.addEventListener('open', r));
ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate',
  params: { expression: 'setTimeout(() => { for (;;) {} }, 300); 1' } }));
await new Promise(r => setTimeout(r, 200)); ws.close();
console.log('hang armed + detached', new Date().toISOString()); process.exit(0);
```

```js
// stall.mjs — FINITE stall of <ms>, then detach (recoverable case)
// same as hang.mjs but expression:
//   `setTimeout(() => { const e = Date.now() + ${ms}; while (Date.now() < e) {} }, 500); 1`
```

```js
// alive.mjs — is the renderer answering? prints ALIVE <url> or HUNG
const [port, idp] = process.argv.slice(2);
const t = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.id.startsWith(idp));
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise(r => ws.addEventListener('open', r));
const timer = setTimeout(() => { console.log('HUNG'); process.exit(3); }, 5000);
ws.addEventListener('message', e => { clearTimeout(timer); console.log('ALIVE', e.data.slice(0, 120)); process.exit(0); });
ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression: 'location.href' } }));
```

To recover between runs: on the recovery page, run
`document.getElementById('reload-btn').click()` over CDP, or relaunch.

## 4. Test cases

Each case: run it **3 times**, and restart the app between cases unless noted.

| # | Case | Steps | Pass criteria |
|---|---|---|---|
| **V1** | Probe round-trip, no false pokes | Launch and leave idle 3 min | `liveness_probe_started` once; **zero** `renderer_probe_silent` for all 4 browsers (main + 3 hidden pool windows); `grep -c __agentmux_liveness__` = 0 in both `cef-debug.log` and the host log |
| **V2** | Probe into a hung renderer doesn't block the host | During V3, watch the host log | Host keeps logging (mem heartbeat, `[wrr]`) through the hang; other windows stay responsive |
| **V3** | **Crux: the host poke alone recovers an idle hang** | Window visible, NOT foreground, cursor outside it. Run `hang.mjs`, then touch nothing for 120 s | Sequence and timing: `renderer_probe_silent` at 25–40 s after the hang → `renderer_poked` → `renderer_unresponsive reports=1` **after** the poke, ~15 s (±3) → `renderer_hung_terminated reports=2` ~15 s later → `renderer_terminated reason="renderer stopped responding"` → `alive.mjs` shows a `data:` URL. Instrumentation shows **zero** cursor-inside / foreground samples |
| V3-kd | Same with `AGENTMUX_SPIKE_POKE_MODE=keydown` | as V3 | as V3. Tells us whether a lone key-up is enough |
| **V3-ctl** | Control: no poke | Relaunch with `AGENTMUX_SPIKE_SILENCE_SECS=100000`; run as V3 for 120 s | **No** `renderer_unresponsive` at all, and `alive.mjs` = HUNG. This proves the poke, not something else, triggers the monitor |
| **V4** | Poke is harmless on healthy windows | `AGENTMUX_SPIKE_SILENCE_SECS=0`. Open a terminal pane (focused, at a shell prompt), an agent pane with the input box focused, a context menu, and the command palette. Leave 2 min, then interact normally | About one `renderer_poked` per window per 10 s, then `renderer_probe_alive_again`. No characters in the terminal (`cat -v` shows nothing), no text in the agent input, no menu/palette/modal closed or opened, no focus change. No `renderer_unresponsive` |
| **V5** | Recoverable stall is not killed | Run `stall.mjs` with 22000 ms, then no input | Possibly `renderer_probe_silent` + poke + `renderer_unresponsive reports=1`, but then `renderer_responsive_again` / `renderer_probe_alive_again` and **no** `renderer_hung_terminated`; app still loaded |
| **V6** | Minimized hung window | Minimize the main window, run `hang.mjs`, wait 90 s, then restore it (`ShowWindow(hwnd, SW_SHOWNOACTIVATE=4)`) and wait 60 s | While minimized: poke logged, **no** kill. After restore: recovered within ~35 s. If it is never recovered after restore, report that: it is a real gap |
| **V7** | Floating pane (Floater) | Tear a pane off into a floating window, find its CDP target (`floating-` label), and run V3 against it | as V3, for the floater |
| **V8** | Fully covered (occluded) window | Cover the main window completely with another app window, then run V3 | Report what happens. Chromium's Windows occlusion tracking may treat it as hidden, so the result may be "recovers only when uncovered". Either way, record it; it decides the spec's §4 wording |
| **V9** | Probe overhead | V1 run with 10 windows/panes open | Host and renderer CPU are not measurably higher with the probe on (compare against `main` without the spike) |
| **V10** | #3594 regression | On the recovery page, click Reload | App returns with window title and layout intact |

## 5. Reporting

Post results as a comment on the tracking issue:

- For each case: pass/fail/observed, the run count, and the timeline of
  `crash` events with timestamps relative to the stimulus.
- The instrumentation counters (cursor-inside, foreground, WinEvent lines on
  the main HWND).
- OS/build details: Windows build, AgentMux version, and commit SHA of the
  spike branch.
- Anything surprising, especially where the report arrived relative to the
  poke in V3, and V6/V8 behavior.

## 6. What is already known

Results from the operator's machine, 2026-09-24, dev build of the spike:

- **V1: passed.** About 10 probe rounds × 4 browsers, zero false pokes, and
  zero sentinel lines in either log.
- **V3: inconclusive.** The window recovered (poke logged at 30 s of silence,
  kill and recovery page followed), but `renderer_unresponsive reports=1`
  arrived 2 s **before** the poke. The host log showed operator
  activate/minimize/restore events on the window at +7…+10 s. Real input
  armed the monitor, not the poke.
- #3594 (the kill path) is independently verified: permanent hang killed at
  ~30 s of pending input, and a 22 s stall not killed.
- **Side observation:** closing a window while it shows the recovery page
  leaves the host reducer counting it as live. The WRR quit watchdog then
  quits ~12 s later with "reducer desync, investigate". The outcome is
  correct (the last window closed, so the app quits), but it's a
  pre-existing quirk worth its own issue if V10 reproduces it.
