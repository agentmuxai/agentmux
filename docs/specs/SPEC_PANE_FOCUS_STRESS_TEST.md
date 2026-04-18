# SPEC: Pane Focus Stress Test

Status: draft
Date: 2026-04-18
Owner: AgentA
Motivation: single pane-click → address-bar-click → type passes smoke test
at v0.33.264 (via `MainFocusReclaimTask`). User reports that under a
multi-pane, multi-round switching workload the focus routing eventually
breaks ("typing stops moving"). Current smoke drive via windows-mcp is
too thin to find the bug. This spec defines the workload that must pass
and the automation scaffolding for running it reliably.

## 1. The workload that must pass

### 1.1 Layout

Three panes side-by-side in a single tab, from left to right:

| Pane | View type | Initial state |
|------|-----------|---------------|
| **P1** | browser  | navigated to `https://google.com` |
| **T**  | terminal | shell prompt at `C:\Users\...` |
| **P2** | browser  | navigated to `https://google.com` |

Rationale: two browser panes plus a terminal exercises every relevant
focus edge — pane ↔ main, pane ↔ pane, pane ↔ terminal, and the terminal's
own xterm.js focus machinery which is separate from main's address bar.

### 1.2 Round definitions — distinct orderings per round

Three rounds, each with a **different ordering** to exercise a
different transition type. A fourth "reverse" round flips round 1.
After each step, assert where the characters ended up.

Typed-text tokens use a unique prefix per step so a misroute is
visible in whichever destination *did* receive the characters.

**Round 1 — left-to-right sweep, then addr→addr, then back to search.**
Exercises the basic pane→terminal→pane→address chain.

| # | Click target        | Type     | Must land in       |
|---|---------------------|----------|--------------------|
| 1 | P1 search box       | `r1a`    | P1 search          |
| 2 | T terminal prompt   | `r1b`    | Terminal buffer    |
| 3 | P2 search box       | `r1c`    | P2 search          |
| 4 | P1 address bar      | `r1d`    | P1 address         |
| 5 | P2 address bar      | `r1e`    | P2 address         |
| 6 | P1 search box       | `r1f`    | P1 search          |

**Round 2 — pane ↔ pane bouncing, terminal interleaved.**
Catches the specific failure mode where direct pane→pane transitions
leak focus via stale `ALLOW_PANE_FOCUS_ONCE`.

| # | Click target        | Type     | Must land in       |
|---|---------------------|----------|--------------------|
| 1 | P2 search box       | `r2a`    | P2 search          |
| 2 | P1 search box       | `r2b`    | P1 search          |
| 3 | P2 search box       | `r2c`    | P2 search          |
| 4 | P1 address bar      | `r2d`    | P1 address         |
| 5 | T terminal prompt   | `r2e`    | Terminal buffer    |
| 6 | P2 address bar      | `r2f`    | P2 address         |

**Round 3 — terminal-heavy, same-target repeats, closing with a search.**
Catches stale main-focus state and duplicate-target idempotency.

| # | Click target        | Type     | Must land in       |
|---|---------------------|----------|--------------------|
| 1 | T terminal prompt   | `r3a`    | Terminal buffer    |
| 2 | T terminal prompt   | `r3b`    | Terminal buffer (same target twice) |
| 3 | P1 search box       | `r3c`    | P1 search          |
| 4 | T terminal prompt   | `r3d`    | Terminal buffer    |
| 5 | P2 search box       | `r3e`    | P2 search          |
| 6 | P1 address bar      | `r3f`    | P1 address         |

**Round 4 — reverse of round 1.** Catches left-right asymmetry
(any "only one direction works" bug).

| # | Click target        | Type     | Must land in       |
|---|---------------------|----------|--------------------|
| 1 | P1 search box       | `r4a`    | P1 search          |
| 2 | P2 address bar      | `r4b`    | P2 address         |
| 3 | P1 address bar      | `r4c`    | P1 address         |
| 4 | P2 search box       | `r4d`    | P2 search          |
| 5 | T terminal prompt   | `r4e`    | Terminal buffer    |
| 6 | P1 search box       | `r4f`    | P1 search          |

24 click→type cycles total across the four rounds. Each round covers
a different transition shape:

