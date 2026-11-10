# agentmux-install: install AgentMux on Windows from a terminal.
#
#   irm https://agentmux.ai/install.ps1 | iex
#
# It uses winget when there is one, so updates come from it:
#   winget install --id AgentMux.AI -e       once that package is in the winget catalogue
#   winget install --id 9P9QCXNNCRK3 --source msstore ...   the Microsoft Store listing, until then
# and otherwise downloads the installer and checks it against the release's signed manifest, then runs it silently
# (per user, no administrator rights).
#
# Plain text, no prompts, ASCII only. Works in Windows PowerShell 5.1 and PowerShell 7.
# Options (a script block, since a pipe cannot take any):
#   & ([scriptblock]::Create((irm https://agentmux.ai/install.ps1))) -DryRun
# Design: docs/specs/SPEC_ONE_COMMAND_INSTALL_ALL_PLATFORMS_2026_11_01.md

function Install-AgentMux {
    [CmdletBinding()]
    param(
        [string]$Version = "",
        [ValidateSet("auto", "package-manager", "download")][string]$Method = "auto",
        [switch]$DryRun,
        [switch]$NoLaunch,
        [switch]$Force,
        [switch]$RequireSignature,
        [switch]$Interactive,
        [switch]$Help
    )

    $ErrorActionPreference = "Stop"
    $ProgressPreference = "SilentlyContinue"

    $ScriptVersion = "1"
    $Repo = "agentmuxai/agentmux"
    $Releases = "https://github.com/$Repo/releases"
    $StoreId = "9P9QCXNNCRK3"
    $WingetId = "AgentMux.AI"
    $SetupKey = "{8654EA96-8EAA-4845-ADE9-352ADA712BC0}_is1"
    # The public keys the app pins to verify a release (crates/common/update-keys.json), as "<id>:<base64 of the 32 raw bytes>".
    # scripts/install/install.test.ps1 fails when this and that file disagree.
    $UpdateKeys = "k1:m9HRZ4UOiBU2vyrjlHXj1D0CjfhAazI2DGdII9JSJLE="

    function Say([string]$Text) { Write-Host "agentmux-install: $Text" }
    function Die([string]$Text) { throw "agentmux-install: error: $Text" }
    # Print a command the way the shell script does.
    function Show([string]$Text) { Write-Host "+ $Text" }

    if ($Help) {
        Write-Host @"
agentmux-install: install AgentMux on Windows.

Usage: install.ps1 [options]

  -Version <x.y.z>      install this release (default: the latest)
  -Method <m>           auto (default), package-manager or download
  -DryRun               print what it would do and change nothing
  -NoLaunch             do not start AgentMux when done
  -Force                install even if AgentMux is already installed
  -RequireSignature     fail when the release's signature cannot be checked here
  -Interactive          accepted and ignored: this script never prompts
  -Help                 this text

Environment: AGENTMUX_INSTALL_INSTALLED, AGENTMUX_INSTALL_WINGET, AGENTMUX_INSTALL_TEST_KEYS and
AGENTMUX_INSTALL_RELEASE_BASE and AGENTMUX_INSTALL_WINGET_EXE replace what it looks at or runs, and AGENTMUX_INSTALL_NO_RUN_SETUP stops
short of running the installer (used by its tests).
"@
        return
    }

    if ($Version -and $Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$') { Die "-Version must look like 0.60.14" }

    # ---- the platform ----
    if ($env:OS -ne "Windows_NT") { Die "this is not Windows: on macOS and Linux run  curl -fsSL https://agentmux.ai/install.sh | sh" }
    $arch = $env:PROCESSOR_ARCHITEW6432
    if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
    if ($arch -ne "AMD64" -and $arch -ne "ARM64") { Die "unsupported processor: $arch" }
    $modeText = ""
    if ($DryRun) { $modeText = ", dry run" }
    Say "agentmux-install $ScriptVersion`: windows $arch$modeText"
    if ($arch -eq "ARM64") { Say "note: there is no native arm64 build; the x64 build runs under Windows' emulation" }

    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }

    # ---- is it already installed? ----
    # Returns "setup" (the installer's per-user copy), "store" (the Microsoft Store's) or "".
    function Get-InstalledKind {
        if ($env:AGENTMUX_INSTALL_INSTALLED) {
            if ($env:AGENTMUX_INSTALL_INSTALLED -eq "none") { return "" }
            return $env:AGENTMUX_INSTALL_INSTALLED
        }
        foreach ($root in @("HKCU:", "HKLM:")) {
            foreach ($sub in @("Software\Microsoft\Windows\CurrentVersion\Uninstall", "Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall")) {
                if (Test-Path "$root\$sub\$SetupKey") { return "setup" }
            }
        }
        if ($env:LOCALAPPDATA -and (Test-Path (Join-Path $env:LOCALAPPDATA "Programs\AgentMux\agentmux.exe"))) { return "setup" }
        try { if (Get-AppxPackage -Name "AgentMux.AgentMux*" -ErrorAction Stop) { return "store" } } catch { }
        return ""
    }

    $installed = Get-InstalledKind
    if ($installed -and -not $Force) {
        if ($installed -eq "store") { $how = "from the Microsoft Store, which updates it" } else { $how = "with the installer: it updates from its version panel, or with  winget upgrade" }
        Say "AgentMux is already installed ($how)."
        Say "Nothing changed. Use -Force to install anyway."
        return
    }

    # ---- the tools ----
    function Test-Winget {
        if ($env:AGENTMUX_INSTALL_WINGET) { return ($env:AGENTMUX_INSTALL_WINGET -ne "none") }
        return [bool](Get-Command winget -ErrorAction SilentlyContinue)
    }

    # Whether the AgentMux.AI package is in the winget catalogue yet.
    function Test-WingetPackage {
        if ($env:AGENTMUX_INSTALL_WINGET) { return ($env:AGENTMUX_INSTALL_WINGET -eq "ai") }
        $old = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try { & winget show --id $WingetId -e --source winget --accept-source-agreements --disable-interactivity *> $null } finally { $ErrorActionPreference = $old }
        return ($LASTEXITCODE -eq 0)
    }

    function Get-Url([string]$Source, [string]$Destination) {
        if ($Source -notmatch '^https://') {
            Copy-Item -LiteralPath $Source -Destination $Destination -Force # a local file: the tests' fixtures
            return
        }
        try { Invoke-WebRequest -UseBasicParsing -Uri $Source -OutFile $Destination } catch { Die "could not download $Source" }
    }

    function Get-FileSha256([string]$Path) {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    }

    # The latest release's version, from where /releases/latest redirects to.
    function Get-LatestVersion {
        try {
            $req = [Net.HttpWebRequest]::Create("$Releases/latest")
            $req.AllowAutoRedirect = $false
            $req.Method = "HEAD"
            $resp = $req.GetResponse()
            $loc = $resp.Headers["Location"]
            $resp.Close()
        } catch { Die "could not look up the latest release" }
        $v = "$loc" -replace '^.*/v', ''
        if ($v -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$') { Die "the latest release's address ($loc) is not a version" }
        return $v
    }

    # True when this openssl can check an Ed25519 signature in one shot (OpenSSL 3).
    function Test-CanCheckSignature {
        if (-not (Get-Command openssl -ErrorAction SilentlyContinue)) { return $false }
        $old = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try { $help = (& openssl pkeyutl -help 2>&1 | Out-String) } finally { $ErrorActionPreference = $old }
        return ($help -match '-rawin')
    }

    # 0 checked and good, 1 checked and bad, 2 cannot check
    function Test-Signature([string]$Envelope, [string]$SigFile, [string]$Work) {
        if (-not (Test-CanCheckSignature)) { return 2 }
        $keys = $UpdateKeys
        if ($env:AGENTMUX_INSTALL_TEST_KEYS) { $keys = $env:AGENTMUX_INSTALL_TEST_KEYS }
        $old = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try {
            foreach ($entry in ($keys -split '\s+')) {
                if (-not $entry) { continue }
                $id = $entry.Substring(0, $entry.IndexOf(":"))
                $b64 = $entry.Substring($entry.IndexOf(":") + 1)
                $line = Get-Content -LiteralPath $SigFile | Where-Object { $_ -like "$id *" } | Select-Object -First 1
                if (-not $line) { continue }
                try {
                    $sig = [Convert]::FromBase64String($line.Substring($id.Length + 1).Trim())
                    # The 12 bytes that turn 32 raw Ed25519 key bytes into a DER public key.
                    $der = [byte[]](0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00) + [Convert]::FromBase64String($b64)
                } catch { continue }
                [IO.File]::WriteAllBytes((Join-Path $Work "sig.bin"), $sig)
                [IO.File]::WriteAllBytes((Join-Path $Work "key.der"), $der)
                & openssl pkey -pubin -inform DER -in (Join-Path $Work "key.der") -out (Join-Path $Work "key.pem") *> $null
                if ($LASTEXITCODE -ne 0) { continue }
                & openssl pkeyutl -verify -pubin -inkey (Join-Path $Work "key.pem") -rawin -in $Envelope -sigfile (Join-Path $Work "sig.bin") *> $null
                if ($LASTEXITCODE -eq 0) { return 0 }
                return 1
            }
        } finally { $ErrorActionPreference = $old }
        return 1
    }

    # Download a release file and check it against the signed manifest. Returns the path of the file.
    function Get-VerifiedFile([string]$Name, [string]$Work) {
        $base = $env:AGENTMUX_INSTALL_RELEASE_BASE
        if (-not $base) { $base = "$Releases/download/v$Version" }
        $manifestFile = Join-Path $Work "manifest.json"
        $envelopeFile = Join-Path $Work "envelope.json"
        $sigFile = Join-Path $Work "envelope.sig"
        Get-Url "$base/AgentMux-$Version-manifest.json" $manifestFile
        Get-Url "$base/AgentMux-$Version-envelope.json" $envelopeFile
        Get-Url "$base/AgentMux-$Version-envelope.json.sig" $sigFile

        $sigState = Test-Signature $envelopeFile $sigFile $Work
        if ($sigState -eq 0) { Say "signature of the release checked" }
        elseif ($sigState -eq 1) { Die "the release's signature does not match: not installing" }
        else {
            if ($RequireSignature) { Die "the release's signature cannot be checked here (it needs OpenSSL 3 on the path) and -RequireSignature was given" }
            Say "signature not checked: this system has no OpenSSL 3; checking the hashes only"
        }

        $envelope = Get-Content -LiteralPath $envelopeFile -Raw | ConvertFrom-Json
        if ($envelope.tag -ne "v$Version") { Die "the envelope is for another release" }
        if (-not $envelope.manifest_sha256 -or (Get-FileSha256 $manifestFile) -ne $envelope.manifest_sha256) { Die "the manifest is not the one the envelope names" }

        $manifest = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json
        $entry = $manifest.files | Where-Object { $_.name -eq $Name } | Select-Object -First 1
        if (-not $entry) { Die "the signed manifest does not list $Name" }

        Show "download $Name"
        $dest = Join-Path $Work $Name
        Get-Url "$base/$Name" $dest
        $size = (Get-Item -LiteralPath $dest).Length
        if ($size -ne [int64]$entry.size) { Remove-Item -LiteralPath $dest -Force; Die "$Name is $size bytes, the manifest says $($entry.size)" }
        if ((Get-FileSha256 $dest) -ne $entry.sha256) { Remove-Item -LiteralPath $dest -Force; Die "$Name does not match the manifest's SHA-256" }
        Say "$Name matches the signed manifest"
        return $dest
    }

    # ---- the ways to install ----
    function Install-Winget([string]$Id, [bool]$FromStore) {
        $wingetArgs = @("install", "--id", $Id)
        if ($FromStore) { $wingetArgs += @("--source", "msstore") } else { $wingetArgs += "-e" }
        $wingetArgs += @("--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity")
        Show "winget $($wingetArgs -join ' ')"
        if ($DryRun) { return $true }
        $wingetExe = "winget"
        if ($env:AGENTMUX_INSTALL_WINGET_EXE) { $wingetExe = $env:AGENTMUX_INSTALL_WINGET_EXE }
        $old = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        # Out-Host: winget's output is for the person (or agent) watching; left in the pipeline it would become part of the return value.
        try { & $wingetExe @wingetArgs | Out-Host } finally { $ErrorActionPreference = $old }
        return ($LASTEXITCODE -eq 0)
    }

    function Install-Download {
        if (-not $Version) { $Version = Get-LatestVersion }
        $name = "AgentMux-$Version-x64-setup.exe"
        Say "installing $Version with the installer, silently, for this user"
        if ($DryRun) {
            Show "download $name and check it against the signed manifest"
            Show "$name /VERYSILENT /NORESTART /SUPPRESSMSGBOXES /SP-"
            return
        }
        $work = Join-Path ([IO.Path]::GetTempPath()) ("agentmux-install-" + [Guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $work | Out-Null
        try {
            $file = Get-VerifiedFile $name $work
            if ($env:AGENTMUX_INSTALL_NO_RUN_SETUP) { Say "test mode: the installer was checked and not run"; return }
            Show "$name /VERYSILENT /NORESTART /SUPPRESSMSGBOXES /SP-"
            $p = Start-Process -FilePath $file -ArgumentList "/VERYSILENT", "/NORESTART", "/SUPPRESSMSGBOXES", "/SP-" -Wait -PassThru
            if ($p.ExitCode -ne 0) { Die "the installer exited with code $($p.ExitCode)" }
        } finally {
            Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    function Start-AgentMux([string]$Kind) {
        if ($NoLaunch -or $DryRun -or $env:AGENTMUX_INSTALL_NO_RUN_SETUP) { return }
        Say "starting AgentMux"
        if ($Kind -eq "store") {
            try {
                $app = Get-StartApps -Name "AgentMux" -ErrorAction Stop | Select-Object -First 1
                if ($app) { Start-Process "explorer.exe" "shell:AppsFolder\$($app.AppID)"; return }
            } catch { }
            Say "installed. Start it from the Start menu."
            return
        }
        $exe = Join-Path $env:LOCALAPPDATA "Programs\AgentMux\agentmux.exe"
        if (Test-Path $exe) { Start-Process -FilePath $exe } else { Say "installed. Start it from the Start menu." }
    }

    # ---- go ----
    # Everything above only defines functions; nothing is installed until here, so a download cut short (irm | iex) does nothing.
    # What is installed decides which way in is open: the Store id is the Store's copy, AgentMux.AI and the download are the
    # installer's, and the two kinds are never mixed.
    $kind = "setup"
    $done = $false
    $useWinget = ($Method -ne "download") -and (Test-Winget)
    $useStore = $false
    if ($useWinget) {
        $inCatalogue = Test-WingetPackage
        if ($installed -eq "store") { $useStore = $true }
        elseif ($installed -eq "setup") { $useWinget = $inCatalogue }
        else { $useStore = -not $inCatalogue }
    }
    if ($useWinget) {
        if ($useStore) {
            $kind = "store"
            $done = Install-Winget $StoreId $true
        } else {
            $done = Install-Winget $WingetId $false
        }
        if (-not $done) {
            if ($Method -eq "package-manager") { Die "winget could not install AgentMux" }
            Say "winget could not install it; using the installer instead"
            $kind = "setup"
        }
    } elseif ($Method -eq "package-manager") {
        if (Test-Winget) { Die "the installer's copy is installed and winget has no AgentMux.AI package yet: use -Method download" }
        Die "winget was not found"
    }
    if (-not $done) {
        if ($installed -eq "store") { Die "the Microsoft Store copy is installed; remove it first (the two kinds do not mix)" }
        Install-Download
    }
    Start-AgentMux $kind
    Say "done"
}

# A bad option is a parameter-binding error: make it stop the script (a non-zero exit), and leave nothing behind in the
# session when this ran through  irm | iex.
$previousPreference = $ErrorActionPreference
$ErrorActionPreference = "Stop"
try { Install-AgentMux @args } catch {
    $message = $_.Exception.Message
    if ($message -notlike "agentmux-install:*") { $message = "agentmux-install: error: $message (try -Help)" }
    throw $message
} finally {
    $ErrorActionPreference = $previousPreference
    Remove-Item -Path Function:\Install-AgentMux -ErrorAction SilentlyContinue
}
