# Spec: install AgentMux with one copyable command on Windows, macOS and Linux

**Status:** active — Phase 1 (`install.sh`, its tests and the release asset) ships in PR #6412 and phase 1b (`install.ps1`, its tests and the release asset) after it; the landing page section, the `AgentMux.AI` winget submission, the Homebrew tap and the README/docs commands are not built. The recommendations are approved (2026-11-04). Verified 2026-11-04.
**Date:** 2026-11-04
**Author:** Winny (winget)
**Verified against:** `agentmux` `main` @ `9a21a01` and release `v0.60.14` (2026-11-03). Everything under "Today" was read from the repository or fetched from the live endpoints on 2026-11-04, from a Linux container. Nothing Windows- or macOS-specific could be run from there; those claims are marked **verify** and are the first items of the build.
**Builds on:** [SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md](SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md) (the apt/dnf repository), [SPEC_WINGET_PACKAGE_BOOTSTRAP_2026_09_10.md](SPEC_WINGET_PACKAGE_BOOTSTRAP_2026_09_10.md) (the `AgentMux.AI` id), [SPEC_MSSTORE_AUTOMATED_RELEASE_2026_06_29.md](SPEC_MSSTORE_AUTOMATED_RELEASE_2026_06_29.md), [SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md](SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md) (the signed manifest).

## 1. Ask

The operator, 2026-11-01: "also update the landing page for linux users, make sure they have a command line they can easily copy, which is popular with agentic tools. can we do the same with winget/windows macos? since that interface is very popular. can they get the full agentmux experience through a command line?"

So: one command per platform that installs AgentMux and keeps it updated, a script form that works anywhere, and a landing page section that shows the right one first with a copy button.

## 2. Short answer

| | Primary command (copy this) | Works today? | What's missing |
|---|---|---|---|
| Linux, Debian/Ubuntu | `curl -fsSL https://agentmux.ai/install.sh \| sh` (adds the signed apt repository, then `apt install agentmux`) | The repository is live at `packages.agentmux.ai` (signed, 0.60.11 to 0.60.14) | The script, the page |
| Linux, Fedora/openSUSE | the same line (adds the dnf/zypper repository, installs) | Same | Same |
| macOS (Apple Silicon) | `brew install --cask agentmuxai/tap/agentmux` | No Homebrew presence | A tap repository, the cask, the bump job |
| Windows | now: `winget install --id 9P9QCXNNCRK3 --source msstore --accept-package-agreements --accept-source-agreements`; after the first `AgentMux.AI` submission merges: `winget install --id AgentMux.AI -e` | The Store listing is live. **verify** the command on a clean Windows machine. `AgentMux.AI` was never submitted | The first winget-pkgs submission, then the release job |
| Anywhere | `curl -fsSL https://agentmux.ai/install.sh \| sh` (macOS, Linux) and `irm https://agentmux.ai/install.ps1 \| iex` (Windows) | No | Both scripts |

Recommendations, each answering an open question (§10 lists them as decisions):

1. **The script is a front end for the package managers, not a second installer.** It picks the platform's own package manager, so updates always come from that manager (§6). Only when none exists does it download a release file.
2. **Scripts ship as release assets, so the signed manifest covers them,** and agentmux.ai serves copies of the latest release's (§5.3).
3. **Windows leads with the Store command today and switches to `AgentMux.AI` once it exists,** for the reasons in §4.1.
4. **No cost and no new external account** is needed (§9).

## 3. Today (verified 2026-11-04)