| Transition         | R1 | R2 | R3 | R4 |
|--------------------|----|----|----|----|
| P1 → T             | ✓ |    | ✓ |    |
| T → P2             | ✓ |    |    |    |
| T → P1             |    |    | ✓ |    |
| P1 → P2 (direct)   |    | ✓ |    |    |
| P2 → P1 (direct)   |    | ✓ |    |    |
| P1 → P1 addr       | ✓ |    |    | ✓ |
| P2 → P2 addr       |    |    |    |    |
| P1 addr → P2 addr  | ✓ |    |    |    |
| P2 addr → P1 addr  |    |    |    | ✓ |
| P1 addr → T        |    | ✓ |    |    |
| T → T (idempotent) |    |    | ✓ |    |
| P2 → P1 addr       |    | ✓ |    |    |
| P2 → T             |    |    |    | ✓ |

### 1.3 Pass criteria

- Every step's typed text ends up in the asserted destination, character-
  for-character. No drops, no misroutes.
- No `[pane-wndproc] key msg=` log entry is attributable to a step
  targeting a non-pane destination (address bar, terminal, or other
  pane). Keys reaching the "wrong" pane HWND is the failure mode.
- `main-focus-reclaim` log entries show `render_found=true` on every
  fire. Any `render_found=false` is a bug.
- No `[main-focus-reclaim] could not resolve Views top-level HWND`
  warnings.

## 2. Why the current single-path smoke test isn't enough

Single-click-into-browser-then-address-bar passes because only one pane
exists. The failure mode only shows up when:

- **The "which render widget is main's" walk picks the wrong one** because
  multiple Chrome_RenderWidgetHostHWNDs are siblings. The fix excludes
  pane outer HWNDs from the walk — but only panes currently in
  `state.browsers` with the `browser-pane-*` prefix. A second pane
  created after the walker ran and cached something stale would escape.
- **The WM_LBUTTONDOWN subclass arms `ALLOW_PANE_FOCUS_ONCE` once per
  click** — but the flag is a single global `AtomicBool`. Two panes
  clicked in rapid succession race on that flag. Second pane's click
  may arm → first pane's still-in-flight WM_SETFOCUS consumes → second
  pane never gets the allow.
