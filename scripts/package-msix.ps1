#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Package AgentMux as an MSIX for Microsoft Store submission (CEF architecture).

.DESCRIPTION
  Builds an MSIX from a `task package` portable build. The resulting .msix can be
  uploaded to Partner Center (Store ID 9P9QCXNNCRK3) — Microsoft re-signs on ingest,
  so a self-signed cert is only needed for LOCAL install testing (-Sign).

  Spec: docs/specs/SPEC_MSIX_PACKAGING_2026_05_30.md

.PARAMETER PortableDir
  Path to an extracted portable build dir (contains agentmux.exe + runtime/).
  Default: newest ~/Desktop/agentmux-*-x64-portable/, or build one via `task package`.

.PARAMETER OutputDir   Output dir for the .msix. Default: dist\msix
.PARAMETER Sign        Self-sign the package for local install testing.
.PARAMETER SkipBuild   Don't fall back to `task package` if no portable is found.

.EXAMPLE  pwsh -File scripts/package-msix.ps1
.EXAMPLE  pwsh -File scripts/package-msix.ps1 -Sign       # build + self-sign for local Add-AppxPackage
#>
param(
  [string]$PortableDir = "",
  [string]$OutputDir   = "dist\msix",
  [switch]$Sign        = $false,
  [switch]$SkipBuild   = $false
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)   # repo root

# Live published identity invariant — the manifest's Publisher MUST hash to this
# (the PFN suffix of Store ID 9P9QCXNNCRK3 = AgentMux.AgentMux_vqr1k32tkfk4y).
$EXPECTED_PFN_HASH = "vqr1k32tkfk4y"

function Get-PublisherHash([string]$publisher) {
  # MSIX PFN publisher hash: SHA-256(UTF-16LE) -> first 8 bytes -> base32(13 chars).
  $bytes = [System.Text.Encoding]::Unicode.GetBytes($publisher)
  $hash  = [System.Security.Cryptography.SHA256]::Create().ComputeHash($bytes)
  $bits  = (($hash[0..7]) | ForEach-Object { [Convert]::ToString($_, 2).PadLeft(8, '0') }) -join ''
  $bits += '0'   # 64 -> 65 bits (13 * 5)
  $alpha = '0123456789abcdefghjkmnpqrstvwxyz'
  -join (0..12 | ForEach-Object { $alpha[[Convert]::ToInt32($bits.Substring($_ * 5, 5), 2)] })
}

Write-Host "`n=== AgentMux MSIX packager ===`n"

# ── makeappx ─────────────────────────────────────────────────────────────────
$makeappx = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
  Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
if (-not $makeappx) { throw "makeappx.exe not found — install the Windows 10/11 SDK." }
Write-Host "  makeappx : $makeappx"

# ── version (X.Y.Z -> X.Y.Z.0) ───────────────────────────────────────────────
$semver = (Get-Content -Raw package.json | ConvertFrom-Json).version
$msixVersion = "$semver.0"
Write-Host "  version  : $msixVersion"

# ── resolve portable dir ─────────────────────────────────────────────────────
if (-not $PortableDir) {
  $desktop = [Environment]::GetFolderPath('Desktop')
  $PortableDir = Get-ChildItem $desktop -Directory -Filter 'agentmux-*-x64-portable' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime | Select-Object -Last 1 -ExpandProperty FullName
}
if (-not $PortableDir -or -not (Test-Path (Join-Path $PortableDir 'agentmux.exe'))) {
  if ($SkipBuild) { throw "No portable build found (looked for agentmux.exe). Run 'task package' first." }
  Write-Host "  No portable found — running 'task package'..."
  task package
  $desktop = [Environment]::GetFolderPath('Desktop')
  $PortableDir = Get-ChildItem $desktop -Directory -Filter 'agentmux-*-x64-portable' |
    Sort-Object LastWriteTime | Select-Object -Last 1 -ExpandProperty FullName
}
Write-Host "  portable : $PortableDir"

