# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Layout-setup smoke test. Proves the programmatic 3-pane flow works
# before pane-focus-stress.ps1 relies on it:
#   1. Read authkey.dev
#   2. New-AgentMuxTestTab in the active workspace
#   3. New-AgentMuxThreePaneLayout (P1 browser | T terminal | P2 browser)
#   4. Verify the new tab's layoutstate has 3 leaf nodes pointing at
#      the block IDs the helper returned
#   5. Remove-AgentMuxTestTab cleanup
#
# Run after pane-focus-smoke.ps1 passes. If this fails, the layout
# wiring is broken; fix it before touching pane-focus-stress.ps1's
# -CreateLayout switch.

[CmdletBinding()]
param(
    [switch]$KeepTab
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'authfile.ps1')
. (Join-Path $PSScriptRoot 'three-pane-layout.ps1')

$auth = Get-AgentMuxAuthFile
Write-Host "[layout-smoke] authfile: instance=$($auth.instance) pid=$($auth.host_pid)"

$tabInfo = $null
try {
    Write-Host "[layout-smoke] Creating test tab…"
    $tabInfo = New-AgentMuxTestTab -Auth $auth -Verbose
    Write-Host "[layout-smoke]   tab=$($tabInfo.tabid) workspace=$($tabInfo.workspaceid)"

    Write-Host "[layout-smoke] Building 3-pane layout…"
    $layout = New-AgentMuxThreePaneLayout -Auth $auth -TabInfo $tabInfo -Verbose
    Write-Host "[layout-smoke]   p1=$($layout.p1) t=$($layout.t) p2=$($layout.p2)"

    # Verify the tab object now lists our 3 blocks.
    $tab = Invoke-AgentMuxService -Auth $auth -Service object -Method GetObject `
        -Args @("tab:$($tabInfo.tabid)")
    $expected = @($layout.p1, $layout.t, $layout.p2) | Sort-Object
    $actual   = @($tab.blockids) | Sort-Object
    if (-not $tab.blockids -or $tab.blockids.Count -lt 3) {
        Write-Error ("Tab blockids ({0}) does not include all three created blocks. Got [{1}], expected superset of [{2}]" -f `
            $tab.blockids.Count,
            ($actual -join ','),
            ($expected -join ','))
    }
    foreach ($id in $expected) {
        if ($tab.blockids -notcontains $id) {
            Write-Error "Tab blockids missing $id — layout action may not have been processed by frontend"
        }
    }
    Write-Host "[layout-smoke] Tab blockids match ($($tab.blockids.Count) blocks)"

    # Verify the LayoutState has a rootnode (frontend drained the
    # pending actions). If pendingbackendactions is still non-empty
    # the frontend hasn't processed yet — either SettleMs is too
    # short for this machine, or the frontend isn't running.
    # Property access is PSObject.Properties-based because strict
    # mode throws on missing properties and the serialized LayoutState
    # omits rootnode/pending when unset.
    $ls = Invoke-AgentMuxService -Auth $auth -Service object -Method GetObject `
        -Args @("layout:$($tab.layoutstate)")
    $rootnode = $ls.PSObject.Properties['rootnode']
    $pending  = $ls.PSObject.Properties['pendingbackendactions']
    if (-not $rootnode -or -not $rootnode.Value) {
        $pendingCount = 0
        if ($pending -and $pending.Value) { $pendingCount = @($pending.Value).Count }
        Write-Warning "LayoutState has no rootnode yet - frontend may not have drained pendingbackendactions. Pending count: $pendingCount"
    } else {
        Write-Host "[layout-smoke] LayoutState rootnode present"
    }

    Write-Host "[layout-smoke] PASS" -ForegroundColor Green
    if ($KeepTab) {
        Write-Host "[layout-smoke] -KeepTab set; tab left open for manual inspection: $($tabInfo.tabid)"
    }
    exit 0
}
catch {
    Write-Host "[layout-smoke] FAIL: $_" -ForegroundColor Red
    throw
}
finally {
    if (-not $KeepTab -and $tabInfo) {
        Remove-AgentMuxTestTab -Auth $auth -TabInfo $tabInfo -Verbose
    }
}
