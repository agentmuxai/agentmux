# Spec: Modularize `agent_session.rs`

**Date:** 2026-07-02
**File:** `agentmux-srv/src/backend/agent_session.rs` (2,801 lines)
**Type:** Pure reorganization — zero logic changes, zero public API changes
**Tier:** Large (follow-on to the critical-tier modularization run, PRs #1880–#1897)

---

## Current state

- **2,801 lines total:** ~1,477 impl + ~1,322 inline `#[cfg(test)] mod tests`
- **No `impl` blocks / no trait impls** — all functions are module-level (simplifies the split)
- Callers across the crate use fully-qualified `crate::backend::agent_session::*` paths, so a `mod.rs` with `pub use` re-exports preserves every call site unchanged

## Public API surface (must remain re-exported from `mod.rs`)

Consumed by: `shell.rs`, `persistent.rs`, `transcript_backfill.rs`, `server/…/session.rs` (RPC), `instance.rs`, `app_api/mod.rs`, `main.rs`, `migrations/m0002_block_zones_v1.rs`, `migrations/m0003_template_sessions_v1.rs`.

Constants: `SNAPSHOT_FILE`, `OUTPUT_FILE`, `MIGRATION_MARKER_V1`, `TEMPLATE_PROMOTE_MARKER_V1`
Zone naming: `is_valid_definition_id`, `agent_current_zone`, `agent_archive_zone`, `validate_and_current`
Global store: `set_global_transcript_store`, `global_transcript_store`, `agent_zone_for_block_meta`
Session IO: `write_session_state`, `read_session_state`, `append_session_output`, `read_snapshot_from`, `normalize_snapshot_for_global`, `heal_global_snapshot_source_block_ids`
Archive: `archive_session`, `archive_global_current`, `clear_local_current_zone`, `clear_global_current_zone`, `list_archives`, `read_archive_preview`, `ArchiveSummary`
Migrations: `migrate_block_zones_v1`, `MigrationStats`, `migrate_promote_template_sessions_v1`, `TemplatePromoteStats`

Private (keep module-private): `now_ms`, `move_zone`, `CopyAction`, `ensure_file`, `write_zone_file`, `read_snapshot_bytes`, `collapse_preview`.

## Proposed layout

```
src/backend/agent_session/
├── mod.rs               (~40 lines: pub use re-exports + module decls)
├── zone_naming.rs       (~130: validation + zone-name builders)
├── global_store.rs      (~50: cross-channel transcript singleton + block-meta resolution)
├── session_io.rs        (~270: read/write/append snapshot state + normalization + heal)
├── archive.rs           (~380: archive lifecycle, browsing, previews, ArchiveSummary)
├── helpers.rs           (~180: FileStore I/O wrappers)
├── migrations/
│   ├── mod.rs           (re-export both migrations + marker consts)
│   ├── v1_blocks.rs     (~180: migrate_block_zones_v1, MigrationStats)
│   └── v1_templates.rs  (~480: migrate_promote_template_sessions_v1, TemplatePromoteStats, move_zone, CopyAction)
└── tests.rs             (~1,322: the existing #[cfg(test)] mod tests, moved via `#[cfg(test)] mod tests;`)
```

`tests.rs` uses `use super::*;` (child of `agent_session`) so it resolves exactly as the inline module did — same pattern proven in store.rs (#1887).

## Execution notes

- mod.rs `pub use self::<submod>::{...}` for every public item listed above — verify no external file's import breaks.
- Each submodule declares its own `use` imports; do NOT add `#![allow(unused_imports)]` (reagent flagged this in #1880 — trim via `cargo check` output).
- The test module references migration internals via `use super::*` — since tests move to `agent_session/tests.rs` (a child of the parent `agent_session` module, NOT of the `migrations` submodule), any migration symbols the tests touch must be reachable from `agent_session::` — re-export them (they already are, per the public list) or add `pub(crate)` where a test uses a currently-private helper. Run `cargo test --lib backend::agent_session` (or `cargo test agent_session`) and fix breakages before pushing.

## Verification gate

- `cargo check` + `cargo check --tests` clean, zero new warnings
- `cargo test agent_session` — all tests pass (was passing before)
- reagent review; pure-reorg, so expect only import/comment nits

## Risk: **Low.** No trait impls, no platform cfg, callers use qualified paths. Main risk is test-module symbol visibility after the move — covered above.
