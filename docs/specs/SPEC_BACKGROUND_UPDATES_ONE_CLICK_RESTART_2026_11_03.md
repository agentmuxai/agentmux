# Spec: background updates, one "Restart to update" click

Date: 2026-11-03. Task #1447. Author: Lazo. Status: approved by the operator 2026-11-03 ("3 specs look good, go with recommendations"; all four decisions take the recommended answer). Ships with the P1 PR.
Overlaps: QTest's agent-driven upgrade draft (`SPEC_AGENT_DRIVEN_UPGRADES_AND_FLEET_ROLLOUT_2026_10_30.md`, tasks #11442/#11443; cited from QTest's summary, not the file). QTest owns the updater and has reviewed the design questions below.
Research behind it: sourced notes (Lazo's research file, not committed); every URL below comes from it.

Operator, 2026-11-03: "what is the best practice that apps use regarding updates? i notice most apps do everything in the background and give u a simple button like restart now. lets also hash that out, we want to streamline updates so we save clicks on the check and the download".

## 1. Recommendation

1. **Do what Chrome, VS Code and Slack do: check, download, verify and stage silently; show one quiet "Restart to update" chip; if it is never clicked, install when the user next quits AgentMux themselves.** The user's click count goes from 5 to 0 (apply on their own quit) or 1 click plus one OK (restart now).
2. **Keep the launcher's native confirm dialog for the "restart now" path** (one OK). The chip is a real mouse target that the UI tool can click, so an agent could press it; the dialog exists because no page or agent tool can click it (`crates/launcher/src/upgrade_restart.rs:18`). Add a **tray menu item "Restart to update"** (launcher-owned, no page can reach it) as the dialog-free one-click route.
3. **Reverse the 2026-10-18 rule "AgentMux makes no request to a server the user didn't ask for"** for the update check only: default on, one first-run line saying so, one switch (`update:auto`) and a policy value to turn it off. This needs the operator's explicit yes; it is the one real decision here (section 9).
4. **Ship in three PRs** (section 7): P1 silent check, download and stage plus the chip (nothing installs by itself); P2 apply on the user's own quit, with rollback and a health check; P3 caps, metered networks, channels and delta updates.
5. **Never auto-install what an agent could be mid-turn in.** Install only on a user-initiated quit, never on crash, logoff, shutdown or a quit caused by another upgrade. Agents resume after the restart exactly as they do for today's upgrade.

## 2. Best practice, with sources

What nearly every mainstream app does (full notes and caveats in the research file):

| Practice | Who does it | Source |
|---|---|---|
| **Check on a schedule without asking** | Chrome/Edge about every 5 h (Chromium updater 4.5 h, 10% jitter); VS Code 30 s after start, then hourly; Snap 4 a day; Sparkle daily; 1Password weekly; MSIX 8 h background task | [Chrome admin](https://support.google.com/chrome/a/answer/6350036), [Chromium updater spec](https://chromium.googlesource.com/chromium/src/+/main/docs/updater/functional_spec.md), [VS Code abstractUpdateService.ts](https://github.com/microsoft/vscode/blob/main/src/vs/platform/update/electron-main/abstractUpdateService.ts), [Snap](https://snapcraft.io/docs/how-to-guides/manage-snaps/manage-updates/), [Sparkle](https://sparkle-project.org/documentation/customization/), [1Password](https://support.1password.com/update-1password/), [MSIX update settings](https://learn.microsoft.com/en-us/uwp/schemas/appinstallerschema/element-update-settings) |
| **Download in the background, no UI** | Chromium updater, VS Code (Downloading/Updating states), Slack, Squirrel.Mac, Electron `autoUpdater` | [Chromium updater spec](https://chromium.googlesource.com/chromium/src/+/main/docs/updater/functional_spec.md), [VS Code updateService.win32.ts](https://github.com/microsoft/vscode/blob/main/src/vs/platform/update/electron-main/updateService.win32.ts), [Slack](https://slack.com/help/articles/360048367814), [Electron](https://www.electronjs.org/docs/latest/api/auto-updater) |
| **Deltas, full package as fallback** | Chrome (Courgette: 10.4 MB full vs 78 KB), Squirrel, Sparkle, AppImage zsync | [Courgette](https://www.chromium.org/developers/design-documents/software-updates-courgette), [Squirrel.Windows](https://github.com/Squirrel/Squirrel.Windows), [Sparkle](https://sparkle-project.org/documentation/), [AppImage](https://docs.appimage.org/packaging-guide/optional/updates.html) |
| **Stage the new copy beside the old; never overwrite the running one** | Chrome `new_chrome.exe` swapped at next launch; Squirrel versioned `app-x.y.z` folders; AppImage writes a new file; Flatpak applies on next launch | [upgrade_util_win.h](https://chromium.googlesource.com/chromium/src/+/lkgr/chrome/browser/first_run/upgrade_util_win.h), [Squirrel.Windows](https://github.com/Squirrel/Squirrel.Windows), [Flatpak](https://docs.flatpak.org/en/latest/flatpak-command-reference.html) |
| **One quiet "restart" affordance, with "Later"** | Chrome toolbar icon ("Not now"); Slack badge + "Restart to Apply Update"; VS Code `update.restartToUpdate`; 1Password "Restart Now / Later" (community-reported) | [Chrome Help](https://support.google.com/chrome/answer/95414), [Slack](https://slack.com/help/articles/360048367814), [VS Code update.contribution.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/update/browser/update.contribution.ts) |
| **Escalate over days, not a modal** | Chrome icon turns green/orange/red at 2/4/7 days; Edge changes at 1/3 and 2/3 of a week; Snap postpones a running app at most 14 days | [upgrade_detector_impl.cc](https://github.com/chromium/chromium/blob/main/chrome/browser/upgrade_detector/upgrade_detector_impl.cc), [Edge policy](https://learn.microsoft.com/en-ca/DeployEdge/microsoft-edge-policies/relaunchnotificationperiod), [Snap refresh awareness](https://snapcraft.io/docs/explanation/how-snaps-work/refresh-awareness/) |
| **Apply on quit or next launch if never clicked** | Electron ("applied the next time the application starts"), Squirrel.Mac and Sparkle on quit, VS Code `/silent` installer on quit, MSIX at next shutdown | [Electron](https://www.electronjs.org/docs/latest/api/auto-updater), [Squirrel.Mac](https://github.com/Squirrel/Squirrel.Mac), [Sparkle delegate](https://sparkle-project.org/documentation/api-reference/Protocols/SPUUpdaterDelegate.html), [MSIX non-store updates](https://learn.microsoft.com/en-us/windows/msix/non-store-developer-updates) |
| **Verify every payload** | Sparkle EdDSA, Tauri mandatory minisign, Squirrel.Mac code signing | [Sparkle publishing](https://sparkle-project.org/documentation/publishing/), [Tauri updater](https://v2.tauri.app/plugin/updater/) |
| **Rollout control, channels, "critical" flag** | Chrome Early Stable, Sparkle `phasedRolloutInterval`/`criticalUpdate`/`channel`, electron-updater `stagingPercentage` | [Early Stable](https://developer.chrome.com/blog/early-stable), [Sparkle publishing](https://sparkle-project.org/documentation/publishing/) |
| **User and admin off switch** | VS Code `update.mode` (`none`/`manual`/`start`/`default`) and `UpdateMode` policy; Chrome/Edge `UpdateDefault`; 1Password "Install updates automatically"; Sparkle `SUEnableAutomaticChecks` | [VS Code enterprise](https://code.visualstudio.com/docs/enterprise/updates), [Edge update policies](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-update-policies), [1Password](https://support.1password.com/update-1password/), [Sparkle](https://sparkle-project.org/documentation/customization/) |
| **Metered connections** | Edge `MeteredUpdates`, VS Code defers checks, Snap `refresh.metered` | [Edge update policies](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-update-policies), [Snap](https://snapcraft.io/docs/how-to-guides/manage-snaps/manage-updates/) |
| **Defer to the true owner of updates** | Mac App Store forbids other updaters (guideline 2.4.5(vii)); Store MSIX updates through the Store; deb/rpm through apt/dnf | [App Store guidelines](https://developer.apple.com/app-store/review/guidelines/), [MSIX install](https://learn.microsoft.com/en-us/windows/msix/app-installer/installing-windows10-apps-web), [VS Code Linux](https://code.visualstudio.com/docs/setup/linux) |

Two useful variants of "safe moment": 1Password installs only while locked ([1Password](https://support.1password.com/update-1password/)); Sparkle installs on quit and prompts only after a week of not quitting (`SUScheduledImpatientCheckInterval`). AgentMux's equivalent of "locked" is "the user quit".

Not found and so not claimed: any mainstream app with a user-visible bandwidth or disk cap setting; Discord's official updater behaviour; the exact look of VS Code's restart item. Weaker evidence (news or search-text only): the Store's 5-week pause limit, Chrome's 1/5/10/50/100% ramp, Snap's 14-day figure.

## 3. Today's AgentMux: click count, measured from the code

There is **no automatic check at all**. The 0.59.32 srv checked 60 s after start and every 6 h (`app:updatecheck`); the operator's 2026-10-18 decision removed it, and `check.rs::init` now "Asks nothing". `check_now` runs only when asked (30 s rate limit, unauthenticated GitHub `releases/latest`). Flow per `SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4.1 and the code (`frontend/app/statusbar/MaintenanceSection.tsx`, `UpdateStatus.tsx`, `crates/srv/src/backend/update/{check,download,install,restart}.rs`, `crates/launcher/src/upgrade_{restart,macos,linux,package}.rs`):

| Install kind | Clicks from "release is out" to running | Steps |
|---|---|---|
| Windows portable (7z) | **5** (+1 if agents are mid-turn and the user will not wait) | open the version panel (status-bar chip) · Check for update · Download · Upgrade & restart · Yes in the launcher's native "Upgrade AgentMux" dialog (150 s to answer) |
| Windows setup.exe | **5** (same) | same; setup.exe is per-user, no UAC |
| macOS DMG (`MacApp`) | **5** | same, native dialog via `osascript` (`upgrade_macos.rs`) |
| Linux AppImage / portable tar.xz | **5** | same; needs `zenity` or `kdialog` or nothing restarts (`upgrade_linux.rs:118`) |
| Linux deb / rpm | **5 + a pkexec password prompt** | download only if the package helper exists; older packages just open the release page (`SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md`) |
| Windows Store MSIX | **0 in AgentMux** | panel says "Update in the Store" (`can_download_with` is false); the Store updates after it certifies, later than GitHub |
| Unknown / dev build | 2 | opens the release page |

The chip "Update to X" appears only after a manual check and only opens the version panel. The label "Restart to update" exists in `UpdateStatus.tsx` only for a host updater that srv does not feed.

Cost of the naive fix: 22 stable releases in 15 days (about 1.5 a day since 2026-10-19). Sizes at v0.60.14: portable 126 MB, setup 148 MB, dmg 219 MB, AppImage 178 MB, tar.xz/deb/rpm 138 MB, msix 209 MB. Downloading every release is roughly **250 MB a day**; there are no delta updates. Section 6 handles this.

## 4. Design

### 4.1 Silent check (srv `update/check.rs`)

- Check 60 s after start (after the splash, never during startup), then every 6 h with up to 20% random jitter, and on the existing "Check again" click. The first-run line and the off switch come first (4.7).
- Use the `releases/latest` redirect first (a HEAD to the web URL does not count against the 60/hour unauthenticated API limit), and the API only for the asset list. On 403/429 or any error: back off exponentially to at most 24 h; never retry in a loop.
- A failed check is silent: no chip, no toast. It is logged and shown in the version panel ("last checked 14:02, failed: offline").

### 4.2 Silent download and verification (srv `update/download.rs`, `signature.rs`)

- Download only the asset for this install kind (the existing per-kind names), only when `can_download_with` is true, resumable (Range/ETag), one at a time, low priority, into `updates/<version>/`.
- Verify the signed `manifest.json` + `envelope.json.sig` (`SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md`, from v0.60.4), then the asset digest, before anything is called "staged". A failure deletes the partial and records why; it never stages.
- **Stage** = downloaded, verified and unpacked to a temp folder, then atomically renamed to `updates/<version>/` (Defender can lock or quarantine files mid-unpack; retry twice, then report "staged failed" in the panel only). Keep one staged version: a newer release replaces it.

### 4.3 The chip and its states

Status-bar chip next to the version, driven by one store (`store/app-update.ts`):

| State | Chip | Click |
|---|---|---|
| nothing new / checking / downloading | none (progress only in the version panel) | n/a |
| staged | **"Restart to update"** with a dot | opens a small popover: "What's new in X" (release notes link), **Restart now**, **Later** |
| staged for days | same chip, colour steps at 2 days and 4 days (Chrome uses 2/4/7) | same |
| agents mid-turn | popover adds "N agents are working" and the existing human-override countdown | **Restart now** waits for turns to finish (today's `restart.rs`), or "Restart anyway" |
| needs the Store / package manager | "Update available in the Store" (no download) | opens the Store page |

**Restart now** sends the existing restart request; the launcher still shows its native "Upgrade AgentMux" dialog (one OK). The tray gains **"Restart to update vX"** whenever a version is staged: this is a launcher-owned menu, so it is one click with no dialog and no page or agent can click it. Agents never trigger a restart: the App tool `upgrade` stays `pending_user` and `self_quit::gate_upgrade` is unchanged (`SPEC_AGENT_SELF_QUIT_2026_09_24.md`).

Recommendation on the dialog (QTest's option a vs c): keep it on the chip path and make the tray item the dialog-free path. If the operator wants a dialog-free chip, that needs a way to tell a human click from a UI-tool click; we do not have one, so I do not recommend it.

### 4.4 Apply on the user's own quit (P2)

If the chip is never clicked, the staged version installs the next time the user quits AgentMux themselves:

- Persist `update:auto` (default on) and the staged version. At quit, the launcher re-runs the staged check (exact version, newer than running, digest, signature; this replaces today's 5-minute `CONFIRMED_FOR` token) and hands off to the relaunch helper, which swaps after the process exits.
- **Counts as the user's quit:** tray Quit, last window closed, the app menu's Quit. **Does not count:** crash, logoff or OS shutdown, a quit caused by another upgrade, `self_quit` by an agent, a restart of srv. Those leave the staged update for next time.
- The quit itself says nothing and adds no dialog; the next start shows "Updated to X" once with "What's new". The user's quit is the consent.
- Never install a downgrade or a version withdrawn between stage and quit: re-fetch the cheap `releases/latest` redirect at quit time (2 s timeout); offline means install what is staged, since it was verified.
- Portable installs are sibling folders (`agentmux-0.60.13-x64/`): the old launcher at start looks for the newest verified sibling and execs it; the previous folder is kept 7 days for rollback, then removed by the new version.
- **Do not ship P2 without rollback and a health check** (QTest, task #11443): after an unattended update, if the new srv does not report healthy within 120 s, or the carry-forward/migration fails, relaunch the previous version and mark the new one "failed, skipped" so it is not staged again. Additive changes must never bump `IDENTITY_STORE_SCHEMA_VERSION`, or an unattended update could strand a rollback.

### 4.5 Agents resume

Same as today: a restart (chip, tray or quit) goes through `restart.rs`, which waits for agents to finish their turn, then the launcher restarts every pane and each agent resumes its session (`SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §6 for data carry-forward). Apply-on-quit has no agents running by definition (the user quit).

### 4.6 Install types that defer

- **Windows Store MSIX:** no check-and-download; the Store owns it. The app may show the "newer version exists" fact in the version panel only. (Slack and Store guidance: defer to the Store.)
- **deb/rpm:** notify only for now; the repository work is blocked on the signing key (#103). When the package helper exists, "Restart to update" means "install via the helper (pkexec) then restart", never a silent privileged install.
- **Mac App Store builds:** none today; if one ever ships, no updater at all (guideline 2.4.5(vii)).
- **macOS DMG:** works from `/Applications` only if writable; if the app is translocated (`/AppTranslocation/`) or not writable, notify only with the reason.
- **AppImage:** needs the file writable; write the new AppImage beside the old, replace after verify.

### 4.7 Control, consent and caps

- **First-run line**, shown once in the version panel and the settings page: "AgentMux checks for updates in the background and downloads them for you. It never installs without a restart. Turn off in Settings > Updates." No modal.
- **Settings** (Settings > Updates, one place): *Check for updates automatically* (`update:auto`, default on); *Download automatically* (default on; off = check only, chip says "Update available" and the old Download click returns); *Channel* (stable default; beta later); *Don't download on metered connections* (default on where the OS says so; Windows `NetworkCost`, macOS/Linux via system APIs where reliable, otherwise a manual switch).
- **Policy/managed override:** the same values readable from a machine-level config file so IT or the Store packaging can force them off (VS Code `UpdateMode` is the model). Off means no network call from the updater at all.
- **Caps (P3):** at most one staged version plus one cached download; refuse to download below 2 GB free (the archive plus the unpacked tree), clean `updates/` partials at start; download speed cap default none, but one download at a time at low priority; no download within 6 h of the previous (coalesce releases: if 0.60.15 appears while 0.60.14 is already staged and unclicked, wait for the next check window and fetch only the latest, so a release burst costs one download, not five).

## 5. Platform differences

| | Windows | macOS | Linux |
|---|---|---|---|
| Stage | unpack to `updates/<v>/` temp then rename; Defender locks → retry; long-path risk with CEF → keep the path short | unpack the dmg/app into `updates/<v>/`; quarantine attribute and notarisation check | AppImage file beside the old one; portable tar.xz unpacked beside |
| Swap | relaunch helper after quit (portable: sibling folder exec; setup.exe: run the per-user installer silently, no UAC) | replace `.app` after quit; fails if translocated or not writable (notify only) | AppImage: rename over after verify; portable: sibling exec |
| Native confirm (restart now) | MessageBox | `osascript` dialog | `zenity`/`kdialog`; if neither exists, tray item only |
| Defers | Store MSIX | Mac App Store (none today) | deb/rpm via package manager/helper |

## 6. Failure modes and limits

Disk full (clean partial, keep one staged version, show the cause in the panel); antivirus lock or quarantine (retry, then "staged failed"); proxies (use the system proxy as the existing check does; failure is silent); GitHub 60/hour unauthenticated limit (HEAD redirect, backoff, jitter so a fleet does not synchronise); release yanked or superseded between stage and apply (re-verify at apply; a newer staged version replaces the old); metered or slow links (pause, resume, cap); 250 MB/day if a user restarts rarely and releases ship 1.5/day (coalescing in 4.7 keeps it to about one download per check window, and delta updates in P3 would bring typical updates well below full size; AgentMux has none today, so size is the real cost of "on by default" and the operator should see this number).

## 7. Phasing

| PR | Content | Ships | Risk |
|---|---|---|---|
| **P1** (srv + UI) | Silent check on a timer with jitter and backoff; silent verified download and stage (all `can_download_with` kinds); `update:auto` and `update:autodownload` settings; the "Restart to update" chip with popover; restart reuses today's flow and native dialog. **Nothing installs without a click.** The tray item (three platform backends, not testable on every host here) and the first-run line are **P1b**, a small follow-up. | Same release | Low: only adds timers, one network call, and files in `updates/` |
| **P2** (launcher + srv) | Apply on the user's own quit; sibling-folder exec for portable; rollback and 120 s health check; "Updated to X" once. Requires QTest's #11443 rollback to land first or in the same PR. | Release after P1 has soaked 2–3 days on Area54/narko | Highest: unattended install. Behind `update:auto`. |
| **P3** | Settings > Updates page, metered handling, free-space guard, burst coalescing, channel; policy file; delta updates (separate spec if pursued: a binary diff against the installed version, signed in the same manifest) | Later | Low |

## 8. Tests

- srv unit: check scheduler (jitter bounds, backoff, rate limit), `update:auto` off makes zero network calls, HEAD-redirect path, version ordering (never stage a downgrade), staged state machine (checking → downloading → staged → applied/failed), disk-full and bad-digest cleanup, coalescing with a newer release.
- Signature: a tampered manifest or asset never reaches `staged` (extend the existing signed-manifest tests).
- Launcher: quit-kind classifier (user quit vs crash/logoff/self_quit/upgrade-caused), staged re-verification at quit, sibling-folder selection, rollback after a failed health check (fake srv that never reports healthy).
- UI vitest: chip states per install kind (Store, package, Unknown show no download), popover buttons, countdown with agents mid-turn.
- Live (per `agentmux:run`, in my own `task dev` build, never the running instance): fake release server serving a signed 2-version manifest; check → stage → chip → quit → relaunched as the new version; then a bad new version → rollback.
- Platform smoke by the platform testers: Clare (macOS), Maricon (Linux), Area54 (Windows).

## 9. Decisions for the operator

1. **Reverse "no request the user didn't ask for" for the update check, default on?** Recommend yes: it is what Chrome, VS Code and Slack all do; the off switch and first-run line keep it honest. Without it, nothing below works.
2. **Apply on the user's own quit with no dialog, default on?** Recommend yes: the user's quit is the consent, and Chrome, Sparkle and Squirrel.Mac do the same. The alternative is "stage only; install only by click".
3. **Keep the native OK on the chip's "Restart now"?** Recommend yes, with a dialog-free tray item; the chip alone cannot tell a human click from an agent's UI click.
4. **Download everything by default, accepting about 250 MB a day at today's release rate?** Recommend yes with coalescing, the metered guard and the free-space guard; delta updates come in P3 if the number worries you.

Open questions for QTest (updater owner), already discussed: P2 sequencing against #11443 rollback, and whether #11442's 2-minute veto reuses this staged state (it should; one staged slot, two consumers).

Agent: lazo@Area54
