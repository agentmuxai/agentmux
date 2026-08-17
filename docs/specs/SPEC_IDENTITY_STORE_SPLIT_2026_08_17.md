# SPEC: Split the multi-concern shared store — permanent global identity data vs. explicitly-disposable Armory test accounts

**Date:** 2026-08-17
**Author:** AgentY (agent, `~/.agentmux/agents/agenty-0629j`), design confirmed with the human operator
(robust fix approved; "engineering time is not a factor").
**Status:** Approved design — implementation in progress, sequenced as several PRs (§6).
**Ground truth basis:** `agentmuxai/agentmux` `origin/main`. Every file:line citation below was independently
verified, not taken from a sub-agent's report on faith.
**Related:**
`docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md` (root-cause verification this
design fixes),
`docs/specs/REPORT_AGENT_IDENTITY_HISTORY_FRAGMENTATION_2026_08_16.md`,
`docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md` (P1 taxonomy this design extends),
`docs/specs/CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17.md`,
`docs/specs/SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md` (original rationale for the store this design
splits),
`docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` (PR #2431, the regression's origin).

---

## 1. Problem, restated briefly

`docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md` traced why reopening a named
agent after a version/channel switch fails: the account-binding lookup (`db_agent_identity_links`,
`db_accounts`) lives in a store whose *location* — global vs. per-channel — is decided by one process-wide
flag, `isolated_auth_enabled()`, which defaults to "isolated" on every non-`"stable"` channel (every local/dev/
portable build). The same flag also happens to gate three other, unrelated tables (`db_bundles` — ABF memory,
`db_drone_definitions`, `db_muxbus_credentials`/`db_agent_credentials`) that a follow-up code audit (this doc's
§2-§3) confirmed have **no legitimate reason to be per-channel at all** — their isolation is incidental, not
intended.

The one thing that *does* need isolation — Armory's disposable test-account flow, so destructive
delete-account testing never touches a real account — currently has **zero code-level signal** distinguishing
a test account from a real one (§2.2). Isolation only ever happens by coincidence of which channel the server
process is running on, which is also exactly what breaks continuity for every ordinary user on every version
bump.

## 2. Full audit (verified against live code)

### 2.1 Every read/write of the two identity tables is already uniformly routed

Confirmed via a full-crate grep + read: `agentmux-srv/src/backend/storage/identities.rs` is the only place raw
SQL touches `db_accounts`/`db_agent_identity_links` (`identity_list/get/upsert/delete`,
`agent_identity_link/unlink/list_for_agent/list_all/link_provider_pairs`, ~lines 215-578). Every production RPC
handler and migration that calls these already captures `state.id_store` (never `state.wstore`) —
`agent_handlers/identity.rs`, `app_api/bundle.rs:1187-1216`, `app_api/mod.rs:634-690`, `app_api/identity.rs`,
`identity/resolver/inject.rs:161-167,398,546`, `identity_handlers.rs:171-358`, `identity_auth_persist.rs:34-97`,
and migrations `m0011`-`m0019`. **There is no scattered-call-site bug to fix here** — the routing is already
disciplined. This means the fix is "point the existing, uniform routing at a different store," not a hunt
through divergent call sites (contrast with the CHECKLIST's incident #2/#3, which *were* scattered-call-site
bugs, already fixed).

### 2.2 There is no "disposable test account" signal anywhere

`auth.start` (`identity_handlers.rs:171-246`) and `account.key.verify` (`agent_handlers/identity.rs:191-341`)
are the only two account-creation paths. Neither request type, nor the `IdentityAccount` struct itself
(`identities.rs:106-134`: `id, name, provider, kind, display_name, secret_ref, context, status, created_at,
updated_at`), carries any field marking an account as a test/throwaway account. Whether an account created via
these paths ends up "isolated" is decided entirely by `isolated_auth_enabled()` at the moment the RPC happens
to run — the exact same code path, same RPC, same struct, for a developer's real personal login and for an
Armory tester's disposable delete-test account.

### 2.3 Bundles/drones/MuxBus creds have no legitimate isolation use case; accounts are the one real exception

- **Memory bundles** (`db_bundles`, `backend/storage/memory_bundles.rs`) — per
  `SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`'s P1 taxonomy, PORTABLE CONFIG is supposed
  to be "always global." The code does not actually enforce this independent of `isolated_auth_enabled()` —
  it's plain `impl Store`, routed wherever the caller's `id_store` happens to resolve. A bundle created on an
  isolated channel is exposed to the identical continuity bug as an account. No test asserting "bundles never
  vary by channel" exists anywhere in the crate today.
- **Drone definitions** (`db_drone_definitions`, `drone/storage.rs`) — same pattern; `drone_handlers.rs:82`
  explicitly comments "routed to id_store (global shared store)" as the *intent*, enforced only by handler
  discipline, not a dedicated resolver. (Its sibling `db_drone_runs` is correctly, deliberately per-channel —
  run history is ephemeral by design, per `SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md` §5 — that split is
  NOT part of this doc's scope and should not change.)
- **MuxBus credentials** (`db_muxbus_credentials`, `db_agent_credentials`, `backend/storage/muxbus.rs`) — per
  `SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md` §4's own stated rationale ("login to muxbus in one build,
  launch a new version, and the cloud subscriber stays disconnected"), this has **no legitimate per-channel use
  case at all** — unlike accounts, there's no Armory-style destructive-testing reason to ever isolate it.
- **Accounts/links** (`db_accounts`, `db_agent_identity_links`) — the one category with a real, intentional
  isolation need (§2.2's Armory case), which is exactly why it needs an explicit signal instead of riding the
  same flag as everything else.

### 2.4 The credential-file path is baked into the account row at creation time

`compute_and_ensure_account_dir` (`identity_auth_dirs.rs:228-330`) resolves `identity_dir(bundle_id)` **once**,
at account-creation time, and the resulting absolute path is stored on the account row as
`SecretRef::OAuthConfigDir { dir }`. At spawn/relaunch time, `inject.rs:398,543-546` reads that **stored**
path — it never re-derives `identities_dir()` fresh. This means the credential-file location is a
creation-time decision baked into data, not a live resolution — consistent with the account row itself needing
a stable, permanent scope decided once, not something recomputed per-process.

### 2.5 Delete-account cleanup currently trusts the process-wide flag for its safety boundary

`cleanup_account_secrets`/`cleanup_oauth_dir` (`identity/cleanup.rs:68-182`) refuse to delete anything outside
a containment root — but that root is `DataPaths::identities_dir()` (`agent_handlers/identity.rs:425-426`),
resolved from the **process-wide** flag, not from the account's own scope. Under this design, a real global
account and a disposable test account created in the same process must resolve to *different* containment
roots per their own scope, or the safety guarantee this function exists for is lost.

## 3. Design

### 3.1 Two permanently-separate physical stores, chosen by explicit creation-time intent

Not a `scope` column filtered by `WHERE` — a **different SQLite file entirely**, so no future unscoped query
can ever cross the boundary by accident (a `scope` column is one missed `WHERE` clause away from exactly the
kind of bug this whole redesign exists to prevent; a separate connection cannot make that mistake).

**Store A — the "identity store," always global, no isolation flag consulted at all:**
`db_agent_identity_links`, `db_bundles`, `db_drone_definitions`, `db_muxbus_credentials`,
`db_agent_credentials`. Also holds `db_accounts` rows for the default (non-test) case — see §3.2. Resolved via
a new `resolve_identity_store_path()` in `agentmux-srv/src/registry/paths.rs`, modeled on
`resolve_shared_registry_dir()`/`resolve_shared_definitions_dir()` (both already correctly unconditional,
`paths.rs:8-39`) — **not** on `resolve_shared_store_path()` (`paths.rs:63-72`), which is the function whose
isolation-flip this design retires for these tables.

**Store B — the per-channel "disposable test accounts" store**, used *only* when an account is explicitly
created as a test account (§3.2). This can reuse the *existing* `<instance_dir>/identity-store.db` file/schema
almost as-is — it already has the right lifecycle (torn down with the channel) and the right schema
(`db_accounts` + `db_agent_identity_links` only, per `run_shared_store_schema`, since bundles/drones/muxbus
never legitimately belonged there either). What changes is *when* something gets written to it: no longer "any
account, if the process happens to be isolated," but "only an account explicitly flagged as disposable."

### 3.2 `disposable_test` flag on account creation, not ambient channel state

Add `disposable_test: bool` (default `false`, `#[serde(default)]`) to `CommandAuthStartData` and
`CommandAccountKeyVerifyData` (`agentmux-srv/src/backend/rpc_types/...`, wherever those are defined — the
`auth.start`/`account.key.verify` request shapes). The handler picks Store A or Store B based on this flag,
not `isolated_auth_enabled()`. Default `false` means every existing caller (nothing sets this field today)
gets Store A — the globally-persistent, continuity-preserving behavior — automatically, with no frontend
change required to *stop* the bug. Setting it `true` is an *opt-in*, not a default, so it can only happen from
a deliberate UI action (§3.5).

`compute_and_ensure_account_dir` gets the same `disposable_test` bool threaded through, and resolves the
credential directory via a new `identities_dir_for_scope(is_test: bool)` (replacing the unconditional
`isolated_auth_enabled()` check in `identities_dir()` for this call site specifically): `false` → always
`shared_dir.join("identities")` (matching `identity_history_dir`'s existing always-global pattern);
`true` → `instance_dir.join("identities")` (today's isolated behavior, now opt-in instead of ambient default).

### 3.3 Delete-account cleanup keys off the account's own recorded scope

`deleteidentityaccount` (`agent_handlers/identity.rs:399-512`) looks the account up (already does, at `:418`,
to capture provider) — extend that lookup to determine which store the row came from (try Store A first, then
Store B, or read a stored scope marker — implementation detail for the PR, not a design fork) and compute the
containment root for `cleanup_oauth_dir` from **that account's own scope**, not the process-wide
`DataPaths::identities_dir()`. This is what actually restores the safety guarantee per-account instead of
per-process.

### 3.4 Migration: backfill existing accounts to Store A, physically moving credential files

A new migration, modeled directly on `m0011_shared_store_backfill.rs`'s own algorithm (open destination,
compute skip flags for idempotent re-run, enumerate every reachable source `db_accounts`/link row across every
channel's existing per-channel store — reusing `registry::enumerate_objects_dbs`/`enumerate_sources`,
`registry/migrate.rs:369-385` — and upsert first-seen-wins). Additionally, for each backfilled account whose
stored `SecretRef::OAuthConfigDir { dir }` points into a per-channel path, physically move the credential
directory's contents into the Store-A-equivalent global `identities_dir` location (reusing
`ensure_history_link`'s "move real directory contents, never overwrite a name collision" logic,
`data_paths.rs:469-482`, as the template — though this is a plain move, not a junction/symlink, since
credential files should live in exactly one place once migrated, unlike history which stays reachable from
multiple isolated paths via a link) and rewrite the row's stored `dir` string to match. Every migrated account
defaults to non-test scope (`disposable_test = false`) — this migration only ever *creates* global,
continuity-preserving accounts; it has no way to know which pre-existing accounts were "meant" to be test
accounts (there was no signal for that, per §2.2), and defaulting to global is the safe choice since it's what
fixes the reported bug for every existing user.

The existing `m0011` isolation-boundary tests (`backfills_sibling_accounts_when_not_isolated`,
`skips_sibling_accounts_when_isolated`, `m0011_shared_store_backfill.rs:170-202`) get superseded by this new
migration's own tests, keyed off the new explicit scope instead of the old process-wide flag.

### 3.5 Armory UI: an explicit opt-in, not a default

The "connect account" flow (wherever `auth.start`/`account.key.verify` are called from the frontend — Armory's
account-connect surface) gets a new, off-by-default checkbox/toggle: *"Create as a disposable test account
(won't persist, and won't be findable by any of your real agents)"* — surfaced only in the Armory account list
context (not the ordinary agent-launch "connect an account" flow, where a disposable account would never make
sense to offer). Exact placement/copy is a UI-polish detail for the implementing PR, not re-litigated here;
the requirement is that it defaults OFF and is a deliberate, visible choice, not ambient state.

## 4. What does NOT change

- `identity_history_dir()`/`ensure_history_link()` (conversation transcripts) — already correct, untouched.
- `db_drone_runs` (ephemeral, correctly per-channel) — untouched.
- `provider_auth_dir()` (the default ambient login dir, unrelated to per-account isolation) — untouched.
- `resolve_shared_store_path()` itself is not deleted — it may still be relevant for other, unrelated future
  uses of "a store scoped by the isolation flag" — but nothing in the identity/bundle/drone/muxbus cluster
  consults it after this design ships.

## 5. Regression tests this design must ship with (P3 discipline, per the checklist)

1. A test asserting `db_agent_identity_links`/`db_bundles`/`db_drone_definitions`/`db_muxbus_credentials`
   resolve to the same location regardless of `AGENTMUX_CHANNEL`/`isolated_auth_enabled()` — the exact test
   class the audit (§2.3) found missing entirely.
2. A test asserting a `disposable_test: true` account and its credential directory land in the per-channel
   Store B, and a default/`false` account lands in the always-global Store A, regardless of which channel the
   process is running on.
3. A test asserting `deleteidentityaccount` never crosses stores — deleting a Store-B test account cannot
   touch Store A and vice versa (the actual safety property this whole redesign exists to preserve).
4. A migration test asserting a pre-existing per-channel-isolated account (the exact broken state live users
   are in today) gets backfilled into Store A with its credential files physically relocated and reachable at
   the new path.

## 6. Sequencing (separate, reviewable PRs — mirrors issue #2603's own 6-step pattern)

1. **Store A plumbing, links only** (`docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md`
   verification's tracking issue #2627, first PR — **shipped**): `run_identity_store_schema` /
   `Store::open_identity_store` / `resolve_identity_store_path` (all new — `db_agent_identity_links` has no
   `account_id` FK here, same cross-DB-FK reasoning `run_shared_store_schema` already uses for the
   `db_agent_definitions` FK). Every link-specific call site (`agent_identity_link`/`_unlink`/
   `_list_all`/`_list_for_agent`) rewired from `id_store` to `identity_store`, including the actual
   relaunch/spawn path (`inject_identity_env_with_broker` → `resolve_bindings_for_instance`) — this is the one
   that fixes the reported bug, not just the explicit link-management RPCs. `db_bundles`/
   `db_drone_definitions`/`db_muxbus_credentials`/`db_agent_credentials`/`db_agent_native_memory`/
   `db_cron_jobs` schema is included in Store A's DDL (so the store is ready for them) but call-site rewiring
   for those is deferred to fast-follow PRs — landing all ~7 tables' call sites in one PR risked an
   unreviewable diff; links alone directly fixes the reported continuity bug. Regression tests: `registry::paths`
   (path never varies by channel/isolation, diverges from `resolve_shared_store_path` under isolation) and
   `backend::storage::identities` (a link written by one process is visible to a later independent open at the
   same path — the literal close/reopen-across-a-version scenario; a link write doesn't require a matching
   `db_accounts` row in this store).
1b. **Bundles/drones/muxbus/native-memory/cron call-site rewiring** (not yet started) — same mechanical shape as
   1, using the schema/store/resolver already shipped. No design risk (§2.3's audit already confirmed these four
   have no legitimate isolation need); purely a call-site migration + a per-table regression test matching the
   ones in step 1.
2. **`disposable_test` flag + Store B split for accounts**: RPC field, handler routing, credential-dir scoping
   (§3.2). Regression test #2.
3. **Delete-account cleanup keyed off per-account scope** (§3.3). Regression test #3.
4. **Migration/backfill for pre-existing accounts** (§3.4). Regression test #4.
5. **Armory UI opt-in toggle** (§3.5).
6. Update `CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17.md` with this incident, per its own stated purpose
   ("if a future PR in this area breaks one of the items above, that's a signal this checklist itself needs
   updating").

Tracked in issue #2627.
