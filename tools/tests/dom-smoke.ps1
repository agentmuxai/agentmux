# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Browser DOM API smoke test. Proves /agentmux/browser/query works
# end-to-end against a running dev instance:
#   1. Create a test tab with TWO browser panes at DISTINCT URLs
#      (the Phase-1 resolver disambiguates by URL — same URL in both
#      panes would be ambiguous).
#   2. Call browser.query for P1 → assert it finds the expected
#      element on that URL.
#   3. Same for P2 — verifies per-pane isolation.
#   4. Cleanup: close the test tab.
#
# Run after layout-smoke.ps1 passes. If this fails, the DOM API
# plumbing has a real bug.
#
# Usage:
#   pwsh tools/tests/dom-smoke.ps1

[CmdletBinding()]
param(
    [switch]$KeepTab
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'authfile.ps1')
. (Join-Path $PSScriptRoot 'three-pane-layout.ps1')

$auth = Get-AgentMuxAuthFile
Write-Host "[dom-smoke] authfile: instance=$($auth.instance) pid=$($auth.host_pid)"

$tabInfo = $null
try {
    Write-Host "[dom-smoke] Creating test tab with two browser panes at distinct URLs"
    $tabInfo = New-AgentMuxTestTab -Auth $auth -Name "DOM Smoke"

    # Re-implement a minimal 2-browser layout inline so we can control
    # the URLs per pane. (The shared helper sends both to google.com
    # which would trip the URL-match resolver.)
    $client = Invoke-AgentMuxService -Auth $auth -Service client -Method GetClientData
    $uicontext = @{ activetabid = $tabInfo.tabid }
    $rtOpts = @{ termsize = @{ rows = 25; cols = 80 } }

    $p1Def = @{ meta = @{ view = "browser"; url = "https://example.com/" } }
    $p2Def = @{ meta = @{ view = "browser"; url = "https://www.google.com/" } }

    $p1 = Invoke-AgentMuxService -Auth $auth -Service object -Method CreateBlock `
        -Args @($p1Def, $rtOpts) -Uicontext $uicontext
    $p2 = Invoke-AgentMuxService -Auth $auth -Service object -Method CreateBlock `
        -Args @($p2Def, $rtOpts) -Uicontext $uicontext

    # Push layout actions one-at-a-time (see three-pane-layout.ps1 for why).
    $tab = Invoke-AgentMuxService -Auth $auth -Service object -Method GetObject `
        -Args @("tab:$($tabInfo.tabid)")
    $lsOid = $tab.layoutstate

    $actions = @(
        @{ actiontype = "insert"; actionid = (New-Guid).ToString();
           blockid = $p1; focused = $true; magnified = $false; ephemeral = $false },
        @{ actiontype = "splithorizontal"; actionid = (New-Guid).ToString();
           blockid = $p2; targetblockid = $p1; position = "after";
           focused = $false; magnified = $false; ephemeral = $false }
    )
    foreach ($act in $actions) {
        $ls = Invoke-AgentMuxService -Auth $auth -Service object -Method GetObject `
            -Args @("layout:$lsOid")
        $payload = @{
            otype = "layout"; oid = $lsOid
            version = $ls.version
            pendingbackendactions = @($act)
        }
        foreach ($p in @("rootnode","magnifiednodeid","focusednodeid","leaforder","meta")) {
            $prop = $ls.PSObject.Properties[$p]
            if ($prop) { $payload[$p] = $prop.Value }
        }
        Invoke-AgentMuxService -Auth $auth -Service object -Method UpdateObject `
            -Args @($payload, $false) | Out-Null
        Start-Sleep -Milliseconds 1000
    }

    # Let the browsers load the pages. example.com is tiny (<100ms);
    # google.com has more assets. 3s is a comfortable ceiling.
    Write-Host "[dom-smoke] Waiting 3s for browsers to load target pages"
    Start-Sleep -Milliseconds 3000

    Write-Host "[dom-smoke] browser.query for h1 in P1 (example.com)"
    $p1Resp = Invoke-AgentMuxBrowserApi -Auth $auth -Method query `
        -Body @{ block_id = $p1; selector = "h1" }
    if (-not $p1Resp.matches -or $p1Resp.matches.Count -lt 1) {
        Write-Error "P1 query returned no <h1> — got $($p1Resp | ConvertTo-Json -Depth 3)"
    }
    $p1H1 = $p1Resp.matches[0]
    if ($p1H1.text -notmatch "Example Domain") {
        Write-Error "P1 <h1> text didn't mention 'Example Domain': '$($p1H1.text)'"
    }
    Write-Host "[dom-smoke]   P1 h1: '$($p1H1.text)' rect=($($p1H1.rect.x),$($p1H1.rect.y),$($p1H1.rect.width)x$($p1H1.rect.height))"

    Write-Host "[dom-smoke] browser.query for input[name='q'] in P2 (google.com)"
    $p2Resp = Invoke-AgentMuxBrowserApi -Auth $auth -Method query `
        -Body @{ block_id = $p2; selector = "input[name='q'], textarea[name='q']" }
    if (-not $p2Resp.matches -or $p2Resp.matches.Count -lt 1) {
        Write-Error "P2 query returned no Google search field"
    }
    $p2Search = $p2Resp.matches[0]
    Write-Host "[dom-smoke]   P2 search: tag=$($p2Search.tag) rect=($($p2Search.rect.x),$($p2Search.rect.y),$($p2Search.rect.width)x$($p2Search.rect.height))"

    Write-Host "[dom-smoke] PASS" -ForegroundColor Green
    if ($KeepTab) {
        Write-Host "[dom-smoke] -KeepTab set; tab left open: $($tabInfo.tabid)"
    }
    exit 0
}
catch {
    Write-Host "[dom-smoke] FAIL: $_" -ForegroundColor Red
    throw
}
finally {
    if (-not $KeepTab -and $tabInfo) {
        Remove-AgentMuxTestTab -Auth $auth -TabInfo $tabInfo
    }
}
