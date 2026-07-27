# Configure the Windows CEF build tree with AgentMux's canonical GN args.
#
# Windows counterpart to configure-cef-build.sh (Linux). Same idempotent
# three-step shape: regenerate the gitignored C-API wrappers (needed
# because the build is cut from the same patched
# agentmux/7778-drag-rightclick-and-transparency fork branch as
# Linux/macOS -- see docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md
# open question #1 for why Windows uses the fork branch even though it
# doesn't call BeginWindowDrag), install args-windows.gn, run `gn gen`.
#
# Usage:
#   pwsh scripts/cef-build/configure-cef-build-windows.ps1
#   $env:AGENTMUX_CEF_SRC = "C:\path\to\chromium\src"; pwsh scripts/cef-build/configure-cef-build-windows.ps1
#
# Prereqs: the chromium+cef tree must already be synced and patched (see
# docs/cef-build/build-patched-cef-windows.md steps 1-3), and depot_tools
# must be on PATH with DEPOT_TOOLS_WIN_TOOLCHAIN=0 set (external/non-
# Google-corp builds -- see that doc's Prerequisites section for why this
# specific env var is the #1 Windows-specific failure point).

$ErrorActionPreference = "Stop"

$ScriptDir = $PSScriptRoot
$CanonicalArgs = Join-Path $ScriptDir "args-windows.gn"
$CefSrc = if ($env:AGENTMUX_CEF_SRC) { $env:AGENTMUX_CEF_SRC } else { Join-Path $HOME "cef-build\chromium_git\chromium\src" }
$OutDir = "out\Release_GN_x64"

if (-not (Test-Path $CanonicalArgs)) {
    Write-Error "ERROR: canonical args not found at $CanonicalArgs"
}
if (-not (Test-Path $CefSrc)) {
    Write-Error @"
ERROR: chromium/src tree not found at $CefSrc
       Sync + patch it first (docs/cef-build/build-patched-cef-windows.md steps 1-3),
       or set `$env:AGENTMUX_CEF_SRC to your chromium/src path.
"@
}
$TranslatorPy = Join-Path $CefSrc "cef\tools\translator.py"
if (-not (Test-Path $TranslatorPy)) {
    Write-Error "ERROR: $TranslatorPy missing -- is the cef checkout in place?"
}

Push-Location $CefSrc
try {
    # 1. Regenerate the translator-produced C-API wrappers (gitignored;
    #    cleaned by `git clean` / fresh checkouts). Same gotcha as Linux --
    #    NO --quiet, it's an invalid option.
    Write-Host "==> Regenerating CEF C-API wrappers (translator.py)..."
    Push-Location "cef"
    try {
        python3 tools\translator.py --root-dir .
    } finally {
        Pop-Location
    }

    # 2. Install the canonical args-windows.gn (back up any existing one).
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $existingArgs = Join-Path $OutDir "args.gn"
    if ((Test-Path $existingArgs) -and (Compare-Object (Get-Content $CanonicalArgs) (Get-Content $existingArgs))) {
        $backup = "$existingArgs.bak-$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())"
        Copy-Item $existingArgs $backup
        Write-Host "==> Backed up existing args.gn -> $backup"
    }
    Copy-Item $CanonicalArgs $existingArgs -Force
    Write-Host "==> Installed canonical args-windows.gn -> $CefSrc\$existingArgs"

    # 3. gn gen -- prefer the in-tree gn (no PATH dependency), fall back to PATH.
    $GnExe = Join-Path $CefSrc "buildtools\win\gn.exe"
    if (-not (Test-Path $GnExe)) {
        $gnCmd = Get-Command gn.exe -ErrorAction SilentlyContinue
        $GnExe = if ($gnCmd) { $gnCmd.Source } else { $null }
    }
    if (-not $GnExe) {
        Write-Error "ERROR: no gn binary -- expected buildtools\win\gn.exe or gn.exe on PATH (depot_tools)."
    }
    Write-Host "==> Running $GnExe gen $OutDir ..."
    & $GnExe gen $OutDir

    Write-Host ""
    Write-Host "Configured $CefSrc\$OutDir with the canonical Windows official-build args."
    Write-Host "Next: build (doc step 5):"
    Write-Host "    ninja -j <N> -C $OutDir cef"
} finally {
    Pop-Location
}
