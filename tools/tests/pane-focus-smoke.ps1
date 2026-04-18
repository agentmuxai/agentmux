# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Auth + service smoke test. Proves the harness->agentmux-srv path
# works end-to-end against a running dev instance:
#   1. Read authkey.dev (Get-AgentMuxAuthFile)
#   2. POST /agentmux/service?service=client&method=GetClientData
#   3. Assert the response carries a non-empty oid
#
# Use this to validate the test infrastructure before running the
# longer pane-focus-stress.ps1, especially after agentmux-cef changes
# that touch auth, sidecar startup, or the /agentmux route.
#
# Usage:
#   pwsh tools/tests/pane-focus-smoke.ps1
#
# Exit 0 = harness can call the API. Exit 1 = wiring is broken; the
# stress test won't work either.

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'authfile.ps1')

$auth = Get-AgentMuxAuthFile
Write-Host "[smoke] authfile resolved:"
Write-Host "        instance     = $($auth.instance)"
Write-Host "        host_pid     = $($auth.host_pid)"
Write-Host "        web_endpoint = $($auth.web_endpoint)"
Write-Host "        service_path = $($auth.service_path)"

Write-Host "[smoke] calling client.GetClientData …"
$client = Invoke-AgentMuxService -Auth $auth -Service client -Method GetClientData -Args @()

if (-not $client) {
    Write-Error "client.GetClientData returned null"
}
if (-not $client.oid -or $client.oid -eq '') {
    Write-Error "client.GetClientData response missing oid: $($client | ConvertTo-Json -Depth 4)"
}

Write-Host "[smoke] PASS — client.oid=$($client.oid)" -ForegroundColor Green
exit 0