# ── stage ────────────────────────────────────────────────────────────────────
# Copy the portable VERBATIM except the portable marker + seed data + readme, so
# the packaged app runs in INSTALLED mode (per-user data dir), not portable mode
# (which would try to write next to the read-only WindowsApps install). Verified
# against agentmux-common::runtime_mode — see spec §4.
$staging = Join-Path $OutputDir 'staging'
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Force -Path $staging | Out-Null
$exclude = @('agentmux-portable.marker', 'data', 'README.txt')
Write-Host "`n[1/5] Staging portable (excluding: $($exclude -join ', '))..."
Get-ChildItem $PortableDir -Force | Where-Object { $exclude -notcontains $_.Name } | ForEach-Object {
  Copy-Item $_.FullName (Join-Path $staging $_.Name) -Recurse -Force
}
if (-not (Test-Path (Join-Path $staging 'agentmux.exe')))        { throw "staging missing agentmux.exe" }
if (-not (Test-Path (Join-Path $staging 'runtime\libcef.dll')))  { throw "staging missing runtime\libcef.dll" }

# ── assets ───────────────────────────────────────────────────────────────────
Write-Host "[2/5] Copying Store assets..."
$assetsSrc = Join-Path $PSScriptRoot '..\packaging\msix\assets'
New-Item -ItemType Directory -Force -Path (Join-Path $staging 'Assets') | Out-Null
Copy-Item (Join-Path $assetsSrc '*.png') (Join-Path $staging 'Assets') -Force

# ── manifest (substitute version) ────────────────────────────────────────────
Write-Host "[3/5] Rendering AppxManifest.xml..."
$tmpl = Get-Content -Raw (Join-Path $PSScriptRoot '..\packaging\msix\AppxManifest.xml.template')
$manifest = $tmpl -replace '\{\{VERSION_4PART\}\}', $msixVersion
$manifestPath = Join-Path $staging 'AppxManifest.xml'
# UTF-8 without BOM (makeappx is picky about a leading BOM).
[System.IO.File]::WriteAllText($manifestPath, $manifest, (New-Object System.Text.UTF8Encoding($false)))

# ── identity guard ───────────────────────────────────────────────────────────
[xml]$doc = Get-Content -Raw $manifestPath
$publisher = $doc.Package.Identity.Publisher
$pfnHash = Get-PublisherHash $publisher
Write-Host "       Publisher : $publisher"
Write-Host "       PFN hash  : $pfnHash (expected $EXPECTED_PFN_HASH)"
if ($pfnHash -ne $EXPECTED_PFN_HASH) {
  throw "Publisher hash mismatch: '$publisher' -> $pfnHash, expected $EXPECTED_PFN_HASH. " +
        "The Store would reject this package — fix the Publisher in AppxManifest.xml.template."
}
Write-Host "       identity  : OK (matches published PFN)"

# ── pack ─────────────────────────────────────────────────────────────────────
Write-Host "[4/5] Packing MSIX..."
$out = Join-Path $OutputDir "AgentMux_${semver}_x64.msix"
& $makeappx pack /d $staging /p $out /overwrite
if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)" }
Remove-Item $staging -Recurse -Force

# ── optional local-test signing ──────────────────────────────────────────────
if ($Sign) {
  Write-Host "[5/5] Self-signing for LOCAL testing..."
  $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $publisher } | Select-Object -First 1
  if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $publisher -KeyUsage DigitalSignature `
      -FriendlyName "AgentMux MSIX test" -CertStoreLocation "Cert:\CurrentUser\My" `
      -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
    Write-Host "       created self-signed cert $($cert.Thumbprint)"
  }
  $signtool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe" |
    Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
  & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint $out
  if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE)" }
  Write-Host "       signed. To trust + install locally (admin):"
  Write-Host "         Export-Certificate -Cert Cert:\CurrentUser\My\$($cert.Thumbprint) -FilePath agentmux-test.cer"
  Write-Host "         Import-Certificate -FilePath agentmux-test.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople"
  Write-Host "         Add-AppxPackage '$out'"
} else {
  Write-Host "[5/5] (unsigned — Store re-signs on ingest; use -Sign for local install test)"
}

$size = [math]::Round((Get-Item $out).Length / 1MB, 1)
Write-Host "`n=== MSIX: $out  ($size MB) ===`n"
Write-Host "Upload to Partner Center -> AgentMux -> new submission. runFullTrust needs a justification."
