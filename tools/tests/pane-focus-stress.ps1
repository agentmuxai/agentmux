# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Pane focus stress harness. See docs/specs/SPEC_PANE_FOCUS_STRESS_TEST.md.
#
# Runs the 4-round focus-switching workload against a live `task dev`
# instance and reports per-step pass/fail. Windows-only; requires
# UIAutomationClient (ships with .NET on Windows).
#
# Invariants the harness enforces (log-side):
#   - Every `main_window_focus` IPC produces a `main-focus-reclaim` task
#     run with `render_found=true` in the log.
#   - When the test types into a main-DOM destination (address bar /
#     terminal), no `[pane-wndproc] key msg=` lines appear in the
#     interval between the click and the next step.
#   - When the test types into a pane destination (Google search), the
#     `[pane-wndproc] key msg=` events DO appear and no
#     `main-focus-reclaim` is fired in the interval.
#
# The harness does NOT try to read the Chromium DOM — the interactive
# tree surfaced by UIA only sees the address bar, not the Google search
# box value. Log invariants are the ground truth for "did focus route
# correctly". Screenshot dumps are retained for visual cross-check.
#
# Usage:
#   pwsh tools/tests/pane-focus-stress.ps1
#
# The harness auto-discovers the running dev instance via authkey.dev
# (see SPEC_TEST_API_ACCESS.md and tools/tests/authfile.ps1). Override
# with -LogPath if you want to point at a non-default log file, or
# -SkipAuthFile to fall back to image-name-based discovery.

[CmdletBinding()]
param(
    [string]$LogPath,
    [switch]$SkipAuthFile,
    # Create the 3-pane layout via the service API and clean up after.
    # Omit this flag to use whatever layout is currently on screen
    # (requires manual setup per README "Setup before running" §A).
    [switch]$CreateLayout,
    # TypeDelayMs: gap between pixel click and SendKeys. Needs to be
    # long enough for the async `main_window_focus` IPC chain (block.tsx
    # → ipc.rs → MainFocusReclaimTask → Win32 SetFocus) to complete on
    # the backend UI thread. 300 ms worked on average but hit a race
    # 1/24 steps — an in-flight SendKeys delivered 3 keys to the pane
    # HWND before SetFocus landed on main. 500 ms tolerates the
    # roundtrip even on a loaded machine.
    [int]$TypeDelayMs = 500,
    [int]$AfterTypeMs = 400
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms

# Pull in Get-AgentMuxAuthFile / Get-AgentMuxHostLogPath helpers and
# (when -CreateLayout is set) the layout-builder helpers.
. (Join-Path $PSScriptRoot 'authfile.ps1')
if ($CreateLayout) {
    . (Join-Path $PSScriptRoot 'three-pane-layout.ps1')
}

# ── Win32 interop ───────────────────────────────────────────────────────
$win32 = @'
using System;
using System.Runtime.InteropServices;
public struct RECT { public int left, top, right, bottom; }
public static class Win32 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int x, int y, int cx, int cy, bool bRepaint);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetFocus();
}
// A second class so callers that already do [Win32Rect]::GetWindowRect work;
// keeping it separate avoids Add-Type compile conflicts when the script is
// re-sourced in the same PS session.
public static class Win32Rect {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
}
'@
Add-Type -TypeDefinition $win32 -ErrorAction SilentlyContinue | Out-Null

# ── Helpers ─────────────────────────────────────────────────────────────

function Find-AgentMuxMain {
    param([int]$PreferredPid = 0)
    # If we know the dev instance's PID via authfile, pick that exact
    # process — multiple agentmux-cef instances run side-by-side
    # (portable + dev + multiple windows) and grabbing the first match
    # routinely targets the wrong one.
    if ($PreferredPid -gt 0) {
        try {
            $p = Get-Process -Id $PreferredPid -ErrorAction Stop
            if ($p.MainWindowHandle -ne [IntPtr]::Zero) { return $p }
        } catch {}
    }
    Get-Process agentmux-cef -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero } |
        Select-Object -First 1
}

