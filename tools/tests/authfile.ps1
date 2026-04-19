# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Auth-file helper for test harnesses. Reads `<data_dir>/authkey.dev`
# written by dev-mode agentmux-cef (see docs/specs/SPEC_TEST_API_ACCESS.md
# §5–§6 and src/dev_authfile.rs).
#
# Why this exists: external harnesses need the per-process auth_key to
# call POST /agentmux/service. The key is random per startup and
# in-process only — debug builds expose it via the auth file as a
# narrow test-fixture path. Release builds do not write the file.
#
# Source this from another script:
#   . "$PSScriptRoot/authfile.ps1"
#   $auth = Get-AgentMuxAuthFile
#   $resp = Invoke-AgentMuxService -Auth $auth -Service client -Method GetClientData

Set-StrictMode -Version Latest

function Get-AgentMuxAuthFile {
    <#
    .SYNOPSIS
    Locates and reads `authkey.dev` from a running dev agentmux-cef instance.

    .DESCRIPTION
    Walks the `%APPDATA%\ai.agentmux.cef.*` directories looking for any
    `authkey.dev` file, picks the newest, validates that its `host_pid`
    is alive, and returns the parsed JSON as a PSCustomObject.

    Throws if no file is found or the recorded host_pid is dead — both
    are signals that no dev instance is currently up.

    .PARAMETER DataDir
    Override the data dir search. Useful when targeting a portable
    instance with a known data_dir.

    .OUTPUTS
    PSCustomObject with fields: version, auth_key, web_endpoint,
    ws_endpoint, ipc_endpoint, ipc_token, service_path, file_path,
    instance, data_dir, host_pid, created_at.
    #>
    [CmdletBinding()]
    param(
        [string]$DataDir
    )

    $candidates = @()
    if ($DataDir) {
        $p = Join-Path $DataDir 'authkey.dev'
        if (Test-Path -LiteralPath $p) { $candidates += Get-Item -LiteralPath $p }
    } else {
        $base = Join-Path $env:APPDATA 'ai.agentmux.cef.*'
        # Wrap with @(...) so a single match doesn't collapse into a
        # bare object — `.Count` and `foreach` both need an array under
        # Set-StrictMode -Version Latest, which this module enables.
        $candidates = @(Get-ChildItem -Path $base -Directory -ErrorAction SilentlyContinue |
            ForEach-Object { Join-Path $_.FullName 'authkey.dev' } |
            Where-Object { Test-Path -LiteralPath $_ } |
            Get-Item |
            Sort-Object LastWriteTime -Descending)
    }

    if ($candidates.Count -eq 0) {
        throw @"
No authkey.dev found under %APPDATA%\ai.agentmux.cef.*\.
Start a dev instance first: ``task dev``. The file is written only by
debug builds (see docs/specs/SPEC_TEST_API_ACCESS.md §5).
"@
    }

    foreach ($file in $candidates) {
        try {
            $raw = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
            $obj = $raw | ConvertFrom-Json
        } catch {
            Write-Verbose "Skipping unreadable $($file.FullName): $_"
            continue
        }

        $pid_alive = $false
        try {
            $proc = Get-Process -Id $obj.host_pid -ErrorAction Stop
            # Match on image name to guard against PID reuse across reboots.
            if ($proc.ProcessName -like 'agentmux-cef*') { $pid_alive = $true }
        } catch {
            $pid_alive = $false
        }

        if ($pid_alive) {
            Write-Verbose "Using authfile: $($file.FullName) (instance=$($obj.instance), pid=$($obj.host_pid))"
            return $obj
        } else {
            Write-Verbose "Stale authfile (pid $($obj.host_pid) dead): $($file.FullName)"
        }
    }

    throw @"
Found authkey.dev file(s) but none belong to a live agentmux-cef process.
Stale files from prior runs are safe to delete; a fresh ``task dev`` will
overwrite them on next startup.
"@
}

function Invoke-AgentMuxService {
    <#
    .SYNOPSIS
    Calls a service method on the agentmux-srv RPC endpoint.

    .DESCRIPTION
    Wraps `POST /agentmux/service?service=<svc>&method=<m>&authkey=<key>`
    with a JSON body of `{service, method, args, uicontext}`. Returns the
    `data` field from the JSON response, or throws on transport / API error.

    .PARAMETER Auth
    The PSCustomObject returned by Get-AgentMuxAuthFile.

    .PARAMETER Service
    Service name (e.g. "client", "object", "workspace").

    .PARAMETER Method
    Method name on the service (e.g. "GetClientData", "CreateBlock").

    .PARAMETER Args
    Positional arguments as an array. Each element is JSON-serialized.
    Pass @() for methods that take no arguments.

    .PARAMETER TimeoutSec
    HTTP timeout. Default 15s.

    .PARAMETER Uicontext
    UIContext to send with the call. Required by methods that depend
    on the active tab (e.g. object.CreateBlock reads `activetabid`
    from uicontext — not args — and returns an error without it).
    Pass @{ activetabid = $tabId }.

    .EXAMPLE
    $client = Invoke-AgentMuxService -Auth $auth -Service client -Method GetClientData -Args @()

    .EXAMPLE
    $blockId = Invoke-AgentMuxService -Auth $auth -Service object -Method CreateBlock `
        -Args @($blockDef, $rtOpts) -Uicontext @{ activetabid = $tabId }
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] $Auth,
        [Parameter(Mandatory)] [string]$Service,
        [Parameter(Mandatory)] [string]$Method,
        [object[]]$Args = @(),
        [hashtable]$Uicontext,
        [int]$TimeoutSec = 15
    )

    $url = "http://{0}{1}?service={2}&method={3}&authkey={4}" -f `
        $Auth.web_endpoint,
        $Auth.service_path,
        [uri]::EscapeDataString($Service),
        [uri]::EscapeDataString($Method),
        [uri]::EscapeDataString($Auth.auth_key)

    $body = @{
        service   = $Service
        method    = $Method
        args      = $Args
        uicontext = $Uicontext
    } | ConvertTo-Json -Depth 20 -Compress

    $resp = Invoke-RestMethod -Method Post -Uri $url `
        -ContentType 'application/json' `
        -Body $body `
        -TimeoutSec $TimeoutSec

    # Under Set-StrictMode -Version Latest, $resp.error throws when
    # the JSON has no `error` field — the server omits it entirely
    # on success. PSObject.Properties lookup is strict-safe.
    $errProp = $resp.PSObject.Properties['error']
    if ($errProp -and $errProp.Value) {
        throw "service $Service.$Method returned error: $($errProp.Value)"
    }
    $dataProp = $resp.PSObject.Properties['data']
    if ($dataProp) { return $dataProp.Value }
    return $null
}

function Get-AgentMuxHostLogPath {
    <#
    .SYNOPSIS
    Resolves the host log file path for the instance described by an authfile.

    .DESCRIPTION
    Logs land in `~/.agentmux/logs/agentmux-host-<instance>.log.<date>`.
    Returns the newest matching file, or $null if none exist.

    .PARAMETER Auth
    The PSCustomObject returned by Get-AgentMuxAuthFile.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] $Auth)

    $logDir = Join-Path $env:USERPROFILE '.agentmux\logs'
    if (-not (Test-Path -LiteralPath $logDir)) { return $null }
    $pattern = "agentmux-host-$($Auth.instance).log.*"
    $f = Get-ChildItem -Path $logDir -Filter $pattern -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($f) { return $f.FullName }
    return $null
}
