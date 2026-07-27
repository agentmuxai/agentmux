#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Register Windows Error Reporting (WER) LocalDumps for the current
  agentmux-srv binary, so a crash produces a minidump instead of vanishing
  with no diagnostic trace.

.DESCRIPTION
  WER's LocalDumps matches by EXACT executable filename. This repo's srv
  binary embeds its version in the filename (`agentmux-srv-{version}-
  windows.x64.exe`, per Taskfile.yml's build:backend:rust:windows task), so
  a WER registration made for one version silently stops matching the next
  time the version bumps — nothing errors, it just quietly captures nothing.

  This is exactly what happened to the registration set up in
  docs/retro/retro-recurring-sidecar-crash-0xC0000409.md (2026-03-27): it
  was registered under the pre-rename name `agentmuxsrv-rs.exe`, which no
  longer matches ANY current binary, so no crash since has produced a dump.
  See docs/retro/retro-agentmux-srv-9min-crash-2026-07-26.md for the
  investigation that found this.

  Re-run this script any time you're chasing a sidecar crash and want to
  confirm WER will actually catch it — especially after a version bump.
  It's idempotent; safe to run repeatedly.

  Requires admin (writes to HKLM) — self-elevates via UAC if needed.

.PARAMETER DumpFolder  Where minidumps are written. Default: C:\CrashDumps
.PARAMETER DumpCount   Max dumps to keep before WER starts overwriting the oldest. Default: 10

.EXAMPLE  pwsh -File scripts/register-crash-dumps.ps1
#>
param(
    [string]$DumpFolder = "C:\CrashDumps",
    [int]$DumpCount = 10
)
$ErrorActionPreference = "Stop"

if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "Elevation required to write HKLM\...\Windows Error Reporting\LocalDumps — relaunching with UAC prompt..."
    # Quote path-like values explicitly — Start-Process builds the elevated
    # child's command line by joining -ArgumentList with spaces, so an
    # unquoted $PSCommandPath/$DumpFolder containing a space (a real
    # possibility for both — repo paths and DumpFolder are user-controlled)
    # would split into extra, garbled arguments. reagent P2.
    $argList = @("-NoProfile", "-File", "`"$PSCommandPath`"", "-DumpFolder", "`"$DumpFolder`"", "-DumpCount", $DumpCount)
    # -PassThru is required to get the elevated child's actual exit code —
    # without it, Start-Process returns nothing and $LASTEXITCODE reflects
    # whatever the last external command run in THIS process set it to, not
    # the elevated child's outcome, silently masking a failure there (e.g.
    # the throw on a missing package.json version) as success. reagent P2.
    $proc = Start-Process pwsh.exe -Verb RunAs -ArgumentList $argList -Wait -PassThru
    exit $proc.ExitCode
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$packageJson = Get-Content (Join-Path $repoRoot "package.json") -Raw | ConvertFrom-Json
$version = $packageJson.version
if (-not $version) {
    throw "Could not read version from package.json — is $repoRoot the repo root?"
}

$binaryName = "agentmux-srv-$version-windows.x64.exe"
$keyPath = "HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\$binaryName"

New-Item -Path $keyPath -Force | Out-Null
New-ItemProperty -Path $keyPath -Name "DumpFolder" -Value $DumpFolder -PropertyType ExpandString -Force | Out-Null
New-ItemProperty -Path $keyPath -Name "DumpCount" -Value $DumpCount -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $keyPath -Name "DumpType" -Value 2 -PropertyType DWord -Force | Out-Null   # 2 = full dump

New-Item -ItemType Directory -Force -Path $DumpFolder | Out-Null

Write-Host "Registered WER LocalDumps for '$binaryName' -> $DumpFolder (keeping last $DumpCount)"
Write-Host ""
Write-Host "NOTE: this key is filename-exact and WILL stop matching on the next version bump."
Write-Host "Re-run this script whenever you need dump capture for a new version."
