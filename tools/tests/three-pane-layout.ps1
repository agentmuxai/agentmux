# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Programmatic 3-pane layout setup for pane-focus stress testing.
# Replaces the "manually right-click + split + replace-with" flow from
# the original pane-focus-stress.ps1 docs.
#
# Architecture note: block layout inside a tab is reduced client-side
# by LayoutModel.treeReducer (see frontend/layout/lib/layoutModel.ts).
# There's no backend `layout.split` RPC — the frontend just watches
# the LayoutState.pendingbackendactions field, drains it, and applies
# each action locally. This helper pushes 3 actions (one insert + two
# splithorizontal) into a freshly-created tab's LayoutState, which the
# running frontend then picks up and reduces.
#
# Depends on `authfile.ps1` being dot-sourced first. All API calls go
# through Invoke-AgentMuxService to /agentmux/service.

Set-StrictMode -Version Latest

function New-AgentMuxTestTab {
    <#
    .SYNOPSIS
    Creates a fresh tab in the active workspace and activates it.

    .DESCRIPTION
    1. GetClientData → windowids[0]
    2. GetObject window → workspaceid
    3. workspace.CreateTab → new tabid (activateTab=true, pinned=false)

    Returns a PSCustomObject with workspaceid + tabid for later use
    with Remove-AgentMuxTestTab.

    .PARAMETER Auth
    Parsed authkey.dev object from Get-AgentMuxAuthFile.

    .PARAMETER Name
    Tab name. Defaults to "Focus Stress Test <short-timestamp>" to
    keep multiple concurrent runs visually distinguishable in the UI.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] $Auth,
        [string]$Name = "Focus Stress Test $(Get-Date -Format 'HHmmss')"
    )

    $client = Invoke-AgentMuxService -Auth $Auth -Service client -Method GetClientData
    if (-not $client.windowids -or $client.windowids.Count -eq 0) {
        throw "client.GetClientData returned no windowids; no window is open"
    }
    $windowId = $client.windowids[0]

    $window = Invoke-AgentMuxService -Auth $Auth -Service object -Method GetObject `
        -Args @("window:$windowId")
    $workspaceId = $window.workspaceid
    if (-not $workspaceId) {
        throw "Window $windowId has no workspaceid"
    }

    # workspace.CreateTab(workspaceId, tabName, activateTab, pinned)
    $tabId = Invoke-AgentMuxService -Auth $Auth -Service workspace -Method CreateTab `
        -Args @($workspaceId, $Name, $true, $false)
    if (-not $tabId) { throw "workspace.CreateTab returned empty tabid" }

    # Explicit SetActiveTab after CreateTab. CreateTab(activate=true)
    # flips workspace.activetabid on the backend, but in practice the
    # frontend's tab switch doesn't always fire on that path alone —
    # the stress harness saw clicks landing on the prior tab's layout.
    # SetActiveTab runs heal_layout + broadcasts a workspace update
    # that the frontend's activeTabId() atom reliably reacts to.
    Invoke-AgentMuxService -Auth $Auth -Service workspace -Method SetActiveTab `
        -Args @($workspaceId, $tabId) | Out-Null

    return [pscustomobject]@{
        workspaceid = $workspaceId
        tabid       = $tabId
        name        = $Name
        windowid    = $windowId
    }
}

function Remove-AgentMuxTestTab {
    <#
    .SYNOPSIS
    Closes a tab created by New-AgentMuxTestTab. Safe to call on a
    tab that's already gone (errors are swallowed).
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] $Auth,
        [Parameter(Mandatory)] $TabInfo
    )
    try {
        Invoke-AgentMuxService -Auth $Auth -Service workspace -Method CloseTab `
            -Args @($TabInfo.workspaceid, $TabInfo.tabid) | Out-Null
        Write-Verbose "Closed tab $($TabInfo.tabid)"
    } catch {
        Write-Verbose "CloseTab for $($TabInfo.tabid) failed (likely already closed): $_"
    }
}

