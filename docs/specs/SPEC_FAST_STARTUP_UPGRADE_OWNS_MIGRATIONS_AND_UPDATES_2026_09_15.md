# SPEC — Fast startup: the Upgrade button owns migrations and updates, the boot path owns nothing deferrable

**Status:** proposed — not implemented
**Date:** 2026-09-15
**Tracking:** [#3258](https://github.com/agentmuxai/agentmux/issues/3258) —
tracking: startup + splash — get migrations and updates off the boot path
(no prior splash/startup tracker existed; #820 and #2146 are both closed and
narrower)
**Goal (repo owner):** "there should be no migrations at startup... any
migrations will run through the upgrade system which will appear as a button
in the status bar, migrations and an app restart would happen then" +
"remove the migrations and any updates from startup so it is as fast as
possible."

**Consolidates / supersedes in part:**
- `docs/specs/SPEC_UPGRADE_PANEL_2026_06_27.md` — the Maintenance section UI
  (partially shipped; its header still says "not yet implemented", which is
  stale). **Its "Post-restart migrations (automatic)" data flow is the thing
  this spec replaces.** Its panel states A–K are reused as-is.
- `docs/specs/app-update-check.md` — update detection + per-install-type
  install flows (entirely unimplemented; `install_update` is still a stub at
  `agentmux-cef/src/commands/stubs.rs:36,56`). **This spec adds a scheduling
  constraint it doesn't currently have: the check must never sit on the boot
  path.**
- `docs/reports/REPORT_STARTUP_MIGRATION_AUDIT_AND_DEFERRAL_2026_09_15.md` —
  the migration half of the audit, including the two prior attempts and why
  they were reverted. Read that first; this spec is its design follow-through.
- `docs/reports/REPORT_SPLASH_MIGRATION_ROW_OVERFLOW_AND_SUMMARY_2026_09_15.md`
  — the splash symptom that goes away once migrations leave the boot path.

---

## 1. The single rule

> **The ordinary boot path does only work that must happen before this
> specific launch can be useful. Everything else — migrations, snapshots,
> update checks, update installs, backfills, sweeps — belongs either behind
> the Upgrade button or after the app is already interactive.**

Everything below is the application of that rule to what's actually on the
boot path today.

## 2. Why this isn't just "move migrations to a thread"

Two prior attempts already bounded the solution space
(`REPORT_STARTUP_MIGRATION_AUDIT_AND_DEFERRAL_2026_09_15.md` §1 has commits
and dates):

- **`1052c985b` (repo owner, 2026-06-27)** reverted an earlier deferral
  because the subprocess-per-boot design taxed *every* boot, including the
  overwhelmingly common one with nothing pending. **Constraint 1: the
  zero-pending boot must not gain a single new process spawn, file open, or
  network call.**
- **`341faa981` (2026-06-26)** removed an on-demand "run migrations now"
  path because running `agentmux-srv migrate` against a data dir a live srv
  still held caused permanent data loss — `m0011_shared_store_backfill`'s
  skip-if-already-non-empty guards
  (`agentmux-srv/src/migrations/m0011_shared_store_backfill.rs:21-25`) saw
  tables the running instance had touched and skipped the backfill for good.
  **Constraint 2: migrations only ever run with no live srv holding that
  data dir — enforced by a real process boundary, not a sleep.**

Constraint 2 is why "and an app restart would happen then" isn't a UX
preference in this design — it's the safety mechanism.

## 3. Current boot path — inventory and verdicts

Order per `agentmux-srv/src/main.rs`; nothing below step 6 can serve a
request, because `AGENTMUXSRV-ESTART` (step 6) is the launcher's only
readiness signal.

| # | Work | Where | Gated? | Verdict |
|---|---|---|---|---|
| 4a | Pre-migration DB snapshot (copies `objects.db` + `store.db`) | `bootstrap.rs:553`, `backend/storage/snapshot.rs:219` | Yes — `needs_snapshot()` fires only when the on-disk schema version differs | **Move to the Upgrade action.** It exists solely as a rollback aid for the migration batch; it should travel with it, not with boot. |
| 4b | **Pending migrations, synchronously** | `bootstrap.rs:569-592`, `migrations/runner.rs:264` | Fast-path `Ok(0)` when none pending | **Move to the Upgrade action** (§4). This is the whole point of this spec. |
| 4c | Registry `session_id` catch-up backfill — "run every startup" | `bootstrap.rs:606-618` | No | **Measure, then move off the boot path** (§6). |
| 4d | Transcript backfill def-id capture | `bootstrap.rs:643-650` | No | Measure; likely post-ESTART. |
| 4e | `heal_global_snapshot_source_block_ids` gap-repair | `bootstrap.rs:797` | No | Measure; likely post-ESTART. |
| 4f | `agent_seed::auto_seed_on_startup` | `bootstrap.rs:870` | Unknown | Measure; likely post-ESTART. |
| 4g | Saga id seed — `saga_log.max_saga_id()` scan | `bootstrap.rs:819` | No | Needed before serving (id allocation); keep, but measure. |
| 5 | Bind listeners, LAN discovery, process tracker | `bootstrap.rs:1352` | — | Keep — this *is* readiness. |
| 5b | Saga resume-on-startup (`compensate_unresolved`) | `main.rs:134` | — | Keep before serving — its own comment explains why (resumed compensation must not interleave with new sagas). |
| 6 | `emit_estart()` | `main.rs:162` | — | Should be reachable in the low hundreds of ms on a warm data dir. |
| 7+ | Cron scheduler, native-memory drift + retention, WAL checkpoint loop, session archiver sweep | `main.rs:107-120`, `bootstrap.rs:1302` | — | Already after `ESTART` — correct, keep. |
| — | **Update check** | nowhere | — | **Not implemented at all today** (no Rust emits `app-update-status`; `UpdateStatus.tsx` reads atoms nothing populates). §5 exists to keep it off the boot path when it *is* built. |

**The honest headline:** with a warm data dir and nothing pending, today's
boot is already fast — `run_pending_migrations` returns `Ok(0)` immediately
and the snapshot is skipped. The unbounded cost is the **first boot after a
version ships new migrations**: that boot runs the full batch (up to all 34
registered migrations) inline, before the listeners are even bound, with the
launcher silently widening its `ESTART` deadline from 30 s to **30 minutes**
(`agentmux-launcher/src/srv_spawner.rs`). That is the boot this spec moves.

## 4. Target design — one Upgrade flow, owned by the launcher

### 4.1 Boot behavior

```
srv boot
  └─ pending check (NOT today's count_pending_migrations — see below)
       ├─ Ok(0)  → continue exactly as today. No snapshot, no migrate, no
       │           spawn, no new I/O. Constraint 1 satisfied by construction.
       ├─ Ok(n)  → DO NOT migrate. Enter upgrade-only mode (§4.2).
       └─ Err(e) → DO NOT migrate, DO NOT boot normally. Upgrade-only mode
                   with an error state. Fail closed.
```

**Today's `count_pending_migrations` cannot be the gate, for two reasons —
both documented in its own contract (`migrations/runner.rs:505-535`):**

1. **It returns `0` on every error, by design** ("Returns 0 on any error so
   startup is never blocked"): an unresolvable shared-store path, a failed
   shared-store open, or a failed channel-store open (which silently drops
   all channel-scoped migrations from the count). Under today's
   always-migrate boot that is a safe default — the migration run is still
   ahead. Under this spec it inverts: a transient open error would report
   "nothing pending", take the fast path, and run the new binary against the
   old schema — the exact outcome the design exists to prevent. **The check
   must become fallible, and an error must route to the blocking state, not
   the fast path.**
2. **It opens both stores read-write and may create `objects.db`** —
   `Store::open` runs DDL/schema setup and mutates the file. That breaks the
   snapshot ordering `bootstrap.rs:529-534` deliberately established (§4.3
   step 2). **The gate needs a genuinely non-mutating pending check** — read
   the applied-migration ids without a schema-setup open — or the snapshot
   must stay strictly ahead of it.

### 4.2 The gated state — full-window blocking upgrade screen

**Decided (repo owner, 2026-09-16): Option A, as a full-window blocking
screen.** With `pending_migrations > 0`, the running binary is newer than the
schema on disk. The app does not open the workspace and does not read the
stale stores — it renders an upgrade screen instead. No mixed-schema
reasoning is needed for any current or future migration, which is the whole
reason to prefer it: the rejected alternative (boot normally, advisory
status-bar button, per-feature gating of DB-touching APIs) makes "is this
migration safe to defer?" a judgment call on every future migration, and the
first wrong answer is a data bug.

A dismissible modal over a dimmed, read-only workspace was also considered
and rejected: rendering panes from a stale schema re-raises exactly the
question this design exists to avoid.

**This is not the splash.** The splash is a non-interactive native window on
all three platforms (`splash.rs:237` creates it
`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TOPMOST`, with
no mouse handling; macOS and Linux are two further separate
implementations). Putting a button there would mean writing input handling
three times in three native toolkits — the triplication
`REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md` §1 identifies as
the root cause of every splash defect so far. Under this design the splash
behaves *normally and dismisses faster than today*, because `ESTART` is no
longer waiting on a migration batch.

Sequence:

1. Launch → splash → dismisses at its usual speed (nothing is migrating).
2. The CEF window opens as it always does.
3. Instead of the workspace, it renders:

```
┌────────────────────────────────────────────────┐
│                                                │
│   ↑  AgentMux needs to finish upgrading        │
│                                                │
│   3 database migrations are pending for        │
│   v0.56.1. They run once, then AgentMux        │
│   restarts.                                    │
│                                                │
│   A backup is written to                       │
│   ~/.agentmux/shared/backups/ first.           │
│                                                │
│            [ Upgrade & Restart ]               │
│                                                │
│   Details ▾                                    │
└────────────────────────────────────────────────┘
```

4. Click → §4.3's flow, rendering `SPEC_UPGRADE_PANEL_2026_06_27.md`'s
   State F live progress list (summarized + scrolled per §4.4).
5. Restart → `0` pending → §4.1's fast path, every launch from then on.

**The gate is in the backend; the screen is only its presentation.** A
frontend-only check would be far too late: after the migration skip,
`open_stores_and_migrate()` still runs `Store::open`, shared/identity-store
attachment, and every repair/backfill/seed pass, and `main.rs` then runs
saga resume-on-startup — all of it reading *and writing* the stale stores
before `ESTART` is even emitted, let alone before `app.tsx` could decline to
mount a pane. So the pending path needs an **upgrade-only srv mode**:

- bind listeners and emit `ESTART` (so the window can come up and talk to
  srv at all),
- serve only what the upgrade screen needs — pending count, migration
  progress, the trigger — plus whatever is genuinely schema-independent,
- **skip** normal store initialization, the repair/backfill/seed passes,
  saga resume, and the background subsystems entirely.

The frontend gate (`workspace.tsx` / `app.tsx`, off
`backendInfo().pending_migrations` / `needs_upgrade`, before any pane
mounts) is then just what the user sees — defense in depth, not the
mechanism. The status-bar `UpdateStatus` chip and Maintenance section
reflect the same state for consistency (§5.5), but in this state the
full-window screen is what the user actually reaches first.

**Relationship to staging (§5.4):** staged updates are what make this screen
*rare*, not what replace it. With staging, the old binary keeps running
against its own matching schema at full speed until the user chooses to
upgrade, so this path is only reached when a newer binary is already what
launched — a manual install, a `task package` build, or a channel switch.
The two are complementary: staging prevents most occurrences, this screen
handles the residue safely.

### 4.3 The Upgrade action (button → done)

Reuses `SPEC_UPGRADE_PANEL_2026_06_27.md`'s states E→F→G/H verbatim for
display. The orchestration is new and **belongs in `agentmux-launcher`**,
which owns the srv process in production:

```
User clicks Upgrade (status bar / Maintenance section)
  1. launcher: ENTER MAINTENANCE STATE — suspend normal srv-exit handling
        (see "supervision" below; without this, step 2 either races a
        respawned daemon or tears the app down mid-upgrade)
  2. launcher: stop srv, WAIT FOR ACTUAL PROCESS EXIT   ← Constraint 2
        (not commands/backend.rs:223's 300 ms sleep heuristic)
  3. launcher: pre-migration snapshot — BEFORE anything opens the stores
        (see "snapshot ordering" below)
  4. launcher: if a staged app update exists, INSTALL IT NOW (§5) — before
        step 5, so the migrations that run are the new version's
  5. launcher: spawn `agentmux-srv --wavedata <dir> migrate`
        - already exists: migrations/runner.rs:75 (run_migrate_command)
        - already streams NDJSON per-migration progress the panel consumes
        - the reader already exists too: srv_spawner.rs::run_migrate,
          currently #[allow(dead_code)] with no callers — revive it
  6. launcher: leave maintenance state, relaunch the app
        → next boot sees 0 pending → §4.1 fast path → ordinary startup
  on failure: panel State H with the failing migration id + error; see
        "partial application" below — the data is NOT untouched
```

**Supervision must be suspended for the whole window.** Stopping srv is not
enough: `supervisor/windows.rs:964` treats every srv exit as unexpected and
immediately respawns it (then deliberately kills the host and rebuilds the
session), and `supervisor/unix.rs:713` treats a clean srv exit as launcher
teardown and brings the host down with it. So the naive "stop srv, run
migrate" either puts a live daemon back on the data dir *while `migrate` is
running* — reintroducing the `341faa981` data-loss hazard this whole design
is built around — or closes the progress UI the user is watching. An
explicit maintenance state (or control-channel command) must suppress both
behaviors until the relaunch in step 6.

**Snapshot ordering is load-bearing.** `bootstrap.rs:529-534` puts
`maybe_snapshot_pre_migration()` before any store open precisely because
`Store::open` runs DDL/schema setup and mutates the file — a snapshot taken
afterwards captures post-DDL state and is not a valid rollback aid. Since
the pending check in §4.1 currently opens stores read-write too, either it
becomes non-mutating or the snapshot moves ahead of it. The snapshot must
be the first thing that touches those files in the upgrade window.

**Partial application is the real failure contract.** `apply_pending` has
no batch transaction: each migration commits and is marked applied as it
goes, and the failing one may itself have written partially. So a mid-batch
failure leaves the data *partly migrated*, not untouched — the backup under
`~/.agentmux/shared/backups/pre-migration-*/` is a recovery source, not a
rollback. State H must say which migrations applied, which failed, and
where the backup is; retry resumes from the first unapplied id (migrations
are individually idempotent), and neither the UI nor any recovery logic may
assume the store is in its pre-upgrade state.

`agentmux-cef/src/commands/backend.rs:173`'s existing `run_migrations`
already implements this shape for **dev mode only** and hard-refuses in
production (`AGENTMUX_BACKEND_PID` guard, lines 180-186). That guard stays
until the launcher-side implementation above exists — then it is replaced by
a call into it, never merely deleted.

### 4.4 Progress UI

Per-migration progress moves off the boot splash entirely (it has nowhere to
render once migrations don't run at boot) and into the Maintenance panel's
State F list, which is scrollable and user-initiated. Apply
`REPORT_SPLASH_MIGRATION_ROW_OVERFLOW_AND_SUMMARY_2026_09_15.md`'s
recommendation there instead: summarize completed migrations into a running
tally ("7/34 applied"), keep the in-flight one in detail, and keep the latest
entry in view when the list overflows.

## 5. Updates: never on the boot path

Nothing exists yet, so this is a constraint on `app-update-check.md`'s
implementation rather than a removal:

1. **No update check before the window is interactive.** Not at process
   start, not during splash, not before first paint. Schedule it after the
   app is idle post-first-paint (the spec's own "launch + 10s" is
   acceptable *if* it is genuinely wall-clock-after-ready, not a boot step).
2. **The check is network I/O — it must never gate anything.** Failure,
   timeout, and offline are all "no badge", never a retry loop on the boot
   path, never a reason a window appears later.
3. **Install-type detection (`app-update-check.md` §"Detection Logic (Rust,
   at startup)") must move off startup too** — it's only needed when the
   user clicks, so compute it lazily at click time (or cache it after first
   computation), not on every launch.
4. **Downloads are staged, never auto-applied.** The running version keeps
   running until the user clicks. This is what makes "old binary, already
   migrated, 0 pending" the steady state, and it's why the Upgrade button
   can own both halves: install the staged update **first**, then run
   `migrate` from the newly installed binary (§4.3 steps 4-5, in that
   order), then restart once. Running `migrate` before the swap would only
   apply the *old* binary's migrations, leaving the new version's own
   migrations pending and the next launch blocked again — the opposite of
   the "next boot sees 0 pending" contract.
   Staging is also what keeps §4.2's blocking screen rare — it is not a
   substitute for it, since a manually-installed build, a `task package`
   build, or a channel switch can still put a newer binary in front of an
   older data dir.
5. **One button, one mental model.** "Update available" and "migrations
   pending" are two inputs to the same status-bar affordance and the same
   Maintenance panel — not two competing buttons that can each demand a
   restart.

## 6. Startup budget + instrumentation

Before moving 4c–4f, measure them — the per-boot passes have never been
individually timed; the splash's `backend` stage reports one number covering
all of them.

- The instrument already exists: the `sub_begin`/`sub_end` stage-telemetry
  path added for per-migration rows
  (`agentmux_common::srv_stderr::{migration_begin_line,migration_end_line}`,
  `agentmux-launcher/src/startup_events.rs`). Wrap each pass in 4c–4f the
  same way, read the numbers off a debug boot, then move anything that isn't
  needed-before-serving to a post-`ESTART` task.
- **Proposed budget for a warm, zero-pending boot:** srv reaches `ESTART` in
  **< 300 ms**, and nothing on the boot path performs network I/O, copies a
  database file, or spawns a process. Treat a regression past that as a bug
  with an owner, not a slow machine.

## 7. Invariants (for reviewers)

A change to `bootstrap.rs`/`main.rs`/`srv_spawner.rs` should be checked
against these:

- **S1** — Ordinary boot (pending check returns `Ok(0)`) performs no
  migration, no snapshot, no update check, no subprocess spawn.
- **S1b** — The pending check fails **closed**: any error reaching it routes
  to the blocking upgrade state, never to the ordinary boot path. "Couldn't
  tell" is not "nothing pending".
- **S1c** — With migrations pending, srv does not open, repair, backfill,
  seed, or otherwise write the stale stores. The gate is in the backend;
  the frontend screen is presentation.
- **S2** — Migrations never run while any srv process holds that data dir:
  the launcher confirms process exit (not elapsed time) **and** suspends its
  own respawn/teardown handling for the whole migration window.
- **S2b** — A staged update is installed before its migrations are run, never
  after.
- **S3** — A migration's idempotency check is `migration_is_applied(id)`,
  never "does this table already have rows" (the `m0011` class of bug —
  see §2).
- **S4** — Nothing on the boot path performs network I/O.
- **S5** — `ESTART` is not gated on work that could run after it. New
  startup work defaults to post-`ESTART` unless it can name what breaks
  otherwise.

## 8. Phasing

| Phase | Scope | Notes |
|---|---|---|
| 1a | `agentmux-launcher/src/upgrade.rs`: `quiesce_srv` (real wait-for-exit, not a sleep — Constraint 2) + `run_migration_upgrade` (quiesces, then calls the revived `srv_spawner::run_migrate`). No snapshot step needed — `agentmux-srv migrate`'s own `apply_pending` already backs up under the migration lock before applying, so a launcher-side snapshot would be redundant, not a replacement. **Done** — landed standalone, not yet called from anywhere live. |
| 1b | Wire 1a into an actual caller: the "suspend the supervisors' respawn/teardown handling" half, plus relaunching srv on success. Deliberately deferred until phase 2 decides the boot-gate shape — an upgrade-only srv mode most likely means srv never becomes a respawn-on-exit child of the normal supervised loop in the first place, which changes what "suspend the loop" even means. Wiring into today's 1000+-line `supervisor/{windows,unix}.rs` loops now would be guessing at an integration shape phase 2 gets to actually decide, against surface this repo has no way to exercise end-to-end outside a real multi-process launcher run (CLAUDE.md's I1–I6 isolation invariants). |
| 2 | Fallible, non-mutating pending check (today's `count_pending_migrations` is neither); upgrade-only srv mode; full-window blocking upgrade screen (§4.2) | The actual "no migrations at startup" change. The backend mode is the gate — the screen alone is not |
| 3 | Panel rewire: State E reachable *before* any automatic attempt; summarize + scroll per §4.4 | Mostly frontend |
| 4 | Instrument + relocate 4c–4f; enforce the §6 budget | Independent of 1–3, can run in parallel |
| 5 | `app-update-check.md` implementation under §5's constraints; Upgrade button installs staged update + migrates in one restart | Unblocks `install_update`, still stubbed today |

## 9. Open items

- `SPEC_UPGRADE_PANEL_2026_06_27.md`'s stale "not yet implemented" header
  needs correcting — its panel components shipped.
- ~~Option A vs. B (§4.2)~~ — **decided 2026-09-16: Option A, full-window
  blocking screen.**
- Whether the launcher relaunch in §4.3 step 5 restarts the whole app
  (simplest, matches "an app restart would happen then") or only respawns
  srv and reconnects the frontend (fewer moving parts for the user, more for
  the code).
