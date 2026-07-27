# Print the absolute path of the directory that holds libcef.dll + the CEF
# runtime files (paks, snapshot data, GL libs, locales, etc.) that the
# AgentMux Windows build pipeline should bundle.
#
# Windows counterpart to scripts/resolve-cef-runtime.sh (Linux) /
# resolve-cef-runtime-darwin.sh (macOS) — same three-tier resolution order,
# same "explicit override is a hard requirement, not a soft fallback"
# behavior (Codex P2 on PR #743, applies identically here). Unlike the
# other two, there was never a functional gap this script exists to fix
# (Windows never needed BeginWindowDrag) — it exists purely so a
# proprietary-codec-enabled libcef.dll can be substituted in. See
# docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md.
#
# Resolution order:
#   1. $env:AGENTMUX_CEF_RUNTIME_DIR_WINDOWS — explicit override.
#   2. $HOME\cef-build\chromium_git\chromium\src\out\Release_GN_x64 — the
#      standard cef-build layout (docs/cef-build/build-patched-cef-windows.md).
#   3. Cargo cef-dll-sys cache: first match of
#      <repo>\target\{debug,release}\build\cef-dll-sys-*\out\cef_windows_x86_64.
#
# Output: stdout gets the absolute path on success; exit 0. Diagnostics go
# to stderr. Exit 1 if no candidate has libcef.dll + icudtl.dat, or if an
# explicit override is set but invalid (never silently falls through —
# same reasoning as the Linux/macOS resolvers).

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

function Test-CefDir {
    param(
        [string]$Dir,
        [string]$Kind = "cef-build"
    )
    if (-not (Test-Path (Join-Path $Dir "libcef.dll")) -or -not (Test-Path (Join-Path $Dir "icudtl.dat"))) {
        return $false
    }
    if ($Kind -eq "cargo-cache") {
        Write-Error -Message @"
WARNING: resolved libcef.dll to the cef-dll-sys cargo cache at $Dir.
         This is the upstream prebuilt CEF -- it lacks proprietary codec
         support, so MP4/MOV/HEVC/AC3 playback will fail
         (DEMUXER_ERROR_NO_SUPPORTED_STREAMS). Build the codec-enabled CEF
         (docs/cef-build/build-patched-cef-windows.md), then either
         (a) place it at ~\cef-build\chromium_git\chromium\src\out\Release_GN_x64, or
         (b) set `$env:AGENTMUX_CEF_RUNTIME_DIR_WINDOWS = "\path\to\your\Release_GN_x64"`.
"@ -Category NotSpecified -ErrorAction Continue
    }
    return $true
}

# NOTE: Test-CefDir returns a plain boolean -- it does NOT Write-Output the
# resolved path itself. Calling a function inside an `if (...)` expression
# in PowerShell captures its entire output stream into the boolean test
# value instead of letting it flow to the console, so a Write-Output inside
# the helper would be silently swallowed on every successful call. Each
# call site below prints the path itself, after the `if` has already
# consumed the boolean result.

# 1. Explicit override -- strict, same reasoning as the Linux/macOS resolvers.
if ($env:AGENTMUX_CEF_RUNTIME_DIR_WINDOWS) {
    if (Test-CefDir -Dir $env:AGENTMUX_CEF_RUNTIME_DIR_WINDOWS -Kind "override") {
        Write-Output $env:AGENTMUX_CEF_RUNTIME_DIR_WINDOWS
        exit 0
    }
    Write-Error @"
ERROR: AGENTMUX_CEF_RUNTIME_DIR_WINDOWS=$($env:AGENTMUX_CEF_RUNTIME_DIR_WINDOWS)
       is set but does not contain libcef.dll + icudtl.dat.
       Treating an explicit override as a hard requirement so a typo in
       CI/release packaging doesn't silently regress to the stock
       cef-dll-sys fallback. Fix the path or unset the env var.
"@
    exit 1
}

# 2. Standard cef-build layout under $HOME.
$CefBuildDir = Join-Path $HOME "cef-build\chromium_git\chromium\src\out\Release_GN_x64"
if (Test-CefDir -Dir $CefBuildDir -Kind "cef-build") {
    Write-Output $CefBuildDir
    exit 0
}

# 3. Cargo cef-dll-sys cache -- the stock prebuilt fallback (no codec support).
$cargoCandidates = Get-ChildItem -Path (Join-Path $RepoRoot "target") -Recurse -Directory -Filter "cef_windows_x86_64" -Depth 6 -ErrorAction SilentlyContinue
foreach ($c in $cargoCandidates) {
    if (Test-CefDir -Dir $c.FullName -Kind "cargo-cache") {
        Write-Output $c.FullName
        exit 0
    }
}

Write-Error @"
ERROR: could not find libcef.dll + icudtl.dat in any of these locations:
  - $CefBuildDir
$(($cargoCandidates | ForEach-Object { "  - $($_.FullName)" }) -join "`n")
Build the codec-enabled CEF (docs/cef-build/build-patched-cef-windows.md) or
run ``cargo build -p agentmux-cef`` to populate the cef-dll-sys fallback cache.
"@
exit 1