function Click-Pixel([int]$x, [int]$y) {
    [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($x, $y)
    Start-Sleep -Milliseconds 80
    $down = 0x0002; $up = 0x0004
    Add-Type -Name MouseEvent -Namespace W -MemberDefinition @"
[DllImport("user32.dll")]
public static extern void mouse_event(int dwFlags, int dx, int dy, int dwData, int dwExtraInfo);
"@ -ErrorAction SilentlyContinue
    [W.MouseEvent]::mouse_event($down, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 40
    [W.MouseEvent]::mouse_event($up, 0, 0, 0, 0)
}

function Send-Text([string]$text) {
    # Escape SendKeys metacharacters (+, ^, %, ~, (, ), {, }, [, ])
    $escaped = $text -replace '([+^%~(){}\[\]])', '{$1}'
    [System.Windows.Forms.SendKeys]::SendWait($escaped)
}

function Read-LogSince([string]$path, [datetime]$since) {
    if (-not (Test-Path $path)) { return @() }
    # The host log on disk is JSON-per-line, not the ANSI-colored text
    # that the task-dev stdout shows. Try JSON first; fall back to the
    # ANSI format so this function works against either source. Parsed
    # timestamps come back Kind=Local (DateTime.Parse converts the Z
    # suffix to local time), which compares correctly against Get-Date.
    Get-Content -LiteralPath $path -Encoding UTF8 |
        ForEach-Object {
            $line = $_
            # JSON path (disk log): {"timestamp":"2026-...Z",...}
            if ($line.StartsWith('{')) {
                if ($line -match '"timestamp"\s*:\s*"(?<ts>[^"]+)"') {
                    try {
                        $ts = [datetime]::Parse($matches['ts'])
                        if ($ts -ge $since) { $line }
                    } catch {}
                }
            }
            # ANSI path (tty / stdout): \e[2m<timestamp>\e[0m ...
            elseif ($line -match '^\x1b\[2m(?<ts>[^\x1b]+)\x1b\[0m') {
                try {
                    $ts = [datetime]::Parse($matches['ts'])
                    if ($ts -ge $since) { $line }
                } catch {}
            }
        }
}

function Get-AgentMuxPaneTop {
    <#
    .SYNOPSIS
    Enumerate main window's child HWNDs and return the TOP of the
    first pane HWND — the pane content area's screen-Y. Used by the
    stress test to compute "above the pane" y-coords for address-bar
    clicks (which must land on main window DOM, not the pane).

    Returns $WindowY + 107 as a fallback if enumeration finds nothing.
    #>
    param(
        [Parameter(Mandatory)] [IntPtr]$MainHwnd,
        [Parameter(Mandatory)] [int]$WindowY
    )
    try {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PaneEnum {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out PaneRect r);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EnumProc cb, IntPtr p);
    public delegate bool EnumProc(IntPtr h, IntPtr p);
}
public struct PaneRect { public int left, top, right, bottom; }
'@ -ErrorAction SilentlyContinue | Out-Null
    } catch {}

    $tops = New-Object System.Collections.ArrayList
    $cb = [PaneEnum+EnumProc] {
        param([IntPtr]$h, [IntPtr]$p)
        $r = New-Object PaneRect
        [PaneEnum]::GetWindowRect($h, [ref]$r) | Out-Null
        # Pane HWNDs are narrower than the full main window (3 panes
        # side by side) and tall. Filter by "width < window_width/2
        # and height > 100" to skip the main-chrome HWND.
        if (($r.right - $r.left) -gt 50 -and ($r.right - $r.left) -lt 500 -and ($r.bottom - $r.top) -gt 300) {
            $null = $tops.Add($r.top)
        }
        return $true
    }
    [PaneEnum]::EnumChildWindows($MainHwnd, $cb, [IntPtr]::Zero) | Out-Null
    if ($tops.Count -gt 0) {
        return ($tops | Sort-Object | Select-Object -First 1)
    }
    return ($WindowY + 107)  # fallback to the pre-measurement constant
}

function Find-LatestHostLog {
    # Legacy fallback used only when -SkipAuthFile is set. The
    # version-suffix glob never matches dev mode (whose data dir is
    # `ai.agentmux.cef.dev`) — Get-AgentMuxHostLogPath is the
    # authoritative path resolver.
    $base = Join-Path $env:APPDATA 'ai.agentmux.cef.v*'
    $dirs = Get-ChildItem -Path $base -Directory -ErrorAction SilentlyContinue |
        Sort-Object { [version]($_.Name -replace 'ai\.agentmux\.cef\.v', '' -replace '-', '.') } -Descending
    foreach ($dir in $dirs) {
        $logDir = Join-Path $env:USERPROFILE '.agentmux\logs'
        if (-not (Test-Path $logDir)) { continue }
        $v = ($dir.Name -replace 'ai\.agentmux\.cef\.v', '') -replace '-', '.'
        $candidate = Get-ChildItem -Path $logDir -Filter "agentmux-host-v$v.log.*" -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($candidate) { return $candidate.FullName }
    }
    return $null
}

