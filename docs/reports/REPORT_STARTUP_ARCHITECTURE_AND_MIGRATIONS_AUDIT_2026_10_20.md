# Report: startup, migrations and the splash: an audit, and a sturdier startup design

**Date:** 2026-10-20
**Author:** AgentA@Area54
**Status:** active — the operator chose upgrade-only migrations on 2026-10-20; the S1 design is now `SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md` §10. The bug fixes (§5) and S0a/S0b (§8) are in PRs; D1 and D2 took the recommended options; D3–D5 are open.
**Requested by:** the operator: *"take a look at the migrations work. we also want to do an audit of the startup. there are points in the splash screen where the numbers freeze, I wonder if the startup framework needs a more robust architecture, consider it all, and oppurtunities for additional DRY"*, following a question about taking migrations off app startup now that the in-app upgrade route exists.
**Related:** `docs/specs/SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md` (the plan this report updates), `docs/reports/REPORT_STARTUP_MIGRATION_AUDIT_AND_DEFERRAL_2026_09_15.md` (the June history), `docs/reports/REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md`, `docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md`, `docs/analysis/RCA_UPGRADE_0_59_30_TO_0_59_34_ISSUES_2026_10_19.md`.

Checked against main at 3f34223e8. Line numbers are from that commit.

## 1. Summary

- **The splash numbers freeze when no stage is open.** The splash only ticks a row that has begun and not ended. Several stretches of real work have no row: between "Host startup" and "CEF initialize", between "CEF initialize" and first paint, and a few inside "backend" on a new version's first start. Two more are by design: the final summary is held still for 2 s, and the HTML loading overlay that follows has no numbers at all. srv's ordinary boot is not the cause. On this machine it reaches ready in 271–696 ms (§3).
- **The startup framework does need one structural rule:** from the moment the splash shows until it goes away, exactly one row is always running. Stages are declared once, in one shared table, and every process reports them in one typed protocol (§6.1). Today three processes report stages in ad-hoc stderr lines and IPC calls, each with its own parser.
- **Migrations can't simply leave startup.** The in-app upgrade route now exists, but it doesn't migrate: the new version still migrates at its own first start (`launcher/src/upgrade.rs:145` `run_migration_upgrade` has no caller). Every other way a new version arrives also needs a migration step at first start: installers, a manual unzip, `task dev`, `task package`, and switching channel. The target is therefore one data-preparation step (carry-forward, one backup, migrations) that the launcher runs before srv. The upgrade route and a first start both use it, and an ordinary boot never touches it (§6.2).
- **Found on the way:**
  - The pre-migration backup can miss committed data (§5.3).
  - An upgrade copies `objects.db` up to three times (§5.2).
  - Every boot opens and re-runs schema setup on the main stores three times (§5.1).
  - The restart splash shows a stage list nothing feeds (§4).

## 2. History

