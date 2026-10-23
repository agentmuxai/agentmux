# RCA: an upgrade comes back with a starter tab instead of the layout it quit with

**Status:** retro
**Date:** 2026-10-22
**Seen on:** macOS, installed build, in-app upgrade 0.59.38 → 0.59.39 (the same thing happened on 0.59.36 and 0.59.38)
**Affects:** every installed or portable AgentMux whose channel folder still has the pre-versioning `channels/<ch>/data/db/` (installs from before data was kept per version), on every platform
**Author:** Clare@starpower

## Summary

The goal of an upgrade is to come back with the same windows, tabs and panes as before it. After the 0.59.38 → 0.59.39 upgrade the app came back with one window holding a "Starter workspace" with a single empty tab, and none of the agents that were open.

The quit before the upgrade worked: 0.59.38 saved every window, tab and pane as its quit record. 0.59.39 never saw that record. Its data wasn't copied from 0.59.38. The launcher copied the channel's **pre-versioning data folder**, `channels/stable/data/db/`, last written on 2026-06-11, into `versions/0.59.39/data/db/` before srv started. srv's carry-forward, which copies the previous version's databases with the quit record in them, found an `objects.db` already in place and copied nothing.

The launcher does this on every first start of a version, so every upgrade on such an install starts from the same June data. srv's carry-forward (#4996, 2026-10-15) has never run on this machine.

## What the user saw

- **0.59.38 before the quit:** one window, workspace "Starter workspace", tab "Tab 1" with three agent panes (Clare, Maop, Masty) and a Swarm pane.
- **0.59.39 after the restart:** one window, "Starter workspace", tab "tab1" with one empty agent pane and nothing else. The agents were not resumed. One of them, Clare, came back only because the user picked it in that empty pane.

## Timeline (2026-10-22, PDT)

| Time | Event | Source |
|---|---|---|
| 18:26:07 | 0.59.38 srv: `update: 0.59.39 is ready` (dmg downloaded and verified) | 0.59.38 srv log |
| 18:26:09 | `upgrade: asking the launcher to restart` | 0.59.38 srv log |
| 18:26:12.95 | `session_restore: snapshot flushed for quit windows=1`, so the quit record is saved in 0.59.38's `objects.db` (in its WAL) | 0.59.38 srv log |
| 18:26:13.00 | `session_restore: keeping the record the quit saved for every window`; the window closes and its workspace, tab and panes are deleted (by design: the record restores them) | 0.59.38 srv log |
| 18:26:13.8 | 0.59.38 srv exits cleanly | 0.59.38 srv log |
| 18:26:32 | 0.59.39 launcher: `[migrate] copying legacy data dir …/channels/stable/data/db → …/channels/stable/versions/0.59.39/data/db` | `~/.agentmux/logs/agentmux-launcher.log` |
| 18:26:33.45 | 0.59.39 srv: `bootstrap loaded 4 workspace(s) + 4 tab(s) + 5 block(s) + 3 window(s)`. No `carry-forward:` line, no `session_restore:` line. | 0.59.39 srv log |
| 18:26:36.5 | Startup cleanup removes the two windowless workspaces and their three panes from the June data | 0.59.39 srv log |
| 18:26:36 | The main window opens on the June layout: "Starter workspace", "tab1", one empty agent pane | 0.59.39 host log, `objects.db` |
| 18:28:55 | The user picks Clare in that empty pane | 0.59.39 srv log |

The launcher log has the same legacy copy for the two upgrades before: `v0.59.36` (2026-10-21 07:07) and `v0.59.38` (2026-10-21 23:31).

## Evidence

