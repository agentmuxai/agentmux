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

    # -Headless: the plan, the refusals, and that the desktop app being installed does not stop it
    $hl = @("-Headless", "-DryRun", "-Version", "0.60.31")
    $lad = @{ LOCALAPPDATA = (Join-Path $work "lad-dry") }
    $headlessEnv = @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = "ai"; LOCALAPPDATA = $lad.LOCALAPPDATA }
    Expect "-Headless dry run names the zip" 0 "agentmux-headless-0.60.31-windows-x64.zip" $sh $headlessEnv $hl
    Expect "-Headless dry run: the folder, the junction and the PATH entry" 0 "junction" $sh $headlessEnv $hl
    Expect "-Headless dry run puts one entry on the PATH, for this user only" 0 "to your PATH (this user only)" $sh $headlessEnv $hl
    Expect "-Headless dry run points to agentmux setup" 0 "agentmux setup" $sh $headlessEnv $hl
    Expect-Absent "-Headless never uses winget" "winget install" $sh $headlessEnv $hl
    Expect "-Headless has no package-manager method" 1 "has no package-manager method" $sh $headlessEnv @("-Headless", "-Method", "package-manager", "-DryRun")
    $withDesktop = @{ AGENTMUX_INSTALL_INSTALLED = "setup"; AGENTMUX_INSTALL_WINGET = "ai"; LOCALAPPDATA = $lad.LOCALAPPDATA }
    Expect "-Headless goes ahead when the desktop app is installed" 0 "agentmux-headless-0.60.31-windows-x64.zip" $sh $withDesktop $hl
    Expect "-Help describes -Headless" 0 "-Headless" $sh $none @("-Help")

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