| When | What | Outcome |
|---|---|---|
| 2026-06 (bf77cadc9) | Deferred migrations to run on demand after startup | Reverted. |
| 2026-06-26 (341faa981) | Removed the on-demand path | A migration ran while a live srv held the data. m0011's "skip if the table isn't empty" guard then caused permanent data loss. Rule since: **never migrate while any srv holds the data dir**, and idempotency is "is this id applied", never "do rows exist". |
| 2026-06-27 (1052c985b) | Owner revert to in-process migration | Rule since: **an ordinary boot with nothing pending gains no new process spawn, file open or network call.** |
| 2026-08-03 | Migration hardening | A failed migration is fatal (`AGENTMUXSRV-MIGRATION-FAILED`, launcher dialog). |
| 2026-09-14 / 09-15 | Splash rethink report; startup audit report and the FAST_STARTUP spec (#3264) | Splash phase 1 shipped (#3222, shared `splash_core`), with migration sub-rows (#3223) and the `boot_timing` line (#3267). Spec phase 1a (#3352) added the launcher's `run_migrate` / `run_migration_upgrade`, still unused. Phases 1b, 2, 3 and 5 are open. Option A (a full-window blocking upgrade screen) was decided on 2026-09-16. |
| 2026-10-15..19 | Upgrade route | Check with an upgrade floor (#5110, `UPGRADE_FLOOR` = 0.55.0), download with SHA-256 and unpack, restart once agents are idle, launcher confirm → quiesce → relaunch. #5197 (today) keeps a staged build out of old version folders. **No migration step.** |
| 2026-10-04 (#4996) | Carry-forward | A new version's first start copies the previous version's channel databases (`VACUUM INTO`), then migrates them. |

The 09-15 spec's invariants still hold, and this report keeps them:

| | Invariant |
|---|---|
| S1 | An ordinary boot does no migration, snapshot, update check or spawn. |
| S1b | The pending check fails closed. |
| S1c | Nothing is written to stale stores while migrations are pending. |
| S2 | No migration while any srv holds the data, and supervision is suspended. |
| S2b | The staged update is installed before migrating. |
| S3 | Idempotency comes from `migration_is_applied`. |
| S4 | No network on the boot path. |
| S5 | ESTART isn't gated on deferrable work. |

## 3. Startup today

Three processes report to one native splash. The launcher owns the splash and its IPC server; srv and the CEF host feed it.

1. **Launcher setup**, then the splash thread starts (`launcher/src/supervisor/windows.rs:73-305`).
2. **"backend":** srv is spawned suspended, then resumed (`launcher/src/srv_spawner.rs:408-728`). The launcher waits 30 s for `AGENTMUXSRV-ESTART`, or 30 minutes once srv prints `AGENTMUXSRV-MIGRATING` (`:651-681`). Inside srv (`bootstrap/stores.rs:72-662`), in order:
   1. Carry-forward (`:154`).
   2. Pre-migration snapshot (`:155`).
   3. Pending count (`:180`).
   4. Migrations (`:205`).
   5. Opening the stores.
   6. Every-boot repairs (§5.1).
   7. Seeds.
   8. Background subsystems, widget scan, listeners, reducer state, cron, saga recovery.
   9. ESTART (`main.rs:256`).
3. **"Host startup"** (`windows.rs:518`): begins at host spawn and ends when the process exists.
4. *(no row)*: the host image loads, the host connects to the launcher (one attempt, no retry), then to srv, and the browser profile is carried forward. "Load CEF framework" (`cef/src/lib.rs:771-772`) is reported after the fact, with begin and end sent together.
5. **"CEF initialize"** (`lib.rs:1133-1248`).
6. *(no row)*: browser creation and navigation.
7. **"paint"** (`cef/src/client/navigation.rs`): ends when the frontend reports first paint after two animation frames, or after a 4 s safety timeout.
8. **Hold**: the final summary stays still for 2 s, or 1 s if the start took under 500 ms (`launcher/src/splash_core.rs:389-396`). Then it fades.
9. **HTML `#startup-loading` overlay** until the workspace has loaded, with no progress shown.

Measured srv boot on this machine, last seven boots (`boot_timing:` line): total 271–696 ms, of which `open_stores_and_migrate` is 249–655 ms. `bind_listeners_and_network` takes 8–20 ms, `agent_auto_seed` 2–12 ms, and every other span 7 ms or less. The gaps in steps 4 and 6 aren't measured anywhere today; S0 (§8) measures them.

## 4. Where the numbers freeze

| # | When | What is actually running | Why nothing ticks | Fix |
|---|---|---|---|---|
| F1 | After "Host startup" ends, before "CEF initialize" | Host image load (and, on a first run, an antivirus scan of the new binaries, which can take tens of seconds), the launcher IPC connection, the srv connection, profile carry | No row is open. The "First run can take longer" sub-row added after 5 s belongs to the host stage and never gets an end event | Real stages: "Connect" and "Browser profile". "Load CEF framework" reported live, with its begin sent before the work |
| F2 | After "CEF initialize", before paint | Browser creation, loading the frontend bundle | No row is open | A "Load interface" stage from browser creation to the first frontend report |
| F3 | Inside "backend" on a new version's first start | Carry-forward copies, snapshot, the wait for the migration lock, `backup_stores`, then every post-migration step | Only individual migrations get sub-rows (#3223). The rest runs under one long-lived parent row whose clock keeps going, but the row's label doesn't change for tens of seconds | Sub-rows from the same calls that already time these steps (§6.1) |
| F4 | The final summary | Nothing | A deliberate 2 s hold on frozen totals. It reads as a stall | Label it "Ready" visibly, or shorten it (decision D3) |
| F5 | The HTML overlay | Workspace, tab and block state loading over WebSocket; the first render | It's a second loading screen with no progress at all | Keep the native splash until the frontend reports "workspace ready" (decision D4) |
| F6 | A restart (srv respawn, or the upgrade relaunch) | The whole sequence again | `respawn_splash_for_restart` creates a sink and drops it (`windows.rs:52`, `let (_sink, rx) = ...`), so the restart splash never receives an event | Give the restart splash the live sink |

Windows and Unix also report different stage lists: Unix has a "prep" stage and Windows doesn't. The stage diagram in `launcher/src/splash.rs:17-25` no longer matches either.

## 5. Migrations today

### 5.1 What runs on every boot, pending or not

- **Schema setup three times.** Every store open runs `check_schema_compat` → `run_*_schema` → `stamp_version`, not gated by `user_version`, which only guards against downgrades. For `objects.db` that means renaming legacy tables, ten conditional `ALTER`s and about 25 `ALTER ADD COLUMN`s that ignore "duplicate column" errors (`migrations.rs:565`, `:1351-1450`). The pending count (`stores.rs:180`), `run_pending_migrations` and the real open each open `objects.db` and `store.db`, so this runs three times per boot.
- **The pending count isn't read-only.** `try_count_pending_migrations` (`runner.rs:573`) is fallible now, which meets half of S1b. But it opens the stores with `Store::open`, which runs the schema setup above, so it writes. The lenient `count_pending_migrations` (`:614`), still used for the ESTART line (`bootstrap/shutdown.rs:13`), reports a fallback number on error.
- **Repairs that are really one-time catch-ups:**
  - the registry `session_id` backfill, which repeats m0010 (`stores.rs:248`);
  - pruning registry records of deleted agents (`:313`);
  - `adoption::run_at_boot`, which checks every channel's identity store (`:450`, 8 s budget);
  - `heal_global_snapshot_source_block_ids`, which reads one JSON per agent definition (`:508`).

  Each walks data that only changes when an old version's data arrives.

### 5.2 An upgrade today

The launcher confirms, quits the host, waits for srv to stop, and starts the new version. The new srv then:

1. **Carry-forward:** `VACUUM INTO` copies of `filestore.db`, `sagas.db` and `objects.db` from the newest older version. It prints `AGENTMUXSRV-MIGRATING copying-previous-version`, which extends the deadline.
2. **Snapshot:** `objects.db` again, into `~/.agentmux/snapshots/`, when the schema version is behind.
3. **`backup_stores`:** `objects.db` a third time and the shared `store.db`, into `shared/backups/pre-migration-*`, when anything is pending.
4. **Migrations**, each marked applied after its own `up()`. There is no batch transaction, so a failure part-way leaves the earlier ones applied. The fix offered is "Restart to retry". Nothing restores the data automatically.

The previous version's folder is never touched, so after a carry-forward it is already a complete backup of the channel data. Copies 2 and 3 of `objects.db` add nothing in that case. Only the shared `store.db` needs a backup of its own.

### 5.3 The backup can miss committed data

`backup_stores` (`migrations/runner.rs:197-222`) uses `std::fs::copy` on `store.db` and `objects.db` and doesn't copy their `-wal` files. Both databases are in WAL mode, and the shared `store.db` is usually held open by every other running AgentMux, whatever its version or channel. Committed writes that haven't been checkpointed yet live only in the `-wal` file, so a backup restored by hand can be missing them or be inconsistent. Carry-forward and the snapshot already use `VACUUM INTO`, which doesn't have this problem. This is from reading the code; I haven't reproduced a bad backup.

### 5.4 The registry, against the upgrade floor

There are 38 migrations, m0000–m0037: 18 global-scope (shared `store.db` and shared files) and 20 channel-scope. With `UPGRADE_FLOOR` at 0.55.0, every install that can upgrade has already applied the ones that shipped before 0.55.0:

- **Can be deleted** (after raising nothing): m0000 and its legacy-marker checks, m0001, m0004–m0006, m0008–m0010, m0012–m0014, m0017–m0019. Also the launcher's copy of the old unversioned data dir (`launcher/src/data_dir.rs:139-171`).
- **Need a change first:**
  - m0011: `stores.rs:435` checks its id.
  - m0015/m0016: these are seeds for new channels; move them into seed code.
  - m0002/m0003/m0007: their `verify()` checks are used by `migrate --verify`.
- **Need the floor raised past 0.55.41:** m0020–m0033, together with the conditional `ALTER`s and legacy table renames in the schema setup.

The checks behind this list: the origin version of each migration was read from `package.json` at the commit that added it, and m0020 was confirmed in `VERSION_HISTORY.md`. Idempotency was confirmed for six of them; for the rest I relied on the framework's documentation.

## 6. Proposed design

### 6.1 One stage protocol, one stage table, one invariant

- **A typed protocol in `agentmux-common`.** A `StartupLine` enum (ESTART, migrating, migration begin/end/failed, stage begin/end, sub-stage begin/end) with one `format` and one `parse`. srv, the launcher and the host use it, and nothing else matches raw `starts_with("AGENTMUXSRV-…")` strings. Today ESTART's format is defined in srv (`bootstrap/shutdown.rs:15`), not beside the other srv stderr lines (`common/src/srv_stderr.rs`). It is parsed twice (`launcher/src/srv_spawner.rs:797`, `cef/src/sidecar.rs:610`). Migration progress travels in two protocols: stdout NDJSON, parsed with hand-written `json_str` / `json_u64`, and stderr BEGIN/END lines.
- **One declared stage table.** Id, label, which process reports it, and order, for every platform. The splash draws the table at once, with future rows shown as waiting, and the supervisors stop emitting their own drifted lists.
- **The invariant, enforced in `splash_core`:** between show and dismiss, one row is always running. When a stage ends and the next one hasn't begun, the next declared row starts as "waiting for <process>" with its own clock. The numbers can't all stop, and a stall shows which process it is waiting on. That turns the next freeze report into a precise one.
- **One instrumentation call feeds both outputs.** srv's `boot_timing::time("name", …)` already wraps each boot step. Have it also emit a sub-stage line, so the log summary and the splash sub-rows come from the same call. Carry-forward, the snapshot, the lock wait and the backup get rows at no extra cost.
- **The frontend's last step joins the table** (decision D4): a "Load workspace" stage reported over the existing host IPC, after which the native splash fades. That removes the progress-less second screen. The 09-14 report §4.5 advised against folding the frontend bootstrap into the native splash. This is narrower: a single end event, not the frontend's internals.

### 6.2 One data-preparation step, owned by the launcher

- **`agentmux-srv migrate --prepare`.** It does, in order:
  1. Carry-forward when this version has no data yet.
  2. One backup, taken with `VACUUM INTO`: of the shared `store.db` always, and of `objects.db` only when there was no carry-forward.
  3. The pending migrations.

  It prints progress in the §6.1 protocol, so the launcher's native splash shows it with no new UI.
- **Two callers, the same step:**
  - **The upgrade route:** in `upgrade_restart`, between `quiesce_srv` and `relaunch`, the launcher runs the *new* version's `agentmux-srv migrate --prepare` from the staged folder. That is the dead `run_migration_upgrade`. Every window is closed at that point, so the native splash is the progress screen, which reopens decision D2.
  - **A first start by any other path:** srv's existing pending check runs first. When something is pending, srv exits with a dedicated "needs preparation" status, *before* it writes anything. The launcher then runs `migrate --prepare` with splash progress and respawns srv.

  An ordinary boot with nothing pending is unchanged: no new spawn, file or network call, as required by the 06-27 rule. The extra spawn happens only when there is work to do.
- **The pending check becomes read-only and fails closed** (S1b, S1c): open with SQLite read-only flags, run no schema setup, and treat any error as "needs preparation".
- **srv no longer migrates in its boot path.** The fatal in-process branch at `stores.rs:205-223` becomes the "needs preparation" exit, and the failure dialog moves to the step that actually failed.
- **Partial failures:** each migration is already recorded only after its `up()`, and retrying runs only what's left. Add an offer to restore the step's own backup when a retry fails again.

### 6.3 The ordinary boot after that

- Open each store once, and pass the handles down.
- Run schema setup only when `user_version` is behind. Keep the downgrade guard.
- Turn the four every-boot repairs (§5.1) into one-time migrations, or move them after ESTART.
- Move the remaining non-essential steps after ESTART (the FAST_STARTUP spec's inventory items 4c–4f, §3 and §6).
- Hold it to the spec's budget: under 300 ms to ESTART on a warm boot with nothing pending, checked by a test.

## 7. Repetition worth removing

| # | What | Where | One version |
|---|---|---|---|
| 1 | Waiting for ESTART | `srv_spawner.rs:651-683`, `cef/src/sidecar.rs:449-475` | One function in common |
| 2 | Parsing ESTART | `srv_spawner.rs:797`, `sidecar.rs:610` | §6.1 protocol |
| 3 | Matching MIGRATING / EVENT lines | Raw `starts_with` in the launcher and the host | §6.1 protocol |
| 4 | Two migration progress protocols | stdout NDJSON, stderr BEGIN/END | One |
| 5 | Stage emission per platform, already drifted | `supervisor/windows.rs`, `supervisor/unix.rs` | One boot-sequence driver with platform hooks for spawning only |
| 6 | The fake splash preview events | `launcher/src/main.rs:248-306`, twice | One fixture |
| 7 | Finding the srv binary | `resolve_srv_binary`, `resolve_backend_binary` | One |
| 8 | Spawn suspended, assign to the job, resume | Repeated per spawn site | One helper |
| 9 | Copying a database | `backup_stores` (`fs::copy`), `snapshot::create_snapshot`, `carry_forward::copy_database`, the launcher's `copy_dir_recursive` | One `VACUUM INTO` helper (fixes §5.3) |
| 10 | "Already done" markers in four forms | `db_migrations` rows, seven legacy flag files, `adoption-ledger.json`, "a `.db` exists" gates | `db_migrations` only, as the floor retires the rest |
| 11 | The "ignore duplicate column" `ALTER` loop | `migrations.rs:1369, 1445, 1772, 2249, 2307` | One helper |
| 12 | Table-exists check | Inline in `run_object_schema` and seven migrations; only m0029 has `table_exists` | One helper |
| 13 | Counting pending migrations | `try_count_pending_migrations`, `pending_of` | One |
| 14 | Legacy-minimize conversion | srv `backend/layout/intent.rs:520` and the frontend's `migrateLegacyMinimize` | Keep one side |

Leave alone: the frozen copies of live code inside m0017, m0020, m0021, m0025 and m0028. They are deliberate (`migrations/mod.rs:35-48`) and go away when those migrations are retired.

## 8. Phases

| Phase | What | Changes behaviour? |
|---|---|---|
| **S0** — see it | §6.1 protocol and stage table; the one-running-row invariant; `boot_timing` feeding sub-rows; the missing stages (F1, F2, F3); the restart splash's sink (F6); the backup via `VACUUM INTO` (§5.3); DRY items 1–9 | Only what the splash shows, and safer backups |
| **S1** — prepare step | `migrate --prepare`; wire it into the upgrade route; srv's "needs preparation" exit; the read-only, fail-closed pending check | Yes: migrations leave srv's boot |
| **S2** — lean boot | Single store open; version-gated schema setup; repairs to one-time migrations or after ESTART; the 300 ms budget test | Faster ordinary boot |
| **S3** — retire | Delete the migrations §5.4 lists as deletable; then, with the floor raised (D5), m0020–m0033 and the matching schema code; DRY items 10–13 | Less code on every boot |
| **S4** — one loading screen | The frontend's "Load workspace" stage; the native splash stays until then; the HTML overlay goes | One screen instead of two (D4) |

S0 is first because it measures F1 and F2. Their real size decides how much of S2 is worth doing. The 09-15 spec should then be updated to this design (it is still "proposed").

## 9. Decisions needed

- **D1. Global migrations and other running versions.** 18 migrations change the shared `store.db`, which other versions and channels may have open while one upgrades. Options:
  - Ask the user to close them first.
  - Require global migrations to be additive, so older versions keep working (expand now, remove later), and check that in review.
  - Keep today's behaviour.

  Recommended: additive, plus the check. **Taken 2026-10-20** (spec §10.3).
- **D2. The upgrade progress screen.** Option A (a full-window blocking screen, decided 2026-09-16) predates the upgrade route. During the route every window is closed, so the native splash is already on screen. Recommended: use the native splash. **Taken 2026-10-20** (spec §10.3).
- **D3. The 2 s hold.** Keep it as a labelled "Ready" summary, shorten it, or remove it.
- **D4. One loading screen.** Fold the HTML overlay into the native splash with a single "Load workspace" end event (§6.1), against the 09-14 report's advice.
- **D5. Raise `UPGRADE_FLOOR` past 0.55.41** to retire m0020–m0033. Installs older than that would have to upgrade in two steps.
