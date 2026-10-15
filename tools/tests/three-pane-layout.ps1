# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Programmatic 3-pane layout setup for pane-focus stress testing.
# Replaces the "manually right-click + split + replace-with" flow from
# the original pane-focus-stress.ps1 docs.
#
# Architecture note: srv owns each tab's layout tree and changes it only
# through layout intents (SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md).
# This helper sends 3 intents (one insert + two splithorizontal) through
# `object.ApplyLayoutIntent`, the same edit a window sends as
# `layout.applyIntent`; the running window shows the stored tree.
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

function New-AgentMuxLayoutLeaf {
    <#
    .SYNOPSIS
    A layout leaf (pane) for one block, with a fresh node id.
    #>
    param([Parameter(Mandatory)] [string]$BlockId)
    return @{ id = [guid]::NewGuid().ToString(); flexDirection = "row"; size = 10; data = @{ blockId = $BlockId } }
}

function Invoke-AgentMuxLayoutIntent {
    <#
    .SYNOPSIS
    Applies one layout intent to a tab's layout (object.ApplyLayoutIntent)
    and throws if srv refuses it.
    #>
    param(
        [Parameter(Mandatory)] $Auth,
        [Parameter(Mandatory)] [string]$TabId,
        [Parameter(Mandatory)] $Intent
    )
    $res = Invoke-AgentMuxService -Auth $Auth -Service object -Method ApplyLayoutIntent `
        -Args @($TabId, $Intent, [guid]::NewGuid().ToString())
    if (-not $res.applied) {
        throw "layout intent $($Intent.type) refused: $($res.error)"
    }
}

function New-AgentMuxThreePaneLayout {
    <#
    .SYNOPSIS
    Creates a horizontal 3-pane layout (P1 browser | T terminal | P2 browser)
    inside a tab, with both browsers pointed at google.com.

    .DESCRIPTION
    Creates three blocks via object.CreateBlock, then sends three layout
    intents for the tab (object.ApplyLayoutIntent):

      1. insert           p1            (creates the root node)
      2. splithorizontal  t  target=p1  (splits, T on the right)
      3. splithorizontal  p2 target=t   (splits again, P2 rightmost)

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
    URL for the right browser pane. Default is also google.com — we
    tried `google.com/?q=p2`, duckduckgo.com, a self-contained
    data: URL, and example.com to keep the URLs distinct for the
    Phase-1 URL-match CDP resolver (see SPEC_BROWSER_DOM_API.md §5.5),
    but every non-google URL caused the second pane to be
    `on_before_close`'d by CEF ~18 ms after create (see retro
    2026-04-19 action item #9). Until that's root-caused, "both
    google.com" is the default that keeps the pane alive — at the
    cost of the resolver being unable to disambiguate P1 from P2.
    Callers who need disambiguation can pass distinct URLs and accept
    the pane-close risk.

    .PARAMETER SettleMs
    Milliseconds to wait after the layout intents so the window can
    render the new tree. Default 1500.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] $Auth,
        [Parameter(Mandatory)] $TabInfo,
        [string]$P1Url = "https://www.google.com/",
        [string]$P2Url = "https://www.google.com/imghp",
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

    # srv computes and stores each edit; the window shows the stored
    # tree. Each intent names its panes by node id.
    $p1Node = New-AgentMuxLayoutLeaf -BlockId $p1
    $tNode  = New-AgentMuxLayoutLeaf -BlockId $t
    $p2Node = New-AgentMuxLayoutLeaf -BlockId $p2
    $plan = @(
        @{ type = "insert"; node = $p1Node; magnified = $false; focused = $true },
        @{ type = "splithorizontal"; targetNodeId = $p1Node.id; newNode = $tNode; position = "after"; focused = $false },
        @{ type = "splithorizontal"; targetNodeId = $tNode.id; newNode = $p2Node; position = "after"; focused = $false }
    )
    foreach ($intent in $plan) {
        Write-Verbose "Layout intent: $($intent.type)"
        Invoke-AgentMuxLayoutIntent -Auth $Auth -TabId $TabInfo.tabid -Intent $intent
    }
    Start-Sleep -Milliseconds $SettleMs

    return [pscustomobject]@{
        tabid = $TabInfo.tabid
        p1    = $p1
        t     = $t
        p2    = $p2
    }
}