1. **0.59.39 started from the June data.** The pre-versioning `channels/stable/data/db/objects.db` (last written 2026-06-11) holds exactly 4 workspaces, 4 tabs, 5 blocks and 3 windows. Its ids are the ones 0.59.39 loaded and then cleaned up: workspaces `1e40f215…` and `99b5a8c0…` and blocks `2a93141b…`, `32758be7…` and `9d43574e…`. 0.59.38's own final data holds 1 workspace, 1 tab, 1 block and 0 windows, plus the quit record.
2. **The files were cloned, not `VACUUM INTO`-copied.** 0.59.39's `objects.db`, `filestore.db` and `sagas.db` carry the birth time of the channel's first database (2026-05-30 09:08:33), and `sagas.db` keeps its June mtime. `std::fs::copy` (the launcher's `copy_dir_recursive`) clones on APFS and keeps both. srv's copy, `db_copy::copy_database`, writes a new file through `VACUUM INTO`.
3. **The quit record exists, but only in 0.59.38's data.** 0.59.38's `objects.db` has `session:last_topology` in the client's meta: `saved_for: "quit"`, one window, "Tab 1" with three agent panes (each with its `agent:sessionid`) and a Swarm pane. 0.59.39's client meta has none.
4. **Counterfactual: srv's carry-forward would have kept the layout.** In a sandbox, with `HOME` and every `AGENTMUX_*` path pointed at a temp folder, the shipped 0.59.39 `agentmux-srv migrate --prepare` ran over `VACUUM INTO` copies of 0.59.38's databases, with no pre-versioning folder present. It printed `AGENTMUXSRV-MIGRATING copying-previous-version:0.59.38` and applied all 40 migrations. The resulting `objects.db` holds the quit record with its one window, Tab 1 and four panes, and the client's window list is empty, so the first window takes the restore path (`session_windows::restore_all`).

## Root cause

`crates/launcher/src/data_dir.rs`, `ensure_dirs` → `migrate_legacy_data_dir`. It runs in the launcher before srv is spawned (`supervisor/unix.rs`, `supervisor/windows.rs`):

- **The trigger is every upgrade, not once.** It copies `channels/<ch>/data/db/` into `versions/<v>/data/db/` whenever the latter has no `.db` file. Its doc calls it a "one-time migration" for the move to per-version data (Phase 2, `SPEC_VERSION_ISOLATION_2026_06_01.md`). But "no database in this version's folder" is true on the first start of every version, and the old folder is copied, never moved, so it is always there.
- **It shadows srv's carry-forward.** `bootstrap/carry_forward.rs` (#4996) treats an existing `objects.db` as "this version has started before" (`source_for` returns `None`). The launcher's copy lands first, so srv's copy-forward from the newest older version never runs, in `migrate --prepare` or in-process.
- **Each upgrade therefore starts from the June data.** Migrations bring that data up to date, startup removes its leftover windows, and the main window opens on its starter tab.

Secondary, made moot by the fix: the launcher's copy is a plain file copy, so it would also miss writes still in a `-wal` file. `REPORT_STARTUP_ARCHITECTURE_AND_MIGRATIONS_AUDIT_2026_10_20.md` §9 flagged it for that reason, without noticing that it shadows carry-forward.

## Why it wasn't caught

- **Unit tests:** carry-forward's tests build a channel with versioned folders only. None has a pre-versioning `data/db/` beside them, and nothing tested the launcher's `ensure_dirs` together with srv's copy.
- **End-to-end checks:** the upgrade checks so far ran on channels without a pre-versioning folder (new installs, local builds with a channel of their own, test harness data). Only long-lived installs have one. That is most dogfooding machines, but not the ones tests run on.
- **No runtime check:** nothing asserted that an upgrade's data came from the version it upgraded from. The missing `carry-forward: started from the previous version's data` line was the only sign, and `bootstrap loaded 4 workspace(s)…` reads like success.

## Fix

Branch `clare/upgrade-keeps-layout-legacy-copy`:

1. **The launcher no longer seeds a version's databases.** `ensure_dirs` only creates folders, and `migrate_legacy_data_dir` and `copy_dir_recursive` are removed.
2. **srv owns the one-time legacy migration.** `carry_forward::source_for` picks the newest older version first. It falls back to the channel's pre-versioning `data/db/` only while no version in the channel has data, which was the original intent. The copy goes through `VACUUM INTO`, so it is WAL-safe. `migrations/gate.rs` uses `source_for`, so the boot gate asks for `migrate --prepare` in that case too.
3. **Provenance and a runtime check.** Every copy-forward writes `data/db/carried-from` with the version it came from, or `pre-versioning data`. On the first start after an upgrade (`upgrade_notice::init`, the start that recorded `upgraded_from`), srv checks `carry_forward::continuity_problem`. If the version it upgraded from has data in this channel and this version's databases weren't copied from it, srv logs an ERROR: `upgrade continuity: this version's data wasn't copied from 0.59.38, …`.

## Verification round

The ask was that an upgrade comes back with the same window, tab and pane layout, and that this is verified both offline and at runtime.

**Offline tests (CI):**

| Test | What it pins |
|---|---|
| `launcher data_dir::tests::ensure_dirs_leaves_a_new_versions_databases_to_srv` | With a pre-versioning folder and 0.59.38's data present, the launcher puts no database in 0.59.39's folder. Fails on the old code (it put `objects.db` there), passes with the fix. |
| `srv carry_forward::tests::pre_versioning_data_never_shadows_the_previous_version` | With both present, srv copies 0.59.38's data and records `carried-from: 0.59.38`. |
| `srv carry_forward::tests::pre_versioning_data_seeds_only_a_channel_without_versioned_data` | The legacy folder still seeds a channel that has no versioned data yet, and never once any version has data, even for an older build started later. |
| `srv carry_forward::tests::the_layout_a_quit_saved_crosses_the_upgrade` | A quit record (one window, a tab, three panes with their sessions) saved through `save_last_session_snapshot` in 0.59.38's store is what 0.59.39's store loads after the copy, and `continuity_problem` finds nothing. |
| `srv carry_forward::tests::an_upgrade_whose_data_isnt_the_previous_versions_is_reported` | Databases already in place before carry-forward (what the launcher did), or a copy from another version, are reported. A version with no data to compare is not. |

Restoring windows from a quit record is already covered by `session_restore` / `session_windows` tests (#5033, #5101, #5200). The new test closes the gap between the two halves: the record has to reach the new version's data.

**At runtime:** the `upgrade continuity:` ERROR on the first start after any upgrade whose data isn't the previous version's. Once the fix ships, the next upgrade on this machine (0.59.39 → 0.59.40) should log `carry-forward: started from the previous version's data from=…/versions/0.59.39` and write `carried-from` = `0.59.39`, with no `upgrade continuity:` line.

## Recovery on the affected machine

- **The layout itself:** the record of the layout 0.59.38 quit with is still in `versions/0.59.38/data/db/objects.db`; nothing is deleted. The agents' conversations are kept apart from the layout.
- **Getting the agents back:** reopening them from the Agents list resumes their sessions.
- **Future upgrades:** after the fix ships, they carry the current version's data forward, so the June folder no longer matters. It can stay, since it is read only while no version has data.

## Follow-ups

- **The upgrade test harness:** it should run with a pre-versioning `data/db/` in the channel, as long-lived installs have, and assert the layout after the restart.
- **The `bootstrap loaded …` log line:** it could name where the data came from (`carried-from`), so a log reader can see it without the new check.
