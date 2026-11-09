# Tests for scripts/install/install.ps1: dry runs for each way to install, the refusals, and the download checks
# against a signed manifest made here with a throwaway Ed25519 key (needs OpenSSL 3 on the path for those).
# It never runs an installer: every run that gets as far as one sets AGENTMUX_INSTALL_NO_RUN_SETUP.
# Run:  pwsh -File scripts/install/install.test.ps1     (or powershell.exe -File ..., from anywhere)
$ErrorActionPreference = "Continue"

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script = Join-Path $here "install.ps1"
$repoRoot = Split-Path -Parent (Split-Path -Parent $here)
$work = Join-Path ([IO.Path]::GetTempPath()) ("agentmux-install-test-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $work | Out-Null
$fails = 0
$utf8 = New-Object System.Text.UTF8Encoding($false)

function Pass([string]$Name) { Write-Host "ok   - $Name" }
function Fail([string]$Name, [string]$Why) { Write-Host "FAIL - $Name ($Why)"; $script:fails++ }

# Run install.ps1 in a fresh PowerShell with these environment variables; returns the exit code and all its output.
function Invoke-Install([string]$Shell, [hashtable]$EnvVars, [string[]]$ScriptArgs) {
    $saved = @{}
    foreach ($k in $EnvVars.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $EnvVars[$k]) }
    try {
        $out = (& $Shell -NoProfile -NonInteractive -File $script @ScriptArgs 2>&1 | Out-String)
        return @{ Code = $LASTEXITCODE; Out = $out }
    } finally {
        foreach ($k in $EnvVars.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }
}

function Expect([string]$Name, [int]$Code, [string]$Want, [string]$Shell, [hashtable]$EnvVars, [string[]]$ScriptArgs) {
    $r = Invoke-Install $Shell $EnvVars $ScriptArgs
    if ($r.Code -ne $Code) { Fail $Name "exit $($r.Code), wanted $Code`: $($r.Out)"; return }
    # An error message is wrapped to the console's width, with a bar in front of each line: read it as one line.
    $flat = $r.Out -replace '\s*\r?\n\s*(\|\s*)?', ' '
    if ($Want -and -not $flat.Contains($Want)) { Fail $Name "missing '$Want': $($r.Out)"; return }
    Pass $Name
}

function Expect-Absent([string]$Name, [string]$Bad, [string]$Shell, [hashtable]$EnvVars, [string[]]$ScriptArgs) {
    $r = Invoke-Install $Shell $EnvVars $ScriptArgs
    if ($r.Out.Contains($Bad)) { Fail $Name "contains '$Bad': $($r.Out)" } else { Pass $Name }
}

$shells = @()
foreach ($candidate in @("pwsh", "powershell")) { if (Get-Command $candidate -ErrorAction SilentlyContinue) { $shells += $candidate } }

foreach ($sh in $shells) {
    Write-Host "== $sh =="
    $none = @{ AGENTMUX_INSTALL_INSTALLED = "none" }
    function WithWinget([string]$W) { return @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = $W } }

    Expect "-Help" 0 "Usage: install.ps1" $sh $none @("-Help")
    Expect "bad -Version" 1 "-Version must look like" $sh $none @("-Version", "1;rm -rf /", "-DryRun")
    $r = Invoke-Install $sh $none @("-Method", "magic", "-DryRun")
    if ($r.Code -ne 0) { Pass "bad -Method" } else { Fail "bad -Method" "exit 0" }
    $r = Invoke-Install $sh $none @("-Nope")
    if ($r.Code -ne 0) { Pass "unknown option" } else { Fail "unknown option" "exit 0" }

    # winget, with the package in the catalogue
    Expect "winget package dry run" 0 "winget install --id AgentMux.AI -e --accept-package-agreements" $sh (WithWinget "ai") @("-DryRun", "-NoLaunch")
    # winget, before the package is in the catalogue: the Store listing
    Expect "winget store dry run" 0 "winget install --id 9P9QCXNNCRK3 --source msstore --accept-package-agreements --accept-source-agreements" $sh (WithWinget "store") @("-DryRun", "-NoLaunch")
    Expect "winget runs unattended" 0 "--disable-interactivity" $sh (WithWinget "store") @("-DryRun", "-NoLaunch")
    Expect "winget mode ends with done" 0 "done" $sh (WithWinget "ai") @("-DryRun", "-NoLaunch")

    # no winget: the installer, a version is given so no network is needed for a dry run
    Expect "installer dry run" 0 "AgentMux-0.60.14-x64-setup.exe" $sh (WithWinget "none") @("-DryRun", "-NoLaunch", "-Version", "0.60.14")
    Expect "installer is silent" 0 "/VERYSILENT /NORESTART" $sh (WithWinget "none") @("-DryRun", "-NoLaunch", "-Version", "0.60.14")
    Expect "-Method download skips winget" 0 "AgentMux-0.60.14-x64-setup.exe" $sh (WithWinget "ai") @("-DryRun", "-NoLaunch", "-Version", "0.60.14", "-Method", "download")
    Expect "-Method package-manager with no winget" 1 "winget was not found" $sh (WithWinget "none") @("-DryRun", "-Method", "package-manager")

    # Already installed: nothing changes, exit 0
    $setupInstalled = @{ AGENTMUX_INSTALL_INSTALLED = "setup"; AGENTMUX_INSTALL_WINGET = "ai" }
    Expect "already installed" 0 "already installed" $sh $setupInstalled @("-NoLaunch")
    Expect-Absent "already installed runs nothing" "+ " $sh $setupInstalled @("-NoLaunch")
    Expect "-Force installs over the same kind" 0 "winget install --id AgentMux.AI" $sh $setupInstalled @("-DryRun", "-NoLaunch", "-Force")
    $storeInstalled = @{ AGENTMUX_INSTALL_INSTALLED = "store"; AGENTMUX_INSTALL_WINGET = "ai" }
    Expect "the store copy is named" 0 "Microsoft Store" $sh $storeInstalled @("-NoLaunch")
    Expect "-Force over the store copy stays with the Store" 0 "winget install --id 9P9QCXNNCRK3 --source msstore" $sh $storeInstalled @("-DryRun", "-NoLaunch", "-Force")
    Expect "-Force does not add a second kind (store copy, no winget)" 1 "do not mix" $sh @{ AGENTMUX_INSTALL_INSTALLED = "store"; AGENTMUX_INSTALL_WINGET = "none" } @("-DryRun", "-NoLaunch", "-Force", "-Version", "0.60.14")
    $setupNoPackage = @{ AGENTMUX_INSTALL_INSTALLED = "setup"; AGENTMUX_INSTALL_WINGET = "store" }
    Expect "-Force over the installer copy falls back to the download when winget has only the Store" 0 "AgentMux-0.60.14-x64-setup.exe" $sh $setupNoPackage @("-DryRun", "-NoLaunch", "-Force", "-Version", "0.60.14")
    Expect-Absent "-Force over the installer copy never uses the Store id" "9P9QCXNNCRK3" $sh $setupNoPackage @("-DryRun", "-NoLaunch", "-Force", "-Version", "0.60.14")
    Expect "-Method package-manager cannot add the other kind" 1 "use -Method download" $sh $setupNoPackage @("-DryRun", "-NoLaunch", "-Force", "-Method", "package-manager")
    Expect "-Interactive is accepted" 0 "done" $sh (WithWinget "ai") @("-DryRun", "-NoLaunch", "-Interactive")

    # A real winget run, with a stand-in for winget: its output is shown, and its exit code decides (a failure is not read as success).
    $fakeOk = Join-Path $work "fake-winget-ok.cmd"
    [IO.File]::WriteAllText($fakeOk, "@echo off`r`necho fake winget output`r`nexit /b 0`r`n")
    $fakeFail = Join-Path $work "fake-winget-fail.cmd"
    [IO.File]::WriteAllText($fakeFail, "@echo off`r`necho fake winget failure`r`nexit /b 1`r`n")
    $wingetOk = @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = "ai"; AGENTMUX_INSTALL_WINGET_EXE = $fakeOk }
    $wingetFail = @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = "ai"; AGENTMUX_INSTALL_WINGET_EXE = $fakeFail }
    Expect "a winget that succeeds shows its output" 0 "fake winget output" $sh $wingetOk @("-NoLaunch")
    Expect "a winget that succeeds ends with done and no fallback" 0 "done" $sh $wingetOk @("-NoLaunch")
    Expect-Absent "a winget that succeeds does not fall back to the installer" "using the installer instead" $sh $wingetOk @("-NoLaunch")
    Expect "a winget that fails is reported (-Method package-manager)" 1 "winget could not install AgentMux" $sh $wingetFail @("-NoLaunch", "-Method", "package-manager")
    Expect "a winget that fails shows its output" 1 "fake winget failure" $sh $wingetFail @("-NoLaunch", "-Method", "package-manager")
    $wingetFailNoRun = @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = "ai"; AGENTMUX_INSTALL_WINGET_EXE = $fakeFail; AGENTMUX_INSTALL_NO_RUN_SETUP = "1"; AGENTMUX_INSTALL_RELEASE_BASE = (Join-Path $work "no-such-release") }
    Expect "a winget that fails falls back to the installer" 1 "using the installer instead" $sh $wingetFailNoRun @("-NoLaunch", "-Version", "9.9.9")
}