- **Release files** (`v0.60.14`): `AgentMux-<v>-x64-setup.exe`, `agentmux-<v>-x64-portable.7z`, `AgentMux_<v>_x64.msix`, `AgentMux_<v>_arm64.dmg` (Apple Silicon only; there is no Intel build), `AgentMux_<v>_amd64.AppImage`, `.deb`, `.rpm`, `-portable.tar.xz` (all x86-64; there is no arm64 Linux build), plus `AgentMux-<v>-manifest.json`, `-envelope.json` and `-envelope.json.sig`. The manifest lists every file with its size and SHA-256; the envelope pins the manifest's hash and carries an Ed25519 signature that the app verifies with a key it ships.
- **Linux repository:** `https://packages.agentmux.ai` serves `apt/` (`dists/stable`, signed `InRelease`), `rpm/x86_64/` (signed `repodata`, versions 0.60.11 to 0.60.14 kept), the key as `agentmux-archive-keyring.gpg` (apt) and `agentmux.asc` (dnf), and ready-made client files `agentmux.sources` (deb822, `Signed-By` the keyring) and `agentmux.repo` (`gpgcheck=1`, `repo_gpgcheck=1`). The packages do not yet subscribe themselves to it (the upgrade spec's Repo-2), and declare no dependencies.
- **Windows:** the Store listing for `AgentMux.AgentMux` (Store id `9P9QCXNNCRK3`) is live: its page answers and is titled "AgentMux - Free download and install on Windows". The landing page already leads Windows visitors to it. `microsoft/winget-pkgs` has no `AgentMux` manifest; the `winget` job in `release-core.yml` is commented out until a first `wingetcreate new` submission merges. The Inno Setup `setup.exe` and the portable `.7z` are not Authenticode-signed (the landing page says so: "code signing is waiting on Microsoft's registration, so SmartScreen may warn"); the Store MSIX is signed by Microsoft.
- **macOS:** the `.dmg` is Developer ID signed and notarized (`build-macos.yml`). There is no tap and no cask; `AgentMux` is not a cask in homebrew/cask (`formulae.brew.sh` answers 404 for it).
- **Install scripts:** `agentmux.ai/install.sh` and `/install.ps1` answer 404.
- **The updater** (`crates/srv/src/backend/update/install.rs`) knows eight install kinds, among them `WindowsSetup`, `WindowsStore`, `MacApp`, `LinuxPackage`, `LinuxAppImage` and the portables, and treats each differently (the Store updates itself; a `.deb`/`.rpm` goes through the `pkexec` helper; a `.app` is replaced in place unless its folder isn't writable).
- **Repository size:** 24 stars, 3 forks, 2 watchers. That is well under what Homebrew asks of a project before it accepts its cask into homebrew/cask (see §4.2).

## 4. Per platform

### 4.1 Windows

**Primary command.** Two ways to install exist or will, and they are different installs (§6 explains why that matters):

| | Store (`9P9QCXNNCRK3`) | `AgentMux.AI` (Inno `setup.exe`) |
|---|---|---|
| Command | `winget install --id 9P9QCXNNCRK3 --source msstore --accept-package-agreements --accept-source-agreements` | `winget install --id AgentMux.AI -e` |
| Exists | today | after the first submission merges (typically days; Microsoft's queue) |
| Signature | signed by Microsoft | unsigned; winget checks the SHA-256 in its manifest, and SmartScreen's behaviour for a winget-run installer is a **verify** item |
| New version reaches users | after Store certification of each release (hours to days) | a bump PR per release, merged within about a day; the in-app updater also works |
| Updater kind | `WindowsStore`: the Store updates it; the panel does not download | `WindowsSetup`: the in-app updater and `winget upgrade` both work |

Recommendation: show the **Store command first until `AgentMux.AI` is merged and has survived one release bump, then switch the page to `winget install --id AgentMux.AI -e`** and keep the Store command as the second tab. Reasons: the Store command is signed and works now; `AgentMux.AI` gives the same bytes as the direct download, so the in-app updater and everything the app does for `WindowsSetup` apply, and it tracks releases faster than certification. Both keep the page honest about the unsigned installer. The msstore form needs `--source msstore` and the two `--accept-*` flags so it runs unattended (agentic tools can't answer the prompts); the id may be passed as `9P9QCXNNCRK3`. **verify** on a clean Windows 11 machine that it installs and launches.

**The first `AgentMux.AI` submission** (through the flow in [SPEC_WINGET_PACKAGE_BOOTSTRAP_2026_09_10.md](SPEC_WINGET_PACKAGE_BOOTSTRAP_2026_09_10.md) §4): `wingetcreate new` against `AgentMux-<v>-x64-setup.exe` of the then-latest release, with `InstallerType: inno`, `Scope: user` or `machine` as the installer does, a stable `AppsAndFeaturesEntries` (`DisplayName`, `Publisher`, `ProductCode` or the Inno `AppId`) so that an in-app update and a `winget upgrade` don't end up as two entries. The Inno `AppId` is fixed (`{{8654EA96-8EAA-4845-ADE9-352ADA712BC0}}`, from `packaging/windows/agentmux.iss`), so the uninstall key `{8654EA96-8EAA-4845-ADE9-352ADA712BC0}_is1` is the same in every version. But `UninstallDisplayName` is `AgentMux <version>`, so the manifest must match on `ProductCode` and not on `DisplayName` (a `DisplayName` match breaks at every release). The installer is per-user (`PrivilegesRequired=lowest`, under `%LOCALAPPDATA%\Programs\AgentMux`, key in HKCU), so `Scope: user`. An older installer wrote a different uninstall key, which the manifest must not assume. Dry-run first (`--out`), then submit. This is public (microsoft/winget-pkgs), and nothing in the manifest or PR text mentions anything private.

**After the merge,** re-enable the `winget` job in `release-core.yml` (`wingetcreate update AgentMux.AI --urls <setup.exe> --version <v> --submit`). It runs after the release is published so the URL is live. Its failure never blocks a release (it is `continue-on-error`), and the resulting submissions are watched until each merges.

**Command line without winget** (Windows 10 before App Installer, or winget blocked): `irm https://agentmux.ai/install.ps1 | iex` (§5).

### 4.2 macOS

**Primary command:** `brew install --cask agentmuxai/tap/agentmux`.

- **A tap, not homebrew/cask, for now.** A new public repository `agentmuxai/homebrew-tap` with `Casks/agentmux.rb`. homebrew/cask asks for a notable project (a star, fork and watcher count we're well under at 24/3/2, and a higher bar for a project's own submission) and refuses unsigned apps; AgentMux is signed and notarized, so the only gap is notability. Revisit when the numbers are there; the cask can be moved with no change for users beyond a shorter command (`brew install --cask agentmux`).
- **The cask:**
  ```ruby
  cask "agentmux" do
    version "0.60.14"
    sha256 "<sha256 of AgentMux_0.60.14_arm64.dmg, from the signed manifest>"
    url "https://github.com/agentmuxai/agentmux/releases/download/v#{version}/AgentMux_#{version}_arm64.dmg"
    name "AgentMux"
    desc "Desktop workspace for AI coding agents"
    homepage "https://agentmux.ai"
    livecheck do
      url :url
      strategy :github_latest
    end
    auto_updates true
    depends_on arch: :arm64
    depends_on macos: ">= :ventura"   # set to what the app's Info.plist LSMinimumSystemVersion says
    app "AgentMux.app"
    zap trash: [ ... ]                 # the app's data folders; list taken from the app's own uninstall notes
  end
  ```
  `depends_on arch: :arm64` makes an Intel Mac fail with Homebrew's own clear message, and the script says the same in its words (there is no Intel build).
- **`auto_updates true`** tells Homebrew the app updates itself, so `brew upgrade` leaves it alone unless asked with `--greedy`. That is the right answer for AgentMux (§6): the in-app updater replaces `/Applications/AgentMux.app` in place, which works because Homebrew copies the app there as the user.
- **Keeping the cask current.** A job in the release workflow (after publish) updates `version` and `sha256` in the tap repository, taking the hash from the signed manifest and not from a download, and opens a PR (or commits to the tap's main; a tap has no review gate, so a PR checked by `brew audit --cask --online` and `brew style` first is preferred). It uses the GitHub App already installed on the organisation, extended to the tap repository. The cask's CI runs `brew audit --cask --strict`, `brew install --cask` and `brew uninstall --cask` on a macOS runner.
- **Command on PATH:** the app has a launcher; a cask `binary` stanza can link it as `agentmux`. **verify** which file in the `.app` is the right target (the Linux package has a `/usr/bin/agentmux` wrapper; macOS has none today).

### 4.3 Linux

**Primary command (every supported distribution):** `curl -fsSL https://agentmux.ai/install.sh | sh`. The script detects the package manager and does the following, with `sudo` only for the steps that need it and a single line saying so first:

- **apt (Debian, Ubuntu):** installs the keyring to `/usr/share/keyrings/agentmux-archive-keyring.gpg` and `agentmux.sources` to `/etc/apt/sources.list.d/`, both from `https://packages.agentmux.ai/`, then `apt-get update` and `apt-get install -y agentmux`.
- **dnf (Fedora), zypper (openSUSE):** `agentmux.repo` into `/etc/yum.repos.d/` (for zypper, `zypper addrepo`), then `dnf install -y agentmux` or `zypper --gpg-auto-import-keys install -y agentmux`.
- **Neither (Arch, NixOS, Alpine, an unknown distribution):** the portable tarball into `~/.local/share/agentmux` with `~/.local/bin/agentmux` linked to it, no root and no FUSE (an AppImage needs `libfuse2`, which current Ubuntu no longer installs); the same download-and-verify path as macOS and Windows without a package manager (§5.2). The updater sees it as `LinuxPortable`. On arm64 it stops with "there is no arm64 Linux build yet".

Copy-friendly explicit forms, shown on the page in a second tab for people who don't pipe to a shell (plain text, no prompts, no placeholders; each runs as written):

```
# Debian, Ubuntu
sudo install -d -m 0755 /usr/share/keyrings
curl -fsSL https://packages.agentmux.ai/agentmux-archive-keyring.gpg | sudo tee /usr/share/keyrings/agentmux-archive-keyring.gpg > /dev/null
curl -fsSL https://packages.agentmux.ai/agentmux.sources | sudo tee /etc/apt/sources.list.d/agentmux.sources > /dev/null
sudo apt-get update && sudo apt-get install -y agentmux

# Fedora
sudo curl -fsSL -o /etc/yum.repos.d/agentmux.repo https://packages.agentmux.ai/agentmux.repo
sudo dnf install -y agentmux

# openSUSE
sudo zypper addrepo https://packages.agentmux.ai/agentmux.repo
sudo zypper --gpg-auto-import-keys install -y agentmux
```

The build checks each block verbatim in the matching container (Ubuntu 24.04 and 26.04, Debian 13, Fedora 42, openSUSE Tumbleweed), which `linux-repo-check.yml` already does for the repository itself. The key's fingerprint is shown beside the commands on the page, so someone who wants to check it can.

Two package gaps the page doesn't hide: no declared dependencies, and Ubuntu 24.04+'s AppArmor profile for the sandbox is shipped as a script but not installed (both in the upgrade spec §5). The script runs `scripts/install-userns-apparmor-fix.sh` after an apt install on a system that needs it, or says what to run; the build decides which once the package owners confirm the state of the packages.

## 5. The scripts

### 5.1 What they are

`install.sh` (POSIX `sh`; macOS and Linux) and `install.ps1` (Windows PowerShell 5.1 and 7), kept in `scripts/install/` in this repository and tested in CI. They are plain text, print every step, make no prompts unless `--interactive` (agentic tools run them unattended), use ASCII only, and exit non-zero with one line saying what failed. Flags (the same names in both; PowerShell spells them `-Version`, `-Method`, `-DryRun`, `-NoLaunch`, `-Force`, `-RequireSignature`, and takes them through `& ([scriptblock]::Create((irm https://agentmux.ai/install.ps1))) -DryRun`, since a pipe cannot pass any): `--version <v>` (default the latest release; with a package repository it only applies to `--method download`), `--method <auto|package-manager|download>`, `--dry-run` (print the plan, change nothing), `--no-launch`, `--force`, `--require-signature`, `--help`.

Each decides in this order:

| Platform | 1. a package manager | 2. else |
|---|---|---|
| Linux | apt, dnf, zypper: the repository (§4.3) | the portable tarball under `~/.local` |
| macOS | `brew install --cask agentmuxai/tap/agentmux` if `brew` exists | the `.dmg`: mount, copy `AgentMux.app` to `/Applications` (or `~/Applications` without write access), unmount |
| Windows | `winget install --id AgentMux.AI -e --accept-package-agreements --accept-source-agreements` (the Store id before `AgentMux.AI` exists) | `setup.exe` run with `/VERYSILENT /NORESTART` (Inno's silent switches, the same ones winget uses) |

It refuses to do anything that would give a second copy: if AgentMux is already installed (by any method), it says which and how it updates, and exits 0 without changing it (`--force` overrides). This is what keeps a Store install and a `setup.exe` install from living side by side (§6).

### 5.2 How they verify what they download

Only the download paths verify anything themselves; the package-manager paths are verified by the package manager (GPG for apt/dnf/zypper, the Developer ID and notarization for the cask's `.dmg` on top of its sha256, the manifest hash for winget).

For a downloaded release file:

1. Fetch `AgentMux-<v>-manifest.json`, `-envelope.json` and `-envelope.json.sig` from `https://github.com/agentmuxai/agentmux/releases/download/v<v>/`, over HTTPS only, the exact URL (never a redirect to elsewhere: `curl --proto '=https'`, PowerShell `-MaximumRedirection 0` on non-GitHub hosts).
2. Check the envelope's signature against the **public key embedded in the script** (the same Ed25519 key the app pins; the script is regenerated when the key changes), check the manifest's SHA-256 equals `manifest_sha256`, and check the file's name, size and SHA-256 equal the manifest's. Any mismatch deletes the file and exits.
3. Where the platform tool can't verify Ed25519 itself (Windows PowerShell 5.1 has none; macOS's LibreSSL may lack `pkeyutl -rawin`; **verify** both), the script still checks the hashes of step 2, says in its output that the signature was not checked ("signature not checked: this system's openssl cannot ..."), and goes on. `--require-signature` turns that into a refusal, for an agent or a person who wants it strict. Asking an already-installed AgentMux to do the check (`agentmux-launcher --verify-update`) was considered and dropped: the script exits early when AgentMux is already installed, so there is nothing to ask on a machine that needs the script, and the case it would help (`--force` over an old install) is rare. A small verifier shipped beside the scripts is the way to close the gap on Windows and macOS if the hash-only result proves too weak; the updater's owner decides that.

### 5.3 Where they are hosted, and how they're trusted

- **Source of truth:** `scripts/install/` in this repository. Each release attaches `install.sh` and `install.ps1` as assets, which the release's signed manifest lists; so each version's scripts are tamper-evident like the app.
- **What `agentmux.ai/install.sh` serves:** the latest release's copy, as a static file the landing site picks up from the release when it is built, with `Content-Type: text/plain` and a short cache. It deploys with the landing site, which deploys only on "ship". Until then the page is not live and the commands are not shown. The alternative, a redirect from agentmux.ai to `https://github.com/agentmuxai/agentmux/releases/latest/download/install.sh`, is simpler and never stale, but puts the pipe-to-shell hop on GitHub's redirect rather than on our domain; the build may use it if the landing site's build step proves awkward.
- **Pipe to shell is a trust decision, not a verification.** The page therefore also gives the two-step form that lets a person read it first (`curl -fsSL https://agentmux.ai/install.sh -o install.sh`, then `sh install.sh`), and the SHA-256 of the script is in the signed manifest.
- **Script version:** both print `agentmux-install <script version>` and the release they resolved.

## 6. How updates work after a command-line install, without fighting the in-app updater

The rule: **each install is updated by whatever installed it, and the app tells the user which.** The command line must never produce an install the app misdescribes.

| Installed by | Updated by | In-app updater (kind) | Notes |
|---|---|---|---|
| `apt` / `dnf` / `zypper` | the system's updates (`apt upgrade`, `dnf upgrade`) | `LinuxPackage`; **Upgrade & restart** still works through the `pkexec` helper (the helper's repository mode is the upgrade spec's Repo-2) | The same package as a downloaded `.deb`/`.rpm`, so no mismatch. |
| `brew install --cask` | the app updates itself in place; `brew upgrade --greedy` also works | `MacApp` | `auto_updates true` stops `brew upgrade` from racing the app. After an in-app update, Homebrew's recorded version is behind until `brew upgrade --greedy` or reinstall; `brew uninstall` still removes the app. The cask's `zap` covers data. |
| `winget install AgentMux.AI` | the in-app updater or `winget upgrade` | `WindowsSetup` | The same Inno AppId, so either path updates the one install entry (**verify** on Windows). |
| Store (`msstore`) | the Store | `WindowsStore` (panel shows no download) | Not combined with the above. |
| Script, download path | the in-app updater | `LinuxAppImage` / `LinuxPortable` / `MacApp` / `WindowsSetup` | Exactly like a hand-downloaded file. |

Because the Store build and the `setup.exe` build are different products to Windows (different identity, different install folder), the scripts and the page never offer both, and `install.ps1` stops if the other is present. A user who has both gets the updater's existing per-kind messages; unifying their data folders is out of scope.

## 7. The landing page section

The landing site lives in a separate, private repository; this section states behaviour only, and its PR is made there. Its change merges on review, and **it does not deploy until the site is explicitly released**.

- **A "Get AgentMux" block** that replaces the first screen of the current download section, not a new page. It detects the visitor's OS on the client (the site already does) and shows that platform's primary command (§2) in a code box with a **Copy** button. The block is server-rendered with the Linux command so that the page works without JavaScript and for crawlers and agentic tools that read the HTML (they see the text, not a canvas).
- **Tabs:** `macOS`, `Windows`, `Linux`, plus (under Windows) `Store`, `winget`, `script`, and (under Linux) `Script`, `apt`, `dnf`, `zypper`. The tab for the detected OS is selected; the others are one click away.
- **The copy button** copies the exact text of the command, no prompt character, no trailing newline, no smart quotes, no comment lines inside the primary box (comments belong to the explicit multi-command forms, and they are plain `#` lines).
- **Plain-text surfaces:** the same commands appear in `llms.txt` and `llms-full.txt` (the site already publishes them), so an agent asked "how do I install AgentMux" finds them. The commands are generated from one data file in the site's repository so the page, those files and the README never disagree.
- **Direct downloads stay:** the current buttons and "all platforms" links remain under a "Download a file instead" heading, as today. The existing alpha warning stays, and the Windows installer's SmartScreen note moves next to the `winget`/`script` tabs it applies to.
- **Honest states:** while `AgentMux.AI` isn't merged, the Windows block shows the Store command and no `AgentMux.AI` tab. While the tap isn't live, the macOS block shows the script and the download. Nothing on the page names a command that doesn't work.
- **Accessibility:** the site's existing accessibility and visual checks run on the new block; the Copy button announces "Copied" politely.
- **The README and docs.agentmux.ai** get the same commands (one table), with the docs site's change as a separate small PR to the docs repository, after the commands are real.

## 8. "The full AgentMux experience through a command line": what it is today

This section is written from the repository as of the date above.

- **What a command line gives today:** install (this spec), launch (`agentmux` on Linux; `open -a AgentMux` on macOS; the Start menu entry or `AgentMux` on Windows: `setup.exe` puts nothing on PATH (no `[Registry]` PATH entry and no `ChangesEnvironment` in the `.iss`, read statically, not yet run), so `install.ps1` or the winget manifest has to add a command, or the page says "start it from the Start menu"), update (§6), and uninstall (each manager's own). The result is the desktop app, the whole product: it is a windowed app (Chromium-based), and its panes, agents and browser are the experience. A command line doesn't change what it is.
- **What works without the window, today:** on SSH hosts, `agentmux-remote` (`crates/remote`) gives the desktop app durable terminal sessions and file browsing there, with no root, installed by the app itself, not by a package. Agents inside the app drive it through its MCP tools (`agentmux-mcp`: Pane, Tasks, Browser, Terminal, Vault and the rest). Neither is a standalone, headless AgentMux a person starts from a terminal.
- **What is missing for "full experience from a terminal":** (1) a supported headless mode (the backend, `agentmux-srv`, runs without a window internally, but I found no supported way to start, sign in, configure and use it without the desktop app; **verify** with the owners of the backend before the page says anything), (2) a command-line client for the things the MCP tools do, so a person or a script outside an agent can open a pane, file a task or send a message, (3) a `agentmux` command on PATH on macOS and Windows with the same verbs everywhere (`agentmux open <path>`, `agentmux upgrade`, `agentmux doctor`), and (4) an unattended first run (accept the terms, sign in, pick a provider) for the case where the install command is run by an agent on a fresh machine. None of them blocks this spec's build; the page words its promise as "install and launch from your terminal", not "use AgentMux from your terminal". (2) and (3) are proposed as follow-up specs, for the product owner to rank.

## 9. What each piece needs

No cost beyond what already exists, and **no new external account.** Specifically:

| Piece | Needs | Cost | New account? |
|---|---|---|---|
| Linux commands and `install.sh` | Nothing. The repository is live; the scripts live in this repository | none | no |
| `install.ps1` | Nothing | none | no |
| Landing page section | Review of the change to the landing site's repository, and its release | none | no |
| Store command | Nothing. The listing is live; nothing is changed in Partner Center | none | no |
| `AgentMux.AI` on winget | A pull request to `microsoft/winget-pkgs` from the project's existing account for package submissions. Moderation is Microsoft's, with no fee | none | no |
| Homebrew tap | A new public repository `agentmuxai/homebrew-tap` in the existing organisation, with the existing GitHub App installed on it | none | no |
| homebrew/cask later | Notability; nothing to buy | none | no |

The submission, the tap and anything else that touches an account are recorded in the project's tracked records when they happen.

## 10. Decisions (all taken as recommended)

1. **The scripts front the package managers** (§5.1) rather than being a separate installer. Alternative: scripts that always download the release file. Rejected: it duplicates what apt/dnf/winget/brew do and bypasses their updates.
2. **Windows primary: Store now, `AgentMux.AI` once merged and proven by one bump** (§4.1). Alternative: wait for `AgentMux.AI` before showing any Windows command. Rejected: the Store command exists and is signed.
3. **macOS: a tap now, homebrew/cask when the numbers allow** (§4.2).
4. **Scripts are release assets under the signed manifest, copied to agentmux.ai by the landing build** (§5.3). Alternative: a redirect to the release asset.
5. **Script-side signature verification where the platform can, hash verification otherwise, and it says which** (§5.2). Alternative: refuse on platforms that can't verify the signature. Rejected for now: it would break the command on Windows PowerShell 5.1, the common case.
6. **One install per machine** (§5.1, §6): the scripts refuse to add a second kind.
7. **The page promises "install and launch", not "use from a terminal"** (§8).

## 11. Phases

1. **`install.sh` and its tests (PR 1, built).** `scripts/install/install.sh` and `install.test.sh` (dry runs per platform, the refusals, and the download checks against a release signed in the test with a throwaway key; the pinned key is checked against `crates/common/update-keys.json`), the `install-script-check.yml` workflow (the tests, dry runs through a pipe, and a real download of the latest release checked with `--require-signature`), and the script as a release asset in `release-core.yml`. The same PR carries this spec. Container installs from the live repository on Debian, Ubuntu, Fedora and openSUSE are phase 2's check, since the commands come from the same files the script uses.
1b. **`install.ps1` (PR 2, built).** Written and tested on a Windows machine, since it cannot be run from a Linux container; its CI job (`install-script-check.yml`) runs on `windows-latest`, in PowerShell 7 and Windows PowerShell 5.1. It picks winget when there is one: `AgentMux.AI` once that package is in the catalogue (it asks winget at run time), the Store id until then; otherwise it downloads `setup.exe`, checks it against the signed manifest (the Ed25519 signature too, when `openssl` 3 is on the path, otherwise the hashes only, and it says so) and runs it silently for the user. It refuses a second kind beside an existing one. It joins the release assets by the same loop as `install.sh`.
2. **Linux commands on the page** (§4.3, §7): live as soon as the landing site ships, since the repository is live.
3. **Windows Store tab** (§4.1): after the **verify** on a clean Windows machine.
4. **`AgentMux.AI` bootstrap submission,** then the release job re-enabled (§4.1); the page switches after one bump has merged.
5. **Tap and cask** (§4.2), with the bump job; then the macOS block on the page.
6. **README and docs site** (§7), once 2 to 5 are true.

Each phase is its own PR with its own changeset; the first PR carries this spec (it is not committed before then). Agree with the owners of the updater (§6), the Linux repository and the Linux packages (the dependency gaps, §4.3) before changing what touches theirs.

## 12. Tests

- Scripts: `--dry-run` output is checked for each platform (and, in CI, run through a pipe as `curl | sh` does); the verification paths are tested with a good file, a file of the wrong size, a hash mismatch, a wrong signature and a manifest for another tag (each must refuse and delete); the "already installed" and "other kind installed" refusals; no output contains a smart quote or a non-ASCII byte (a check).
- Linux commands: each block of §4.3 in its container, then `agentmux --version`.
- Cask: `brew audit --cask --strict --online`, `brew style`, install and uninstall on a macOS runner; `zap` removes the data folders.
- winget: `winget validate` on the manifest, then the sandboxed install winget-pkgs' own validation does; `winget list` shows one entry after an in-app update.
- Page: the commands on the page are compared with the data file that feeds `llms.txt`; the Copy button's clipboard text is compared byte for byte.