function New-AgentMuxThreePaneLayout {
    <#
    .SYNOPSIS
    Creates a horizontal 3-pane layout (P1 browser | T terminal | P2 browser)
    inside a tab, with both browsers pointed at google.com.

    .DESCRIPTION
    Creates three blocks via object.CreateBlock, then pushes three
    pendingbackendactions into the tab's LayoutState:

      1. insert           blockid=p1            (creates the root node)
      2. splithorizontal  blockid=t  target=p1  (splits, T on the right)
      3. splithorizontal  blockid=p2 target=t   (splits again, P2 rightmost)

    The frontend's LayoutModel drains pendingbackendactions and calls
    treeReducer for each — see frontend/layout/lib/layoutPersistence.ts.

    Returns a PSCustomObject with p1, t, p2 block IDs for later
    interaction (e.g. coordinate-based clicks in the stress test).

    .PARAMETER Auth
    Parsed authkey.dev.

    .PARAMETER TabInfo
    PSCustomObject returned by New-AgentMuxTestTab, holding
    workspaceid + tabid. The helper sends UIContext.activetabid = tabid
    on every CreateBlock call so the new blocks land in THIS tab, not
    the previously-focused one.

    .PARAMETER P1Url
    URL for the left browser pane. Default google.com (search box).

    .PARAMETER P2Url
    URL for the right browser pane. The Phase-1 CDP resolver matches
    panes by URL, so **this should differ from P1Url** — two panes at
    the same URL can't be told apart and harness calls will resolve
    to whichever target CEF's /json returned first, swapping P1/P2
    for the caller. Default: google.com with a `?q=p2` query param,
    which keeps Google's search-box DOM intact while producing a
    distinct URL string.

    .PARAMETER SettleMs
    Milliseconds to wait after pushing the layout actions so the
    frontend can drain them and render. Default 1500.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] $Auth,
        [Parameter(Mandatory)] $TabInfo,
        [string]$P1Url = "https://www.google.com/",
        [string]$P2Url = "https://www.google.com/",
        [int]$SettleMs = 3500
    )

    $uicontext = @{ activetabid = $TabInfo.tabid }

    # BlockDefs for the two browser panes. Keep their URLs distinct
    # so the Phase-1 CDP resolver can disambiguate them by URL (see
    # parameter doc above and SPEC_BROWSER_DOM_API.md §5.5).
    $p1Def = @{ meta = @{ view = "browser"; url = $P1Url } }
    $p2Def = @{ meta = @{ view = "browser"; url = $P2Url } }
    # Terminal block. controller=shell is required: without it the
    # view renders a "Disconnected from local" overlay (see
    # CLAUDE.md "Terminal blockDef must include `controller: shell`").
    $termDef = @{
        meta = @{
            view       = "term"
            controller = "shell"
        }
    }
    # CreateBlock's second arg is RuntimeOpts. The frontend passes a
    # default termsize {25, 80}; matching that keeps parity.
    $rtOpts = @{
        termsize = @{ rows = 25; cols = 80 }
    }

    Write-Verbose "Creating P1 (browser)…"
    $p1 = Invoke-AgentMuxService -Auth $Auth -Service object -Method CreateBlock `
        -Args @($p1Def, $rtOpts) -Uicontext $uicontext
    Write-Verbose "Creating T (terminal)…"
    $t  = Invoke-AgentMuxService -Auth $Auth -Service object -Method CreateBlock `
        -Args @($termDef, $rtOpts) -Uicontext $uicontext
    Write-Verbose "Creating P2 (browser)…"
    $p2 = Invoke-AgentMuxService -Auth $Auth -Service object -Method CreateBlock `
        -Args @($p2Def, $rtOpts) -Uicontext $uicontext

    # Fetch the Tab to get its layoutstate oid. The LayoutState oid
    # differs from the tab oid — the tab stores a reference.
    $tab = Invoke-AgentMuxService -Auth $Auth -Service object -Method GetObject `
        -Args @("tab:$($TabInfo.tabid)")
    $layoutStateOid = $tab.layoutstate
    if (-not $layoutStateOid) {
        throw "Tab $($TabInfo.tabid) has no layoutstate ref"
    }

    # Load the current LayoutState. It'll be freshly created, empty
    # (no rootnode, no pendingactions).
    $layoutState = Invoke-AgentMuxService -Auth $Auth -Service object -Method GetObject `
        -Args @("layout:$layoutStateOid")

    # Push layout actions ONE AT A TIME, letting the frontend drain
    # between each. Pushing all three in one batch doesn't work: the
    # frontend's processPendingBackendActions loops the actions but
    # calls getNodeByBlockId against `model.leafs` (a memoized signal)
    # — that signal is only refreshed by updateTree() at the END of
    # the loop, so action 2's splithorizontal can't find the target
    # block that action 1 just inserted. The loop silently no-ops on
    # the misses. Per-action push forces updateTree + persistToBackend
    # to run between each, so the next push sees the freshly-materialized
    # target. See frontend/layout/lib/layoutPersistence.ts and
    # layoutNodeModels.ts `getNodeByBlockId`.
    $plan = @(
        @{
            actiontype = "insert"
            actionid   = [guid]::NewGuid().ToString()
            blockid    = $p1
            focused    = $true
            magnified  = $false
            ephemeral  = $false
        },
        @{
            actiontype    = "splithorizontal"
            actionid      = [guid]::NewGuid().ToString()
            blockid       = $t
            targetblockid = $p1
            position      = "after"
            focused       = $false
            magnified     = $false
            ephemeral     = $false
        },
        @{
            actiontype    = "splithorizontal"
            actionid      = [guid]::NewGuid().ToString()
            blockid       = $p2
            targetblockid = $t
            position      = "after"
            focused       = $false
            magnified     = $false
            ephemeral     = $false
        }
    )

    # Budget the settle time across 3 pushes. Default 3500ms → ~1200ms
    # per push, which is generous for a locally-running dev instance.
    $perStepSettle = [int]($SettleMs / $plan.Count)

    for ($i = 0; $i -lt $plan.Count; $i++) {
        Write-Verbose "Layout action $($i + 1)/$($plan.Count): $($plan[$i].actiontype) $($plan[$i].blockid)"

        # Re-fetch the LayoutState each iteration — the frontend wrote
        # back rootnode/version from the previous action via
        # persistToBackend, and UpdateObject takes the whole object.
        $currentLs = Invoke-AgentMuxService -Auth $Auth -Service object -Method GetObject `
            -Args @("layout:$layoutStateOid")

        # Rebuild as a hashtable: under Set-StrictMode assigning to a
        # property the PSCustomObject doesn't have throws.
        $payload = @{
            otype                 = "layout"
            oid                   = $layoutStateOid
            version               = $currentLs.version
            pendingbackendactions = @($plan[$i])
        }
        foreach ($propName in @("rootnode", "magnifiednodeid", "focusednodeid", "leaforder", "meta")) {
            $prop = $currentLs.PSObject.Properties[$propName]
            if ($prop) { $payload[$propName] = $prop.Value }
        }

        Invoke-AgentMuxService -Auth $Auth -Service object -Method UpdateObject `
            -Args @($payload, $false) | Out-Null

        Start-Sleep -Milliseconds $perStepSettle
    }

    return [pscustomobject]@{
        tabid = $TabInfo.tabid
        p1    = $p1
        t     = $t
        p2    = $p2
    }
}
