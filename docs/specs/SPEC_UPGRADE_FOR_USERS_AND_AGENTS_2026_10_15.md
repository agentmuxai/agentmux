# SPEC: upgrading AgentMux, for users and for agents

**Date:** 2026-10-15
**Status:** active — phase 1 (detection, the chip, Maintenance's "What's
new" and Download) in #4983; phases 2–5 are not started. The operator took the §8 recommendations on 2026-10-15.
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
| "Update available" chip and the panel's update row | UI built (`UpdateStatus.tsx`, `MaintenanceSection.tsx`), never shown: nothing emits `app-update-status`, so `updaterStatusAtom` stays `up-to-date` |
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

Detected lazily, when the user clicks (fast-startup §5.3).

| Install | Download | Apply |
|---|---|---|
| Windows portable `.7z` | The release's `.7z` | Extract next to the running folder, then the launcher starts the new folder's launcher and exits. The old folder is left in place; Maintenance offers to delete old versions |
| Windows installer (`setup.exe`) | The `setup.exe` | Run it silently after srv stops; it relaunches |
| Windows MSIX | — | Open the `.msix` (App Installer handles the swap) |
| macOS `.dmg` | The `.dmg` | Mount, verify, replace `AgentMux.app` after quit, relaunch |
| Linux AppImage / portable `.tar.xz` | The file | Replace and relaunch, like the Windows portable |
| Linux `.deb` / `.rpm` | — | Show the command or download link: the package manager owns the files |

### 4.3 Verifying a download

- Assets come only from `github.com/agentmuxai/agentmux` releases, over TLS,
  by the asset ids the release lists.
- Each release publishes SHA-256 sums (a new release step: `SHA256SUMS`, and
  later a signature over it). A download whose hash doesn't match is deleted
  and reported.
- On Windows and macOS the installed binaries' code signatures are checked
  before swapping (Authenticode, `codesign --verify` / notarization) where
  the release is signed.
- Never a downgrade: a tag older than the running build is ignored.

## 5. Agents

### 5.1 `RequestUpgrade`

An MCP tool:

```
RequestUpgrade({ reason: "…", version?: "0.59.27" })
→ { state: "started" | "pending_user" | "up_to_date" | "unavailable", … }
```

- **It never restarts on the agent's word alone.** A restart stops every
  agent, interrupts the user's own work, and an agent can be prompt-injected.
  Same trust model as `QuitSelf` (`self_quit::gate`):
  - the user's own turn asked for it (the turn was started by the user, and
    nothing else was delivered since) → `started`: the §4.1 flow begins
    (download if needed, then "when agents are idle");
  - anything else (an agent's own idea, a jekt, a broadcast, cron) →
    `pending_user`: a banner in every window, "QTest asks to upgrade
    AgentMux to 0.59.27: <reason>. Upgrade now / Later", which the user
    clicks.
- **No version is installed that the update check didn't find**: the tool
  can't name a URL or a file.
- A companion read-only tool, `VersionStatus`, answers what is running, the
  latest release, and whether a download is staged, so an agent can say "an
  update is available" without asking for anything.

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
across versions. I found no code that carries the previous version's
`objects.db` forward (`launcher/src/data_dir.rs` migrates only the old
unversioned layout). If that's right, an upgraded stable install starts with
an empty `objects.db`:
- the window layout and session snapshot are lost;
- each pane's `agent:sessionid` is lost, so agents resume only through the
  shared registry (OpenAgent's resume).

**To verify first, then fix:** on the first start of a new version in a
channel, copy the newest older version's `data/` into the new version's, then
let migrations upgrade it (with the pre-migration snapshot that already
exists). Local `task package` builds get a fresh channel per build on
purpose and are out of scope (`REPORT_AGENT_HISTORY_LOST_ON_NEW_BUILD_2026_09_25.md`).

## 7. Phases

Each phase is small and useful on its own.

1. **Detection and the chip.** The srv check, the `app-update-status` event,
   the chip, and the panel's "What's new" with **Download from GitHub**
   (opens the release page). This is enough for users to know.
2. **Data carries forward** (§6). Verify, then fix. Needed before any
   in-app upgrade.
3. **In-app upgrade for the Windows portable and macOS.** Download, verify,
   stage, then the launcher flow: wire `run_migration_upgrade` (fast-startup
   phase 1b) and the relaunch.
4. **Agents.** `VersionStatus`, `RequestUpgrade` with the gate and the
   banner, the post-restart note, and the result to the asker.
5. **The other installers** (`setup.exe`, `.msix`, AppImage, `.deb`/`.rpm`
   guidance), the `SHA256SUMS` release step, and cleanup of old versions.

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
