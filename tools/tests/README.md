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

## `pane-focus-smoke.ps1`

Minimum-viable harness sanity check: reads the auth file and calls
`client.GetClientData`. Use this to confirm the harness↔backend path
is wired correctly before debugging the longer stress test.

```powershell
pwsh tools/tests/pane-focus-smoke.ps1
```

Exit 0 = ready to run pane-focus-stress.ps1. Exit 1 = no dev
instance, stale auth file, or auth/route mismatch.

## `pane-focus-stress.ps1`

Drives the 4-round pane-focus stress workload from
[`docs/specs/SPEC_PANE_FOCUS_STRESS_TEST.md`](../../docs/specs/SPEC_PANE_FOCUS_STRESS_TEST.md).
Asserts log-side invariants (Chrome_RenderWidgetHostHWND located,
no key events leaked to pane HWNDs during main-focus steps, etc.).
Does not attempt to read Chromium DOM values — UIA doesn't expose
them — so the log is the ground truth.

### Setup before running

1. `task dev` in the repo root. Wait for the window to appear.
2. **Manually** set up a 3-pane layout (programmatic layout
   creation is tracked as a follow-up — see "Limits" below):
   - Right-click the existing single block → **Replace With…** →
     **browser** → it becomes P1.
   - Right-click P1 → **Split Right** → the new pane is a terminal
     by default; if not, Replace With → terminal. This is T.
   - Right-click T → **Split Right** → Replace With → browser. This is P2.
3. Click into P1 and navigate to `https://google.com` (address bar +
   Enter). Same for P2.
4. Take a screenshot of the window and measure pixel coordinates for
   each of the five targets:
   - P1 Google search box
   - P1 address bar
   - P2 Google search box
   - P2 address bar
   - T terminal prompt
5. Copy `pane-focus-stress.targets.json.example` to
   `pane-focus-stress.targets.json`, fill in the coords.

### Run

```powershell
pwsh tools/tests/pane-focus-stress.ps1
```

The harness auto-locates the running dev instance, its log file, and
its main window via `authkey.dev`. With `-SkipAuthFile` it falls back
to image-name discovery, which can target the wrong process if
multiple `agentmux-cef` instances are running.

Pass: exits 0 with `PASS (24/24 steps)`.

Fail: exits 1, writes a full per-step failure report to
`$env:TEMP/pane-focus-stress-<ts>.log` with the log delta around
each failed step.

### Limits

- **Manual layout setup.** Steps 2–5 above are still hand-driven. The
  blocker is that block layout within a tab is reduced client-side
  by `LayoutModel.treeReducer`; backend has no `layout.split` RPC
  today. Two paths forward: write the layout actions into
  `LayoutState.pendingbackendactions` directly (matches what the
  `agent.open` handler does for one block), or add a backend
  `layout.setup_three_pane` test fixture method gated on
  `cfg(debug_assertions)`. Tracked as PR 4b.
- **Pixel coordinates** break if the window is resized or the layout
  changes. Re-run the coord-capture step with a fresh `targets.json`.
- **Host-log only** (no sidecar log). If a failure looks like it lives
  in `agentmux-srv`, add a `-SrvLogPath` parameter — kept out for now
  to keep the harness narrow.