# ASCII only, no smart quotes
$bytes = [IO.File]::ReadAllBytes($script)
$bad = @($bytes | Where-Object { $_ -gt 127 }).Count
if ($bad -eq 0) { Pass "install.ps1 is ASCII only" } else { Fail "install.ps1 is ASCII only" "$bad non-ASCII bytes" }

# ---- the pinned keys are the app's keys ----
$keysJson = Get-Content -LiteralPath (Join-Path $repoRoot "crates/common/update-keys.json") -Raw | ConvertFrom-Json
$want = ($keysJson.keys | ForEach-Object { "$($_.id):$($_.public_key)" }) -join " "
$have = ""
foreach ($line in (Get-Content -LiteralPath $script)) { if ($line -match '^\s*\$UpdateKeys = "(.*)"\s*$') { $have = $Matches[1] } }
if ($want -eq $have) { Pass "UpdateKeys equals crates/common/update-keys.json" } else { Fail "UpdateKeys equals crates/common/update-keys.json" "'$have' differs from '$want'" }

# ---- the download checks, against a release made here ----
$openssl = Get-Command openssl -ErrorAction SilentlyContinue
$canSign = $false
if ($openssl) { $canSign = ((& openssl pkeyutl -help 2>&1 | Out-String) -match '-rawin') }
if (-not $canSign) {
    Write-Host "skip - this openssl cannot check Ed25519 signatures; download checks not run"
} else {
    Write-Host "== download checks =="
    $ver = "9.9.9"
    $name = "AgentMux-$ver-x64-setup.exe"
    $priv = Join-Path $work "priv.pem"
    & openssl genpkey -algorithm ed25519 -out $priv 2>$null
    $pubDer = Join-Path $work "pub.der"
    & openssl pkey -in $priv -pubout -outform DER -out $pubDer 2>$null
    $pubBytes = [IO.File]::ReadAllBytes($pubDer)
    $testKeys = "t1:" + [Convert]::ToBase64String($pubBytes[($pubBytes.Length - 32)..($pubBytes.Length - 1)])

    # A release: a stand-in installer, its manifest, the envelope and the signature, in the shape release-manifest.mjs writes.
    function New-Release([string]$Dir, [string]$Tag, [string]$Listed) {
        if (-not $Listed) { $Listed = $name }
        New-Item -ItemType Directory -Path $Dir -Force | Out-Null
        $bytes = New-Object byte[] 4096
        (New-Object Random 7).NextBytes($bytes)
        [IO.File]::WriteAllBytes((Join-Path $Dir $name), $bytes)
        $size = (Get-Item (Join-Path $Dir $name)).Length
        $sha = (Get-FileHash -LiteralPath (Join-Path $Dir $name) -Algorithm SHA256).Hash.ToLowerInvariant()
        $manifest = "{`n  `"format`": 1,`n  `"version`": `"$ver`",`n  `"tag`": `"$Tag`",`n  `"files`": [`n    {`n      `"name`": `"$Listed`",`n      `"size`": $size,`n      `"sha256`": `"$sha`"`n    }`n  ]`n}`n"
        [IO.File]::WriteAllText((Join-Path $Dir "AgentMux-$ver-manifest.json"), $manifest, $utf8)
        $msha = (Get-FileHash -LiteralPath (Join-Path $Dir "AgentMux-$ver-manifest.json") -Algorithm SHA256).Hash.ToLowerInvariant()
        $envelope = "{`n  `"format`": 1,`n  `"tag`": `"$Tag`",`n  `"manifest_sha256`": `"$msha`",`n  `"created_at`": `"2026-11-04T00:00:00Z`"`n}`n"
        $envPath = Join-Path $Dir "AgentMux-$ver-envelope.json"
        [IO.File]::WriteAllText($envPath, $envelope, $utf8)
        $sigBin = Join-Path $work "sign.bin"
        & openssl pkeyutl -sign -inkey $priv -rawin -in $envPath -out $sigBin 2>$null
        [IO.File]::WriteAllText((Join-Path $Dir "AgentMux-$ver-envelope.json.sig"), ("t1 " + [Convert]::ToBase64String([IO.File]::ReadAllBytes($sigBin)) + "`n"), $utf8)
    }

    function Copy-Release([string]$From, [string]$To) { Copy-Item -LiteralPath $From -Destination $To -Recurse }

    function Run-Release([string]$Sh, [string]$Dir, [string]$Keys) {
        return @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = "none"; AGENTMUX_INSTALL_NO_RUN_SETUP = "1"; AGENTMUX_INSTALL_TEST_KEYS = $Keys; AGENTMUX_INSTALL_RELEASE_BASE = $Dir }
    }
    $dl = @("-NoLaunch", "-Method", "download", "-Version", $ver)

    foreach ($sh in $shells) {
        Write-Host "== $sh, download checks =="
        $good = Join-Path $work "good-$sh"
        New-Release $good "v$ver" ""
        Expect "a good release is accepted" 0 "matches the signed manifest" $sh (Run-Release $sh $good $testKeys) $dl
        Expect "its signature is checked" 0 "signature of the release checked" $sh (Run-Release $sh $good $testKeys) $dl
        Expect "-RequireSignature accepts a checked one" 0 "matches the signed manifest" $sh (Run-Release $sh $good $testKeys) ($dl + "-RequireSignature")

        $badsize = Join-Path $work "badsize-$sh"; Copy-Release $good $badsize
        [IO.File]::AppendAllText((Join-Path $badsize $name), "x")
        Expect "a changed file is refused" 1 "bytes, the manifest says" $sh (Run-Release $sh $badsize $testKeys) $dl

        $badhash = Join-Path $work "badhash-$sh"; Copy-Release $good $badhash
        $size = (Get-Item (Join-Path $good $name)).Length
        $rnd = New-Object byte[] $size; (New-Object Random 99).NextBytes($rnd); [IO.File]::WriteAllBytes((Join-Path $badhash $name), $rnd)
        Expect "a same-size different file is refused" 1 "does not match the manifest's SHA-256" $sh (Run-Release $sh $badhash $testKeys) $dl

        $badsig = Join-Path $work "badsig-$sh"; Copy-Release $good $badsig
        $sigPath = Join-Path $badsig "AgentMux-$ver-envelope.json.sig"
        $text = [IO.File]::ReadAllText($sigPath)
        $first = $text.Substring(3, 1); $swap = "A"; if ($first -eq "A") { $swap = "B" }
        [IO.File]::WriteAllText($sigPath, $text.Substring(0, 3) + $swap + $text.Substring(4), $utf8)
        Expect "a wrong signature is refused" 1 "signature does not match" $sh (Run-Release $sh $badsig $testKeys) $dl

        $badman = Join-Path $work "badman-$sh"; Copy-Release $good $badman
        $manPath = Join-Path $badman "AgentMux-$ver-manifest.json"
        [IO.File]::WriteAllText($manPath, ([IO.File]::ReadAllText($manPath)).Replace('"format": 1', '"format":  1'), $utf8)
        Expect "a changed manifest is refused" 1 "not the one the envelope names" $sh (Run-Release $sh $badman $testKeys) $dl

        $other = Join-Path $work "othertag-$sh"; New-Release $other "v1.2.3" ""
        Expect "an envelope for another release is refused" 1 "another release" $sh (Run-Release $sh $other $testKeys) $dl

        $unlisted = Join-Path $work "unlisted-$sh"; New-Release $unlisted "v$ver" "Other.exe"
        Expect "a file the signed manifest does not list is refused" 1 "does not list $name" $sh (Run-Release $sh $unlisted $testKeys) $dl

        # With the app's real key the test signature must not verify.
        Expect "another key's signature is refused" 1 "signature does not match" $sh (Run-Release $sh $good "k1:m9HRZ4UOiBU2vyrjlHXj1D0CjfhAazI2DGdII9JSJLE=") $dl
    }
}

Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
Write-Host ""
if ($fails -eq 0) { Write-Host "all passed" } else { Write-Host "$fails failed"; exit 1 }
