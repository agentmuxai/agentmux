#Requires -Version 7
<#
.SYNOPSIS
  Checks what setup.exe removes from runtime\ when it upgrades an install (packaging/windows/prune-old-versions.iss).

.DESCRIPTION
  An all-users install cannot be run without a UAC prompt, so the check builds the installer from a stub portable
  folder twice, from copies of agentmux.iss with a throwaway AppId, and installs each per-user into a throwaway folder:
    1. as shipped: a per-user install must leave every old file alone (the launcher prunes those itself);
    2. with the `IsAdminInstallMode` condition removed from the post-install step, which is what an all-users install
       runs: versions older than the previous one must go, the previous version, the version installed, a newer
       one and every other file must stay, and a file that is in use must be skipped without failing setup.
  It touches nothing but its own temp folder and one HKCU uninstall key it removes again.

  Needs Inno Setup 6 (ISCC.exe). Run: pwsh scripts/test-installer-prune.ps1
#>
param([string]$Iscc = '')

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$winDir = Join-Path $repo 'packaging\windows'
$issPath = Join-Path $winDir 'agentmux.iss'
$version = '0.60.31'

if (-not $Iscc) {
  $Iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
  if (-not $Iscc) {
    $Iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "${env:ProgramFiles}\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
      Where-Object { Test-Path $_ } | Select-Object -First 1
  }
}
if (-not $Iscc) { throw 'ISCC.exe (Inno Setup 6) not found' }

$work = Join-Path ([IO.Path]::GetTempPath()) ('agentmux-prune-test-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$appId = [guid]::NewGuid().ToString().ToUpper()
$failures = New-Object System.Collections.Generic.List[string]
$copies = New-Object System.Collections.Generic.List[string]

function Check([bool]$ok, [string]$what) {
  if ($ok) { Write-Host "  ok   $what" } else { Write-Host "  FAIL $what"; $script:failures.Add($what) }
}

# A stand-in for the unpacked portable folder: just the files the installer names.
$src = Join-Path $work 'portable'
New-Item -ItemType Directory -Force -Path (Join-Path $src 'runtime') | Out-Null
Set-Content -Path (Join-Path $src 'agentmux.exe') -Value 'stub'
foreach ($n in 'libcef.dll', 'agentmux-cef.exe', "agentmux-$version.exe", "agentmux-$version.dll", "agentmux-srv-$version-windows.x64.exe") {
  Set-Content -Path (Join-Path $src "runtime\$n") -Value 'stub'
}

function Versioned([string]$v) { @("agentmux-$v.exe", "agentmux-$v.dll", "agentmux-srv-$v-windows.x64.exe") }

# What a long-lived install holds before this setup runs. 0.60.9 vs 0.60.10 pins the numeric order; 0.61.0 is newer
# than the version being installed (a downgrade keeps it); the rest are files the prune must never name.
$old = '0.60.9', '0.60.10', '0.60.28', '0.60.29'
$previous = '0.60.30'
$newer = '0.61.0'
$others = 'libcef.dll', 'agentmux-cef.exe', 'agentmux-0.60.28.txt', 'agentmux-srv-0.60.29-linux.x64'
$locked = 'agentmux-srv-0.60.28-windows.x64.exe'

function Build-Installer([string]$label, [bool]$adminGateOff) {
  $text = (Get-Content -Raw $issPath).Replace('8654EA96-8EAA-4845-ADE9-352ADA712BC0', $appId)
  if ($adminGateOff) {
    $gated = $text.Replace(' and IsAdminInstallMode', '')
    if ($gated -eq $text) { throw 'the post-install step no longer has the IsAdminInstallMode condition this test removes' }
    $text = $gated
  }
  $copy = Join-Path $winDir ("agentmux-prune-test-$label.iss")
  $copies.Add($copy)
  Set-Content -Path $copy -Value $text -NoNewline
  $out = Join-Path $work "out-$label"
  & $Iscc "/DAppVersion=$version" "/DSourceDir=$src" "/DOutputDir=$out" $copy | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "ISCC failed for $label" }
  Join-Path $out "AgentMux-$version-x64-setup.exe"
}

function Install-Over([string]$setup, [string]$label, [bool]$lockOne) {
  $dir = Join-Path $work "install-$label"
  $runtime = Join-Path $dir 'runtime'
  New-Item -ItemType Directory -Force -Path $runtime | Out-Null
  foreach ($v in $old + $previous + $newer) { foreach ($n in Versioned $v) { Set-Content -Path (Join-Path $runtime $n) -Value 'old' } }
  foreach ($n in $others) { Set-Content -Path (Join-Path $runtime $n) -Value 'other' }
  $hold = $null
  if ($lockOne) { $hold = [IO.File]::Open((Join-Path $runtime $locked), 'Open', 'Read', 'None') }
  try {
    $log = Join-Path $work "setup-$label.log"
    $p = Start-Process -FilePath $setup -ArgumentList @('/CURRENTUSER', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOICONS', "/DIR=$dir", "/LOG=$log") -Wait -PassThru
    Check ($p.ExitCode -eq 0) "${label}: setup exits 0 (got $($p.ExitCode))"
  } finally {
    if ($hold) { $hold.Dispose() }
  }
  $dir
}

function Present([string]$dir, [string]$name) { Test-Path (Join-Path $dir "runtime\$name") }

try {
  Write-Host "work folder: $work"

  Write-Host "`n1. per-user install, installer as shipped"
  $dir = Install-Over (Build-Installer 'shipped' $false) 'shipped' $false
  foreach ($v in $old + $previous + $newer) { foreach ($n in Versioned $v) { Check (Present $dir $n) "$n is still there" } }
  foreach ($n in Versioned $version) { Check (Present $dir $n) "$n was installed" }

  Write-Host "`n2. the all-users path (admin condition removed), one old file held open"
  $dir = Install-Over (Build-Installer 'adminpath' $true) 'adminpath' $true
  foreach ($v in $old) {
    foreach ($n in Versioned $v) {
      if ($n -eq $locked) { Check (Present $dir $n) "$n is skipped while in use" } else { Check (-not (Present $dir $n)) "$n is removed" }
    }
  }
  foreach ($n in (Versioned $previous) + (Versioned $newer) + (Versioned $version) + $others) { Check (Present $dir $n) "$n is untouched" }
  Check ((Get-Content -Raw (Join-Path $work 'setup-adminpath.log')) -match 'old versions: removed 11 files') 'setup log says 11 files removed (4 old versions x 3 files, minus the one held open)'
}
finally {
  foreach ($label in 'shipped', 'adminpath') {
    $un = Join-Path $work "install-$label\unins000.exe"
    if (Test-Path $un) { Start-Process -FilePath $un -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES' -Wait | Out-Null }
  }
  Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{$appId}_is1"
  foreach ($c in $copies) { Remove-Item -Force -ErrorAction SilentlyContinue $c }
  Start-Sleep -Seconds 2
  Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $work
}

if ($failures.Count -gt 0) {
  Write-Host "`n$($failures.Count) check(s) failed"
  exit 1
}
Write-Host "`nall checks passed"
