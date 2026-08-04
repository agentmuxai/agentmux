# Post-Work Status — Schema Flatten + De-Forge (PR #934)

**Date:** 2026-05-19
**Author:** AgentA
**Branch:** `agenta/schema-flatten-deforge`
**PR:** https://github.com/agentmuxai/agentmux/pull/934
**Spec:** [`SPEC_SCHEMA_FLATTENING_2026_05_19.md`](../SPEC_SCHEMA_FLATTENING_2026_05_19.md)
**Status:** ✅ Merge-ready — both bot reviewers approve the latest commit; backend smoke verified.

---

## 1. Outcome

`objects.db`'s 11-step incremental migration chain (`run_forge_v1` … `run_forge_v11`)
is replaced by a single flat `run_object_schema`, the dead "forge" vocabulary is
retired from the Rust storage layer, and a `PRAGMA user_version` downgrade
tripwire is added to all four SQLite files. Closes AUDIT_SQLITE_SYSTEMS §8.5;
absorbs §8.1 / PR #933.

| | |
|---|---|
| Commits | 3 (`4a9c21d9`, `5fa23c69`, `2d3bf916`) |
| Diff | 22 files, +1,573 / −2,289 — net ~−1,360 lines of code (the SPEC adds 647 doc lines) |
| Build | `agentmux-srv` + `agentmux-launcher` clean |
| Reviews | reagent **APPROVED** · codex **clean** — both on HEAD `2d3bf916` |
| Mergeable | `clean` |

---

## 2. What shipped

- **Flatten** — `run_object_schema` defines the final schema directly in one
  idempotent `CREATE TABLE IF NOT EXISTS` batch. v1–v11, the v6→v7
  shadow-migrate, and the v9→v10 workflow→drone copy are deleted.
- **De-forge (Rust side)** — `db_forge_*` tables → `db_agent_*`; structs
  `ForgeAgent`/… → `AgentDefinition`/…; `forge_*` store methods →
  `agent_def_*`/`agent_content_*`/`agent_skill_*`/`agent_history_*`; files
  `forge_handlers.rs` → `agent_handlers.rs`, `forge_seed.rs` → `agent_seed.rs`.
- **Dead tables dropped** — `db_workflow_*` + `db_v10_migrated_legacy_*` are no
  longer created; `adopt_legacy_table_names` drops them from any pre-flatten
  dev DB (their data was already in `db_drone_*`).
- **`adopt_legacy_table_names`** — the single surviving rename step; carries a
  developer's pre-flatten `objects.db` forward. Non-destructive on the
  both-tables-present case (warns, never drops).
- **`user_version` tripwire** — `stamp_and_check_version` on `objects.db`,
  `filestore.db`, `sagas.db`, `launcher-sagas.db`; warns loudly on downgrade.

---

## 3. Final review checklist

- [x] Both bot reviewers green on the latest commit (`2d3bf916`).
- [x] `cargo build` clean for both crates.
- [x] Targeted test suites green (see §5).
- [x] Backend schema smoke-verified on a fresh portable (see §6).
- [x] Wire contract unchanged — RPC command strings + serde field names
      untouched (decision A1); frontend not modified.
- [x] Cross-version registry migration unaffected — `registry/migrate.rs`
      only reads `db_agent_instances` (name unchanged) with graceful
      missing-table handling.
- [x] FK cascades survive the table rename (SQLite ≥ 3.25 auto-update;
      covered by test).
- [x] Behaviour-preserving — no schema column lost (the `accounts` mistake
      was caught and reverted, see §7).

---

## 4. Review evidence

Two rounds:

1. **Reagent — 2× P2** (`5fa23c69`): dead `accounts`-handling code in
   `agent_handlers.rs` + `agent_seed.rs`. *(Superseded — see §7.)*
2. **Codex — 1× P1** (`2d3bf916`): dropping the `accounts` column would
   silently wipe every user's per-agent provider→account assignments. A real
   regression — fixed by reverting the column drop.

Final: reagent **APPROVED** (`2d3bf916`), codex **"Didn't find any major
issues"** (`2d3bf916`).

---

## 5. Test evidence

All targeted module suites pass against the renamed surface:

```
storage 94 · launcher 183 · identity 39 · registry 39
drone 47 · agents 29 · reducer 93 · server 27
```

New migration tests: flat-schema creation / idempotency / dead-table
omission, adopt rename + fresh no-op + both-present non-destructive + FK
cascade, `user_version` stamp.

> The unfiltered `cargo test -p agentmux-srv` hangs on a **pre-existing,
> unrelated** slow test (same behaviour before this branch). Targeted runs
> cover every surface this PR touches.

---

## 6. Smoke evidence (fresh v0.37.1 portable, branch build)

Verified directly against the live `objects.db`:

- App launches; backend ready; version-isolated data dir
  `~/.agentmux/versions/0.37.1/`.
- All 4 DB files created: `objects.db`, `filestore.db`, `sagas.db`,
  `launcher-sagas.db`.
- `objects.db` = 19 flat de-forged tables. **No `db_forge_*`, no
  `db_workflow_*`, no sentinels.**
- `PRAGMA user_version` = 1 on all four files.
- `db_agent_definitions.accounts` column present (the Codex-P1 path).
- Blank Identity + Memory singletons seeded.

**Pending (optional, user-driven):** interactive UI round-trip — create an
agent, assign an account, create Identity + Memory bundles, relaunch, confirm
persistence. The DB layer those writes go through is verified; this is an
eyeball check only.

---

## 7. The `accounts` correction (worth recording)

The early plan (spec decision D-D) was to drop the `accounts` column as dead
weight, trusting a v6 doc comment that called it "deprecated, superseded by
the identity-links junction." **That was wrong.** Codex review proved the
column is live: the Agent pane's Identity tab (`AgentIdentityPanel`) writes
per-provider account assignments into it as a JSON blob via `updateforgeagent`,
`parseAgentAccounts` reads it back, and startup credential resolution depends
on it. The v6 "deprecation" never completed.

The column drop was reverted in full (`2d3bf916`); a flatten must be
behaviour-preserving. Lesson: a stale "deprecated" comment is not evidence a
column is unused — verify against live readers.

---

## 8. Decisions & follow-ups

**Resolved**
- **A1** — RPC wire command strings (`COMMAND_*_FORGE_*` = `"listforgeagents"`
  etc.) kept; frontend untouched; wire stable.
- **D-D** — `accounts` column **kept** (see §7).
- **D-E** — `identity_id` / `memory_id` left as `''`-sentinel `TEXT` columns,
  not promoted to real FKs (current code depends on the sentinel).

**Open follow-ups (not blocking this PR)**
- **A2** — coordinated srv + frontend PR to rename the RPC wire command
  strings and the frontend `forge` view (`frontend/app/view/forge/`,
  `ForgeAgent` TS type). After A2 the word "forge" is fully gone.
- **Two account-assignment mechanisms coexist** — the `accounts` JSON blob
  *and* the `db_agent_identity_links` junction. The v6 migration that was
  meant to unify them never finished. A future decision should pick one
  canonical store.
- **Minor leftover** — `default_forge_icon()` in `rpc_types.rs` keeps its
  lowercase `forge` name (cosmetic; harmless; sweep with A2).
- **AUDIT §8.6** — FileStore PRAGMA documentation, still open from the
  original audit.

---

## 9. Recommendation

Merge PR #934. Both reviewers approve the latest commit, the backend schema
is smoke-verified on a fresh build, and the change is behaviour-preserving.
The remaining UI round-trip is an optional eyeball check, not a blocker.
