# Report: what the first real in-app upgrade (0.59.31 → 0.59.32) lost, and what to fix

**Date:** 2026-10-18
**Author:** QTest
**Status:** analysis
**Context:** the operator upgraded their own working instance (a Windows portable,
local channel `local-main-…`, 0.59.31 built from `main`) to the v0.59.32 release
from Maintenance, with several agents running in it. This was the first
release-to-release style upgrade of a real working instance with everything from
`SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` in it. This report collects what
the operator noticed, what the logs and data show, the cause of each, and a
proposed priority order, together with the findings from the same day's review
of the update check.

## 1. What worked

- The upgrade itself: download, verify, unpack, confirm, restart, 0.59.32 running
  in the same channel. The 0.59.31 folder is still beside it for a rollback.
- Data: the new version started from 0.59.31's databases (`carry-forward: started
  from the previous version's data`). No migration was pending, so none ran.
- Agents: every agent pane resumed its conversation (`session:resume_preflight …
  verdict: resume`), and the resumed agents got the `[AgentMux] AgentMux was
  upgraded from 0.59.31 to 0.59.32` note.
- Logs: no `ERROR` from srv, no panic. The warnings are the same kinds the
  0.59.31 log already had (blank `identity_id` on legacy rows, a Kimi account's
  secret resolution, file watches on agent memory folders that don't exist yet).

## 2. What the upgrade lost

### 2.1 Window names (operator report)

**What happened.** The window came back without its name. A workspace name
would be lost the same way.

**Cause.** A quit saves the session (`session_windows.rs::snapshot_session`) with,
per window, its tabs and its position and size only (`x`, `y`, `width`,
`height`). The window's name is `window:displayname` in the Window row's meta;
the workspace's name is the Workspace row's `name`. Neither is in the record.
The quit then deletes the windows and workspaces, and the restore creates new
ones: an unnamed window, and a workspace made with `CreateWorkspace { name: "" }`
(`session_restore.rs`). Tab names are saved and do come back.

**Scope.** Not specific to upgrades: any quit and relaunch goes through the same
path, so a plain restart should lose the names too (follows from the code; not
separately reproduced).

**Fix (small).** Save `window:displayname` and the workspace name in each window
entry of the record; on restore, give the workspace its name and set the name on
the window that is created for it (the main window, and each window
`open_restored_window` opens). Unit tests beside the existing session-window
tests.

### 2.2 MuxBus sign-in (operator report)

**What happened.** After the restart MuxBus was signed out, and the operator had
to sign in again (`muxbus: PKCE login started` at 07:24, a minute after the
restart).

**Cause.** The sign-in itself was kept: this channel's row is still in
`db_muxbus_credentials`. What didn't happen is the reconnect. srv logged
`cloud_subscriber: init_global skipped (ISOLATED — channel-scoped, no automatic
MuxBus reconnect)`. `isolated_muxbus_reconnect_reason()` (`common/src/data_paths.rs`)
skips the reconnect at launch on every channel that isn't `stable` or `dev-*`,
which includes every local package channel (`local-…`). The reason is macOS
only: each local build has a new random channel and bundle id, so the
keychain asks for consent again on every build. The rule is decided by the
channel name, so it also applies:

- on Windows and Linux, where reading the stored sign-in never prompts;
- to a release build, whose identity is stable, once it runs in a local channel,
  which is exactly what a local build becomes after it upgrades to a release.

So every restart of a local build, and every upgrade of one, signs MuxBus out
in practice, and review and CI notices stop arriving until the user notices.

**Fix (small).** Skip the reconnect at launch only where the prompt problem
exists: on macOS, for a local build (its build label carries `+…`, e.g.
`0.59.31+g468f89ca5…`; a release's is the bare version). Everywhere else,
reconnect at launch when the channel has a stored sign-in. A unit test per case
beside the existing ones in `data_paths.rs`.

### 2.3 An agent's dev instance was killed (operator report)

**What happened.** Korp had a full `task dev` instance running (launcher, host,
srv and Vite, window titled KorpForms) since the previous evening. The upgrade
killed it, without a warning to the user or to Korp.

**How it was started** (Korp's own account): from Korp's Bash tool, with
`powershell Start-Process … bash -lc "… task dev TITLE=KorpForms"`, hidden. So
it looked detached, but it was still a descendant of Korp's agent process.

**Cause.** Each pane's processes run in a Windows job object of their own
(`process_tracker/windows.rs`), created with `KILL_ON_JOB_CLOSE` and
deliberately without `BREAKAWAY_OK`, so nothing an agent starts can leave it
(`Start-Process` included). On Linux it is a cgroup and on macOS a process
group, with the same effect. The quit that the upgrade starts closes every
pane, and the 0.59.31 log shows `[process-tracker] dropped tracker on pane close`
for each at 07:23:45. Closing the job killed everything in it, Korp's dev
instance included. The launcher's own job (J0) would have killed it
regardless a moment later.

**Scope.** Every quit does this, not only an upgrade's; an upgrade just makes a
quit happen when the user didn't plan one. Agents start dev instances and
dev servers routinely (it is the documented way to test AgentMux), so this
will recur.

**Can the scheme detect it? Yes, cheaply.** srv already knows each pane's live
processes: `AgentProcessRegistry::members_by_block()` reads the job's process
list (`JobObjectBasicProcessIdList`; `cgroup.procs` on Linux). Declared
background Bash tasks are also recorded in `db_background_tasks` with their
pid, label and start time.

**Graceful handling, proposed:**

1. **Detect.** When an upgrade (or any quit) is about to stop the agents,
   list each pane's processes beyond the agent's own CLI and its tool calls in
   flight, and pick out the long-lived ones: an AgentMux instance (its
   launcher, host or srv executable), a dev server (Vite, node), a running
   build. Name each in words: "Korp: an AgentMux dev instance (`task dev`,
   running 9 h)".
2. **Warn.** Show the list where the upgrade is confirmed: the Maintenance
   row while it waits, the launcher's confirm dialog, and the answer an agent
   gets from `App` `upgrade`. Upgrade anyway, or cancel.
3. **Stop it gracefully.** Ask an AgentMux instance to quit through its own
   quit path (so it saves its windows and its agents' sessions, as this
   instance does), and give other processes a polite stop with a short grace
   period, before the job is closed.
4. **Tell the owner.** After the restart, the upgrade note to each agent lists
   what of its own was stopped (label or command, start time), so it can start
   it again. The list is saved with the upgrade record that the note already
   reads.

Keeping such processes alive across a quit is not proposed. On Windows it
would need job breakaway, which `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md`
§0 rejected on purpose, because it would let any process escape a pane's
containment.

### 2.4 Input history (operator report)

**What happened.** ArrowUp in an agent's composer no longer recalls the
messages sent before the upgrade.

**Cause.** The sent-message history is kept per *block* in the window's
`sessionStorage` (`frontend/app/view/agent/reload-persistence.ts`, up to 200
entries). It survives a reload of the window, but not an app restart, and a
restore gives the panes new block ids anyway. The unsent draft is kept the
same way and is lost the same way.

**Proposal.** A per-agent, durable input history that lives with the agent,
in a spec of its own that lands with its first phase.

### 2.5 Browser-pane sign-ins and the app's own local storage (found while checking)

**What happened.** Not reported yet, but the data shows it: the browser panes'
cookie jar in 0.59.31 held 17 cookies for 4 sites, and 0.59.32 started with an
empty one, so any site a browser pane was signed in to is signed out.

**Cause.** CEF's cache folder, which holds the Personal cookie jar, the named
browser profiles (`profile-<id>`) and the app UI's own `localStorage`, is per
version (`channels/<ch>/versions/<v>/cef-cache`), and carry-forward copies only
`data/db`. The app UI keeps a few things in `localStorage` (8 modules: agent and
memory sort orders, Swarm's retired rows, split sizes, widget ports, and
others); those reset too.

**Fix (medium).** On a version's first start, copy the profile data that must
persist from the previous version's `cef-cache`, before CEF starts:
`Default/Network/Cookies`, `Default/Local Storage`, `Default/IndexedDB`, the
`profile-*` folders, and `Local State`. `Local State` matters: it holds the key
the cookies are encrypted with (`os_crypt`), and without it the copied cookies
can't be read. Copy rather than move, so the older version still works for a
rollback, and copy only forward, never into an older version, since Chromium
refuses a profile written by a newer Chromium. An alternative is one
`cef-cache` per channel instead of per version, which breaks that rollback
property; copying is the safer choice.

## 3. Findings from the update-check review (same day)

- **No background update check.** srv asks GitHub's release API a minute after
  start and every 6 hours (`update_check.rs`, setting `app:updatecheck`, on by
  default). The operator wants no polling: check only when the user clicks
  Check for update or an agent asks (`App` `version`). Agreed; in progress.
- **Check for update must always be there.** It shows only while no newer
  release is known. Once a release is offered, the user can't look again, so
  an install can be left offering an outdated release. Show it in every state
  ("Check again" once a release is known); a new check replaces the offer.
- **A check pins its result.** Download and upgrade use exactly the release,
  file and SHA-256 that check found, never "latest" read again later. Mostly
  how it works already; make it explicit and clear a stale offer on a new
  check.
- **Skipping versions works by design but is untested end to end.** The check
  offers the latest release, migrations run every pending one by id, and the
  objects schema steps from any version. No test takes a real old install to
  current in one start. Proposed: a long-jump test from old official portables
  (0.58.x, 0.59.1x), then fixtures in CI.
- **No declared upgrade floor.** `migrations/mod.rs` says migrations may be
  retired once "the minimum supported upgrade path" passes them, but no such
  minimum exists. Proposed: a `min_upgrade_from` the release publishes and the
  check reads; below it, say what to do (download the installer, or update to
  a named version first).
- **State outside the databases** (registry record schemas, settings, session
  JSON) has its own version checks; audit them for long jumps.
- **A delivered upgrade note isn't logged**, only a failed one; add an info
  line so the logs can show delivery.

## 4. Proposed priority order

The user-visible losses come first, because every upgrade from now on repeats
them; then the update-check changes the operator asked for; then robustness.

| # | Item | Size | Why this place |
|---|------|------|----------------|
| 1 | Update check on demand only; button always shown; check pins its result (§3) | S | Operator decision, in progress |
| 2 | MuxBus stays signed in across restarts and upgrades (§2.2) | S | Silent: review and CI notices stop until noticed |
| 3 | Window and workspace names restored (§2.1) | S | Every quit loses them |
| 4 | Agents' background processes: detect, warn, stop gracefully, tell the agent (§2.3) | M | Kills agents' work without a word; recurs with every upgrade |
| 5 | Browser-pane sign-ins and UI local storage carried forward (§2.5) | M | Every upgrade signs every browser pane out |
| 6 | Agent input history, durable and per agent (§2.4, own spec) | M–L | Framework work; phase 1 alone fixes the visible loss |
| 7 | Long-jump test from old releases, then CI fixtures (§3) | M | Proves skipping versions before users depend on it |
| 8 | Upgrade floor `min_upgrade_from` (§3) | S | Guards migration retirement |
| 9 | Audit non-database state for long jumps; log note delivery (§3) | S | Hygiene |

Still open from the upgrade spec, unchanged by this report: macOS (waiting on
the permission-prompt result from starpower), phase 5 hardening (signed
checksums, a signature check before the swap, removing old versions), `.deb`
and `.rpm`, the Microsoft Store hand-off, restoring the main window's position,
and the `-x64-setup.exe` real-install test on Area54.

## 5. Status (same day)

Every item in §4 is done or in review:

| # | Item | PR |
|---|------|----|
| 1 | Update check on demand only; button always shown; a check pins its result | #5099 |
| 2 | MuxBus stays signed in across restarts and upgrades | #5098 |
| 3 | Window and workspace names restored | #5101 |
| 4 | Agents' background work: listed before, another AgentMux's windows kept, the agent told after | #5104 |
| 5 | Browser-pane sign-ins carried forward | #5105 |
| 6 | Agent input history, phase 1 (its spec: `SPEC_AGENT_INPUT_HISTORY_2026_10_18.md`) | #5109 |
| 7 | Long jumps checked on real old data (`scripts/check-long-jump.sh`) | #5110 |
| 8 | Upgrade floor (`UPGRADE_FLOOR`) | #5110 |
| 9 | Note delivery logged (#5099); state outside the databases audited (below) | #5110 |

**The audit (item 9).** What else an install keeps, and whether a long jump
can trip on it:

- Registry and definition records (`registry/schema.rs`, `def_schema.rs`):
  both still load schema 1 (`MIN_SUPPORTED_SCHEMA`, `DEF_MIN_SUPPORTED_SCHEMA`),
  so every older record loads. A record below the minimum is skipped, not
  migrated; both constants now say to raise `UPGRADE_FLOOR` with them.
- Settings (`settings.json`, `widgets.json`): unknown keys are kept as they
  are, and renamed ones are moved by a migration (`m0037`), which a long jump
  runs like any other.
- The session record: a record without `windows` restores as one window, and
  one without names as unnamed windows (#5101).
- The upgrade notice record and the channel record compare versions, not
  steps, so any older version reads correctly.
- The browser profile and the databases are copied from the newest older
  version present, whichever it is.

Nothing found needs a change beyond the two comments.

## 6. Evidence

- New version: `channels/local-main-b28b7a-651ced27/versions/0.59.32/logs/agentmuxsrv-v0.59.32.log.2026-10-18`
  (carry-forward, resume, MuxBus lines, `init_global skipped (ISOLATED …)`).
- Old version: `…/versions/0.59.31/logs/agentmuxsrv-v0.59.31.log.2026-10-18`
  (`upgrade: asking the launcher to restart` 07:23:28, `snapshot flushed for
  quit` 07:23:44, `dropped tracker on pane close` ×7 at 07:23:45).
- The quit's session record in 0.59.31's `objects.db` (`Client.meta
  ["session:last_topology"]`): one window, its `window` entry holding only
  `x`, `y`, `width`, `height`.
- `cef-cache/Default/Network/Cookies`: 17 cookies for 4 sites in 0.59.31, a new
  jar in 0.59.32.
- Korp's description of how the dev instance was started (agent message,
  2026-10-18).