# ---- the SSH signature floor is the app's ----
$wantFloor = ""
foreach ($line in (Get-Content -LiteralPath (Join-Path $repoRoot "scripts/release-manifest.mjs"))) { if ($line -match '^export const SSHSIG_FLOOR = "(.*)";\s*$') { $wantFloor = $Matches[1] } }
$haveFloor = ""
foreach ($line in (Get-Content -LiteralPath $script)) { if ($line -match '^\s*\$SshsigFloor = "(.*)"\s*$') { $haveFloor = $Matches[1] } }
if ($wantFloor -and $wantFloor -eq $haveFloor) { Pass "SshsigFloor equals scripts/release-manifest.mjs's" } else { Fail "SshsigFloor equals scripts/release-manifest.mjs's" "'$haveFloor' differs from '$wantFloor'" }

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

        # ---- AgentMux Headless: the zip is checked, unpacked and linked ----
        $hver = "9.9.9"
        $zipName = "agentmux-headless-$hver-windows-x64.zip"
        $isWin = ($PSVersionTable.PSEdition -eq "Desktop") -or ($PSVersionTable.Platform -eq "Win32NT")
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        function New-HeadlessZip([string]$Path, [string[]]$Files, [string]$ExtraEntry) {
            $stage = Join-Path $work ("zipstage-" + [Guid]::NewGuid().ToString("N"))
            foreach ($f in $Files) {
                $full = Join-Path $stage $f
                New-Item -ItemType Directory -Path (Split-Path -Parent $full) -Force | Out-Null
                [IO.File]::WriteAllText($full, "stand-in $f", $utf8)
            }
            if (-not $Files) { New-Item -ItemType Directory -Path $stage -Force | Out-Null }
            [IO.Compression.ZipFile]::CreateFromDirectory($stage, $Path)
            if ($ExtraEntry) {
                $za = [IO.Compression.ZipFile]::Open($Path, "Update")
                try { $e = $za.CreateEntry($ExtraEntry); $w = New-Object IO.StreamWriter($e.Open()); $w.Write("x"); $w.Close() } finally { $za.Dispose() }
            }
        }
        # A signed release with the Headless zip as its one file (the manifest lists $Listed).
        function New-HeadlessRelease([string]$Dir, [string]$ZipPath, [string]$Listed) {
            if (-not $Listed) { $Listed = $zipName }
            New-Item -ItemType Directory -Path $Dir -Force | Out-Null
            Copy-Item -LiteralPath $ZipPath -Destination (Join-Path $Dir $zipName)
            $size = (Get-Item (Join-Path $Dir $zipName)).Length
            $sha = (Get-FileHash -LiteralPath (Join-Path $Dir $zipName) -Algorithm SHA256).Hash.ToLowerInvariant()
            $manifest = "{`n  `"format`": 1,`n  `"version`": `"$hver`",`n  `"tag`": `"v$hver`",`n  `"files`": [`n    {`n      `"name`": `"$Listed`",`n      `"size`": $size,`n      `"sha256`": `"$sha`"`n    }`n  ]`n}`n"
            [IO.File]::WriteAllText((Join-Path $Dir "AgentMux-$hver-manifest.json"), $manifest, $utf8)
            $msha = (Get-FileHash -LiteralPath (Join-Path $Dir "AgentMux-$hver-manifest.json") -Algorithm SHA256).Hash.ToLowerInvariant()
            $envPath = Join-Path $Dir "AgentMux-$hver-envelope.json"
            [IO.File]::WriteAllText($envPath, "{`n  `"format`": 1,`n  `"tag`": `"v$hver`",`n  `"manifest_sha256`": `"$msha`",`n  `"created_at`": `"2026-11-16T00:00:00Z`"`n}`n", $utf8)
            $sigBin = Join-Path $work "hsign.bin"
            & openssl pkeyutl -sign -inkey $priv -rawin -in $envPath -out $sigBin 2>$null
            [IO.File]::WriteAllText((Join-Path $Dir "AgentMux-$hver-envelope.json.sig"), ("t1 " + [Convert]::ToBase64String([IO.File]::ReadAllBytes($sigBin)) + "`n"), $utf8)
        }
        # The production package: made by scripts/package-server.sh and scripts/zip-deterministic.mjs (one top folder with explicit folder
        # entries, bin\ with the five programs, web\ with assets, VERSION), as the release carries it.
        $goodZip = Join-Path $here "fixtures\agentmux-headless-$hver-windows-x64.zip"
        $topName = "agentmux-headless-$hver-windows-x64"
        $flatZip = Join-Path $work "flat-headless.zip"
        New-HeadlessZip $flatZip @("bin\agentmux.exe", "bin\agentmux-srv.exe", "web\index.html") ""
        $hrel = Join-Path $work "hrel-$sh"
        New-HeadlessRelease $hrel $goodZip ""
        $lad2 = Join-Path $work "lad-$sh"
        function HeadlessEnv([string]$Dir, [string]$Local) {
            $e = Run-Release $sh $Dir $testKeys
            $e["LOCALAPPDATA"] = $Local
            $e["AGENTMUX_INSTALL_NO_PATH"] = "1"
            return $e
        }
        $hargs = @("-Headless", "-Version", $hver)

        # What needs a link runs where a junction can be made (Windows, and PowerShell on Linux makes a link of its own).
        $canLink = $false
        try {
            $probe = Join-Path $work ("probe-" + [Guid]::NewGuid().ToString("N")); New-Item -ItemType Directory -Path "$probe-t" -Force | Out-Null
            New-Item -ItemType Junction -Path $probe -Target "$probe-t" -ErrorAction Stop | Out-Null; $canLink = (Test-Path -LiteralPath $probe)
        } catch { $canLink = $false }
        if ($canLink) {
            Expect "Headless: a good zip is installed" 0 "installed AgentMux Headless $hver" $sh (HeadlessEnv $hrel $lad2) $hargs
            Expect "Headless: it says the package is not code-signed" 0 "not code-signed" $sh (HeadlessEnv $hrel (Join-Path $work "lad2b-$sh")) $hargs
            if (-not (Test-Path (Join-Path $lad2 "AgentMux\Headless\$hver\VERSION"))) { Fail "Headless: the package's top folder is stripped (VERSION is in the version folder)" "VERSION missing" } else { Pass "Headless: the package's top folder is stripped (VERSION is in the version folder)" }
            $hflat = Join-Path $work "hflat-$sh"; New-HeadlessRelease $hflat $flatZip ""
            Expect "Headless: a zip with bin and web at its root is taken too" 0 "installed AgentMux Headless $hver" $sh (HeadlessEnv $hflat (Join-Path $work "lad2c-$sh")) $hargs
            $folder = Join-Path $lad2 "AgentMux\Headless"
            if ((Test-Path (Join-Path $folder "$hver\bin\agentmux.exe")) -and (Test-Path (Join-Path $folder "current\bin\agentmux-srv.exe")) -and (Test-Path (Join-Path $folder "$hver\web\index.html"))) { Pass "Headless: the programs are in the version folder and reached through current" } else { Fail "Headless: the programs are in the version folder and reached through current" "missing under $folder" }
            $cur = Get-Item -LiteralPath (Join-Path $folder "current") -Force
            if ($cur.Attributes -band [IO.FileAttributes]::ReparsePoint) { Pass "Headless: current is a link" } else { Fail "Headless: current is a link" "not a reparse point" }
            Expect "Headless: running it again changes nothing" 0 "already installed" $sh (HeadlessEnv $hrel $lad2) $hargs
            Expect "Headless: -Force installs it again" 0 "installed AgentMux Headless $hver" $sh (HeadlessEnv $hrel $lad2) ($hargs + "-Force")
            # an older version folder is removed once the link has moved
            New-Item -ItemType Directory -Path (Join-Path $folder "9.9.8\bin") -Force | Out-Null
            Expect "Headless: an update moves the link" 0 "installed AgentMux Headless $hver" $sh (HeadlessEnv $hrel $lad2) ($hargs + "-Force")
            if (-not (Test-Path (Join-Path $folder "9.9.8"))) { Pass "Headless: the older version folder is removed" } else { Fail "Headless: the older version folder is removed" "still there" }
            # a "current" that is a real folder is never touched
            $lad3 = Join-Path $work "lad3-$sh"
            New-Item -ItemType Directory -Path (Join-Path $lad3 "AgentMux\Headless\current") -Force | Out-Null
            Expect "Headless: a current that is not a link is refused" 1 "not touching it" $sh (HeadlessEnv $hrel $lad3) $hargs
            if ($isWin) {
                # A version folder with a running program in it cannot be replaced: the install that is there stays whole.
                $held = Join-Path $folder "$hver\bin\agentmux-srv.exe"
                $fs = [IO.File]::Open($held, "Open", "Read", "None")
                try {
                    Expect "Headless: a folder in use is not replaced, and the script says so" 1 "unchanged" $sh (HeadlessEnv $hrel $lad2) ($hargs + "-Force")
                } finally { $fs.Dispose() }
                if ((Test-Path $held) -and (Test-Path (Join-Path $folder "current\bin\agentmux-srv.exe"))) { Pass "Headless: the install that was there is whole after a refused replace" } else { Fail "Headless: the install that was there is whole after a refused replace" "files are gone" }
            }
        } else {
            # No junction can be made here: the run must still unpack and check the package (top folder stripped, layout checked) and then say the link could not be made, never report an install.
            foreach ($z in @(@("real package, one top folder", $hrel), @("flat zip", (Join-Path $work "hflat-$sh")))) {
                if (-not (Test-Path $z[1])) { New-HeadlessRelease $z[1] $flatZip "" }
                $ladN = Join-Path $work ("lad-nolink-" + [Guid]::NewGuid().ToString("N"))
                $r = Invoke-Install $sh (HeadlessEnv $z[1] $ladN) $hargs
                $hasProgram = Test-Path (Join-Path $ladN "AgentMux\Headless\$hver\bin\agentmux.exe")
                if ($r.Code -ne 0 -and $r.Out.Contains("could not make the link") -and $hasProgram -and -not $r.Out.Contains("installed AgentMux Headless")) { Pass "Headless ($($z[0])): unpacked and checked; no link, so no 'installed'" } else { Fail "Headless ($($z[0])): unpacked and checked; no link, so no 'installed'" "code $($r.Code), program there: $hasProgram, $($r.Out)" }
            }
            Write-Host "skip - the install steps after the link run where a junction can be made"
        }

        $hbad = Join-Path $work "hbad-$sh"; Copy-Release $hrel $hbad
        [IO.File]::AppendAllText((Join-Path $hbad $zipName), "x")
        $lad4 = Join-Path $work "lad4-$sh"
        Expect "Headless: a changed zip is refused" 1 "bytes, the manifest says" $sh (HeadlessEnv $hbad $lad4) $hargs
        if (-not (Test-Path (Join-Path $lad4 "AgentMux\Headless\$hver")) -and -not (Test-Path (Join-Path $lad4 "AgentMux\Headless\current"))) { Pass "Headless: a refused zip leaves no version folder and no link" } else { Fail "Headless: a refused zip leaves no version folder and no link" "something was written" }

        $hunlisted = Join-Path $work "hunlisted-$sh"; New-HeadlessRelease $hunlisted $goodZip "Other.zip"
        Expect "Headless: a release whose manifest does not list the zip says so" 1 "no AgentMux Headless package for Windows" $sh (HeadlessEnv $hunlisted (Join-Path $work "lad5-$sh")) $hargs

        $hwrongsig = Join-Path $work "hwrongsig-$sh"; Copy-Release $hrel $hwrongsig
        $wrongKeyEnv = HeadlessEnv $hwrongsig (Join-Path $work "lad9-$sh"); $wrongKeyEnv["AGENTMUX_INSTALL_TEST_KEYS"] = "k1:m9HRZ4UOiBU2vyrjlHXj1D0CjfhAazI2DGdII9JSJLE="
        Expect "Headless: another key's signature is refused" 1 "signature does not match" $sh $wrongKeyEnv $hargs

        $notPkg = Join-Path $work "notpkg.zip"
        New-HeadlessZip $notPkg @("readme.txt") ""
        $hnotpkg = Join-Path $work "hnotpkg-$sh"; New-HeadlessRelease $hnotpkg $notPkg ""
        Expect "Headless: a zip that is not a Headless package is refused" 1 "is not an AgentMux Headless package" $sh (HeadlessEnv $hnotpkg (Join-Path $work "lad6-$sh")) $hargs

        $slipZip = Join-Path $work "slip.zip"
        New-HeadlessZip $slipZip @("bin\agentmux.exe", "bin\agentmux-srv.exe", "web\index.html") "..\evil.txt"
        $hslip = Join-Path $work "hslip-$sh"; New-HeadlessRelease $hslip $slipZip ""
        Expect "Headless: a zip with an entry outside its folder is refused" 1 "outside its own folder" $sh (HeadlessEnv $hslip (Join-Path $work "lad7-$sh")) $hargs
        if (-not (Test-Path (Join-Path $work "lad7-$sh\AgentMux\evil.txt")) -and -not (Test-Path (Join-Path $work "lad7-$sh\evil.txt"))) { Pass "Headless: nothing was written outside the folder" } else { Fail "Headless: nothing was written outside the folder" "evil.txt exists" }

        # The SSH signature, with this system's real ssh-keygen when it can sign (OpenSSH 8.0 or later): the same envelope, signed by a throwaway key.
        $realKeygen = Get-Command ssh-keygen -ErrorAction SilentlyContinue
        if ($realKeygen) {
            $sk = Join-Path $work "sshkey-$sh"
            & $realKeygen.Source -q -t ed25519 -N '""' -f $sk 2>$null | Out-Null
            if (-not (Test-Path -LiteralPath "$sk.pub")) { & $realKeygen.Source -q -t ed25519 -N "" -f $sk 2>$null | Out-Null }
            if (Test-Path -LiteralPath "$sk.pub") {
                $pubLine = (Get-Content -LiteralPath "$sk.pub" -TotalCount 1).Split(" ")[1]
                $blob = [Convert]::FromBase64String($pubLine)
                $sshKeys = "s1:" + [Convert]::ToBase64String($blob[($blob.Length - 32)..($blob.Length - 1)])
                $srel = Join-Path $work "hssh-$sh"; Copy-Release $hrel $srel
                $envFile = Join-Path $srel "AgentMux-$hver-envelope.json"
                Copy-Item -LiteralPath $envFile -Destination (Join-Path $work "env-to-sign.json") -Force
                & $realKeygen.Source -Y sign -f $sk -n agentmux-release (Join-Path $work "env-to-sign.json") 2>$null | Out-Null
                $signed = Join-Path $work "env-to-sign.json.sig"
                if (Test-Path -LiteralPath $signed) {
                    Copy-Item -LiteralPath $signed -Destination (Join-Path $srel "AgentMux-$hver-envelope.json.sshsig") -Force
                    $sshEnv = HeadlessEnv $srel (Join-Path $work "lad10-$sh"); $sshEnv["AGENTMUX_INSTALL_NO_OPENSSL"] = "1"; $sshEnv["AGENTMUX_INSTALL_TEST_KEYS"] = $sshKeys; $sshEnv["AGENTMUX_INSTALL_SSHSIG_FLOOR"] = "0.0.1"
                    $r = Invoke-Install $sh $sshEnv $hargs
                    if ($r.Out.Contains("signature of the release checked: Ed25519 (SSH signature), with ssh-keygen")) { Pass "SSH signature: a good one is checked with ssh-keygen" } else { Fail "SSH signature: a good one is checked with ssh-keygen" $r.Out }
                    # the same signature over a changed envelope is refused
                    $changed = Join-Path $work "hssh-changed-$sh"; Copy-Release $srel $changed
                    [IO.File]::AppendAllText((Join-Path $changed "AgentMux-$hver-envelope.json"), " ")
                    Expect "SSH signature: a signature of another envelope is refused" 1 "signature does not match" $sh (@{} + (HeadlessEnv $changed (Join-Path $work "lad11-$sh")) + @{ AGENTMUX_INSTALL_NO_OPENSSL = "1"; AGENTMUX_INSTALL_TEST_KEYS = $sshKeys; AGENTMUX_INSTALL_SSHSIG_FLOOR = "0.0.1" }) $hargs
                    # from the floor on, a release with no SSH signature is not installed
                    $nosig = Join-Path $work "hssh-none-$sh"; Copy-Release $srel $nosig
                    Remove-Item -LiteralPath (Join-Path $nosig "AgentMux-$hver-envelope.json.sshsig")
                    Expect "SSH signature: a release at or above the floor with none is refused" 1 "should carry" $sh (@{} + (HeadlessEnv $nosig (Join-Path $work "lad12-$sh")) + @{ AGENTMUX_INSTALL_NO_OPENSSL = "1"; AGENTMUX_INSTALL_TEST_KEYS = $sshKeys; AGENTMUX_INSTALL_SSHSIG_FLOOR = "0.0.1" }) $hargs
                } else { Write-Host "skip - ssh-keygen here cannot sign (-Y sign); the SSH signature checks are not run" }
            } else { Write-Host "skip - ssh-keygen here could not make a key; the SSH signature checks are not run" }
        } else { Write-Host "skip - no ssh-keygen on this system; the SSH signature checks are not run" }

        # No OpenSSL 3 and no usable ssh-keygen: Headless is never installed from hashes alone, and says what to do.
        $noSsl = HeadlessEnv $hrel (Join-Path $work "lad8-$sh"); $noSsl["AGENTMUX_INSTALL_NO_OPENSSL"] = "1"; $noSsl["AGENTMUX_INSTALL_SSH_KEYGEN"] = (Join-Path $work "no-such-ssh-keygen")
        Expect "no OpenSSL and no ssh-keygen: Headless is refused, never installed from hashes alone" 1 "not installed from hashes alone" $sh $noSsl $hargs
        Expect "that says what to do: the OpenSSH client or OpenSSL 3" 1 "OpenSSH client" $sh $noSsl $hargs
        if (-not (Test-Path (Join-Path $work "lad8-$sh\AgentMux\Headless\$hver"))) { Pass "no OpenSSL and no ssh-keygen: nothing was unpacked" } else { Fail "no OpenSSL and no ssh-keygen: nothing was unpacked" "a version folder exists" }
        # The desktop installer path keeps its hashes-only fallback, said plainly (and -RequireSignature still refuses it).
        $noSslDesktop = Run-Release $sh $good $testKeys; $noSslDesktop["AGENTMUX_INSTALL_NO_OPENSSL"] = "1"; $noSslDesktop["AGENTMUX_INSTALL_SSH_KEYGEN"] = (Join-Path $work "no-such-ssh-keygen")
        $r = Invoke-Install $sh $noSslDesktop $dl
        if ($r.Code -eq 0 -and $r.Out.Contains("signature NOT checked") -and $r.Out.Contains("what is checked instead")) { Pass "desktop installer, no OpenSSL and no ssh-keygen: the hashes only, said plainly" } else { Fail "desktop installer, no OpenSSL and no ssh-keygen: the hashes only, said plainly" $r.Out }
        Expect "desktop installer: -RequireSignature refuses when nothing can check" 1 "cannot be checked here" $sh $noSslDesktop ($dl + "-RequireSignature")

        # Headless below the floor: nothing to install, with a dry run too
        $floorEnv = HeadlessEnv $hrel (Join-Path $work "lad13-$sh")
        Expect "Headless below the floor: refused" 1 "is in releases from 0.60.31 on" $sh $floorEnv @("-Headless", "-Version", "0.60.30")
        Expect "Headless below the floor: refused in a dry run too" 1 "is in releases from 0.60.31 on" $sh $floorEnv @("-Headless", "-Version", "0.60.30", "-DryRun")
        Expect "Headless at the floor: goes ahead (dry run)" 0 "agentmux-headless-0.60.31-windows-x64.zip" $sh $floorEnv @("-Headless", "-Version", "0.60.31", "-DryRun")

        # A "latest" below the floor is not the latest: refused, on both paths; at or above, it is used
        $latestOld = @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = "none"; AGENTMUX_INSTALL_LATEST = "0.60.30"; LOCALAPPDATA = (Join-Path $work "lad14-$sh") }
        Expect "latest below the floor: the desktop path is refused" 1 "is older than 0.60.31" $sh $latestOld @("-Method", "download", "-DryRun")
        Expect "latest below the floor: Headless is refused" 1 "is older than 0.60.31" $sh $latestOld @("-Headless", "-DryRun")
        $latestOk = $latestOld.Clone(); $latestOk["AGENTMUX_INSTALL_LATEST"] = "0.60.31"
        Expect "latest at the floor: the desktop path uses it" 0 "AgentMux-0.60.31-x64-setup.exe" $sh $latestOk @("-Method", "download", "-DryRun")
        Expect "latest at the floor: Headless uses it" 0 "agentmux-headless-0.60.31-windows-x64.zip" $sh $latestOk @("-Headless", "-DryRun")
        Expect "an older release is still installable with -Version (the desktop path)" 0 "AgentMux-0.60.14-x64-setup.exe" $sh $latestOld @("-Method", "download", "-DryRun", "-Version", "0.60.14")

        # A setting that changes what is checked is named before anything is downloaded (its name, never its value)
        $named = @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = "none"; AGENTMUX_INSTALL_TEST_KEYS = "t1:SECRETVALUE123"; AGENTMUX_INSTALL_SSHSIG_FLOOR = "0.0.1"; LOCALAPPDATA = (Join-Path $work "lad15-$sh") }
        Expect "overrides are named in one line" 0 "warning: AGENTMUX_INSTALL_TEST_KEYS, AGENTMUX_INSTALL_SSHSIG_FLOOR change" $sh $named @("-Headless", "-DryRun", "-Version", "0.60.31")
        Expect-Absent "an override's value is never printed" "SECRETVALUE123" $sh $named @("-Headless", "-DryRun", "-Version", "0.60.31")
        Expect-Absent "no override, no warning" "warning:" $sh @{ AGENTMUX_INSTALL_INSTALLED = "none"; AGENTMUX_INSTALL_WINGET = "none"; LOCALAPPDATA = (Join-Path $work "lad16-$sh") } @("-Headless", "-DryRun", "-Version", "0.60.31")
    }
}

Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
Write-Host ""
if ($fails -eq 0) { Write-Host "all passed" } else { Write-Host "$fails failed"; exit 1 }
