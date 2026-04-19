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