- **Terminal focus** goes through a different path (no WM_LBUTTONDOWN
  subclass on main's render widget). The terminal → pane → terminal
  transitions aren't covered.
- **Defocus timing**: `defocus_all` on every `main_window_focus` can
  race with Chromium's own internal re-focus of a pane when the pane
  has a pending JS `window.focus()`. User's reported "eventually
  breaks" sounds exactly like a state leak after 5–10 transitions.

## 3. Automation — windows-mcp test harness

### 3.1 Setup phase (driver-side, before the rounds)

1. Kill any running `agentmux-cef` processes.
2. Start `task dev`; wait until `agentmux-cef` main process has a
   `MainWindowHandle != 0` (poll every 2s, max 180s).
3. Foreground + position AgentMux at a known (x, y, w, h) so pane
   coordinates are deterministic across runs. Use `1300x900` at
   `(50, 50)`.
4. Snapshot the accessibility tree. Locate the `+` new-tab button and
   any existing tab controls so we can reset to a known workspace.
5. Capture `data_dir` from the task dev log (`Using data_dir: %APPDATA%…`)
   so the test can tail `agentmux-host-v*.log.*` later.
6. **Blank-slate reset**: right-click the one existing block, choose
   **Close Block**, confirm empty tab. Then:
   - Right-click the empty tab area → choose "browser" from the
     widget context menu.
   - Right-click the new browser pane → **Split Right** → choose
     "terminal" from the replace menu.
   - Right-click the terminal pane → **Split Right** → choose
     "browser" again.
7. Navigate P1 and P2 to `https://google.com` by clicking their
   address-bar elements and typing `google.com<Enter>`.

All of steps 6 and 7 are driven via `Snapshot` (interactive element IDs)
so the harness does not depend on pixel coordinates for menu items.

### 3.2 Round execution (three iterations)

Per round, driver does:

```
for round in 1..=3:
    for (target_label, text, expected_destination) in round_steps:
        click(target_label)
        sleep 200ms
        sendkeys(text)
        sleep 400ms
        snapshot_and_assert(expected_destination, text)
        tail_log_since_last_checkpoint()
```

`click(target_label)` uses the element ID from `Snapshot`, not pixel
coords. This survives layout reflow between rounds.

`snapshot_and_assert(dest, text)` reads the accessibility tree value
of `dest` and verifies it ends with `text`. This catches the three
failure shapes:

- No text anywhere → Win32 SetFocus dropped somewhere.
- Text in the **wrong** destination → focus routed to the wrong HWND.
- Text in the **right** destination → pass.

`tail_log_since_last_checkpoint()` greps the host log for every
`main-focus-reclaim`, `pane-wndproc`, `pane-focus`, `WM_KILLFOCUS`,
`WM_LBUTTONDOWN`, and `main_window_focus` line produced since the
last step. Stored per-step so a failure shows the exact CEF-side
sequence that led up to it.

### 3.3 Teardown

1. Capture the full host log for the session to
   `tmp/pane-focus-stress-run-<timestamp>.log`.
2. Screenshot the final window state.
3. Close all panes (for cleanliness).
4. Kill agentmux-cef.

### 3.4 Failure localisation

When the harness trips an assertion, emit this report to stderr:

```
FAIL round=<N> step=<M>
  target:       <label>
  typed:        "<text>"
  expected:     "<destination element value ends with text>"
  observed in:  <element where the text actually ended up, or "nowhere">
  last 30 log lines relevant to focus routing:
    ...
  screenshot:   tmp/fail-round<N>-step<M>.png
  accessibility snapshot: tmp/fail-round<N>-step<M>.tree.json
```

## 4. What to do with failures

For every documented failure, add a targeted regression test to
`agentmux-cef/tests/focus_integration.rs` (a new integration harness
that posts synthetic WM_LBUTTONDOWN / WM_SETFOCUS messages at specific
HWNDs and asserts the resulting SetFocus target). That way the next
iteration of the pane-focus fix can't re-break the same scenario.

Even if L3 integration tests for the CEF callback path are too heavy
(per `SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md`), a **pure state-machine
test** of the "which HWND should SetFocus land on given { main_hwnd,
pane_hwnds[], candidate_render_widgets[] }" function is trivial and
reproduces every failure this harness finds.

## 5. First deliverable (before doing another fix)

1. Build the harness script at `tools/tests/pane-focus-stress.mjs`
   (or `.ps1` — any shell works, PowerShell is easier for windows-mcp
   bindings).
2. Run it against current main. Capture a PASS or the exact FAIL.
3. If FAIL: add the regression test described in §4 before shipping
   the next Rust change. No "guess and retry" loops.
4. If PASS: mark this bug-report CLOSED in the PR description and keep
   the harness as a nightly CI job.

## 6. Not in scope

- Multi-window drag/drop focus. Separate test suite.
- Pane focus inside iframes of the embedded page.
- Terminal-specific keyboard quirks (`Ctrl+C`, arrow keys, etc.) — the
  workload only tests plain character typing.

## 7. Extension rounds (nightly only)

The four rounds from §1.2 are the minimum viable coverage. For the
nightly CI job, additional rounds add stress-scale variations:

- **Round 5 — rapid fire**: 20 consecutive P1↔P2 search-box clicks
  with minimal delay (<50ms). Catches focus operations that serialise
  badly under pressure.
- **Round 6 — nav-interleaved**: between each focus switch, trigger a
  page navigation in one of the browsers (e.g. type a URL + Enter).
  Catches the `install_pane_focus_redirect` reinstall-on-load-end path
  from `SPEC_BROWSER_PANE_LIFECYCLE.md` §5 race #5 under concurrent
  focus transitions.
- **Round 7 — pane lifecycle**: close a pane mid-round, create a new
  one, continue. Catches focus state that leaked from a destroyed
  pane's outer HWND.

These aren't part of the pre-merge gate — only the four rounds in §1.2
are. Nightly extension is aspirational once the core is stable.

## 8. Target completion

- Harness script + this spec landed in the next pane-fix PR.
- Each subsequent pane-focus PR must run this harness locally **and**
  as part of the PR's test plan, with the pass/fail output captured
  in the PR description.
