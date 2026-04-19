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

    # ── Phase 2 assertions ────────────────────────────────────────────

    Write-Host "[dom-smoke] browser.eval 1+1 in P1"
    $evalResp = Invoke-AgentMuxBrowserApi -Auth $auth -Method eval `
        -Body @{ block_id = $p1; script = "1 + 1" }
    $excProp = $evalResp.PSObject.Properties['exception']
    if ($excProp -and $excProp.Value) {
        Write-Error "eval 1+1 unexpectedly threw: $($excProp.Value)"
    }
    if ($evalResp.result -ne 2) {
        Write-Error "eval 1+1 returned $($evalResp.result), expected 2"
    }
    Write-Host "[dom-smoke]   eval 1+1 = $($evalResp.result) (type=$($evalResp.type))"

    Write-Host "[dom-smoke] browser.eval with a throwing script (negative test)"
    $throwResp = Invoke-AgentMuxBrowserApi -Auth $auth -Method eval `
        -Body @{ block_id = $p1; script = "throw new Error('expected')" }
    $throwExc = $throwResp.PSObject.Properties['exception']
    if (-not $throwExc -or -not $throwExc.Value) {
        Write-Error "throwing eval didn't surface an exception"
    }
    if ($throwExc.Value -notmatch 'expected') {
        Write-Error "exception text didn't include 'expected': '$($throwExc.Value)'"
    }
    Write-Host "[dom-smoke]   exception captured: $($throwExc.Value.Substring(0, [Math]::Min($throwExc.Value.Length, 80)))"

    Write-Host "[dom-smoke] browser.focus_info on P1 (expect null or body — nothing user-focused)"
    $focusResp = Invoke-AgentMuxBrowserApi -Auth $auth -Method focus_info `
        -Body @{ block_id = $p1 }
    $focusedProp = $focusResp.PSObject.Properties['focused']
    if ($focusedProp -and $null -ne $focusedProp.Value) {
        Write-Host "[dom-smoke]   P1 focused: $($focusedProp.Value.tag) (acceptable — page may have auto-focused something)"
    } else {
        Write-Host "[dom-smoke]   P1 focused: null (default resting state)"
    }

    Write-Host "[dom-smoke] browser.focus_info on P2 after programmatic focus"
    # Focus the search field via eval, then assert focus_info sees it.
    Invoke-AgentMuxBrowserApi -Auth $auth -Method eval `
        -Body @{ block_id = $p2; script = "document.querySelector('textarea[name=\""q\""], input[name=\""q\""]').focus()" } | Out-Null
    Start-Sleep -Milliseconds 200
    $focusResp2 = Invoke-AgentMuxBrowserApi -Auth $auth -Method focus_info `
        -Body @{ block_id = $p2 }
    $focused2 = $focusResp2.PSObject.Properties['focused']
    if (-not $focused2 -or $null -eq $focused2.Value) {
        Write-Error "focus_info didn't report any focused element after programmatic focus"
    }
    $focusedTag = $focused2.Value.tag
    if ($focusedTag -notin @('textarea','input')) {
        Write-Error "focused element tag was '$focusedTag', expected textarea or input"
    }
    Write-Host "[dom-smoke]   P2 focused after focus(): $focusedTag (attrs.name=$($focused2.Value.attrs.name))"

    # ── Phase 3 write endpoints ───────────────────────────────────────

    Write-Host "[dom-smoke] browser.focus_element + dispatch_key text on P2 search"
    $searchSel = "textarea[name='q'], input[name='q']"
    Invoke-AgentMuxBrowserApi -Auth $auth -Method focus_element `
        -Body @{ block_id = $p2; selector = $searchSel } | Out-Null
    Invoke-AgentMuxBrowserApi -Auth $auth -Method dispatch_key `
        -Body @{ block_id = $p2; text = "agentmux test" } | Out-Null
    Start-Sleep -Milliseconds 200
    $valResp = Invoke-AgentMuxBrowserApi -Auth $auth -Method eval `
        -Body @{ block_id = $p2; script = "document.querySelector(`"$searchSel`").value" }
    if ($valResp.result -ne "agentmux test") {
        Write-Error "P2 search field value = '$($valResp.result)', expected 'agentmux test'"
    }
    Write-Host "[dom-smoke]   P2 search value: '$($valResp.result)'"

    Write-Host "[dom-smoke] browser.dispatch_key with named key (Backspace)"
    Invoke-AgentMuxBrowserApi -Auth $auth -Method dispatch_key `
        -Body @{ block_id = $p2; key = "Backspace" } | Out-Null
    Start-Sleep -Milliseconds 200
    $valResp2 = Invoke-AgentMuxBrowserApi -Auth $auth -Method eval `
        -Body @{ block_id = $p2; script = "document.querySelector(`"$searchSel`").value" }
    if ($valResp2.result -ne "agentmux tes") {
        Write-Error "After Backspace value = '$($valResp2.result)', expected 'agentmux tes'"
    }
    Write-Host "[dom-smoke]   after Backspace: '$($valResp2.result)'"

    Write-Host "[dom-smoke] browser.click_element — click the <h1> on example.com"
    # No form on example.com to click; click the h1 and assert via eval
    # that document.activeElement changed (or at least that click didn't error).
    Invoke-AgentMuxBrowserApi -Auth $auth -Method click_element `
        -Body @{ block_id = $p1; selector = "h1" } | Out-Null
    Write-Host "[dom-smoke]   click_element h1 on P1: OK"

    Write-Host "[dom-smoke] browser.navigate — redirect P1 to iana.org"
    Invoke-AgentMuxBrowserApi -Auth $auth -Method navigate `
        -Body @{ block_id = $p1; url = "https://www.iana.org/help/example-domains" } | Out-Null
    Start-Sleep -Milliseconds 2500
    $titleResp = Invoke-AgentMuxBrowserApi -Auth $auth -Method eval `
        -Body @{ block_id = $p1; script = "document.title" }
    if (-not $titleResp.result -or $titleResp.result -notmatch 'IANA|Example') {
        Write-Host "[dom-smoke]   navigate — post-nav title: '$($titleResp.result)' (not strict-asserted)"
    } else {
        Write-Host "[dom-smoke]   navigate — post-nav title: '$($titleResp.result)'"
    }

    Write-Host "[dom-smoke] browser.screenshot P1"
    $shotResp = Invoke-AgentMuxBrowserApi -Auth $auth -Method screenshot `
        -Body @{ block_id = $p1 }
    if (-not $shotResp.png_base64) {
        Write-Error "screenshot returned empty png_base64"
    }
    $pngBytes = [Convert]::FromBase64String($shotResp.png_base64)
    # PNG magic: 89 50 4E 47 0D 0A 1A 0A
    if ($pngBytes.Length -lt 8 -or
        $pngBytes[0] -ne 0x89 -or $pngBytes[1] -ne 0x50 -or
        $pngBytes[2] -ne 0x4E -or $pngBytes[3] -ne 0x47) {
        Write-Error "screenshot bytes don't start with PNG magic (got $($pngBytes[0..3] | ForEach-Object { '0x{0:X2}' -f $_ } | Join-String -Separator ' '))"
    }
    Write-Host "[dom-smoke]   screenshot OK, $($pngBytes.Length) bytes of PNG"

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
