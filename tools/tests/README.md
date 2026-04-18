# Tools/tests

Ad-hoc test harnesses for behaviour that's hard to unit-test.

## `pane-focus-stress.ps1`

Drives the 4-round pane-focus stress workload from
`docs/specs/SPEC_PANE_FOCUS_STRESS_TEST.md` against a running
`task dev` instance. Asserts log-side invariants (Chrome_RenderWidgetHostHWND
located, no key events leaked to pane HWNDs during main-focus steps,
etc.). Does not attempt to read Chromium DOM values — UIA doesn't expose
them — so the log is the ground truth.

### Setup before running

1. `task dev` in the repo root. Wait for the window to appear.
2. Position AgentMux at `(50, 50, 1300, 900)`. The harness does this
   on its own via `SetForegroundWindow` + `MoveWindow`.
3. **Manually** set up a 3-pane layout (tab-spec-driven context menu
   walk is a follow-up):
   - Right-click the existing single block → **Replace With...** →
     **browser** → it becomes P1.
   - Right-click P1 → **Split Right** → the new pane is a terminal
     by default; if not, Replace With → terminal. This is T.
   - Right-click T → **Split Right** → Replace With → browser. This is P2.
4. Click into P1 and navigate to `https://google.com` (address bar +
   Enter). Same for P2.
5. Take a screenshot of the window and measure pixel coordinates for
   each of the five targets:
   - P1 Google search box
   - P1 address bar
   - P2 Google search box
   - P2 address bar
   - T terminal prompt
6. Copy `pane-focus-stress.targets.json.example` to
   `pane-focus-stress.targets.json`, fill in the coords.

### Run

```powershell
pwsh tools/tests/pane-focus-stress.ps1
```

Pass: exits 0 with `PASS (24/24 steps)`.

Fail: exits 1, writes a full per-step failure report to
`$env:TEMP/pane-focus-stress-<ts>.log` with the log delta around
each failed step.

### Known limits

- Manual layout setup (see step 3). The harness is just the
  click-and-assert loop.
- Pixel coordinates break if the window is resized or layout changes.
  Re-run setup with a fresh `targets.json`.
- The harness reads only the host log (not the sidecar). This is
  usually enough; if not, add a `-SrvLogPath` parameter in a
  follow-up.
