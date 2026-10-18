# SPEC: upgrading AgentMux, for users and for agents

**Date:** 2026-10-15
**Status:** active — phase 1 (detection, the chip, Maintenance's "What's
new" and Download) in #4983; phase 2 (data carries forward) in #4996;
phase 3a (download and stage, Windows portable) in #5000; phase 3b
(Upgrade & restart) in #5005; phase 3d (`setup.exe`) in #5015; phase 4a
(the `App` tool) in #5012; phase 4b (the note after an upgrade) in #5014;
phase 5a (the version panel's clicks only through the host) in #5019;
every window coming back after an upgrade (§6.1) in #5033; Check for
update and local builds upgrading (§3) in #5048; the rest of phases 3–5 are
not started. The operator took the §8
recommendations on 2026-10-15.
**Asked for:** "agents need a way to upgrade their running instances and
continue on. Users also need a button to let them know. We did work on this
in the past, find the history, let's consolidate; we have partial work in the
status bar version panel under Maintenance."
**Consolidates:**
- `app-update-check.md` (2026-03-28): the update check, built for Tauri and
  deleted with it in April;
- `SPEC_UPGRADE_PANEL_2026_06_27.md`: the Maintenance section, partly built
  in #1813;
- `SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md`: one
  Upgrade button owns updates and migrations, the launcher runs it. Its
  launcher flow (§4.3) and boot rules (§5) stand; this spec adds what it left
  open: where versions come from, how each kind of install is replaced, and
  agents.

**Builds on:** `SPEC_AGENT_SELF_QUIT_2026_09_24.md` (the trust gate for
an agent asking for something that affects the user),
`SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`,
`SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md`,
`SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md`.

## 1. Short answer

- **AgentMux learns that a newer release exists** from GitHub Releases, after
  the window is up and then every few hours. It never checks during startup.
- **The user sees it** as a status-bar chip, "Update to 0.59.27", which opens
  the version panel's Maintenance section: what's new, then **Download**, then
  **Upgrade & restart**. Nothing installs on its own.
- **An agent can ask for it** with a `RequestUpgrade` tool. AgentMux never
  restarts on an agent's word alone: the user's own turn asking for it, or a
  click on "Upgrade now" in the request it raises, is what starts it. A
  restart touches every agent, not just the one asking.
- **Everyone continues.** After the restart the windows and panes come back,
  each agent resumes its conversation, messages sent meanwhile are delivered,
  and each agent gets one system note: what changed, and that its session
  resumed.

## 2. Where things are today

| Piece | State |
|---|---|
| Version panel (click the version chip) with a Maintenance section | Built (#1813). Shows the version, channel, commit and build; Maintenance shows migrations and vacuum |
| "Update available" chip and the panel's update row | UI built (`UpdateStatus.tsx`, `MaintenanceSection.tsx`), never shown: the host never sends its updater status, so `updaterStatusAtom` stays `up-to-date`. Phase 1 feeds both from srv's `app:updatestatus` |
| `install_update` | A host stub that logs and returns (`crates/cef/src/commands/stubs.rs`) |
| Update check | None. GitHub Releases does publish every version with every installer kind (portable `.7z`, `setup.exe`, `.msix`, `.dmg`, `.deb`, `.rpm`, `.AppImage`, Linux portable `.tar.xz`) |
| Launcher: stop srv cleanly, run migrations from a binary | `quiesce_srv` is live (every quit); `run_migration_upgrade` is written and has no caller (`crates/launcher/src/upgrade.rs`) |
| Migrations / vacuum buttons | Refused under the launcher ("Restart AgentMux…"); vacuum is a stub |
| Restart after a crash | Built: the launcher respawns srv and the host within budgets |
| A restart the user or an agent asks for | None |
| Agents after a restart | Eager resume with `--resume` (#3465, #3513, #3551, #4708); held messages replayed (`jekt_held.rs`); windows restored (#2560); an "interrupted" banner for the user only |
| Telling an agent what happened | None. `cli_notice.rs` tracks the CLI's own version, for the user only |
| An agent restarting or upgrading the app | No tool. `QuitSelf` closes only the agent's own tab |
| UI vs srv version skew | Built (#4517): "Reload for vX" |

## 3. Detecting a release

- **Source:** `GET https://api.github.com/repos/agentmuxai/agentmux/releases/latest`,
  unauthenticated, from srv (it already has an HTTP client and the network
  settings). Compare the tag's semver with this build's. Pre-releases are
  ignored.
- **When: only when asked** (`update_check::check_now`), never on its own
  (the operator's decision, 2026-10-18: AgentMux makes no request to a
  server the user didn't ask for). Maintenance's **Check for update**
  (`app.update-check`), the `App` tool's `version`
  (`GET /api/v1/app/update-status?check=1`) and its `upgrade` ask GitHub at
  once, at most every 30 s between them (GitHub allows 60 unauthenticated
  requests an hour); asked again sooner, they answer what the last check
  found. A failed check says why and keeps what the last one found. Up to
  0.59.32 srv also checked 60 s after start and every 6 h, under the
  `app:updatecheck` setting; both are gone.
- **The button is always there.** Maintenance shows **Check for update** in
  every state, as **Check again** once a release is on offer, so an install
  is never left offering a release that has since been superseded. Beside
  it, where this one stands: "Not checked for updates yet", "v0.59.28 is up
  to date (checked 14:05)", "Checked 14:05" under an offered release, or
  for a `task dev` build which release is latest. It is disabled only while
  a check, a download or a restart is under way.
- **A check pins what is installed.** The release, file and SHA-256 that
  check found are what Download fetches and the restart verifies; nothing
  reads "latest" again later. A new check replaces the offer, except while a
  download or a restart into it is under way (`downloading`, `unpacking`,
  `waiting`, `restarting`): that one finishes first, and a check after it
  offers the newer release.
- **What's kept** (`backend::update_check::AppUpdateStatus`): this build's
  version, the latest release's version, page and notes (the release body,
  which is the `VERSION_HISTORY.md` section, up to 16 KB), when it was
  published, and when it was checked. Phase 3 adds its assets and their sizes.
- **Published as:** the typed `app:updatestatus` event when the status
  changes, and the `app.update-status` command for a window that opens
  later. States: `up-to-date`, `available`, `local-build` and `unknown`
  (not checked yet). Phase 3 adds the download states.
- **Local builds upgrade too.** Agents develop AgentMux and run what they
  build, so a portable from `task package` (channel `local-…`) compares
  versions like a release: a 0.59.60 local portable is offered 0.59.61 once
  it is published, to the user and to an agent alike, through the same flow.
  Only a `task dev` build (channel `dev-…`), which runs from source, doesn't
  upgrade; the panel says "Development build. The latest release is
  v0.59.27." and links the release.
- **Linux asks with `zenity` or `kdialog`** (`crates/launcher/src/upgrade_linux.rs`):
  a separate program no page or agent in AgentMux can click, as Windows'
  native dialog, closed by a newer request or after the same 150 s. With
  neither installed the new version stays staged and nothing restarts; the
  launcher's log says to start it. The Unix supervisor handles srv's request
  as Windows' does (`supervisor/unix.rs`): staged check, the dialog, the
  host's `quit_app`, then the new version once srv has stopped.
- **The upgrade keeps the data it came from.** A local build's windows,
  agents and settings are in its own channel, and a release is built for
  `stable`, so the unpack records this AgentMux's channel in the new folder
  (`runtime/agentmux-channel`, `update_download.rs`), and every binary in a
  portable folder opens the channel it records
  (`agentmux_common::runtime_mode::recorded_channel`): after an explicit
  `AGENTMUX_CHANNEL`, before the build-time one. Carry-forward, restoring
  every window and the agents' note then work as for a release. A staged
  folder recording another channel (two AgentMux side by side upgrading to
  the same release) isn't shared: a copy is unpacked beside it.

## 4. Applying it

### 4.1 The user's flow (one button, from fast-startup §4.3)

1. Chip: **Update to 0.59.27**. The panel shows the release notes and
   **Download** (size). Downloading is a background job with progress; the
   app keeps running.
2. When it's staged: **Upgrade & restart**. The panel lists what will happen:
   which agents are working and will be paused, and that everything resumes.
   **When agents are idle** (the default) waits for working agents to finish
   their turn, with a "Now" override.
3. The launcher runs fast-startup §4.3:
   1. pause supervision;
   2. save the session snapshot;
   3. stop srv and wait for it to exit;
   4. install the staged release;
   5. run `migrate` from the new binary;
   6. relaunch once.
4. The new version comes up with the same windows, panes and agents (§6).

### 4.2 By kind of install

srv reads how it was installed from where its own binary is
(`backend/update_install.rs`), and names the release file for that kind as
CI publishes it (`.github/workflows/build-*.yml`).

| Install | How it's recognised | Release file | Apply | Phase |
|---|---|---|---|---|
| Windows portable | `runtime/agentmux-portable.marker` beside srv, `agentmux.exe` above it | `agentmux-<v>-x64-portable.7z` | Unpack with Windows' `tar.exe` into `agentmux-<v>-x64-portable` next to the running folder (or under `updates/<v>/` in the running version's data folder when that can't be written); the launcher starts the new folder's launcher and exits. The old folder stays; Maintenance offers to delete old versions | 3a, 3b |
| Windows `setup.exe` (Inno Setup, per-user by default, fixed AppId) | Inno Setup's uninstaller (`unins000.exe`) beside `agentmux.exe` (the install has the portable's `runtime/` and marker too) | `AgentMux-<v>-x64-setup.exe` | Downloaded and verified into this version's `updates/<v>/`. After quit, a hidden Windows PowerShell (by its full path, the script passed `-EncodedCommand` so no shell parses the paths) waits for the launcher to be gone, opens the installer so no other process can write, rename or delete it (the folder is the user's own, and an all-users install runs it elevated), checks its SHA-256 through that handle against the release's (which srv sends with the request, and the launcher also checks before asking), and only on a match runs it silently, holding the handle until it exits (`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-`; the install keeps its own mode, all users or the current one, through Inno's `UsePreviousPrivileges`, and Windows asks for permission for an all-users one), which replaces the install in place, then starts AgentMux again even if the installer failed or was declined, so the old version runs rather than none | 3d |
| Microsoft Store (`.msix`, Store ID 9P9QCXNNCRK3) | under `WindowsApps` | — | The Store updates it; AgentMux can't replace its own files. "Update in the Store" opens the Store page, or asks the Store API to install it. The Store certifies a release after GitHub publishes it, so the chip defers to the Store | 3f |
| macOS `.dmg` | inside an `.app` | `AgentMux_<v>_arm64.dmg` | Mount, check `codesign --verify` and the Gatekeeper assessment, replace `AgentMux.app` after quit, relaunch. The bundle id carries the version (`ai.agentmux.<channel>.<version>`), so check first whether macOS asks for permissions again after each upgrade | 3c |
| Linux AppImage | `$APPIMAGE`, and srv itself runs from it (inside `$APPDIR` or AppRun's extraction, `$AGENTMUX_EXTRACTED_RUN`): `APPIMAGE` alone is inherited by anything started from an AppImage instance's pane | `AgentMux_<v>_amd64.AppImage` | Downloaded and verified beside the running AppImage (or in `updates/<v>/` when that folder can't be written), made executable, this AgentMux's channel recorded beside it (`<file>.agentmux-channel`: the AppImage itself can't be written). After quit the launcher starts it with `APPIMAGELAUNCHER_DISABLE=1` (AppImageLauncher would otherwise stop it at its "integrate?" window until someone answers, tested on a GNOME host) and without what AppRun set for this version (`LD_LIBRARY_PATH`, `APPIMAGE`, `APPDIR`, `OWD`, `ARGV0`). The old file stays | 3e |
| Linux portable (the release tarball) | `AgentMux/agentmux.sh` two levels above srv's `usr/bin/` (the tarball has no portable marker) | `AgentMux_<v>_amd64-portable.tar.xz` | Unpacked with the system `tar` (with `xz`) into `AgentMux-<v>` beside the running folder (or under `updates/<v>/`), checked by its `usr/share/agentmux/VERSION` and that version's srv, the channel recorded at its top. After quit the launcher starts its `agentmux.sh`, which sets the library path for the new folder, without the old one's environment (as for the AppImage) | 3e |
| Linux `.deb` / `.rpm` | under `/usr` or `/opt` | `AgentMux_<v>_amd64.deb` / `AgentMux-<v>-1.x86_64.rpm` | Install with the desktop's password prompt (`pkexec`); later an apt/dnf repository so system updates do it | 3g |

### 4.3 Verifying a download

- Release files come only from
  `https://github.com/agentmuxai/agentmux/releases/download/`, as the
  release lists them; a file the release lists without a SHA-256 isn't
  offered.
- GitHub lists a SHA-256 for every release file (`digest` in the releases
  API). The download is hashed as it arrives and must be exactly that size
  and hash, or it is deleted and the reason shown.
- That digest comes from the same place as the file, so it proves the file
  arrived whole, not who built it. A `SHA256SUMS` signed with a key the app
  pins is phase 5.
- What is unpacked must be a portable of exactly that version (the
  launcher, the marker, `runtime/agentmux-<v>.exe`) before it is moved into
  place. `tar.exe` refuses `..` and absolute paths, and is run by its full
  path (`%SystemRoot%\System32\tar.exe`), never whatever `tar` is on `PATH`.
- On Windows and macOS the installed binaries' code signatures are checked
  before swapping (Authenticode, `codesign --verify` / notarization) where
  the release is signed.
- Never a downgrade: a tag older than the running build is ignored.

## 5. Agents

### 5.1 The `App` tool: `version` and `upgrade`

One MCP tool with an `action`, as every MuxAPI family is
(`SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md`). It is a tool of its own,
like `Stop`, because `upgrade` restarts every agent and a harness can gate
it (`crates/mcp/src/tools/app_tool.rs`):

```
App({ action: "version" })
App({ action: "upgrade", reason: "…", user_instruction?: "…", version?: "0.59.27" })
→ started | pending_user | up_to_date | unavailable
```

- **`version`** (`GET /api/v1/app/update-status`): the running version and
  install kind, the latest release, whether it is downloaded, and whether
  this install can upgrade itself. An agent can say "an update is
  available" without asking for anything.
- **`upgrade`** (`POST /api/v1/agent/self/upgrade`,
  `backend/upgrade_request.rs`) **never restarts on the agent's word
  alone.** A restart stops every agent, interrupts the user's own work, and
  an agent can be prompt-injected. Same trust model as `QuitSelf`
  (`self_quit::gate`):
  - the user's own turn asked for it (the turn was started by the user,
    nothing else was delivered since, and `user_instruction` quotes them)
    → `started`: the §4.1 flow begins (download if needed, then "when agents
    are idle");
  - anything else (an agent's own idea, a jekt, a broadcast, cron) →
    `pending_user`: every window shows "QTest asks to upgrade AgentMux to
    0.59.27" with the reason, **Upgrade now** / **Later**
    (`frontend/app/block/upgrade-requests.tsx`; `app:upgraderequests`,
    `app.upgrade-requests`), with the waiting tone. The answer reaches srv
    only through the host (`approvals.decideUpgrade` →
    `POST /api/v1/host/upgrade_request`, checked with the host's own token),
    like a widget approval, so an agent holding the instance key can't answer
    its own request. It names the release the prompt showed: if a newer one
    came out since, nothing is installed and the agent is told to ask again.
    The agent gets an `[AgentMux]` note with the answer; Upgrade now answers
    every waiting request for that release.
  - Either way the launcher asks the user once more in its own dialog
    before it restarts (§4.1, phase 3b).
- **Only what the update check found is installed**: the latest release,
  verified against GitHub's checksum. The tool can't name a URL or a file,
  and a `version` other than the latest is refused. An install that can't
  upgrade itself yet answers `unavailable` with the release page.

### 5.2 Continuing after the restart

What already happens, and what's added:

- **Windows, tabs and panes** are restored from the session snapshot (#2560).
- **Each agent resumes its conversation** (eager resume, `--resume`).
- **Messages sent while AgentMux was down** are held and replayed
  (durable jekts).
- **New: a system note to each resumed agent**, once, soon after it resumes (a message queued during the resume may come first; the note asks nothing)
  (`backend/upgrade_notice.rs`, phase 4b):
  `[AgentMux] AgentMux was upgraded from 0.59.26 to 0.59.27 at 14:02. Your session resumed; nothing is needed from you. Its tools may have changed: a tool description you loaded before the upgrade can be out of date, so load it again before relying on the actions it lists.`
  The last sentence (added 2026-10-17): a resumed conversation still holds any tool schema the agent loaded before the upgrade, so it can miss actions the new tool has.
  srv records the version it runs in the channel folder
  (`channels/<ch>/agentmux-last-version.json`, like `cli_notice`'s CLI
  versions: each version's data folder is new after an upgrade). A start on
  a newer release than the one recorded is an upgrade, and the record keeps
  it, with the panes already told, for an hour: a crash or another restart
  soon after still tells the agents not yet told, never one twice. The hook
  is registration with the reactive handler (`reactive::registered`), which
  every agent that can receive a note goes through, whatever runs it; the
  note goes to a pane that already had a session (`agent:sessionid`), not a
  new one. A first start, the same version, or an older build started again
  says nothing.
- **The agent that asked** hears every step: the user's answer to its
  request as a note (phase 4a), and, after the restart, the note above.
  A failed download or a restart that didn't happen shows in the version
  panel, with the reason.
- **An agent that was mid-turn** when the user chose "Now" resumes with the
  CLI's own interrupted-turn handling and the note.

## 6. Prerequisite: the data must follow the upgrade

Installed and portable builds keep their data per version
(`channels/<ch>/versions/<v>/data`); config, agents and accounts are shared
across versions. Nothing carried the previous version's databases forward
(`launcher/src/data_dir.rs` migrates only the old unversioned layout), so an
upgraded install started with an empty `objects.db`:
- the window layout and session snapshot were lost;
- each pane's `agent:sessionid` was lost, so agents resumed only through the
  shared registry (OpenAgent's resume).

**Verified** on a real stable install: 0.55.42, 0.56.9 and 0.58.1 each have
a different `db_client` id, so each started from nothing. Agent definitions
came back only because they are also kept in the shared store.

**Fixed in phase 2** (`crates/srv/src/bootstrap/carry_forward.rs`): on the
first start of a version (no `objects.db` yet), before the pre-migration
snapshot and migrations, srv copies `filestore.db`, `sagas.db` and then
`objects.db` from the newest older version in the same channel. Each copy
is `VACUUM INTO` a temporary file, renamed into place:
- a consistent snapshot even if the older version is still running (two
  portable folders side by side; on Windows `srv.lock` doesn't exclude);
- never a half-written file; any failure removes what was copied and the
  version starts fresh, as before;
- never from a newer version, and never over a version that has started;
  a database already in the new version's folder is kept, and a failed copy
  removes only what it copied;
- srv first prints `AGENTMUXSRV-MIGRATING`, so the launcher allows 30 minutes
  instead of 30 s for a large `filestore.db`.

The other files in `data/` are not copied: logs, locks and `authkey.dev` are
per run, `shell/` is regenerated, and the zones flag only matters to a store
with no migration records. Dev builds keep data per branch and local
`task package` builds get a fresh channel per build on purpose, so neither
has an older version to copy from
(`REPORT_AGENT_HISTORY_LOST_ON_NEW_BUILD_2026_09_25.md`).

### 6.1 Every window comes back

Carrying the databases forward isn't enough on its own. A simulated
0.59.28 → 0.59.29 upgrade of a portable (two local builds in a channel of
their own, a local server standing in for GitHub's release) showed every
step working (download, the launcher's dialog, the restart into the new
folder, `carry-forward: started from the previous version's data`), but the
windows came back only as far as the last one:

- An upgrade quits the way the tray's Quit does (`quit_app` in
  `crates/cef/src/commands/window/lifecycle.rs`): it closes the windows one
  by one, and each close deletes its workspace.
- The session record (`Client.meta["session:last_topology"]`,
  `SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md` Feature 1) was saved
  only by the close that empties `Client.windowids`, so it held the last
  window's workspace alone; the OS-shutdown flush
  (`SPEC_CONTINUOUS_SESSION_PERSISTENCE_2026_09_08.md` Phase 0) held the first.
- On the next start the main window restored that one workspace; nothing
  reopened the others.

**Fixed** (`crates/srv/src/server/service/session_windows.rs`,
`crates/cef/src/commands/window/restored.rs`):

- **Saved before anything closes.** `quit_app` asks srv to save the session
  (`window.SaveSessionSnapshot` with `for_quit`) after it marks the app as
  quitting, which stops new windows, and before it closes the first one. The
  record holds every full window, the main one first, in `windows`; each
  entry is the single-workspace shape plus the window's position and size.
  The OS-shutdown flush saves every window the same way.
- **Kept through the closes.** The record is marked as a quit's (`saved_for`,
  `saved_at_ms`). The close that empties `Client.windowids` keeps such a
  record if it is under two minutes old; otherwise (closing windows one by
  one, or a quit abandoned partway) it saves its own workspace as before.
- **Restored.** The main window's restore rebuilds the first entry into
  itself, as before, and each other entry into a workspace of its own. The
  main window then takes those (`window.TakeRestoredWindows`, each handed out
  once) and asks the host to open each (`open_restored_window`).
- **Where they were.** srv keeps a window's position and size as
  `GetWindowRect` reports them, in physical pixels, and the host creates
  windows in DIP, so the host scales them by the monitor the window was on
  (`dpi_scale_at`). A spot on no monitor now (one unplugged since) opens the
  window where a new window goes.
- **Compatible both ways.** The record keeps the main window's tabs at its
  top level, so an older AgentMux reading it after a downgrade restores that
  window; a record without `windows` restores as one window.
- **A close the quit outran.** The host closes windows concurrently and srv
  can stop before the last close arrives; a simulated upgrade of a local
  build left the second window's row behind, and the main window then took
  that row instead of restoring. A quit's record lists the windows it saved
  (`window_ids`); at startup srv forgets any of those still listed, with its
  workspace, tabs and panes (`forget_windows_left_by_quit`, from
  `bootstrap/stores.rs`): the record holds it, and the restore brings it back.
  Any other window (no quit's record, as after a crash, or a later run's
  window under a record an unfinished restore left) is kept for the crash
  recovery to reopen.

The same simulation, with three windows (the default three panes, one
sysinfo pane, one memory pane) and two of them moved, brought all three back
with their panes, at their positions and sizes.

**Not yet:** the main window opens where the host puts it at startup, not
where it was. macOS's Cmd+Q doesn't reach `quit_app`, so there the
last-close record still applies.

### 6.2 What agents left running

Added 2026-10-18, after the operator's own upgrade killed an AgentMux an
agent had started (`task dev`, through `Start-Process` from its Bash tool),
without a word to anyone.

Everything an agent starts runs in its pane's process tracker (a Windows job
object, a Linux cgroup) that nothing can leave, so a quit, an upgrade's
included, stops it with the pane. That stays: letting processes escape a
pane was rejected on purpose (`SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md`
§0). What changes is that it no longer happens silently
(`crates/srv/src/backend/background_work.rs`):

- **Listed before.** While an upgrade's restart waits or is asked for, the
  version panel lists what agents left running, one line each ("Korp: an
  AgentMux instance (`task dev TITLE=KorpForms`), running 9 h"), from each
  agent pane's tracked processes (`AgentProcessRegistry::list_block`, one
  item per process tree; another AgentMux is recognised by its launcher,
  host or srv). The launcher's dialog names the same list (srv leaves it in
  `updates/stopping.txt` beside the staged update), and an agent whose own
  turn started the upgrade (`App` `upgrade`) is told in the answer.
- **Stopped gracefully where it matters.** On Windows the quit's session
  save (`SaveSessionSnapshot` with `for_quit`) then asks another AgentMux
  among it to close its windows (`WM_CLOSE` to its host's visible windows),
  so it saves its own session as a quit does, and waits up to 0.9 s for it
  within the host's 1.5 s. On Linux and macOS it still stops with its pane:
  a signal to its host ends it without its quit, and reaching its own quit
  (its tray command) is left for later.
- **Told after.** The quit records the list in the channel folder
  (`agentmux-stopped-at-quit.json`); after the next start each agent that
  resumes within the hour is told, once, what of its own was stopped:
  `[AgentMux] When AgentMux quit at 00:23, it stopped what you had running in
  the background: an AgentMux instance (`task dev TITLE=KorpForms`, started
  22:30). Start it again if you still need it.`

Any quit does this, not only an upgrade's.

## 7. Phases

Each phase is small and useful on its own.

1. **Detection and the chip.** The srv check, the `app:updatestatus` event
   and `app.update-status` command, the chip, and the panel's "What's new"
   with **Download** (opens the release page). This is enough for users to know.
2. **Data carries forward** (§6). Verified and fixed in #4996. Needed
   before any in-app upgrade.
3. **In-app upgrade**, one install kind at a time (§4.2):
   - **3a**: the Windows portable is downloaded, verified and unpacked next
     to the running folder (`backend/update_download.rs`); Maintenance shows the progress, then "Show
     folder". Every other kind still opens the release page.
   - **3b**: **Upgrade & restart** (`backend/update_restart.rs`,
     `crates/launcher/src/upgrade_restart.rs`):
     1. srv waits for working agents to finish their turn, up to 10 minutes;
        **Restart now** skips the wait and **Cancel** stops it (§8);
     2. srv writes `AGENTMUXSRV-UPGRADE version:<v> launcher:<path>` on its
        stderr, which the launcher already reads;
     3. the launcher checks the folder again (a portable of exactly that
        version, newer than itself, and itself a portable, not a `setup.exe`
        install) and asks the user in a native dialog. No page or agent tool
        in AgentMux can click it, so a restart always takes the user's own
        Yes, whoever asked srv;
     4. it quits the host like the tray's Quit (windows close, the session is
        saved), stops srv, and only then starts the new launcher without this
        instance's `AGENTMUX*` environment, and exits. A Yes that hasn't led
        to a quit within 5 minutes lapses; srv shows "didn't restart" after 3.

     Verified end to end on a packaged portable: download, verification,
     staging, the wait, the dialog, the quit, and the new launcher coming up
     on its own channel within a second of the old one exiting.

     Only the last-closed window's tabs come back after any quit today
     (`session:last_topology`); restoring every window is a follow-up.
   - **3d**: `setup.exe` (§4.2): the same flow, with the installer as
     what srv stages (`installer` in the download status; the startup line
     names `installer:<path>`), which the launcher accepts only as that
     version's installer in its own version's `updates/<v>/` on a
     `setup.exe` install. The panel's **Show installer** reveals it in
     Explorer; it never opens it.
   - **3c** macOS, **3e** AppImage and Linux portable, **3f** the Store,
     **3g** `.deb`/`.rpm`.
4. **Agents.**
   - **4a**: the `App` tool's `version` and `upgrade`, with the gate, the
     prompt in every window and the answer to the asker (§5.1).
   - **4b**: the post-restart note to each resumed agent, and "done" to the
     agent that asked (§5.2).
5. **Hardening and cleanup:**
   - **5a**: the version panel's **Download**, **Upgrade & restart** (and
     **Restart now**) and **Cancel** reach srv only through the host
     (`POST /api/v1/host/app_update`, the CEF host's `app_update_action`),
     like the answer to an agent's request (§5.1). They were RPCs any caller
     with the instance auth key could send, which every agent holds; the
     `agent_id` check on them stopped only callers that said who they were.
     The `app.update-download`, `app.upgrade-restart` and
     `app.upgrade-cancel` commands are gone.
   - Later: a signed `SHA256SUMS` release step, code signature checks
     before a swap, and deleting old versions.

## 8. Decisions (the operator took these recommendations, 2026-10-15)

1. **The whole app restarts**, not only srv: a release changes the host and
   the launcher too. (Settles fast-startup §9.)
2. **Timing:** "Upgrade & restart" waits for working agents to finish their
   turn, up to 10 minutes, then asks again; **Now** is always available.
3. **`RequestUpgrade`:** the user's own turn asking for it is enough, as for
   `QuitSelf`; anything else raises the banner and waits for a click.
4. ~~The update check is on by default (`app:updatecheck`), one
   unauthenticated request to GitHub every 6 hours.~~ Replaced 2026-10-18:
   **no automatic check.** AgentMux asks GitHub only when the user clicks
   Check for update or an agent asks; the button is always shown, and a
   check pins what is installed (§3).
5. **Old versions:** keep the running version and the one before it; after
   a successful upgrade, Maintenance offers to delete older version folders
   and their data.
6. **Local and dev builds:** no upgrade path. The panel says which release
   is latest and links it.

## 9. Docs this replaces

When this ships, the three specs listed under **Consolidates** get Status
`superseded` pointing here: `app-update-check.md`, `SPEC_UPGRADE_PANEL` (its
"not yet implemented" is already wrong) and fast-startup §5. Fast-startup's
migration design and launcher flow stay.
