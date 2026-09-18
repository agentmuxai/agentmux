# Runbook — data migration failed, stuck, or suspected incomplete

**Status:** living — operator runbook; keep current as the migration framework changes.
**Date:** 2026-09-07
**Author:** Korp
**Related:** `docs/specs/SPEC_MIGRATION_FRAMEWORK_2026_06_24.md` (what ships), `docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md` (why each safeguard exists; this runbook is its Phase 6 deliverable and supersedes its Part 4), `docs/exe-return-codes.md`.

This is the short version for support / on-call. Every step is something an operator can run without reading source. Nothing here is destructive until §5, and §5 says so.

---

## 1. Which situation are you in?

| You see | Go to |
|---|---|
| Dialog **"AgentMux — database migration failed"** at launch, app does not start | §2 |
| App starts, status bar says **"Migration failed — restart to retry"** | §3 |
| App starts but agents / definitions are **missing or empty** after an upgrade | §4 |
| Something about "another runner may still hold it" in the log | §6 |

## 2. Launch dialog: migration failed, app will not start

Since 2026-09-06 (#3043) a failed migration is **fatal by design** — srv exits before it would have booted against a half-migrated store. The dialog text *is* the reason srv gave. Three facts you can rely on:

1. **Migrations only run after a copy of the store is written** to `~/.agentmux/shared/backups/pre-migration-<version>-<timestamp>/` (`store.db` and `objects.db`). If the reason in the dialog is about writing that copy ("backup failed"), **nothing was changed** — the run stopped before any migration. Fix the cause (disk full, permissions, path) and relaunch.
2. Otherwise a backup exists from immediately before the attempt. A best-effort snapshot may also exist under `~/.agentmux/snapshots/`.
3. **Relaunching retries.** srv re-runs every migration that is not recorded as applied. Many failures (a locked file from a still-exiting previous instance, a transient disk error) clear on the second attempt.

Get the full reason and context:

```
muxlog srv                     # the srv log; look for "startup: migration failed"
muxlog launcher                # the launcher side: "[srv <pid> MIGRATION FAILED] …"
```

If it fails the same way twice, continue to §4.

## 3. Status bar: "Migration failed — restart to retry"

This message means the **count** of pending migrations at startup was non-zero — not that a migration threw (that is fatal, §2). Since the Phase 1a change the two cases the count still covers are benign: a fresh install whose channel store was created *after* the pending count was taken, and a stale cached count in the host (known cosmetic issue, Phase 0f of the hardening spec). **Restart once.** If it persists, run `--verify` (§4).

## 4. Suspected incomplete migration (data missing after upgrade)

The known failure shape (the incident behind the hardening spec) is a migration **recorded as applied that wrote nothing** — a marker file or a `db_migrations` row said "done" while the target table stayed empty. The doctor pass checks exactly this, read-only.

**If the app is running, start here** — same report, no binary hunting, from any agent or shell pane inside the instance:

```
muxspect migrations            # or: node ~/.agentmux/shell/muxspect.mjs migrations
```

It exits 3 on a `MISMATCH` or error, exactly like `--verify` below. It only doctors the instance you are inside; for another channel, or when the app will not start, use the CLI:

```
# Find the srv binary: <install>/runtime/agentmux-srv-<version>-<os>.<arch>[.exe]
# (Windows installer: %LOCALAPPDATA%\AgentMux\runtime\; portable ZIP: <unzipped>\runtime\;
#  Linux AppImage: ~/.local/share/agentmux/extracted/<version>/usr/bin/)

agentmux-srv --wavedata <data-dir> migrate --list      # every migration, applied/pending
agentmux-srv --wavedata <data-dir> migrate --verify    # post-condition check of each APPLIED one
```

`<data-dir>` is the channel data dir — `~/.agentmux/channels/<channel>/versions/<v>/data/` — the same value the launcher exports as `AGENTMUX_DATA_DIR`. If you run from a shell the launcher spawned, `--wavedata` can be omitted.

`--verify` exit codes: **0** every applied migration's check holds (or is not verifiable), **3** at least one mismatch or an unreadable tracking table (each line says which), **2** you mistyped a flag, **1** the command itself failed. It never writes.

A `MISMATCH` line names the migration and what is missing, e.g. `0007_agents_consolidate … db_agent_definitions=3 db_agent_instances=2 but db_agents=0`. That is the one case §5 applies to.

## 5. Re-running one migration (the only destructive step)

Only for a confirmed `MISMATCH` from §4, and only after confirming a backup exists (§2 point 1). The framework's own guard (Phase 0b) already re-runs `0007_agents_consolidate` automatically when its marker is stale *and* the target is empty — so first simply relaunch. If the mismatch persists:

1. Quit AgentMux fully (no `agentmux-srv` process left).
2. Copy the backup dir aside — the pre-migration copy is your rollback.
3. Delete the migration's `db_migrations` row so the runner treats it as pending. For a channel-scoped migration that row lives in `<data-dir>/db/objects.db`; for a global one in `~/.agentmux/shared/store.db`:
   ```
   sqlite3 <data-dir>/db/objects.db "DELETE FROM db_migrations WHERE id = '0007_agents_consolidate';"
   ```
   Neither `0002_block_zones_v1` nor `0007_agents_consolidate` needs its flag file removed: since Phase 0b (`0007`) and Phase 2 (`0002`) both decide from the data, not the flag, and re-run safely over already-migrated rows.
4. Relaunch. The runner takes a fresh backup, re-applies that migration, and marks it. Run `--verify` again.

Do **not** delete `db_migrations` wholesale, and do not edit data tables by hand — re-running a migration is what the backup-then-apply path is for.

## 6. "another runner may still hold it"

Since Phase 3 (#3062) migrations run under two cross-process locks, taken in a fixed order: `~/.agentmux/shared/migrations.lock.db` (beside the shared store) and then `<data-dir>/db/migrations.lock.db` (beside the channel store), each held with a SQLite `BEGIN IMMEDIATE` for the length of the batch. Two locks because isolated-auth instances can share a data dir while resolving different shared stores. A second `agentmux-srv` (or a `migrate` CLI run) **waits up to 30 minutes** for the first, then fails with that message. It is an OS-level lock, so it cannot go stale: if you see the message, another process really is (or was, within the wait) migrating. Find it:

```
# Windows
tasklist | findstr agentmux-srv
# macOS / Linux
pgrep -fl agentmux-srv
```

Wait for it or stop it, then relaunch. Deleting either `migrations.lock.db` is safe when no srv is running (they hold no data) but is never necessary.

## 7. What to include in a bug report

- the dialog text or the `MISMATCH` line, verbatim
- `muxlog srv` and `muxlog launcher` output around the failure
- `migrate --list` and `migrate --verify` output
- the name of the newest `~/.agentmux/shared/backups/pre-migration-*` directory and the versions involved (`from` in the backup name, `to` in the About dialog)
