---
type: patch
---

refactor(storage): flatten objects.db schema, retire the "forge" vocabulary, add user_version tripwire

Implements `docs/specs/SPEC_SCHEMA_FLATTENING_2026_05_19.md`. Closes
AUDIT_SQLITE_SYSTEMS §8.5 (and absorbs the §8.1 / PR #933 v11 rename).

**Flatten.** The 11-step incremental migration chain (`run_forge_v1` …
`run_forge_v11`) is replaced by a single flat `run_object_schema` that defines
the final `objects.db` schema directly. Per-version data dirs mean every new
version was always born with a fresh DB and ran the whole chain in one shot
anyway — the intermediate states were never reachable. ~1,400 lines of
migration code + tests deleted.

**De-forge.** "Forge" is dead vocabulary (replaced by Memory / Identity /
agent-definition). Renamed Rust-side: `db_forge_agents` → `db_agent_definitions`,
`db_forge_content` → `db_agent_content`, `db_forge_skills` → `db_agent_skills`,
`db_forge_history` → `db_agent_history`, `db_forge_agent_identities` →
`db_agent_identity_links`; structs `ForgeAgent`/`ForgeContent`/… →
`AgentDefinition`/`AgentContent`/…; `forge_*` store methods → `agent_def_*` /
`agent_content_*` / `agent_skill_*` / `agent_history_*`; files
`forge_handlers.rs` → `agent_handlers.rs`, `forge_seed.rs` → `agent_seed.rs`.
The RPC wire command strings are intentionally unchanged (decision A1) — the
frontend is untouched and the wire contract is stable.

**Dead tables dropped.** `db_workflow_definitions` / `db_workflow_runs` and the
`db_v10_migrated_legacy_*` sentinels are no longer created; `adopt_legacy_table_names`
drops them from any pre-flatten dev DB — their data had already been copied
into `db_drone_*`.

**Safety net.** `adopt_legacy_table_names` runs once per startup: it renames
any pre-flatten table names found (the single surviving fragment of the old
chain — it also subsumes the v11 bundle rename) so a developer's pre-flatten
`objects.db` carries forward without data loss. SQLite ≥ 3.25 auto-updates FK
references when a parent table is renamed.

**user_version tripwire.** `stamp_and_check_version` stamps `PRAGMA user_version`
on all four SQLite files (`objects.db`, `filestore.db`, `sagas.db`,
`launcher-sagas.db`) and logs a loud warning if a file was written by a newer
build (downgrade detection — the bug class behind PR #933's Codex P1). A
tripwire, not a migration gate: idempotent DDL remains the schema mechanism.
