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
# With -Headless it installs AgentMux Headless instead (no desktop app: it runs in the background and its web UI is a browser on its
# own loopback address), from the same signed release, for this user only, no administrator rights:
#   & ([scriptblock]::Create((irm https://agentmux.ai/install.ps1))) -Headless
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
        [switch]$Headless,
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
    # The first release that carries AgentMux-<v>-envelope.json.sshsig: the same envelope signed by the same key in SSH signature form,
    # which this system's ssh-keygen can check where there is no OpenSSL 3 (most Windows machines). From this release on, a release
    # without one is not installed. scripts/install/install.test.ps1 fails when this and install.sh's value differ.
    $SshsigFloor = "0.60.31"
    $SshsigNamespace = "agentmux-release"
    $ssh = @{ Why = ""; Out = "" }

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
  -Headless             install AgentMux Headless (no desktop app) for this user, from the signed release:
                        %LOCALAPPDATA%\AgentMux\Headless\<version>, a "current" junction to it, and its bin folder on your PATH.
                        Run it again to update. Then run  agentmux setup
  -DryRun               print what it would do and change nothing
  -NoLaunch             do not start AgentMux when done
  -Force                install even if AgentMux is already installed
  -RequireSignature     fail when the release's signature cannot be checked here
  -Interactive          accepted and ignored: this script never prompts
  -Help                 this text

