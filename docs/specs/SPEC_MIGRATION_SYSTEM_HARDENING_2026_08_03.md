# Migration System Audit & Hardening Plan
**Date:** 2026-08-03
**Status:** Proposed
**Scope:** agentmux-srv, agentmux-launcher, agentmux-cef
**Related:** [`docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`](./SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md) — written the same day, deliberately the same shape. Both audits found the identical underlying pattern: a marker that claims a state (a migration flag / a doc's `Status:` field) is trusted without ever being checked against ground truth, and nothing re-verifies it once written.
**Trigger:** Live incident — a launched agent ("Agent1", channel `local-main-b28b7a-9172ff88`, srv v0.54.9 / commit `1899c5ed`) failed to start, and a `FOREIGN KEY constraint failed` warning on `linkagentidentity` appeared in the logs around the same time.

**Correction (superseded initial hypothesis):** The first pass of this investigation (before direct SQL verification) hypothesized that `0007_agents_consolidate` had left `db_agents`/`db_agent_definitions` empty in this channel's `objects.db`, based on a `grep` for the two agents' definition IDs coming back empty. **That grep was a false negative** (binary SQLite file, not a reliable text-search target) and the hypothesis is **wrong for this incident**: direct `sqlite3`-equivalent queries (Part 0 below) confirm `db_agents`/`db_agent_definitions` are fully and correctly populated, including both agents in question. The migration-framework findings in Parts 1–3 below are independently verified against the source code (via `git show`, not grep) and stand on their own merits as a general audit — but they are **not** what caused this specific incident. Part 0 documents what actually did. Left the rest of this doc's original "Anatomy of the incident" framing in §1.2 intact as a *plausible failure mode this architecture allows*, explicitly marked as unconfirmed for this incident — worth fixing regardless, just not what happened here.

All file:line references below are against commit `1899c5ed` (`origin/main`, matches the running build). Verified by direct read (`git show 1899c5ed:<path>`), not just grep.

---

## Part 0 — What actually caused today's Agent1 incident

Verified directly against the live databases (`objects.db` for channel `local-main-b28b7a-9172ff88`, and `~/.agentmux/shared/store.db`), not inferred from logs alone.

**This is a recurrence of an already-diagnosed, still-unfixed issue.** See `agent1-stuck-error-retro.md` (2026-08-02/03, this workspace) for the original incident — a *different* Agent1 pane (block `e913dc16-...`, channel `agent1-06309`) hit the identical symptom the day before. Same agent **definition** id both times (`938e343c-98b2-417a-8d74-787e0a501e97`) — definitions are global (`0006_definitions_global`), shared across channels, so a broken definition follows the agent into every new channel it's launched in until the definition itself is fixed.

**Correction #2 (post-review):** the paragraphs below originally claimed Agent1's stray `github` identity link disqualified it from `0017_ambient_login_grandfather`'s ambient-login grandfathering, leaving it in fail-by-default mode with no usable `claude` credential. **That mechanism is wrong**, caught in review and independently re-verified: `agentmux-srv/src/identity/resolver/provider.rs:53` classifies `"github"` as `ProviderClass::ApiKey`, and `m0017`'s own doc comment is explicit that only **OAuth-class** links disqualify an agent from grandfathering — "Api-key-class links (e.g. a github PAT) do NOT count... must be grandfathered, not broken." Direct query of this channel's `db_agents` table confirms `938e343c-...` (Agent1) has `use_ambient_login = 1` — it **was** correctly grandfathered, exactly as the migration's logic dictates. The original claim conflated this incident with the prior day's retro without re-verifying the specific disqualification rule against source.

**Confirmed facts (re-verified):**
- `db_agents` / `db_agent_definitions` in this channel's `objects.db` are correctly populated: 11 rows each, including `938e343c-...` (Agent1) and `8e5f7b6d-...` (AgentX), with matching `db_agent_instances` rows. `0007_agents_consolidate` ran correctly here — not the bug.
- The shared store's `db_agent_identity_links` (61 rows total) shows AgentX with the normal three links (`claude`, `github`, `kimi`) — Agent1 has exactly one, for `github`, pointing at `acct-agenty-github-1782788341` ("AgentY GitHub") — no `claude` link. Same shape as the prior day's retro, but (per the correction above) this shape does **not** by itself explain a spawn refusal, since `github` doesn't gate the grandfather decision.
- Agent1's `db_agents.use_ambient_login = 1` (AgentX's is `0`, i.e. managed) — confirmed grandfathered, not fail-by-default.
- `muxspect describe 7943f67e-594c-4289-a61c-ecff68a0302d`, re-run with a newer build of the tool than was available earlier in this investigation (it now surfaces a `last_error` field that an earlier check of the same block did not show — the tool appears to have gained this during the session, matching the Phase-1.5 extension proposed in `REPORT_MUXSPECT_SPAWN_REFUSAL_DIAGNOSIS_EXTENSION_2026_08_03.md`), gives the actual, authoritative answer directly from Agent1's own persisted output rather than inference:
  ```
  last_error:
    message: [AgentMux] no credentials for claude: the bound account was deleted
             or is unresolvable. Bind an account for this provider in the Armory.
    source:  identity
    age:     200m
  ```
  This is a real identity/credential failure (confirming the general shape of both this incident and the prior day's retro — a `claude`-provider credential problem), but the *precise* mechanism by which an ambient-login agent ends up with "no credentials for claude" isn't fully traced in this doc — plausibly the ambient fallback itself resolves to a now-invalid or missing local CLI login state rather than a `db_agent_identity_links` row at all, since Agent1 has no `claude` link to begin with (ambient or not). Flagged as needing a proper trace through `identity/resolver/inject.rs`'s ambient-login path rather than guessed at further here.

**The `FOREIGN KEY constraint failed` warning that originally triggered this whole investigation is a separate, non-blocking bug**, not confirmed as Agent1's blocker (and per the above, likely unrelated to it). This channel's per-channel `db_accounts` table has **zero rows** — accounts live only in the shared store. Every one of the shared store's identity-link account IDs fails the `account_id REFERENCES db_accounts(id)` FK check when something (a frontend flow labeled `linkagentidentity`, "direct identity link write-through") tries to mirror it into the channel-local table. This fires for **any** agent's identity link in this channel — it's caught and logged as `WARN`, non-fatal. Still worth its own fix (Phase 0e below).

**Fix for Agent1:** the tool's own error message says it directly — "Bind an account for this provider in the Armory" — so bind a `claude` provider account (e.g. the shared `Claude (personal)` account, `a1990489-6de6-484a-9e20-83688c641524`) to definition `938e343c-98b2-417a-8d74-787e0a501e97`. I do not have a verified exact UI click-path for *linking an account to a specific agent definition* — I asserted one earlier in this investigation ("Settings → Identity," then corrected to "Identity & Memory → Armory → Accounts tab") and neither has been confirmed against the actual binding flow; this repo's own `CLAUDE.md` states the per-agent Identity tab is **read-only** ("No create/edit/delete/bind/unbind; new agent identities are created from the launch flow directly") — meaning the binding this agent needs may not happen through Armory at all. This is a write to shared identity state every running agent depends on, so — same as the prior retro — not making it unilaterally; flagging for an explicit go-ahead with the caveat that the exact mechanism (launch-flow-driven vs. an Armory action) needs confirming first, not assumed from doc comments a third time.

**New Phase 0e (added to the hardening plan in Part 3):** mirror `db_accounts` into per-channel stores (or stop attempting the per-channel write-through and rely solely on the shared store, if the per-channel copy isn't actually load-bearing for anything — needs a call from whoever owns the identity write-through code) so this FK warning stops firing on every identity-link sync, for every agent, in every channel.

---

## TL;DR

- The migration framework (`agentmux-srv/src/migrations/`) is well-designed on paper (`docs/specs/SPEC_MIGRATION_FRAMEWORK_2026_06_24.md`) but two of its own stated goals were quietly dropped during a June 2026 performance change, and one of its 19 migrations (`0007_agents_consolidate`) uses a marker pattern the codebase itself has since proven unsafe elsewhere.
- **Migrations run unconditionally in-process at every `agentmux-srv` startup** (`bootstrap.rs:533-541`), not as a separate pre-flight step as the spec designed. This was a deliberate, reasoned trade for launch latency (commit `1052c985`, "near-instant launch") — not an accident — but it silently downgraded failure handling from "block startup, show error modal" to "log a warning and boot anyway" (`bootstrap.rs:540`).
- There are, in effect, **three independently-evolved migration mechanisms** in the codebase (versioned `db_migrations` framework, legacy boolean flag files, and a separate JSON-registry marker system), and they disagree with each other about what "done" means. The incident happened at exactly that seam.
- A **genuinely more robust pattern already exists in this codebase** (`registry/def_migrate.rs`'s versioned, content-validated marker) and simply was never applied to the migration that broke.
- A real, multi-quarter **app-wide refactor is stalled mid-flight**: the "agent concept consolidation" (Phase 3a shipped, 3b partial, 3c/R/O not started) plus a second, parallel global-registry consolidation layered on top of it. The tracking doc (`docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md`) hasn't been updated in ~2 months while work continued past it.
- None of this shows up as "pending migrations" in the app's own health signal, because the failure mode is "marked applied, but wrote nothing" — a case the observability layer doesn't check for.

This doc: (1) inventories current state, (2) explains the startup-timing decision and gives a recommendation, (3) proposes a phased hardening plan, (4) gives an immediate, low-risk recovery path for the live broken instance.

---

## Part 1 — Audit

### 1.1 Three migration mechanisms, not one

| Mechanism | Where | Design | Used by |
|---|---|---|---|
| **A. Versioned framework** | `agentmux-srv/src/migrations/{mod,runner}.rs` | `Migration` trait, static `REGISTRY` (19 entries `m0000`–`m0018`, `mod.rs:111-130`), tracked in a `db_migrations` SQL table (shared store + per-channel `objects.db`) | The "real" system; all new migrations since June 2026 |
| **B. Legacy boolean flag files** | e.g. `agents_consolidate.rs:51`, `agent_session/migrations/v1_blocks.rs:19`, `v1_templates.rs:27` | A file's mere *existence* means "done"; content (if any) is never re-validated | `0002`, `0003`, `0007` still gate internally on these underneath the framework's own gate |
| **C. JSON-registry markers** | `registry/def_migrate.rs`, `registry/migrate.rs` | **Versioned** (marker stores a version number; older version ⇒ re-run) and **deferred-write** (marker only written if the whole pass succeeded) | `0004`–`0006` |

Mechanism C is measurably better-designed than B — it was built later, explicitly citing lessons from earlier bugs (`registry/def_migrate.rs:8-12`: *"A marker older than the current version... re-runs the scan ONCE, so users whose earlier pass was incomplete recover automatically"*). `docs/architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md` recommendation **I4** even names this explicitly: *"Make migrations structurally safe... use a versioned, recoverable marker... never finalize a poisoned partial pass."* **That recommendation was never back-ported to mechanism B**, which is what `0007_agents_consolidate` still uses — a latent risk (see §1.2), even though it's confirmed *not* the cause of today's incident (Part 0).

Framework A was supposed to *subsume* B and C (`m0000_bootstrap_migration_state` exists specifically to translate their state into `db_migrations` rows), but it only reads their *existence*, not their *correctness* — so A inherits every weakness of B and C rather than fixing them.

### 1.2 Anatomy of a plausible failure (hypothetical case study — NOT what happened today, see Part 0)

This walks through how mechanism B's design *could* produce a silent, marker-says-done-but-data-is-missing failure. It was the initial (incorrect) hypothesis for today's incident, disproven by direct SQL verification in Part 0. Kept here because it's a real gap in the code, independent of today's actual root cause — file:line citations are all real and re-verified.

1. `agents_consolidate.rs:79-88` — marker gate:
   ```rust
   if let Some(dir) = data_dir {
       let marker = dir.join(CONSOLIDATE_MARKER);
       if marker.exists() {
           return Ok(ConsolidateStats { already_done: true, ..Default::default() });
       }
   }
   ```
   Existence-only check. `already_done: true` with **zero rows written** is a valid, silent return value.

2. `m0007_agents_consolidate.rs:20-26` — the framework wrapper discards that signal:
   ```rust
   wstore.run_agents_consolidate(Some(&ctx.data_dir))
       .map(|_| ())   // ConsolidateStats — including already_done — thrown away
       .map_err(...)
   ```
   `Ok(())` either way. The runner has no way to distinguish "did the backfill" from "found a stale marker and did nothing."

3. **Two independent paths can produce the stuck state**, not just one:
   - *Direct*: `m0007.up()` runs, hits the marker short-circuit in step 1 (possible if the `.flag` file is stale relative to `objects.db` — e.g. a rebuilt/restored DB sitting next to an old flag file).
   - *Indirect, and more insidious*: `m0000_bootstrap.rs:87-90` runs once, on *any* install, and does this **without ever calling into `agents_consolidate.rs` at all**:
     ```rust
     if ctx.data_dir.join("migration_agents_consolidate_v1.flag").exists() {
         stamp_channel("0007_agents_consolidate");
     }
     ```
     This permanently writes a `db_migrations` row saying `0007_agents_consolidate` is applied, based on file existence alone, with **zero verification** that `db_agents` contains anything. Its own doc comment (`m0000_bootstrap.rs:8-11`) says the opposite is intended ("if we cannot determine whether a migration ran, we leave it unstamped") but the actual check doesn't live up to that principle for this migration — file-exists is treated as proof, not as a fallback signal.

4. Once `db_migrations` says `0007_agents_consolidate` is applied, **nothing ever revisits it.** `count_pending_migrations` (`runner.rs:389-396`, used for the "Migration failed" status-bar signal) only reads `db_migrations` — so this failure mode reports **`pending_migrations: 0`**, i.e. "everything's fine," while `db_agents` is actually empty or missing the affected agent.

5. Downstream, `db_agent_identity_links` (channel-scope schema, `backend/storage/migrations.rs:268-275`) still has:
   ```sql
   FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
   ```
   — **not** `db_agents(id)`. This FK was never updated when Phase 3a/3b introduced `db_agents` as the consolidated table. If `linkagentidentity` is called with a `db_agents`-space id that was never a `db_agent_definitions.id` (e.g. an instance-derived clone id from Pass 3/4 of the consolidation backfill, `agents_consolidate.rs:267-403`), the FK fails — independent of, and possibly compounding, the marker bug. **This needs to be checked directly against the broken data dir** (`local-main-b28b7a-9172ff88`) to confirm which of the two mechanisms actually fired — see Part 4.

6. `repair_def_gaps` (`agents_consolidate.rs:450-572`, called unconditionally every boot from `bootstrap.rs:794`, not marker-gated) is a partial safety net but does not cover this case: it only reads `db_agent_definitions` (never `db_agent_instances`), and inserts with **empty `identity_id`/`memory_id`** (`agents_consolidate.rs:538`). An agent whose only backing row was in `db_agent_instances` (exactly the named/identity-linked shape) is not recovered by it.

### 1.3 Full migration inventory (framework registry, `mod.rs:111-130`)

| ID | Scope | Legacy marker underneath? | Notes |
|---|---|---|---|
| `0000_bootstrap_migration_state` | Global | — | Stamps pre-framework state; see 1.2 for its gap |
| `0001_legacy_data_dir` | Global | dir-existence proxy | |
| `0002_block_zones_v1` | Channel | `migration_agent_zones_v1.flag` (bool) | |
| `0003_template_sessions_v1` | Channel | dir-existence proxy | |
| `0004_registry_from_sqlite` | Global | `.migrated_from_sqlite` (versioned, deferred-write) | Mechanism C |
| `0005_registry_source_bases` | Global | `.backfilled_source_bases` (bool) | |
| `0006_definitions_global` | Global | `.migrated_definitions` (versioned, mechanism C) | Best-designed marker in the codebase |
| `0007_agents_consolidate` | Channel | `migration_agents_consolidate_v1.flag` (bool, content ignored) | **The incident** |
| `0008_default_bundle` | Global | — | Now a documented no-op (Phase 4c of `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`) |
| `0009_transcript_backfill` | Global | — | |
| `0010_session_ids` | Global | — | |
| `0011_shared_store_backfill` | Global | — | Gates whether `id_store` binds to shared vs. per-channel store |
| `0012_dedup_identity_accounts` | Global | — | |
| `0013_agent_direct_bindings` | Global | — | |
| `0014_agent_direct_bindings_rerun` | Global | — | Existence of a `_rerun` migration is itself a signal that `0013` shipped with a bug once already |
| `0015_seed_starter_skills` | Channel | — | |
| `0016_seed_starter_mcp_servers` | Channel | — | |
| `0017_ambient_login_grandfather` | Channel | — | Per-channel pass over that channel's `db_agents` rows; see Part 0 for the actual disqualification rule (OAuth-class links only — corrected after an initial wrong claim here) |
| `0018_ambient_login_registry` | Global | — | Sibling pass for the global JSON registry, sharing `oauth_linked_agent_ids` with `0017` so the two can't disagree |

*(This table was corrected after review — five of nineteen scopes were originally listed backwards. Every row above is re-verified directly against each migration's `fn scope()` in `1899c5ed`.)*

Plus, outside the framework entirely: `backend/storage/migrations.rs` (raw SQL DDL — table creation/rename, separately versioned by nothing but `CREATE TABLE IF NOT EXISTS` idempotency).

### 1.4 In-flight app-wide refactors discovered

Two overlapping, both stalled:

1. **SQL agent-concept consolidation** (`docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md`, referenced by `agents_consolidate.rs:6`): Phase 3a (backfill into `db_agents`) shipped and is what `0007` runs. Phase 3b (flip ~8 read paths over) is **partially** shipped (`e30395bb` Phase 3b.2, `83a3e240` "Phase 3b unblock", others). Phase 3c (drop `db_agent_definitions`/`db_agent_instances`) has visible commits (`72797274` "Phase 3c PR1", `b6d64266` "Phase 3c, Decision B") but **the tables still exist** in current schema (`backend/storage/migrations.rs:190,298`) — confirmed live, not just per the (stale) spec. The spec's own acceptance criteria ("`db_agent_definitions` and `db_agent_instances` tables do not exist") is not met, 2+ months after the spec was last edited.
2. **Global JSON-registry cross-channel migration** (`registry/def_migrate.rs`, `registry/migrate.rs`) — a *second*, parallel consolidation of agent data, layered on top of #1. `docs/architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md` explicitly calls out that having two independent consolidation efforts in flight at once is itself a source of migration/marker/schema confusion.
3. Independent corroboration: `docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_20.md` §1.2 found and fixed a real data-loss bug in `agents_consolidate.rs` itself (`working_directory` hardcoded to `''` in two passes) — this migration has a track record of shipping with unnoticed defects, this is not its first incident.

**Net:** this is a real, not-hypothetical half-finished refactor. Any hardening plan has to either (a) treat the still-incomplete Phase 3b/3c as in scope, or (b) explicitly fence it off and say so — silently ignoring it will produce another SPEC that goes stale like the last one. **See `SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md` §1.3/§1.5 — this exact staleness pattern (a tracking doc frozen while work moves on) is not unique to this spec; it's a repo-wide, previously-audited-twice problem.**

### 1.5 Why migrations run on startup — and whether they should

The original design (`docs/specs/SPEC_MIGRATION_FRAMEWORK_2026_06_24.md`) was explicit and is worth restating because it's *better* than what's running today:

- A separate `agentmux-srv migrate` **subprocess**, invoked by the launcher, exits 0/1. Daemon only starts on exit 0.
- On failure: rollback, retain backup, write `migration-error.log`, **exit non-zero**, launcher shows a blocking modal ("Update failed... Your data has not been modified"). This satisfied the spec's Goal 3: *"A failed migration never bricks the app"* by never letting the app start in a half-migrated state at all.
- Goal 1: *"Zero migration code in `main.rs`."*

This subprocess path still exists in the code (`agentmux-srv migrate` subcommand, `runner.rs:41 run_migrate_command`; launcher-side caller `srv_spawner.rs::run_migrate`) but is **dead code today** — `srv_spawner.rs:83-90`'s own comment: *"Not called during normal startup... Preserved here as a fallback subprocess path... but has no active callers."* It's marked `#[allow(dead_code)]`.

**What actually happens now**, per commit `1052c985` ("run migrations in-process at srv startup for near-instant launch", 2026-06-27): migrations run inside the daemon process itself, unconditionally, before `AGENTMUXSRV-ESTART` is emitted (`bootstrap.rs:533-541`). This was a deliberate, reasoned trade — the subprocess model meant every launch paid a full second process spawn + teardown even when zero migrations were pending, and that's a real cost for a desktop app's perceived launch speed. Legitimate concerns this addressed:
- The ESTART deadline was extended to 30 minutes on `AGENTMUXSRV-MIGRATING` specifically so large migrations aren't killed mid-flight (a real fix, not a regression).
- A pre-migration snapshot is still taken (`backup_stores`, `runner.rs`) — the backup half of the safety story survived the change.

**What was lost in the trade, and shouldn't have been:**
- `bootstrap.rs:540`: `Err(e) => tracing::warn!("startup: migration error (continuing): {}", e)`. On migration failure, the daemon **boots anyway** — this directly contradicts the spec's own stated goal and is a strictly worse guarantee than the subprocess model gave. A failed migration today produces a running app in an unknown data state, discoverable only via generic downstream errors (exactly what happened with `Agent1`).
- The "applied but wrote nothing" failure class (marker/db_migrations desync, §1.2) isn't even a "failure" in this model — it returns `Ok`. There's no equivalent of the subprocess model's `--dry-run`/verification step that could have caught it before ESTART.

**Recommendation:** Don't revert to the subprocess model — the latency concern was real and the extended-ESTART-deadline mechanism is worth keeping regardless. Instead, close the specific gap: keep migrations in-process, but (a) make a failed migration **fatal** (stop before ESTART, surface to the launcher/status-bar as a blocking error, not a warning that boots anyway) and (b) add a **post-migration verification pass** so "applied" can't mean "silently wrote nothing" — detailed in Part 3, Phase 1. This keeps the performance win and restores the safety goal the original spec asked for. Full subprocess-mode reversion is a fallback only if verification-in-daemon turns out to be architecturally awkward (unlikely, given the check is a handful of row-count queries).

### 1.6 Observability gap

- `count_pending_migrations` / the launcher's "Migration failed — restart to retry" status-bar signal (`bootstrap.rs:1553` `emit_estart`, doc: *"Non-zero causes the status-bar to show a 'Migration failed' message"*) is **the only user-visible migration health signal**, and it only reflects `db_migrations` row presence — never data correctness. It reported `pending_migrations: 0` throughout this incident. **Separately confirmed live in this session:** the status-bar's cached copy of this value (`agentmux-cef`'s `AppState.pending_migrations`) is also sticky — `lib.rs:663-670` only ever *overwrites* it when a new ESTART reports `> 0`, never clears it back to 0 on a clean run, so a stale nonzero count from an earlier launch/auto-update can persist in the UI indefinitely. Worth folding into Phase 1's verification work.
- `muxspect` (PR #2390, `agentmux-srv/src/server/muxspect_handlers.rs`, `docs/MUXSPECT.md`) — genuinely new, shipped same day as this build — is purely about agent process spawn/execution errors (`last_error_frame()` tail-reads a block's persisted output for `error_during_execution` frames: identity-gate refusal, container `ensure_running` failure, queued-message drain failure). It has **no awareness of migration state** and wouldn't have surfaced this root cause even though it's the tool best-positioned to (it already exists specifically to answer "why did this agent fail to start").
- There is no "migration doctor" / self-check command that compares expected vs. actual outcomes (e.g. row counts in `db_agents` vs. `db_agent_definitions` + `db_agent_instances`) independent of the marker/`db_migrations` state.

### 1.7 Concurrency gap

No cross-process locking exists at the migration-runner level — `Store::conn` is an in-process `std::sync::Mutex` only. Two `agentmux-srv` processes against the same data dir (crash-restart race, misconfigured channel) can both see a migration as pending and both call `up()`; SQLite's WAL locking serializes individual statements but not the runner's check→run→mark-applied sequence (classic cross-process TOCTOU). Not implicated in this specific incident, but a latent risk the hardening pass should close while touching this code anyway.

---

## Part 2 — Failure-mode taxonomy

For prioritizing Part 3, the failure modes found:

**Note:** Part 0 confirms this specific incident's actual cause was an identity/credential issue (see the muxspect `last_error` finding), not F1/F2/F3 — the marker/data-loss hypothesis was disproven by direct SQL verification. F1/F2/F3 remain real, independently-verified gaps in the codebase (worth fixing regardless — see Part 3), just not what happened here. The "Caused this incident?" column below reflects that correction.

| # | Failure mode | Caused this incident? | Severity |
|---|---|---|---|
| F1 | Existence-only marker treated as proof of completion, content/effect never re-validated | No (disproven — `db_agents` was correctly populated) | High — silent data loss disguised as success, a real latent risk |
| F2 | `m0000_bootstrap` stamps `db_migrations` from marker existence with zero data verification | No (same disproof as F1) | High — same latent risk, bypasses the migration's own logic entirely |
| F3 | Stale FK reference (`db_agent_identity_links` → `db_agent_definitions` instead of `db_agents`) | No (confirmed — `agent_id` referenced a real, existing row) | Low — downgraded per Part 3 Phase 0d; still worth a one-time audit, not urgent |
| F4 | Migration failure is non-fatal at startup (`tracing::warn!`, boot continues) | No (this incident produced no `Err`, it "succeeded") but is the general-case version of the same risk class | High — turns any future migration bug into a silent one |
| F5 | No post-hoc verification / "migration doctor" | No — but would have caught F1/F2 *if* they had occurred; unrelated to this incident's actual cause | High — closing this gap is still valuable independent of this incident |
| F6 | No cross-process locking | No | Medium — latent |
| F7 | Stalled multi-phase refactor (Phase 3b/3c incomplete, tracking doc stale) | No, but is the reason `0007` still exists in its fragile Phase-3a-only form at all | Medium — structural risk multiplier |
| F8 | Stale, sticky status-bar cache (`pending_migrations` only set-on-positive, never cleared-on-zero) | No, but produces false "still broken" signals after the fact | Low-Medium — misdiagnosis risk, not data risk |
| F9 | Ambient-login agent (`use_ambient_login=1`) still fails with "no credentials for claude" — mechanism not fully traced (see Part 0) | Yes — this incident's actual, still-not-fully-explained cause | High — this is the real bug; not yet root-caused to a specific code path in this doc |

---

## Part 3 — Hardening plan

Phased so each phase ships independently and de-risks the next. Sizes: S = ~1 PR/day, M = ~2-4 PR/days, L = multi-week.

### Phase 0 — Stop the bleeding (S, do first, low risk)
Goal: make the exact bug class in this incident impossible to hit silently again, without restructuring anything.

- **0a.** `m0000_bootstrap.rs`: stop stamping `0007_agents_consolidate` (and ideally `0002`/`0003`) from marker existence alone. Add a cheap verification query (e.g. `SELECT COUNT(*) FROM db_agents` — if the source tables had any rows and `db_agents` is empty, leave unstamped so the real migration runs). This directly closes F2.
- **0b.** `agents_consolidate.rs` marker gate: before returning `already_done: true`, add a cheap sanity check — if `db_agent_definitions` or `db_agent_instances` has rows and `db_agents` doesn't, treat the marker as stale, log a warning, and re-run instead of trusting it. Closes F1 for this migration specifically.
- **0c.** `m0007_agents_consolidate.rs::up()`: stop discarding `ConsolidateStats`. Log it (`tracing::info!`) at minimum, so a future "wrote 0 rows" case is at least visible in logs even before 0a/0b land.
- **0d.** F3 (stale FK target on `db_agent_identity_links.agent_id`) — direct verification in Part 0 found `agent_id` was **not** the problem (the referenced `db_agent_definitions` rows existed correctly). Downgrade this item: still worth a one-time audit of whether `agent_id` should point at `db_agents.id` post-consolidation, but it's not urgent — deprioritize below 0e.
- **0e. (found during Part 0 investigation, real and confirmed, not hypothetical).** Per-channel `objects.db`'s `db_accounts` table is populated nowhere in the migration set — accounts live only in the shared store. Any "identity link write-through" that tries to mirror a shared `db_agent_identity_links` row into the per-channel copy hits `FOREIGN KEY (account_id) REFERENCES db_accounts(id)` and fails, for every agent, in every channel, every time (confirmed: 0 rows in a live channel's `db_accounts`, and the WARN recurs across the day for multiple agents). It's swallowed as non-fatal today, so it's not blocking anything currently, but it means: either add a migration/backfill step that mirrors `db_accounts` from the shared store into new/existing channel stores, or stop attempting the per-channel write-through entirely if the per-channel copy of identity links isn't actually load-bearing for anything (confirm with whoever owns the `linkagentidentity` write-through code before choosing).
- **0f. (found live, §1.6).** Fix the sticky `pending_migrations` status-bar cache: either recompute on every `get_backend_info` call instead of caching, or explicitly clear to 0 when a clean ESTART reports 0. Cheap, closes F8.

Acceptance: a synthetic test data dir with a stale `.flag` file and an empty `db_agents` table, run through `run_pending_migrations`, ends with `db_agents` correctly populated (not silently skipped). Separately for 0e: a fresh channel's `db_accounts` ends up non-empty (or the per-channel write-through is removed), and the FK warning stops appearing in logs. Separately for 0f: after a clean boot with 0 pending migrations, the status-bar popover shows no "Migrations" row even if a prior launch in the same session had a nonzero count.

### Phase 1 — Make failure fatal, add verification (M)
Goal: close F4/F5 — restore the spec's "failure is visible, never silent" goal within the in-process model (per the 1.5 recommendation — no subprocess reversion).

- Change `bootstrap.rs:537-541`: on `Err`, do not continue to ESTART. Surface a distinct `AGENTMUXSRV-MIGRATING-FAILED` (or similar) signal the launcher/status-bar already has plumbing for (`BackendStatus.tsx`, `emit_estart` path) — reuse the "Migration failed — restart to retry" UI rather than inventing new UI.
- Add a `--verify` (or `doctor`) mode to `run_pending_migrations` / a new `agentmux-srv migrate --verify` subcommand: for each *applied* migration that has a known verifiable postcondition (row counts, file existence of produced artifacts), check it and report mismatches. Start with `0007` (the incident) and `0002`/`0003` (same legacy-marker pattern); expand opportunistically.
- Wire this verify pass into `muxspect` or alongside it — muxspect already exists to answer "why did this agent fail," and migration-state mismatch is a legitimate answer it currently can't give (§1.6). Coordinate with whoever owns PR #2390 rather than duplicating its output-frame mechanism.

Acceptance: killing the process mid-migration (Phase 0's synthetic test, extended to inject a mid-transaction panic) results in a clearly-surfaced failure on next launch, not a silent partial state.

### Phase 2 — Unify the marker pattern (M)
Goal: close F1 structurally, not just for `0007`. Stop mechanism B (boolean flag files) from existing as a distinct, weaker pattern.

- Adopt mechanism C's design (versioned, content-checked, deferred-write marker — already proven in `def_migrate.rs`/`migrate.rs`) as the **only** sanctioned pattern for any migration that still needs a marker outside `db_migrations`.
- Port `0002_block_zones_v1`, `0003_template_sessions_v1`, `0007_agents_consolidate`'s internal markers to this pattern (or, better, retire the internal marker files entirely now that `db_migrations` plus Phase 1's verification exists — evaluate whether the internal marker is still pulling weight once verification exists, vs. being redundant state that can itself drift).
- Document the decision in `mod.rs`'s framework doc comment: "New migrations must not introduce their own marker file; use `db_migrations` (+ a verification postcondition per Phase 1) as the sole source of truth."

Acceptance: no migration in the registry has an internal marker whose *existence* (rather than *verified effect*) is trusted as complete.

### Phase 3 — Concurrency safety (S)
Goal: close F6.

- Wrap the check-pending → run → mark-applied sequence in `run_pending_migrations` in a cross-process advisory lock (a `flock`-style lockfile in the data dir is simplest and matches the portable/no-service-manager deployment model; SQLite `BEGIN IMMEDIATE` around the whole batch is an alternative if lock-file lifecycle management is judged riskier than it's worth).
- Add a test: two concurrent `run_pending_migrations` calls against the same fresh data dir; assert the migration's `up()` body executes exactly once.

### Phase 4 — Finish or explicitly fence the stalled refactor (L, needs product/eng call — not purely a hardening task)
Goal: address F7 and 1.4. This is the one phase that isn't purely mechanical — it's a scope decision.

- Get an explicit decision: either (a) resource finishing Phase 3b/3c of the SQL consolidation (flip remaining readers, drop `db_agent_definitions`/`db_agent_instances`, which also lets `0007` and its marker retire entirely per the framework's own "Retiring Old Migrations" section) — or (b) formally park it and update `SPEC_AGENT_ARCHITECTURE_2026_05_27.md` to say so, so the doc stops being silently stale.
- Same for the parallel global-registry consolidation (`registry/def_migrate.rs`) — determine whether it's still needed once/if the SQL side finishes, or whether it should be the long-term design and the SQL side should defer to it. Running two consolidation efforts indefinitely in parallel is itself a standing risk (per `ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md`'s own framing).
- Whichever direction is chosen, update the tracking doc in the same PR that does the work — the staleness here (2+ months) is what let Part 1.4 hide. Apply `SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`'s Phase 1 Status/`Superseded-by:` convention to whatever doc comes out of this.

### Phase 5 — Tests (M, can run parallel to Phases 1-3)
- Orphaned/stale marker + empty target table (the exact incident) — Phase 0's acceptance test, promoted to a permanent regression test.
- Crash mid-migration (kill between transaction commit and marker write; assert next run resumes correctly) — already claimed safe by a code comment (`agents_consolidate.rs:405-407`); turn that claim into an actual test.
- Upgrade skipping multiple versions (fresh `db_migrations` with only `0000`–`0005` applied; assert `0006`–`0018` all apply in order on next boot).
- Concurrent-boot test from Phase 3.
- `m0000_bootstrap` stamping correctness: pre-framework data dir shapes (marker files present/absent in each combination) → assert correct `db_migrations` rows, including the Phase 0a verification.

### Phase 6 — Documentation consolidation (S)
- One doc (or a clearly-marked update to `SPEC_MIGRATION_FRAMEWORK_2026_06_24.md`) that describes the *actual* current architecture (in-process, fatal-on-failure, verified — post Phases 0-2), superseding the subprocess-model description that's no longer what ships. Keep the subprocess path's rationale documented as "why it exists but is unused" rather than deleting it outright, unless Phase 4 decisions make it truly dead.
- A short **recovery runbook** for support/on-call: "an install is stuck with a stale marker / FK errors on agent identity linking" → the manual steps (Part 4 below), so this doesn't have to be re-derived from source next time.
- Both new docs this doc references should themselves follow `SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`'s Phase 1 conventions once that ships — closed `Status:` vocabulary, structural `Superseded-by:` — rather than adding two more free-text-status docs to the pile that doc is trying to fix.

---

## Priority order

`0a → 0b → 0c → 0d → 0e → 0f` (this week — cheap, directly closes the incident's root cause plus the two secondary bugs found live) → `Phase 1` (next — restores the spec's safety goal) → `Phase 5` tests written alongside 0/1 → `Phase 2` and `Phase 3` (can run in parallel once 0/1 are stable) → `Phase 6` docs as each phase lands → `Phase 4` on its own timeline since it's a scope/resourcing decision, not an engineering task, but should be raised explicitly rather than left to keep drifting.

---

## Part 4 — Immediate recovery for the live broken instance

Superseded by Part 0's direct verification: `db_agents`/`db_agent_definitions`/`db_agent_instances` in `local-main-b28b7a-9172ff88` are correctly populated — **no marker deletion, DB surgery, or migration re-run is needed or appropriate here.** The instance isn't broken at the data layer at all.

The actual fix is an identity-link change, not a migration-recovery action: bind a `claude` account to agent definition `938e343c-98b2-417a-8d74-787e0a501e97` (Agent1) — per `muxspect`'s own persisted error message, "Bind an account for this provider in the Armory." This is a shared-store write affecting every channel Agent1 runs in, not a local `objects.db` fix — do it once, not per-channel. The exact UI mechanism for binding an account *to an agent* (as opposed to managing accounts themselves in Armory) is not verified in this doc — see Part 0's caveat.

Separately, low-priority: the `db_accounts`-empty-per-channel FK warning (§0e) is cosmetic today (swallowed, non-blocking) and doesn't need manual intervention — it's a Phase-0e code fix, not a per-instance recovery step. Same for the stale status-bar migration count (§0f) — refreshing/restarting clears the display even before the code fix ships.

This section should be superseded by Phase 6's runbook once written — including a corrected version of the generic "stuck migration" recovery steps from the original (pre-Part-0) draft, which are still valid *if* the F1/F2 hypothetical in §1.2 is ever confirmed for a real instance, just not this one.
