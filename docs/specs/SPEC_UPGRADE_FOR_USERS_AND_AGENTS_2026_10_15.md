# SPEC: upgrading AgentMux, for users and for agents

**Date:** 2026-10-15
**Status:** active — phase 1 (detection, the chip, Maintenance's "What's
new" and Download) in #4983; phase 2 (data carries forward) in #4996;
phase 3a (download and stage, Windows portable) in #5000; phase 3b
(Upgrade & restart) in #5005; phase 4a (the `App` tool) in #NNNN; the
rest of phases 3–5 are not started. The operator took the §8 recommendations on 2026-10-15.
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
- **When:** 60 s after srv starts, then every 6 h while running. Never
  during startup, never blocking anything (fast-startup §5). Offline,
  rate-limited or any error means "no news", not a retry loop.
- **What's kept** (`backend::update_check::AppUpdateStatus`): this build's
  version, the latest release's version, page and notes (the release body,
  which is the `VERSION_HISTORY.md` section, up to 16 KB), when it was
  published, and when it was checked. Phase 3 adds its assets and their sizes.
- **Published as:** the typed `app:updatestatus` event when the status
  changes, and the `app.update-status` command for a window that opens
  later. States: `up-to-date`, `available`, `local-build`, `off` (the check
  is turned off) and `unknown` (not checked yet). Phase 3 adds the download
  states.
- **Setting:** `app:updatecheck` (on by default; off means no request is
  ever made). It is re-read every minute: turning it off clears the chip at
  once, turning it on checks at once.
- **Local and dev builds** (`task package`, `task dev`; channel `local-…` or
  `dev-…`) don't offer an update: they aren't released versions. The panel
  says "Local build. The latest release is v0.59.27." and links the release.

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
| Windows `setup.exe` (Inno Setup, per-user by default, fixed AppId) | any other Windows install | `AgentMux-<v>-x64-setup.exe` | The launcher exits, the installer runs silently (`/VERYSILENT /SUPPRESSMSGBOXES`) and replaces the install in place; a `[Run]` entry for silent runs relaunches it. An all-users install asks Windows for permission | 3d |
| Microsoft Store (`.msix`, Store ID 9P9QCXNNCRK3) | under `WindowsApps` | — | The Store updates it; AgentMux can't replace its own files. "Update in the Store" opens the Store page, or asks the Store API to install it. The Store certifies a release after GitHub publishes it, so the chip defers to the Store | 3f |
| macOS `.dmg` | inside an `.app` | `AgentMux_<v>_arm64.dmg` | Mount, check `codesign --verify` and the Gatekeeper assessment, replace `AgentMux.app` after quit, relaunch. The bundle id carries the version (`ai.agentmux.<channel>.<version>`), so check first whether macOS asks for permissions again after each upgrade | 3c |
| Linux AppImage | `$APPIMAGE` | `AgentMux_<v>_amd64.AppImage` | Replace that file, relaunch | 3e |
| Linux portable | the portable marker | `AgentMux_<v>_amd64-portable.tar.xz` | Like the Windows portable, with the system `tar` | 3e |
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
    `app.upgrade-requests`, `app.upgrade-request-answer`, which refuses an
    agent connection), with the waiting tone. The agent gets an `[AgentMux]`
    note with the answer; Upgrade now answers every waiting request.
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
- **New: a system note to each resumed agent**, once, as its next input:
  `[AgentMux] Upgraded from 0.59.26 to 0.59.27 at 14:02, at QTest's request. Your session resumed; nothing you sent was lost.`
  srv records the version it last ran in the shared store; on start, if it
  differs, every agent that resumes gets the note (a system note, like the
  other `[AgentMux]` notes; never a jekt).
- **New: the agent that asked** gets the result: "Upgrade to 0.59.27 done",
  or why not ("cancelled by the user", "the download failed: …").
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

## 7. Phases

Each phase is small and useful on its own.

1. **Detection and the chip.** The srv check, the `app:updatestatus` event
   and `app.update-status` command, the chip, and the panel's "What's new"
   with **Download** (opens the release page). This is enough for users to know.
2. **Data carries forward** (§6). Verified and fixed in #4996. Needed
   before any in-app upgrade.
3. **In-app upgrade**, one install kind at a time (§4.2):
   - **3a**: the Windows portable is downloaded, verified and unpacked next
     to the running folder (`backend/update_download.rs`,
     `app.update-download`); Maintenance shows the progress, then "Show
     folder". Every other kind still opens the release page.
   - **3b**: **Upgrade & restart** (`backend/update_restart.rs`,
     `app.upgrade-restart` / `app.upgrade-cancel`,
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
   - **3c** macOS, **3d** `setup.exe`, **3e** AppImage and Linux portable,
     **3f** the Store, **3g** `.deb`/`.rpm`.
4. **Agents.**
   - **4a**: the `App` tool's `version` and `upgrade`, with the gate, the
     prompt in every window and the answer to the asker (§5.1).
   - **4b**: the post-restart note to each resumed agent, and "done" to the
     agent that asked (§5.2).
5. **Hardening and cleanup:** a signed `SHA256SUMS` release step, code
   signature checks before a swap, and deleting old versions.

## 8. Decisions (the operator took these recommendations, 2026-10-15)

1. **The whole app restarts**, not only srv: a release changes the host and
   the launcher too. (Settles fast-startup §9.)
2. **Timing:** "Upgrade & restart" waits for working agents to finish their
   turn, up to 10 minutes, then asks again; **Now** is always available.
3. **`RequestUpgrade`:** the user's own turn asking for it is enough, as for
   `QuitSelf`; anything else raises the banner and waits for a click.
4. **The update check is on by default** (`app:updatecheck`), one
   unauthenticated request to GitHub every 6 hours.
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