Environment: AGENTMUX_INSTALL_INSTALLED, AGENTMUX_INSTALL_WINGET, AGENTMUX_INSTALL_TEST_KEYS and
AGENTMUX_INSTALL_RELEASE_BASE, AGENTMUX_INSTALL_SSH_KEYGEN, AGENTMUX_INSTALL_SSHSIG_FLOOR, AGENTMUX_INSTALL_NO_OPENSSL, AGENTMUX_INSTALL_LATEST, AGENTMUX_INSTALL_NO_PATH and AGENTMUX_INSTALL_WINGET_EXE replace what it looks at or runs, and AGENTMUX_INSTALL_NO_RUN_SETUP stops
short of running the installer (used by its tests).
"@
        return
    }

    if ($Headless -and $Method -eq "package-manager") { Die "-Headless installs from the release download: it has no package-manager method" }
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

    # A setting that replaces the keys, the floor, the place a release is downloaded from, the tool that checks a signature or the latest version
    # is for tests: when one is set, it is said before anything is downloaded (its name, never its value).
    $overridden = @("AGENTMUX_INSTALL_TEST_KEYS", "AGENTMUX_INSTALL_SSHSIG_FLOOR", "AGENTMUX_INSTALL_RELEASE_BASE", "AGENTMUX_INSTALL_SSH_KEYGEN", "AGENTMUX_INSTALL_NO_OPENSSL", "AGENTMUX_INSTALL_LATEST") | Where-Object { [Environment]::GetEnvironmentVariable($_) }
    if ($overridden) { Say "warning: $($overridden -join ', ') changes what this script checks or where it downloads from (a setting for tests). Unset it to install as released" }

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

    # Headless runs beside the desktop app of the same version, so the desktop app being installed is not a reason to stop.
    $installed = ""
    if (-not $Headless) { $installed = Get-InstalledKind }
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

    # The latest release's version, from where /releases/latest redirects to. This script ships with a release at or above the SSH
    # signature floor, so a "latest" below it is not the latest: it is refused, on both paths. An older release is given with -Version.
    function Get-LatestVersion {
        if ($env:AGENTMUX_INSTALL_LATEST) { $v = $env:AGENTMUX_INSTALL_LATEST }
        else {
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
        }
        if ($v -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$') { Die "the latest version ($v) is not a version" }
        $floor = Get-SshsigFloorVersion
        if (-not (Test-VersionAtLeast $v $floor)) { Die "the latest release the network gives ($v) is older than $floor, which this script ships with, so it is not the latest: not installing. If you mean an older release, give it with -Version" }
        return $v
    }

    # True when this openssl can check an Ed25519 signature in one shot (OpenSSL 3).
    function Test-CanCheckSignature {
        if ($env:AGENTMUX_INSTALL_NO_OPENSSL) { return $false } # for the tests: a system with no OpenSSL 3
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

    # Is x.y.z (maybe with a suffix) at or above the floor? A suffix on the last part is ignored, so 0.60.31-rc1 compares as 0.60.31:
    # at the floor, never below it (a pre-release of the floor's version must carry the signature too).
    function Test-VersionAtLeast([string]$V, [string]$Min) {
        $a = @($V -split '\.')
        $b = @($Min -split '\.')
        for ($i = 0; $i -lt 3; $i++) {
            $na = 0
            $nb = 0
            if ($i -lt $a.Count -and $a[$i] -match '^([0-9]+)') { $na = [int]$Matches[1] }
            if ($i -lt $b.Count -and $b[$i] -match '^([0-9]+)') { $nb = [int]$Matches[1] }
            if ($na -ne $nb) { return ($na -gt $nb) }
        }
        return $true
    }

    function Get-SshsigFloorVersion {
        if ($env:AGENTMUX_INSTALL_SSHSIG_FLOOR) { return $env:AGENTMUX_INSTALL_SSHSIG_FLOOR }
        return $SshsigFloor
    }

    function Get-SshKeygen {
        if ($env:AGENTMUX_INSTALL_SSH_KEYGEN) { return $env:AGENTMUX_INSTALL_SSH_KEYGEN }
        $c = Get-Command ssh-keygen -ErrorAction SilentlyContinue
        if ($c) { return $c.Source }
        return $null
    }

    # True when this system's ssh-keygen can check an SSH signature (-Y verify, OpenSSH 8.0 and later).
    function Test-CanCheckSshsig([string]$Exe) {
        if (-not $Exe) { return $false }
        $old = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try { $out = (& $Exe -Y verify 2>&1 | Out-String) } catch { return $false } finally { $ErrorActionPreference = $old }
        return ($out -notmatch 'illegal option|invalid option|unknown option|unsupported option')
    }

    # 0 checked and good, 1 checked and bad, 2 cannot check (no ssh-keygen -Y, or a release from before the floor: $ssh.Why says which).
    # From the floor on, a missing .sshsig is a failure, not a fallback. The signer is only ever one of the pinned keys, put in an
    # allowed-signers file here; the key carried inside the signature is checked against them by ssh-keygen, never trusted.
    function Test-Sshsig([string]$Envelope, [string]$Base, [string]$Work) {
        $ssh.Why = ""
        $ssh.Out = ""
        $exe = Get-SshKeygen
        if (-not (Test-CanCheckSshsig $exe)) { $ssh.Why = "tool"; return 2 }
        $floor = Get-SshsigFloorVersion
        if (-not (Test-VersionAtLeast $Version $floor)) { $ssh.Why = "floor"; return 2 }
        $sshsig = Join-Path $Work "envelope.sshsig"
        try { Get-Url "$Base/AgentMux-$Version-envelope.json.sshsig" $sshsig } catch {
            Die "this release ($Version) should carry AgentMux-$Version-envelope.json.sshsig (every release from $SshsigFloor on does) and it could not be downloaded: not installing"
        }
        $keys = $UpdateKeys
        if ($env:AGENTMUX_INSTALL_TEST_KEYS) { $keys = $env:AGENTMUX_INSTALL_TEST_KEYS }
        $lines = @()
        foreach ($entry in ($keys -split '\s+')) {
            if (-not $entry) { continue }
            try { $raw = [Convert]::FromBase64String($entry.Substring($entry.IndexOf(":") + 1)) } catch { continue }
            if ($raw.Length -ne 32) { continue }
            # The key as an SSH key blob: the string "ssh-ed25519" (4-byte length 11), then the 32 raw bytes (length 32).
            $blob = [byte[]](0, 0, 0, 11) + [Text.Encoding]::ASCII.GetBytes("ssh-ed25519") + [byte[]](0, 0, 0, 32) + $raw
            $lines += ("{0} namespaces=`"{0}`" ssh-ed25519 {1}" -f $SshsigNamespace, [Convert]::ToBase64String($blob))
        }
        $allowed = Join-Path $Work "allowed_signers"
        [IO.File]::WriteAllText($allowed, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
        # The envelope goes to ssh-keygen's standard input as its exact bytes (a redirect that adds a line ending would fail a good release).
        $psi = New-Object Diagnostics.ProcessStartInfo
        $psi.FileName = $exe
        $psi.Arguments = '-Y verify -f "' + $allowed + '" -I ' + $SshsigNamespace + ' -n ' + $SshsigNamespace + ' -s "' + $sshsig + '"'
        $psi.UseShellExecute = $false
        $psi.CreateNoWindow = $true
        $psi.RedirectStandardInput = $true
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $proc = [Diagnostics.Process]::Start($psi)
        $bytes = [IO.File]::ReadAllBytes($Envelope)
        try { $proc.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length); $proc.StandardInput.Close() } catch { }
        $outText = $proc.StandardOutput.ReadToEnd()
        $errText = $proc.StandardError.ReadToEnd()
        $proc.WaitForExit()
        if ($proc.ExitCode -eq 0) { return 0 }
        foreach ($text in @($outText, $errText)) {
            if (-not $ssh.Out -and $text) { $ssh.Out = ("" + ($text -split "`r?`n" | Select-Object -First 1)).Trim() }
        }
        return 1
    }

    # Download a release file and check it against the signed manifest. Returns the path of the file.
    function Get-VerifiedFile([string]$Name, [string]$Work, [switch]$MustCheck) {
        $base = $env:AGENTMUX_INSTALL_RELEASE_BASE
        if (-not $base) { $base = "$Releases/download/v$Version" }
        $manifestFile = Join-Path $Work "manifest.json"
        $envelopeFile = Join-Path $Work "envelope.json"
        $sigFile = Join-Path $Work "envelope.sig"
        Get-Url "$base/AgentMux-$Version-manifest.json" $manifestFile
        Get-Url "$base/AgentMux-$Version-envelope.json" $envelopeFile
        Get-Url "$base/AgentMux-$Version-envelope.json.sig" $sigFile

        # The signature is checked with OpenSSL 3 when this system has it, else with ssh-keygen (the SSH signature of the same envelope).
        $sigState = Test-Signature $envelopeFile $sigFile $Work
        $how = "Ed25519, with OpenSSL"
        if ($sigState -eq 2) { $sigState = Test-Sshsig $envelopeFile $base $Work; $how = "Ed25519 (SSH signature), with ssh-keygen" }
        if ($sigState -eq 0) { Say "signature of the release checked: $how" }
        elseif ($sigState -eq 1) {
            $extra = ""
            if ($ssh.Out) { $extra = " (ssh-keygen says: $($ssh.Out))" }
            Die "the release's signature does not match: not installing$extra"
        } else {
            $why = "there is no ssh-keygen of OpenSSH 8.0 or newer to check the SSH signature that releases from $SshsigFloor on carry"
            if ($ssh.Why -eq "floor") { $why = "this release ($Version) is older than $SshsigFloor, the first with an SSH signature" }
            if ($MustCheck) { Die "the release's signature cannot be checked here (no OpenSSL 3 on the path, and $why). AgentMux Headless is not installed from hashes alone: install the Windows OpenSSH client (Settings, Apps, Optional features) or OpenSSL 3, and run this again" }
            if ($RequireSignature) { Die "the release's signature cannot be checked here (no OpenSSL 3 on the path, and $why) and -RequireSignature was given" }
            Say "signature NOT checked: this system has no OpenSSL 3, and $why"
            Say "what is checked instead: the file's size and SHA-256 against the manifest. That catches a damaged download, not a release that was replaced upstream"
        }

        $envelope = Get-Content -LiteralPath $envelopeFile -Raw | ConvertFrom-Json
        if ($envelope.tag -ne "v$Version") { Die "the envelope is for another release" }
        if (-not $envelope.manifest_sha256 -or (Get-FileSha256 $manifestFile) -ne $envelope.manifest_sha256) { Die "the manifest is not the one the envelope names" }

        $manifest = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json
        $entry = $manifest.files | Where-Object { $_.name -eq $Name } | Select-Object -First 1
        if (-not $entry) {
            if ($Name -like "agentmux-headless-*") { Die "this release has no AgentMux Headless package for Windows (the signed manifest does not list $Name)" }
            Die "the signed manifest does not list $Name"
        }

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

    # ---- AgentMux Headless (-Headless) ----
    # %LOCALAPPDATA%\AgentMux\Headless\<version>\ holds the zip's contents (bin\, web\); "current" is a directory junction to it (a
    # junction needs no administrator rights, a symlink would); the one PATH entry is ...\Headless\current\bin. Everything stays in
    # the profile: nothing runs as administrator and nothing is written outside it. `agentmux setup` registers the sign-in task.

    # True when $Path is a junction (or link) to $Target.
    function Test-LinkTo([string]$Path, [string]$Target) {
        $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
        if (-not $item -or -not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { return $false }
        $t = @($item.Target) | Select-Object -First 1
        if (-not $t) { return $false }
        return ([IO.Path]::GetFullPath("$t").TrimEnd('\', '/') -eq [IO.Path]::GetFullPath($Target).TrimEnd('\', '/'))
    }

    # Point the "current" junction at $Target. A folder that is not a link is never touched.
    function Set-CurrentJunction([string]$Path, [string]$Target) {
        $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
        if ($item) {
            if (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { Die "$Path exists and is not a link made by this installer: not touching it" }
            [IO.Directory]::Delete($Path) # removes the link, not the folder it points to
        }
        New-Item -ItemType Junction -Path $Path -Target $Target | Out-Null
        # Read it back: a link that was not made must not be reported as installed.
        if (-not (Test-LinkTo $Path $Target)) { Die "could not make the link $Path to $Target" }
    }

    # Add $Dir to the user's PATH once (any entry that expands to it counts), keeping the entries as they were written.
    function Add-ToUserPath([string]$Dir) {
        if ($env:AGENTMUX_INSTALL_NO_PATH) { Say "test mode: your PATH was not changed"; return }
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
        try {
            $raw = [string]$key.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $parts = @($raw -split ';' | Where-Object { $_ })
            $has = $false
            foreach ($part in $parts) {
                if ([Environment]::ExpandEnvironmentVariables($part).TrimEnd('\').Equals($Dir.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) { $has = $true }
            }
            if (-not $has) { $key.SetValue("Path", (($parts + $Dir) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString) }
        } finally { $key.Close() }
        # Setting and clearing a throwaway user variable makes Windows tell the running programs that the environment changed.
        [Environment]::SetEnvironmentVariable("AGENTMUX_PATH_NOTIFY", "1", "User")
        [Environment]::SetEnvironmentVariable("AGENTMUX_PATH_NOTIFY", $null, "User")
        if (-not (($env:Path -split ';') -contains $Dir)) { $env:Path = "$env:Path;$Dir" }
    }

    function Install-Headless {
        if (-not $env:LOCALAPPDATA) { Die "LOCALAPPDATA is not set: there is no profile folder to install into" }
        if (-not $Version) { $Version = Get-LatestVersion }
        $floor = Get-SshsigFloorVersion
        if (-not (Test-VersionAtLeast $Version $floor)) { Die "AgentMux Headless for Windows is in releases from $floor on, and $Version is older: there is nothing to install" }
        $root = Join-Path $env:LOCALAPPDATA "AgentMux\Headless"
        $target = Join-Path $root $Version
        $current = Join-Path $root "current"
        $binDir = Join-Path $current "bin"
        $name = "agentmux-headless-$Version-windows-x64.zip"
        Say "installing AgentMux Headless $Version into $target"
        if ($DryRun) {
            Show "download $name and check it against the signed manifest"
            Show "unpack it into $target"
            Show "junction $current -> $target"
            Show "add $binDir to your PATH (this user only)"
            Show "remove the folders of other versions under $root"
            Say "next: open a new PowerShell window, then run  agentmux setup"
            return
        }
        if (-not $Force -and (Test-Path -LiteralPath (Join-Path $target "bin\agentmux.exe")) -and (Test-LinkTo $current $target)) {
            Say "AgentMux Headless $Version is already installed: $target"
            Say "Nothing changed. Use -Force to install it again."
            return
        }
        $work = Join-Path ([IO.Path]::GetTempPath()) ("agentmux-install-" + [Guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $work | Out-Null
        New-Item -ItemType Directory -Path $root -Force | Out-Null
        # Unpacked beside its final place (the same drive), then moved: a download or unpack that fails leaves nothing half-installed.
        $stage = Join-Path $root (".stage-" + [Guid]::NewGuid().ToString("N"))
        try {
            $zip = Get-VerifiedFile $name $work -MustCheck
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $archive = [IO.Compression.ZipFile]::OpenRead($zip)
            try {
                foreach ($entry in $archive.Entries) {
                    if ($entry.FullName -match '(^|[\\/])\.\.([\\/]|$)' -or $entry.FullName -match '^([A-Za-z]:|[\\/])') { Die "$name has an entry outside its own folder ($($entry.FullName)): not installing" }
                }
            } finally { $archive.Dispose() }
            [IO.Compression.ZipFile]::ExtractToDirectory($zip, $stage)
            # The package is one top folder (agentmux-headless-<v>-windows-x64) holding bin\, web\ and VERSION; a zip with those at its
            # root is taken as well. Whatever is inside is moved to its place as the version folder.
            $pkg = $stage
            if (-not (Test-Path -LiteralPath (Join-Path $stage "bin"))) {
                $tops = @(Get-ChildItem -LiteralPath $stage -Force)
                if ($tops.Count -eq 1 -and $tops[0].PSIsContainer) { $pkg = $tops[0].FullName }
            }
            foreach ($want in @("bin\agentmux.exe", "bin\agentmux-srv.exe", "web\index.html")) {
                if (-not (Test-Path -LiteralPath (Join-Path $pkg $want))) { Die "$name is not an AgentMux Headless package (no $want)" }
            }
            # An existing folder of this version (-Force) is renamed aside, not deleted in place: a folder with a running program in it
            # cannot be renamed, so the install that is there is left whole and the script stops; a delete would have removed what it
            # could and left a broken install behind the link.
            $aside = ""
            if (Test-Path -LiteralPath $target) {
                $aside = Join-Path $root (".old-" + [Guid]::NewGuid().ToString("N"))
                try { [IO.Directory]::Move($target, $aside) } catch { Die "could not replace $target (is AgentMux Headless running from it? run  agentmux stop  and try again); the install that is there is unchanged" }
            }
            try {
                Move-Item -LiteralPath $pkg -Destination $target
                Set-CurrentJunction $current $target
            } catch {
                if ($aside -and -not (Test-Path -LiteralPath $target)) { try { [IO.Directory]::Move($aside, $target); $aside = "" } catch { } }
                throw
            }
            if ($aside) { Remove-Item -LiteralPath $aside -Recurse -Force -ErrorAction SilentlyContinue }
        } finally {
            Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
        }
        Add-ToUserPath $binDir
        # The folders of the other versions go once the link has moved. One that is in use stays, and is named.
        foreach ($dir in @(Get-ChildItem -LiteralPath $root -Directory -Force | Where-Object { $_.Name -ne $Version -and $_.Name -match '^[0-9]+\.[0-9]+\.[0-9]+' })) {
            $old = $dir.FullName
            try { Remove-Item -LiteralPath $old -Recurse -Force -ErrorAction Stop } catch { Say "could not remove the old folder $old (is AgentMux Headless running from it?): delete it once it has stopped" }
        }
        Say "installed AgentMux Headless ${Version}: $target"
        Say "this package is not code-signed: Windows may ask before it lets the programs run the first time"
        Say "next: open a new PowerShell window (so your PATH is read), then run  agentmux setup  to choose a port, start AgentMux Headless and sign in"
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
    if ($Headless) {
        Install-Headless
        Say "done"
        return
    }
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
