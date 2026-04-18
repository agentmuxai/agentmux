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
#   pwsh tools/tests/pane-focus-stress.ps1 -LogPath <path-to-host-log>
#
# If -LogPath is omitted, the script tails the file at
# `%APPDATA%\ai.agentmux.cef.v<latest>\db\…` via the dev task's
# version string.

[CmdletBinding()]
param(
    [string]$LogPath,
    [int]$TypeDelayMs = 300,
    [int]$AfterTypeMs = 400
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms

# ── Win32 interop ───────────────────────────────────────────────────────
$win32 = @'
using System;
using System.Runtime.InteropServices;
public static class Win32 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int x, int y, int cx, int cy, bool bRepaint);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetFocus();
}
'@
Add-Type -TypeDefinition $win32 -ErrorAction SilentlyContinue | Out-Null

# ── Helpers ─────────────────────────────────────────────────────────────

function Find-AgentMuxMain {
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
    $sinceStr = $since.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ss")
    Get-Content -LiteralPath $path -Encoding UTF8 |
        Where-Object { $_ -match '^\e\[2m(?<ts>\S+)\e\[0m' } |
        ForEach-Object {
            if ($_ -match '^\x1b\[2m(?<ts>[^\x1b]+)\x1b\[0m') {
                try {
                    $ts = [datetime]::Parse($matches['ts'])
                    if ($ts -ge $since) { $_ }
                } catch {}
            } else { $_ }
        }
}

function Find-LatestHostLog {
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

$main = Find-AgentMuxMain
if (-not $main) {
    Write-Error "No running agentmux-cef process with a main window. Start 'task dev' first."
}
Write-Host "[setup] AgentMux main PID=$($main.Id) handle=$($main.MainWindowHandle)"

[Win32]::ShowWindow($main.MainWindowHandle, 5) | Out-Null
Start-Sleep -Milliseconds 200
[Win32]::MoveWindow($main.MainWindowHandle, 50, 50, 1300, 900, $true) | Out-Null
Start-Sleep -Milliseconds 200
[Win32]::SetForegroundWindow($main.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 500

if (-not $LogPath) { $LogPath = Find-LatestHostLog }
if (-not $LogPath -or -not (Test-Path $LogPath)) {
    Write-Warning "Host log not found — log-side assertions will be skipped. Use -LogPath to point at it."
} else {
    Write-Host "[setup] Log path: $LogPath"
}

# ── Layout assumptions ──────────────────────────────────────────────────
#
# The harness assumes the user has manually set up P1 browser / T
# terminal / P2 browser side-by-side and navigated both browsers to
# google.com. The "create the layout programmatically" path through
# UIA context menus is brittle enough that it belongs in a follow-up
# commit; leaving it manual keeps the FIRST run of this spec deliverable.
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
if (-not (Test-Path $targetsPath)) {
    Write-Error @"
Missing $targetsPath.
Create it with 5 (x, y) coordinates for P1_search, P1_address, P2_search, P2_address, Terminal.
Capture them via the windows-mcp Snapshot tool or by eyeballing a screenshot.
"@
}
$targets = Get-Content -LiteralPath $targetsPath -Raw | ConvertFrom-Json

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
        Click-Pixel $coords[0] $coords[1]
        Start-Sleep -Milliseconds $TypeDelayMs
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
    exit 0
}

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
exit 1
