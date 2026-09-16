# REPORT — Startup audit: get migrations off the critical path, into an explicit upgrade action

**Date:** 2026-09-15
**Status:** active — audit stands; the design follow-through is
`docs/specs/SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md`,
tracked by [#3258](https://github.com/agentmuxai/agentmux/issues/3258)
**Ask:** "there should be no migrations at startup... any migrations will run
through the upgrade system which will appear as a button in the status bar,
migrations and an app restart would happen then." Audit startup (including
migrations) and propose how to remove them from the ordinary boot path.
**Scope:** `agentmux-srv`'s boot sequence (`main.rs`, `bootstrap.rs`,
`migrations/runner.rs`), the launcher's readiness handshake
(`agentmux-launcher/src/srv_spawner.rs`), and the existing (partially built)
Maintenance/upgrade UI (`frontend/app/statusbar/{UpdateStatus,MaintenanceSection,
BackendStatus}.tsx`, `agentmux-cef/src/commands/backend.rs`,
`docs/specs/SPEC_UPGRADE_PANEL_2026_06_27.md`,
`docs/specs/app-update-check.md`).
**Related:** `docs/reports/REPORT_SPLASH_MIGRATION_ROW_OVERFLOW_AND_SUMMARY_2026_09_15.md`
(today, earlier — the per-migration splash-row overflow symptom this report's
fix makes moot for ordinary boots), `docs/reports/REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md`.

---

## 1. This has been tried before — twice, by the repo owner, one day apart

This is not a new idea and the two prior attempts are directly relevant
constraints, not just history:

- **`bf77cadc9`** (2026-06-26, "feat(startup): defer migrations and saga
  vacuum to upgrade panel") — built almost exactly what's being asked for:
  removed the synchronous migration run from both launcher startup paths,
  added a read-only `count_pending_migrations` check, and threaded
  `pending_migrations` into `AGENTMUXSRV-ESTART` for a status-bar indicator.
- **`341faa981`** (same day) — a same-day follow-up that had to remove the
  on-demand "run migrations now" path it had just added, **because it caused
  real, permanent data loss**: spawning `agentmux-srv migrate` while the real
  srv was still running let `0011_shared_store_backfill`'s
  skip-if-already-non-empty guard (§5 below) see tables the *already-running*
  instance had touched and skip backfilling other channels' data — for good,
  since a migration only ever runs once.
- **`1052c985b`** (2026-06-27, **authored by `asaf <asafebgi@gmail.com>` —
  the repo owner, not an agent** — "feat(startup): run migrations in-process
  at srv startup for near-instant launch") — reverted the deferral entirely.
  Migrations moved back in-process, before `AGENTMUXSRV-ESTART`, specifically
  to cut the subprocess-spawn latency the deferred design added **to every
  ordinary boot**, not just the rare migration boot. This is the architecture
  that ships today.
- `docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md` §1.5
  independently re-derives the same tradeoff a month later and explicitly
  recommends *against* re-reverting to the subprocess model, closing the
  safety gap a different way instead (fatal-on-failure before `ESTART`,
  shipped; `--verify` for post-hoc consistency checks, shipped 2026-09-07).

**Read literally, those two commits look like they contradict each other.**
They don't — they're establishing two separate constraints that any new
design has to satisfy simultaneously, and neither prior attempt satisfied
both at once:

1. **The ordinary boot (zero pending migrations) must stay in-process and
   effectively free.** This is what `1052c985b` protected and is already
   true today (`run_pending_migrations`'s own doc comment: "Fast-path:
   returns `Ok(0)` immediately when all migrations are already applied" —
   `agentmux-srv/src/migrations/runner.rs:255`). **Migrations are not
   actually slowing down the boot you take 99% of the time** — they cost
   effectively nothing when there's nothing to apply, which is every boot
   except the first one after a version bump ships new migrations. This
   matters for scoping the fix: the real problem is narrower than "startup
   is slow because of migrations" — it's "the one-time first-boot-after-
   upgrade migration run currently borrows the ordinary boot's UI (the
   splash) and blocking behavior (nothing works until it's done), instead of
   being its own explicit, visible step."
2. **A migration must never run while a live srv process holds the same
   data dir open.** This is what `341faa981` closed and is the hazard any
   "run migrations from a button, without a full quiesce-first restart"
   design will reintroduce if it isn't respected (§5).

## 2. Current mechanism: nothing works until every migration finishes

`agentmux-srv/src/main.rs`'s boot order (comment numbering is the code's
own):

```
-1/0. crash monitor, process watchers, logging
2.  parse CLI args / config
4.  bootstrap::open_stores_and_migrate()   ← MIGRATIONS RUN HERE, SYNCHRONOUSLY
    spawn_background_subsystems()
5.  bind_listeners_and_network()           ← first point ANY request could be served
    spawn_reducer_plumbing(), build_app_state()
    saga resume-on-startup
6.  emit_estart()                          ← "ready" signal to the launcher
7.  build_router() + axum::serve()         ← actually starts serving
```

`open_stores_and_migrate` (`bootstrap.rs:495-592`) calls
`migrations::run_pending_migrations()` at line 573 — **before the TCP
listeners are even bound**, before `AppState` exists, before `ESTART`. There
is no partial-availability window: literally nothing in the process can
respond to anything until the migration batch (of however many migrations
are pending) has completed.

The launcher knows this is happening and compensates by widening its own
timeout rather than treating it as fast: `count_pending_migrations` runs
first and, if `>0`, emits `AGENTMUXSRV-MIGRATING migrations:<n>` on stderr
(`bootstrap.rs:569-571`); the launcher's stderr reader
(`agentmux-launcher/src/srv_spawner.rs`) extends its normal **30-second**
`AGENTMUXSRV-ESTART` wait to **30 minutes** on seeing that line. A failed
migration is fatal (`SPEC_MIGRATION_SYSTEM_HARDENING` Phase 1, shipped
2026-09-06/#3043): srv emits `AGENTMUXSRV-MIGRATION-FAILED` and exits 1
before `ESTART`, so at least a bad migration no longer boots into a
half-migrated store — but a *slow* one (up to the full 30-minute budget) is
still a silent, splash-locked wait with the entire app unusable, which is
the "no migrations at startup" complaint being raised here.

**Why so many migrations can be pending at once:** the registry currently
has 34 modules (`agentmux-srv/src/migrations/m0000_bootstrap.rs` through
`m0033_narrow_skill_global_uniqueness_index.rs`), and
`run_pending_migrations` applies every one the local DB hasn't recorded as
applied, in one batch, on whatever boot first sees them pending — a fresh
install, a machine that's been off AgentMux for a while, or a freshly
isolated `task package` per-build data dir.

## 3. The existing Maintenance/upgrade UI does NOT actually gate this today

`docs/specs/SPEC_UPGRADE_PANEL_2026_06_27.md` (header still literally says
**"Status: Spec — not yet implemented"**, though the components it
describes have since shipped — `frontend/app/statusbar/UpdateStatus.tsx`,
`MaintenanceSection.tsx`, `getApi().runMigrations()` /
`cef-api.ts:796`, `agentmux-cef/src/commands/backend.rs:173` — the doc
status line is stale and should be corrected regardless of this report's
outcome). It's tempting to read "the upgrade panel already exists" as "the
ask is already done." **It isn't, and the spec's own data-flow section says
so explicitly:**

> ### Post-restart migrations (automatic)
> ```
> Launcher starts agentmux-srv
>   → srv: count_pending_migrations() > 0 → emit AGENTMUXSRV-MIGRATING
>   → launcher: extend ESTART deadline to 30 min
>   → srv: run_pending_migrations() → JSON events to stderr
>   → srv: emit AGENTMUXSRV-ESTART pending_migrations:0   (or N on failure)
> ```

The panel's "Run Migrations" button (State E/F/G/H) is designed and
implemented as a **failure-recovery retry**, not a pre-boot gate: it only
becomes relevant when the automatic in-process run already happened and
either failed or (some edge case) left `pending_migrations > 0` afterward.
Confirmed directly in the shipped code, not just the spec:
`agentmux-cef/src/commands/backend.rs:180-186` — the button is **hard-blocked
in every launcher-managed production run**:

```rust
if std::env::var("AGENTMUX_BACKEND_PID").is_ok() {
    return Err(
        "Cannot run migrations while the backend is launcher-managed. \
         Restart AgentMux — the startup migration will run cleanly on next boot."
            .to_string(),
    );
}
```

It only actually runs in dev mode (`AGENTMUX_BACKEND_PID` unset, host owns
the sidecar directly), and even there it kills the sidecar, sleeps 300ms,
then spawns `agentmux-srv migrate` — i.e. it already knows it must quiesce
first (§5), it just isn't wired up to do that for the production,
launcher-managed case the ask is actually about. **The scaffolding this
report needs — the status-bar button, the live per-migration progress list,
the `upgrade:migration-event` WPS channel, the `pending_migrations` ESTART
field — already exists.** What's missing is making it the *only* trigger
for a production migration run, instead of a same-process retry for when the
automatic one already fired.

Also worth flagging while auditing this surface: `install_update` (the
actual download-a-new-version half of "Restart to Install") is **still a
stub** (`agentmux-cef/src/commands/stubs.rs:36,56` — Phase 7 of the same
spec was never implemented). There is no live auto-updater today. That's
orthogonal to this report — a manually-installed newer build is enough to
exercise "first boot after upgrade has pending migrations" — but it means
"an app restart would happen then" should be scoped to *this app's own*
restart (relaunch the already-installed newer binary), not to a
download-and-swap flow that doesn't exist yet.

## 4. Recommendation

Two constraints from §1 plus one goal from the ask, satisfied together:

1. **Zero-pending boot stays exactly as fast as it is today.** Keep the
   cheap `count_pending_migrations` pre-check
   (already called at `bootstrap.rs:569`) as the very first thing
   `open_stores_and_migrate` does. If it's `0` — true for essentially every
   boot in practice — fall straight through to today's path unchanged. This
   preserves `1052c985b`'s win completely; nothing about the common case
   should change.
2. **When it's `> 0` (only the first boot after a version's migrations were
   never applied), don't run them inline.** Instead of calling
   `run_pending_migrations()`, srv (or the launcher, even earlier) treats
   this as "upgrade required" and surfaces it exactly the way State E of the
   Maintenance panel already renders it (`⚠ N migrations pending` +
   `[Run Migrations]`) — except reached *before* any automatic attempt, not
   after one failed.
3. **Clicking the button runs the same safe sequence dev mode already
   proved out, promoted to production:** quiesce the running srv (wait for
   actual process exit, not `commands/backend.rs`'s current 300ms sleep
   heuristic — see §5), spawn `agentmux-srv migrate` (the existing CLI
   subcommand, `agentmux-srv/src/migrations/runner.rs:75`, already streams
   the same per-migration NDJSON progress the panel's State F consumes),
   then have the launcher relaunch srv. Relaunching now sees `0` pending and
   takes the fast path from (1) — so the *next* boot (and every boot after
   it) is fast again. This is where "an app restart would happen then"
   naturally falls out of the design rather than needing to be bolted on:
   the safest way to guarantee no live srv is touching the store during
   `migrate` is a real process boundary, not an in-place mode switch.
4. **This must move from `agentmux-cef` to `agentmux-launcher`.** The
   current implementation lives in the CEF host
   (`agentmux-cef/src/commands/backend.rs`) and only knows how to manage a
   dev-mode, host-owned sidecar (`state.sidecar_child`). In production the
   **launcher** owns the srv process (`agentmux-launcher/src/srv_spawner.rs`)
   — it's the only component that can safely quiesce-and-relaunch it. The
   `AGENTMUX_BACKEND_PID` guard in `backend.rs:180` should be replaced with
   a real implementation routed through the launcher's IPC
   (`agentmux-launcher/src/ipc/server.rs` already has a command dispatch
   this could extend), not deleted outright — deleting it without the
   launcher-side implementation would reopen the exact hazard `341faa981`
   fixed.

This directly resolves today's `REPORT_SPLASH_MIGRATION_ROW_OVERFLOW_AND_SUMMARY_2026_09_15.md`
as a side effect rather than needing its own fix on the boot path: since
migrations never run inside the ordinary boot's "backend" splash stage
anymore, there are never per-migration sub-rows on the splash to overflow.
That report's summarize/scroll recommendation still applies — just to the
Maintenance panel's own State F/G live list (`SPEC_UPGRADE_PANEL_2026_06_27.md`
§"State F"), which is a much rarer surface (once per upgrade, user-initiated,
already a scrollable panel context) than the boot splash was.

## 5. The hazard that must not be reintroduced

`m0011_shared_store_backfill`'s guard
(`agentmux-srv/src/migrations/m0011_shared_store_backfill.rs:21-25`) decides
whether to backfill each shared-store table by checking **whether it's
already non-empty**, e.g.:

```rust
let skip_accts = !shared.identity_list(None)...?.is_empty();
```

This is a generic hazard, not a one-off bug: any migration whose idempotency
check is "is there already data here" rather than "is this migration ID
recorded as applied" will silently and *permanently* skip real work if a
live srv process has written anything to that store between snapshot and
migrate — because migrations don't re-run once marked applied. This is
exactly what `341faa981` caught. Recommendations for whoever implements §4:

- **Wait for actual process exit**, not a fixed sleep, before spawning
  `migrate`. `commands/backend.rs:223`'s `sleep(300ms)` is a heuristic that
  happens to usually work for a local dev sidecar; a production
  quiesce-and-relaunch should wait on the child handle / PID directly.
- **Document the rule for future migration authors**: an idempotency check
  must be based on `migration_is_applied(id)` tracking, never on "does the
  target data already look populated" — the former is safe to run
  concurrently with nothing (migrations are always meant to run exclusive
  of everything else); the latter is only safe if NOTHING else can have
  written to that table since the batch started, which the whole point of
  this report is to make an explicit, rare, user-triggered event instead of
  an implicit one racing ordinary boots.
- The cross-process migration lock added in
  `SPEC_MIGRATION_SYSTEM_HARDENING` Phase 3
  (`agentmux-srv/src/migrations/runner.rs:306-329`,
  `migrations.lock.db`, 30-minute wait) prevents two *migration runners*
  from racing each other — it does **not** prevent the m0011 class of hazard,
  which is about a live *daemon* (not another migration run) mutating the
  same tables mid-batch. Don't mistake the existing lock for already having
  closed this gap.

## 6. Everything else in startup is not the bottleneck here

For completeness (the ask was to audit startup as a whole, not just
migrations): `docs/reports/REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md`
already audited the rest of the boot sequence (prep → backend → host spawn →
cef_init → window creation → paint → frontend bootstrap → tab reveal) and
found its issues to be about **visibility** (gaps that look stuck but
aren't, three divergent per-platform implementations of the same splash
logic) rather than real, avoidable wall-clock cost — nothing else in that
sequence has anything resembling migrations' up-to-30-minute worst case.
Squashing migrations out of the critical path per §4 is the one change here
with an actual, unbounded latency number attached to it; the rest of
startup is already fast and just needs the DRY/visibility fixes that report
already proposed.
