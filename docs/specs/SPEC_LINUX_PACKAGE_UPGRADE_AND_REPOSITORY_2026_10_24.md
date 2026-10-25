# Spec: upgrading a .deb or .rpm install from the app, and an apt/dnf repository

**Status:** active (#5528, #5536). The operator approved all five decisions of §7 as recommended (2026-10-25). Phase 3g-1 (§3.1–3.3, §3.5, the "Installs from before 3g" note of §3.4) is built; so is 3g-2 (go back, §3.4); Repo-1 and Repo-2 are not.
**Date:** 2026-10-24
**Author:** Maricon
**Verified against:** `agentmux` `main` @ `0d73be7a0` (2026-10-24); Ubuntu 26.04 (GNOME 50.1) for what a desktop has installed.
**Builds on:** [SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md](SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md) §4.2 (the `.deb`/`.rpm` row, phase 3g), §4.3 (verifying a download) and phase 5 (a signed `SHA256SUMS`); [SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md](SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md) (the packages themselves).

## 1. Ask

Phase 3g of the upgrade spec: a `.deb` or `.rpm` install should upgrade itself from the version panel and the `App` tool like every other install kind, using the desktop's own password prompt, never a stored password. Then a plan for an apt and a dnf repository, so the system's own updates keep AgentMux current.

## 2. Today

- **Detection** (`crates/srv/src/backend/update_install.rs`): srv under `/usr` or `/opt` is `LinuxPackage { debian }`, where `debian` is whether `/etc/debian_version` exists. That guesses the distribution, not which package manager installed AgentMux. `can_download()` is false for it, so the panel only offers the release page.
- **The packages** (`scripts/build-deb-linux.sh`, `scripts/build-rpm-linux.sh`, built by fpm in `build-linux.yml`): package `agentmux`, files in `/opt/agentmux/`, a wrapper `/usr/bin/agentmux`, a desktop file and icons. No maintainer scripts, no declared dependencies, and neither package is signed.
- **The release files**: `AgentMux_<v>_amd64.deb` and `AgentMux-<v>-1.x86_64.rpm`, each with a SHA-256 in the release's `digest`.
- **What a desktop has**: Ubuntu 26.04 has `pkexec` (a package of its own on Debian and Ubuntu, so not every system has it), `polkitd`, GNOME's polkit agent and the PackageKit daemon, but not `pkcon`.

## 3. The upgrade

### 3.1 Flow

The same flow as `setup.exe` (phase 3d): srv downloads and verifies the release file into this version's `updates/<v>/`, the user clicks **Upgrade & restart**, agents finish their turns, and srv hands the launcher an `Installer { installer, sha256 }` target. The Linux launcher (`crates/launcher/src/upgrade_linux.rs`) then:

1. **Asks for permission first, while AgentMux is still open.** It starts `pkexec /opt/agentmux/bin/agentmux-install-update <file> <version> <launcher pid>`. The desktop shows its password prompt for the policy in §3.3. If the user cancels it, nothing quits and nothing is installed; the panel shows no error (a cancel is an answer, not a failure). This replaces the launcher's own zenity question for this kind, so the user answers once.
2. **The helper waits** (as root) for the launcher's process tree to exit, up to 120 s, then quits AgentMux's own way: the launcher, once pkexec has started, exits like any other upgrade restart.
3. **The helper installs** (§3.2), writes nothing into the user's folders, and exits with a status.
4. **A small user-side waiter** (a `sh -c` the launcher started before exiting, holding pkexec's exit status and stderr) writes the result into `updates/<v>/install-result.json` and starts `/usr/bin/agentmux` again **whether or not the install worked**, so the old version runs rather than none. srv reads the result on start: a version that is still the old one shows "The install didn't finish: <reason>" in the panel.

AgentMux never runs as root: only the helper does, and it starts nothing.

### 3.2 The helper: `/opt/agentmux/bin/agentmux-install-update`

A short POSIX `sh` script shipped in both packages, owned by root, mode 0755. It trusts nothing it was given except as a request:

- **The file.** It copies the given path, opened without following symlinks, into a new root-only directory (`mktemp -d` under `/var/cache/agentmux`, mode 0700), and works only on that copy, so the user's copy can't change between the check and the install.
- **The checksum.** It fetches the release's own digest for `<version>` from the GitHub releases API itself, over HTTPS (`curl --proto =https`), and compares the copy's SHA-256 with it. It doesn't take the expected checksum from the caller, so a process running as the user can't get a file of its own installed by naming its hash. When phase 5's signed `SHA256SUMS` exists, the helper checks that signature with a key in `/opt/agentmux/share/` instead (root-owned, installed by the package).
- **The package.** Name `agentmux`, version exactly `<version>`, architecture `amd64`/`x86_64` (`dpkg-deb --field`, `rpm -qp --queryformat`). Not older than or equal to the installed one (`dpkg --compare-versions`; rpm's own comparison, `rpm.vercmp`), so it can't be used to downgrade (§3.4 has the one exception).
- **Which package manager.** The one that owns `/opt/agentmux/bin/agentmux-launcher` (`dpkg-query -S`, else `rpm -qf`), not the distribution's name.
- **Nothing else running from the install.** No process of any user whose executable is under `/opt/agentmux/` (another user's session, another channel). Replacing `libcef.so` under a running CEF can crash its next subprocess. Otherwise it stops with "AgentMux is running for another user".
- **The install.**
  - `.deb`: `apt-get install -y --no-install-recommends <copy>` (resolves dependencies if the package ever declares some); `dpkg --install` when there is no `apt-get`.
  - `.rpm`: `dnf install -y <copy>`; `zypper --non-interactive install --allow-unsigned-rpm <copy>` on openSUSE (while the rpm is unsigned); `rpm -U` when neither is there.
- **Its record.** `/var/lib/agentmux/last-upgrade` (root-owned): the version it replaced and the one it installed, for §3.4.

### 3.3 The privilege step: polkit

Both packages install `/usr/share/polkit-1/actions/ai.agentmux.install-update.policy` with one action:

- `org.freedesktop.policykit.exec.path` = `/opt/agentmux/bin/agentmux-install-update`, so `pkexec` shows this action's own message ("Authentication is required to install an AgentMux update") instead of a generic one.
- Defaults: `allow_active` = `auth_admin` (every time; not `auth_admin_keep`, so a cached authorization can't be reused by something else in the next five minutes), `allow_inactive` and `allow_any` = `no`.

Never a password field of our own, never `sudo -S`, and nothing stored. Without a polkit agent (an SSH session, a headless box) `pkexec` fails at once. The panel then shows the command to run by hand (`sudo apt install ./AgentMux_<v>_amd64.deb` or `sudo dnf install ./AgentMux-<v>-1.x86_64.rpm`, with the staged file's path). The **Show file** button is not built (3g-1 puts the command in the panel text only).

An agent's `App upgrade` goes through the same prompt, so a person always types the password. That's stricter than the other kinds, where the user's own turn or the window prompt is enough (upgrade spec §5.1).

### 3.4 Rollback

- **A failed install leaves the old version.** dpkg undoes a failed unpack, and an rpm transaction installs all or nothing. The packages have no maintainer scripts, so there's no half-configured state. The waiter restarts whatever is installed, and the panel says why the install failed.
- **A new version that won't start (built in 3g-2).** When the launcher gives up on a version that the helper itself installed (the helper's record says it replaced the one before: `/var/lib/agentmux/last-upgrade`, now world-readable, written by the helper only) within a week of the upgrade, it asks "Go back to <old version>?". The give-ups are: srv can't be started at launch, srv can't be started again after it stopped (past its restart budget), and the window crashing past its budget. A yes runs `pkexec agentmux-install-update --go-back <old version> <pid>`. With no file from the user at all, the helper downloads the old version's release file itself over HTTPS, checks it against GitHub's listing like any install, and installs it with an explicit downgrade (`apt-get install --allow-downgrades`, `dnf downgrade`, `zypper install --oldpackage`, `rpm -U --oldpackage`). It accepts a downgrade only to the version its record says it replaced, only while the version the record says it installed is still the installed one, and only once (it removes the record). The waiter then starts the old version. The old version starts with its own data folder (`versions/<old>/data`, still there), so whatever happened in the new version since is not carried back. If the install fails before it starts, the launcher says why in a dialog and exits as it would have; if the user says no, nothing changes. Not built: a "Go back" button in the version panel for a version that starts but isn't wanted.
- **Installs from before 3g** have no helper. Their panel offers the release page (3g-1 shows no manual command for them); once installed, they can use the helper from then on.

### 3.5 Code

- `update_install.rs`: `LinuxPackage { debian }` decided by ownership (dpkg's own file list for the package, `/var/lib/dpkg/info/agentmux.list`, naming srv's own path; the helper itself uses `dpkg-query -S` / `rpm -qf`), and `can_download()` true for it once the helper exists at `/opt/agentmux/bin/agentmux-install-update`.
- `upgrade_package.rs` (new, Linux only), used from `upgrade_restart.rs`: the `Staged::Package` kind (only `updates/<v>/` of this version, only that version's file name, the digest srv names), the question, the pkexec start and its waiter, the result file. The launcher's own yes/no question is asked first and the password prompt follows it; nothing quits until the helper has said `READY`.
- The two build scripts: the helper, the policy file, `/var/cache/agentmux` and `/var/lib/agentmux` created by the helper on first use.
- Shared code (`update_download.rs`, `update_restart.rs`, `upgrade_restart.rs`, the panel and the `App` tool) belongs to the updater. Changes there are agreed with QTest first.

### 3.6 Tests

- Unit: detection by ownership; the staged-file checks; the helper's refusals (wrong name, wrong version, older version, mismatched digest, a symlink, something still running), run against fixtures with a stub package manager.
- Live, in containers and on charlie: Ubuntu 24.04 and 26.04 and Debian 13 (`.deb`), Fedora 42 and openSUSE Tumbleweed (`.rpm`). For each: an install of release N upgrades to N+1 from the panel; cancelling the password prompt changes nothing; a corrupted file is refused; a second user's running AgentMux blocks the install; going back restores N.

## 4. The apt and dnf repositories

What they need, in the order they'd be built. Where they are hosted is decided elsewhere.

1. **A signing key.** An OpenPGP key: RSA 4096, which every apt, dnf and zypper in our targets accepts (older rpm can't verify EdDSA). The primary key stays offline; a signing subkey with a two-year expiry is used by the release job, held in the release pipeline's secret store and never in the repository. A revocation certificate is kept offline. The fingerprint is published on the downloads page and in this repository.
2. **Signed packages.** `rpmsign --addsign` for the `.rpm`, so `gpgcheck=1` works and zypper no longer needs `--allow-unsigned-rpm`. A `.deb` isn't signed itself (apt checks the repository's signed index instead).
3. **apt repository.** The standard layout: `dists/stable/{InRelease,Release,Release.gpg}`, `dists/stable/main/binary-amd64/Packages{,.xz}` with `by-hash`, and `pool/main/a/agentmux/*.deb`. Built by `aptly` or `reprepro` in the release job. Clients use a deb822 `/etc/apt/sources.list.d/agentmux.sources` with `Signed-By: /usr/share/keyrings/agentmux-archive-keyring.gpg`.
4. **dnf/zypper repository.** `repodata/` from `createrepo_c`, with `repomd.xml.asc`. Clients use `/etc/yum.repos.d/agentmux.repo` (and zypper's own `.repo`) with `gpgcheck=1`, `repo_gpgcheck=1` and the key's URL.
5. **Hosting.** A static HTTPS origin on a stable name under the project's domain (apt needs the index layout, which GitHub Releases can't serve). Package files are immutable. Indices are replaced only after the packages they list are uploaded, with a short cache lifetime. The last several versions stay, so a downgrade is possible.
6. **Publishing.** After the GitHub release: sign the `.rpm`, add both packages, regenerate and sign the indices, upload. A check job then installs and upgrades from the live repository in Debian, Ubuntu, Fedora and openSUSE containers.
7. **Subscribing.** The `.deb` ships the sources file and keyring as conffiles, and the `.rpm` its `.repo` and key, as Chrome and VS Code do, so installing either once keeps it updated. Removing the file unsubscribes, and an upgrade doesn't put it back (dpkg conffile handling; `%config(noreplace)`).
8. **The app, once a repository exists.** An install whose package manager knows the repository gets "Updates come with your system's updates" in the panel. **Upgrade & restart** still works, through the helper and the repository (`apt-get install --only-upgrade agentmux`, `dnf upgrade agentmux`) instead of the release file.
9. **Later:** arm64, and a `beta` suite for pre-releases.

## 5. Not in this spec

- The packages' own gaps: no declared dependencies (a minimal system lacks `libnss3` and friends), and the AppArmor user-namespace profile that Ubuntu 24.04+ needs for the sandbox, which the `.deb` ships as a script but doesn't install. Both need maintainer scripts; that's a separate change.
- macOS and Windows; Flatpak and Snap.

## 6. Phases

1. **3g-1:** detection by ownership, the helper and policy in both packages, the launcher's package kind, the result file, the headless fallback. The first release with them still has to be installed by hand once.
2. **3g-2:** "Go back to <old version>" (§3.4). Built.
3. **Repo-1:** the key, signed `.rpm`, both repositories, the publishing and check jobs.
4. **Repo-2:** the packages subscribe themselves, and the panel's repository mode.

## 7. Decisions for the operator

1. **The helper fetches the checksum itself** (§3.2), instead of trusting the one srv passes. Recommended: yes. It's what keeps a process running as you from getting its own package installed as root. It needs network access from the helper, which the download needed anyway.
2. **Password before quitting** (§3.1): AgentMux stays open until you've authenticated, and Cancel changes nothing. Recommended: yes. The alternative (quit, then ask) leaves you with no AgentMux while the prompt waits.
3. **A fresh prompt for each install** (`auth_admin`, not `auth_admin_keep`). Recommended: yes.
4. **Packages subscribe to the repository on install** (§4.7). Recommended: yes, as Chrome and VS Code do. Removing the sources file opts out.
5. **RSA 4096 for the repository key** (§4.1). Recommended: yes, for older rpm. Ed25519 can come at the next key rotation.
