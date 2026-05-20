---
type: patch
---

refactor(storage): rename `db_identities` → `db_identity_bundles` and `db_memories` → `db_memory_bundles` (v11)

Closes AUDIT_SQLITE_SYSTEMS §8.1 — the schema-naming drift where v7 created
tables under the bare names but the rest of the codebase + UI vocabulary used
"identity bundle" / "memory bundle." The "bundle" suffix conveys that each row
carries multiple facets (provider bindings + display name for identities;
instructions + context_files + mcp_servers + skills for memories).

- New `run_forge_v11_migrations`: idempotent `ALTER TABLE … RENAME` for both
  tables, plus DROP+CREATE on the `is_blank` indexes. SQLite ≥ 3.25
  auto-updates the FK reference in `db_identity_bindings`, so the binding
  cascade-delete still works through the rename (covered by a new test).
- v7 now guards its legacy-name DDL/seed/shadow-migrate block on
  `db_identity_bundles` not yet existing — prevents re-creating empty old
  tables alongside the renamed ones on every subsequent startup.
- v1 (the base `db_forge_agents` CREATE) extracted into its own
  `run_forge_v1_migrations` so tests can stage a pre-v7 schema and exercise
  the legacy path independently.
- All `wstore.rs` queries + doc comments updated to the bundle names.
- `db_identity_bindings` is intentionally NOT renamed — its name was already
  bundle-consistent (a binding binds an identity bundle to a provider
  account; the surrounding object IS the binding, not the bundle).