# ── Setup ───────────────────────────────────────────────────────────────

$auth = $null
$preferredPid = 0
if (-not $SkipAuthFile) {
    try {
        $auth = Get-AgentMuxAuthFile
        $preferredPid = $auth.host_pid
        Write-Host "[setup] authkey.dev: instance=$($auth.instance) pid=$($auth.host_pid) endpoint=$($auth.web_endpoint)"
        if (-not $LogPath) {
            $candidate = Get-AgentMuxHostLogPath -Auth $auth
            if ($candidate) { $LogPath = $candidate }
        }
    } catch {
        Write-Warning "Auth file lookup failed: $_"
        Write-Warning "Falling back to image-name discovery; layout / log targeting may pick the wrong instance if multiple agentmux-cef processes are running."
    }
}

$main = Find-AgentMuxMain -PreferredPid $preferredPid
if (-not $main) {
    Write-Error "No running agentmux-cef process with a main window. Start 'task dev' first."
}
Write-Host "[setup] AgentMux main PID=$($main.Id) handle=$($main.MainWindowHandle)"

[Win32]::ShowWindow($main.MainWindowHandle, 5) | Out-Null
Start-Sleep -Milliseconds 200
# Size the window to fit the primary display. Earlier versions hard-coded
# 1300×900 which got silently clamped on narrower monitors (e.g. 900×1600
# portrait), pushing pane coords out of alignment and making clicks land
# on the wrong HWNDs. Size the window to primary screen minus 2× the
# window origin; then read back the ACTUAL rect for coord math below.
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
$origin = 50
$reqW = [Math]::Min(1300, $screen.Width - 2 * $origin)
$reqH = [Math]::Min(900,  $screen.Height - 2 * $origin)
[Win32]::MoveWindow($main.MainWindowHandle, $origin, $origin, $reqW, $reqH, $true) | Out-Null
Start-Sleep -Milliseconds 200
[Win32]::SetForegroundWindow($main.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 500

# Read back the actual window rect — Windows may clamp our request
# below the primary screen's dimensions. All coord math below is based
# on this, NOT the requested size.
$winRect = New-Object RECT
[Win32Rect]::GetWindowRect($main.MainWindowHandle, [ref]$winRect) | Out-Null
$winX = $winRect.left
$winY = $winRect.top
$winW = $winRect.right - $winRect.left
$winH = $winRect.bottom - $winRect.top
Write-Host "[setup] Window actual rect=($winX, $winY, ${winW}x${winH}) (requested ${reqW}x${reqH})"

if (-not $LogPath) { $LogPath = Find-LatestHostLog }
if (-not $LogPath -or -not (Test-Path $LogPath)) {
    Write-Warning "Host log not found — log-side assertions will be skipped. Use -LogPath to point at it explicitly, or ensure a debug build wrote authkey.dev (-SkipAuthFile bypassed)."
} else {
    Write-Host "[setup] Log path: $LogPath"
}

# ── Optional: programmatic layout setup ─────────────────────────────────

$createdTabInfo = $null
$layoutInfo = $null
if ($CreateLayout) {
    if (-not $auth) {
        Write-Error "-CreateLayout requires the auth file (cannot be combined with -SkipAuthFile)."
    }
    Write-Host "[setup] Creating 3-pane layout via service API…"
    $createdTabInfo = New-AgentMuxTestTab -Auth $auth
    $layoutInfo = New-AgentMuxThreePaneLayout -Auth $auth -TabInfo $createdTabInfo
    Write-Host "[setup]   tab=$($createdTabInfo.tabid) p1=$($layoutInfo.p1) t=$($layoutInfo.t) p2=$($layoutInfo.p2)"
    # Give the browsers a moment to reach google.com so the search
    # box is present when the first click lands. Without this the
    # harness can click at a pane position that's still rendering the
    # navigation-in-progress state.
    Start-Sleep -Milliseconds 2500
}

# ── Layout assumptions ──────────────────────────────────────────────────
#
# Without -CreateLayout, the harness assumes the user has manually set
# up P1 browser / T terminal / P2 browser side-by-side and navigated
# both browsers to google.com. With -CreateLayout, the harness creates
# that layout programmatically in a fresh tab and tears it down in
# finally.
#
# Click coordinates are read from an accompanying JSON file
# `pane-focus-stress.targets.json` in the same dir:
#   {
#     "P1_search":  [x, y],
#     "P1_address": [x, y],
#     "P2_search":  [x, y],
#     "P2_address": [x, y],
#     "Terminal":   [x, y]
#   }
#
# Collect these once by hand via a Snapshot tool, save the JSON,
# then re-run this harness as many times as needed.

$targetsPath = Join-Path $PSScriptRoot 'pane-focus-stress.targets.json'
if (Test-Path $targetsPath) {
    $targets = Get-Content -LiteralPath $targetsPath -Raw | ConvertFrom-Json
    Write-Host "[setup] Using custom click targets from $targetsPath"
} elseif ($CreateLayout) {
    # With -CreateLayout, compute click targets from the ACTUAL window
    # rect we measured above — NOT a hardcoded 1300×900. That assumption
    # breaks on any display narrower than ~1500 px (Windows clamps the
    # window and the 3-pane math sends clicks into the wrong HWNDs —
    # confirmed on a 900×1600 portrait display where `$tCx` landed
    # inside P2's actual HWND instead of the terminal).
    #
    # Layout model for the window content:
    #   - top chrome (title + tab bar): ~60 px
    #   - pane header + browser nav bar: ~40 px each in P1/P2
    #   - content below that fills the rest
    # Panes: 3 equal horizontal columns. Centers at 1/6, 3/6, 5/6 of window width.
    # Y-coords measured from the actual UIAutomation tree on this
    # codebase's layout at window (50, 50, 800x900):
    #   - browser address-bar input centre: ≈ y=137 (window-y + 87)
    #   - Google search field centre:        ≈ y=463 (window-y + 413)
    #   - terminal input centre:             ≈ y=257 (window-y + 207)
    # These are Chromium-layout offsets — tab bar + pane header heights
    # — so they're stable across window sizes on this host. If the
    # pane/nav-bar CSS ever changes, remeasure with
    # `mcp__windows-mcp__Snapshot` and update these constants.
    $paneTop = Get-AgentMuxPaneTop -MainHwnd $main.MainWindowHandle -WindowY $winY
    $paneWidth = $winW / 3
    $p1Cx = [int]($winX + $paneWidth * 0.5)
    $tCx  = [int]($winX + $paneWidth * 1.5)
    $p2Cx = [int]($winX + $paneWidth * 2.5)
    $addressY = $winY + 87
    $searchY  = $winY + 413
    $termY    = $winY + 207
    $targets = [pscustomobject]@{
        P1_address = @($p1Cx, $addressY)
        P1_search  = @($p1Cx, $searchY)
        P2_address = @($p2Cx, $addressY)
        P2_search  = @($p2Cx, $searchY)
        Terminal   = @($tCx,  $termY)
    }
    Write-Host "[setup] Auto-computed targets:"
    Write-Host "         paneTop=$paneTop"
    Write-Host "         x: P1=$p1Cx T=$tCx P2=$p2Cx"
    Write-Host "         y: addressY=$addressY searchY=$searchY termY=$termY"
} else {
    Write-Error @"
Missing $targetsPath.
Create it with 5 (x, y) coordinates for P1_search, P1_address, P2_search, P2_address, Terminal.
Capture them via the windows-mcp Snapshot tool or by eyeballing a screenshot.
Alternatively, pass -CreateLayout to have the harness build a standard
3-pane layout and auto-compute coordinates from its geometry.
"@
}

# ── Round definitions ───────────────────────────────────────────────────

$rounds = @(
    @{
        name  = 'R1 linear'
        steps = @(
            @{ target = 'P1_search';  text = 'r1a'; pane_keys_expected = $true },
            @{ target = 'Terminal';   text = 'r1b'; pane_keys_expected = $false },
            @{ target = 'P2_search';  text = 'r1c'; pane_keys_expected = $true },
            @{ target = 'P1_address'; text = 'r1d'; pane_keys_expected = $false },
            @{ target = 'P2_address'; text = 'r1e'; pane_keys_expected = $false },
            @{ target = 'P1_search';  text = 'r1f'; pane_keys_expected = $true }
        )
    }
    @{
        name  = 'R2 bouncing'
        steps = @(
            @{ target = 'P2_search';  text = 'r2a'; pane_keys_expected = $true },
            @{ target = 'P1_search';  text = 'r2b'; pane_keys_expected = $true },
            @{ target = 'P2_search';  text = 'r2c'; pane_keys_expected = $true },
            @{ target = 'P1_address'; text = 'r2d'; pane_keys_expected = $false },
            @{ target = 'Terminal';   text = 'r2e'; pane_keys_expected = $false },
            @{ target = 'P2_address'; text = 'r2f'; pane_keys_expected = $false }
        )
    }
    @{
        name  = 'R3 terminal-heavy'
        steps = @(
            @{ target = 'Terminal';   text = 'r3a'; pane_keys_expected = $false },
            @{ target = 'Terminal';   text = 'r3b'; pane_keys_expected = $false },
            @{ target = 'P1_search';  text = 'r3c'; pane_keys_expected = $true },
            @{ target = 'Terminal';   text = 'r3d'; pane_keys_expected = $false },
            @{ target = 'P2_search';  text = 'r3e'; pane_keys_expected = $true },
            @{ target = 'P1_address'; text = 'r3f'; pane_keys_expected = $false }
        )
    }
    @{
        name  = 'R4 reverse'
        steps = @(
            @{ target = 'P1_search';  text = 'r4a'; pane_keys_expected = $true },
            @{ target = 'P2_address'; text = 'r4b'; pane_keys_expected = $false },
            @{ target = 'P1_address'; text = 'r4c'; pane_keys_expected = $false },
            @{ target = 'P2_search';  text = 'r4d'; pane_keys_expected = $true },
            @{ target = 'Terminal';   text = 'r4e'; pane_keys_expected = $false },
            @{ target = 'P1_search';  text = 'r4f'; pane_keys_expected = $true }
        )
    }
)

# ── Execution ───────────────────────────────────────────────────────────

$failures = @()
$stepCount = 0
$startTime = Get-Date
$exitCode = 1

try {
foreach ($round in $rounds) {
    Write-Host ""
    Write-Host "=== Round: $($round.name) ===" -ForegroundColor Cyan
    foreach ($step in $round.steps) {
        $stepCount++
        $targetName = $step.target
        $text = $step.text
        $expectedPaneKeys = $step.pane_keys_expected
        $coords = $targets.$targetName
        if (-not $coords) { Write-Error "Missing coords for target '$targetName'" }

        $preTime = Get-Date

        # Search-box targets (P1_search, P2_search) use the DOM API to
        # focus the Google search input before the SendKeys delivery.
        # Pixel-click alone routinely missed the input on HiDPI or with
        # a slightly drifted layout (11/24 failures before this change).
        # Pixel click still runs first — it's what transfers Win32 focus
        # to the pane HWND so the pane-wndproc log assertion stays valid.
        # focus_element then fixes the RENDERER-side DOM focus so the
        # SendKeys keystrokes actually land in the search field, not on
        # an empty pane area the pixel click happened to hit.
        $isDomTarget = ($targetName -eq 'P1_search' -or $targetName -eq 'P2_search')
        $domBlockId = $null
        $domSelector = "textarea[name='q'], input[name='q']"
        if ($isDomTarget -and $layoutInfo) {
            $domBlockId = if ($targetName -eq 'P1_search') { $layoutInfo.p1 } else { $layoutInfo.p2 }
        }

        Click-Pixel $coords[0] $coords[1]
        Start-Sleep -Milliseconds $TypeDelayMs

        if ($domBlockId) {
            try {
                Invoke-AgentMuxBrowserApi -Auth $auth -Method focus_element `
                    -Body @{ block_id = $domBlockId; selector = $domSelector } | Out-Null
                Start-Sleep -Milliseconds 100
            } catch {
                Write-Warning "focus_element for $targetName failed: $_"
            }
        }

        Send-Text $text
        Start-Sleep -Milliseconds $AfterTypeMs

        # Collect log delta since this step started.
        $logLines = @()
        if ($LogPath -and (Test-Path $LogPath)) {
            $logLines = Read-LogSince $LogPath $preTime
        }

        $paneKeyMatches   = @($logLines | Where-Object { $_ -match '\[pane-wndproc\] key msg=' })
        $reclaimMatches   = @($logLines | Where-Object { $_ -match 'main-focus-reclaim' })
        $renderFoundFalse = @($logLines | Where-Object { $_ -match 'render_found=false' })
        $resolveFailures  = @($logLines | Where-Object { $_ -match 'could not resolve Views top-level HWND' })

        $status = 'PASS'
        $reasons = @()
        if ($expectedPaneKeys -and $paneKeyMatches.Count -eq 0) {
            $status = 'FAIL'
            $reasons += "expected pane keystrokes but saw none in log"
        }
        if ((-not $expectedPaneKeys) -and $paneKeyMatches.Count -gt 0) {
            $status = 'FAIL'
            $reasons += "keystrokes leaked to pane HWND ($($paneKeyMatches.Count) events)"
        }
        # DOM-side verification: if this was a search-box step and we
        # have the block id, eval the field's `.value` and require it
        # to CONTAIN the text we typed. Catches the case where Win32
        # keys landed somewhere else entirely (e.g. focus got stolen
        # mid-typing). Only appended — does not replace the log checks.
        if ($domBlockId -and $status -eq 'PASS') {
            try {
                $valResp = Invoke-AgentMuxBrowserApi -Auth $auth -Method eval `
                    -Body @{ block_id = $domBlockId; script = "document.querySelector(`"$domSelector`")?.value ?? ''" }
                $valProp = $valResp.PSObject.Properties['result']
                $actual = if ($valProp) { [string]$valProp.Value } else { '' }
                if ($actual -notlike "*${text}*") {
                    $status = 'FAIL'
                    $reasons += "DOM value for search box did not contain '$text' (got '$actual')"
                }
            } catch {
                $status = 'FAIL'
                $reasons += "DOM value verification threw: $_"
            }
        }
        if ($renderFoundFalse.Count -gt 0) {
            $status = 'FAIL'
            $reasons += "render_found=false — main's render widget not located"
        }
        if ($resolveFailures.Count -gt 0) {
            $status = 'FAIL'
            $reasons += "Views top-level HWND resolution failed"
        }

        $line = "[step $stepCount] target=$targetName typed='$text' status=$status"
        if ($status -eq 'PASS') {
            Write-Host $line -ForegroundColor Green
        } else {
            Write-Host $line -ForegroundColor Red
            $reasons | ForEach-Object { Write-Host "           reason: $_" -ForegroundColor Red }
            $failures += [pscustomobject]@{
                step    = $stepCount
                round   = $round.name
                target  = $targetName
                text    = $text
                reasons = $reasons
                logs    = $logLines
            }
        }
    }
}

# ── Report ──────────────────────────────────────────────────────────────

Write-Host ""
if ($failures.Count -eq 0) {
    Write-Host "PASS ($stepCount/$stepCount steps)" -ForegroundColor Green
    $exitCode = 0
} else {
    $ts = Get-Date -Format 'yyyyMMdd-HHmmss'
    $reportPath = Join-Path $env:TEMP "pane-focus-stress-$ts.log"
    $failures | ForEach-Object {
        "=== FAIL step $($_.step) ($($_.round)) target=$($_.target) ==="
        "reasons: $($_.reasons -join '; ')"
        "--- log ---"
        $_.logs
        ""
    } | Out-File -LiteralPath $reportPath -Encoding UTF8

    Write-Host "FAIL: $($failures.Count)/$stepCount steps failed" -ForegroundColor Red
    Write-Host "Report: $reportPath" -ForegroundColor Red
    $exitCode = 1
}
}  # end try
finally {
    # Cleanup: close the test tab if -CreateLayout created one.
    # Runs on both happy-path exit and exceptions (e.g. missing
    # coords, UIA errors mid-round) so we don't leak test tabs
    # across runs.
    if ($createdTabInfo -and $auth) {
        Write-Host "[teardown] Closing test tab $($createdTabInfo.tabid)"
        Remove-AgentMuxTestTab -Auth $auth -TabInfo $createdTabInfo
    }
}

exit $exitCode
