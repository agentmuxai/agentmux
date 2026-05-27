# SPEC: `wstore` → `store` rename + modularization

**Date:** 2026-05-27
**Author:** AgentA
**Status:** Design — multi-PR refactor proposal. No tracking discussion yet.
**Related:** `docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md` (Phase 3b reads from this same store; some methods retire when Phase 3c lands).

---

## Why this exists

`agentmux-srv/src/backend/storage/store.rs` is **5,530 lines** in a single file. It holds:

- The `WaveStore` struct + connection management + migrations.
- Generic `WaveObj`-based CRUD (`get`, `insert`, `update`, `delete`, …).
- A transaction handle (`StoreTx`).
- **All** agent-system reads & writes: definitions, instances, content, skills, history.
- **All** identity-system reads & writes: identity bundles, identity accounts, identity bindings, per-agent identity links.
- Memory bundles.
- Phase 3a dual-write mirrors (~400 LOC of helpers that mirror legacy writes into `db_agents`).
- Registry JSON mirror calls (`registry_upsert_if_named`, etc).
- ~1,200 LOC of inline tests at the bottom (under `mod tests {}`).

Two problems:

1. **The `w` prefix is dead.** It's a relic of the AgentMux fork from the [Wave Terminal](https://github.com/wavetermdev/waveterm) codebase. We're no longer Wave, no longer pronouncing it, and the `w` doesn't disambiguate anything anymore. Every reference to `wstore`, `WaveStore`, `WaveObj` carries dead branding cost.
2. **The single file is a contributor tax.** 5,530 lines means:
   - PRs touching one subsystem (agents, identities, memory) collide on git history.
   - `cargo check`'s incremental compile recompiles the whole TU on any change.
   - Code review struggles — reviewers can't easily verify "this PR only touches identities" without scanning the whole file.
   - Onboarding requires a 5kLOC slog before understanding the surface.

The fix: rename + split. **One PR for rename** (mechanical, low risk). **Six follow-up PRs** to extract subsystems one at a time, each ~200–1,500 LOC moved.

---

## End state

```
agentmux-srv/src/backend/storage/
├── mod.rs                  (re-exports — unchanged from today's interface)
├── store.rs                (renamed from wstore.rs; ~2,000 LOC after splits)
│                           │ Holds: `Store` struct, connection mgmt,
│                           │ migrations setup, generic WaveObj-based CRUD,
│                           │ `StoreTx`, helper traits.
├── agents.rs               (~1,500 LOC) — `impl Store { agent_def_*, instance_* }`
├── identities.rs           (~600 LOC) — `impl Store { bundle_identity_*, agent_identity_* }`
├── memory_bundles.rs       (~300 LOC) — `impl Store { bundle_memory_* }`
├── content.rs              (~250 LOC) — `impl Store { agent_content_* }`
├── skills.rs               (~200 LOC) — `impl Store { agent_skill_* }`
├── history.rs              (~250 LOC) — `impl Store { agent_history_* }`
├── dual_write.rs           (~400 LOC) — Phase 3a `db_agents` mirror helpers
├── registry_mirror.rs      (~200 LOC) — JSON registry write hooks (retires in Phase R)
├── agents_consolidate.rs   (unchanged — one-shot consolidation migration)
└── migrations/             (unchanged)
    ├── mod.rs
    ├── identity_migration.rs
    └── …
```

Rust supports `impl T {}` blocks split across files; each subsystem file is just a sibling module that adds methods to `Store`. Public API at the call-site level: unchanged.

Naming:
- `WaveStore` → `Store`.
- `WaveObj` (the trait that flags persistable types) → `StorableObj` (or keep `WaveObj` if the rename causes too much churn — see "Open questions").
- `StoreError` already drops the `W` prefix; unchanged.
- `StoreTx` already drops the `W` prefix; unchanged.

---

## Today's `wstore.rs` — method clusters

Counted via `grep -nE '^    pub fn '`:

| Prefix | Count | Subsystem |
|---|---|---|
| `agent_def_*` | 8 | Agent definitions (templates + user-clones) |
| `bundle_identity_*` | 7 | Identity bundles |
| `agent_skill_*` | 5 | Per-agent skills |
| `bundle_memory_*` | 4 | Memory bundles |
| `agent_content_*` | 4 | Per-agent content blobs |
| `instance_*` | 6 | Agent instances (get/list/create/update/delete + named variants) |
| `agent_identity_*` | 3 | Agent ↔ identity links |
| `agent_history_*` | 3 | Per-agent history |
| `agents_dual_write_*` | ~9 | Phase 3a mirror helpers (in private impl block) |
| `registry_*` (private) | ~4 | Registry mirror hooks |
| Generic CRUD (`get`, `insert`, `update`, `delete`, `get_all`, `count`, `with_tx`, `get_raw`, `update_raw`, `delete_by_otype`, `exists_raw`, `must_get`) | ~12 | `WaveObj`-based generic interface |
| Connection mgmt (`open`, `open_in_memory`, `configure_and_migrate`, `set_registry`, `registry`, `shared_agent_registry`) | 6 | Bootstrap |

---

## Phase plan

### R.0 — Rename only (1 PR)

**Goal:** `wstore.rs` → `store.rs`, `WaveStore` → `Store`. No code moves; only renames.

**Steps:**
1. `git mv agentmux-srv/src/backend/storage/wstore.rs agentmux-srv/src/backend/storage/store.rs`.
2. Update `agentmux-srv/src/backend/storage/mod.rs` to declare `mod store` instead of `mod wstore` (and re-export accordingly).
3. Sed across the entire codebase:
   - `WaveStore` → `Store`
   - `wstore::` → `store::`
   - `wstore.` (as method receiver) → `store.` (only inside doc comments / strings — Rust identifiers are different scope)
   - Variable names like `wstore` → `store` (catch any `let wstore = ...` patterns; verify carefully with `cargo check`)
4. Re-export `Store as WaveStore` in `mod.rs` as a deprecated alias for one release cycle (lets downstream branches catch up).
5. Decide: rename `WaveObj` trait too? If yes, alias as deprecated.

**Risk:** Low — mechanical search/replace. `cargo check` catches every miss. Major risk is conflict with in-flight PRs that also touch `wstore.rs`; mitigate by landing during a quiet window (e.g. right before a release cut).

**Acceptance:**
- [ ] `cargo build --release` clean.
- [ ] `cargo test -p agentmux-srv --release` all pass.
- [ ] No remaining `WaveStore` references except the deprecated alias.
- [ ] `git grep -i wstore` returns only doc-file references (which become a follow-up doc-refresh PR).

**Scope estimate:** ~50 files touched (mostly imports + variable names), ~200 lines changed net. The actual `wstore.rs` → `store.rs` rename is `git mv` plus a `mod` decl change.

### R.1 — Extract `agents.rs` (1 PR)

**Goal:** Move all `agent_def_*` + `instance_*` methods + the `AgentDefinition` / `AgentInstance` structs to `store/agents.rs` (or sibling file).

**Mechanism:**
- Create `agents.rs` next to `store.rs` (whichever layout we pick).
- Add `pub mod agents;` in `storage/mod.rs`.
- Cut the relevant `impl Store {}` block + struct definitions; paste into the new file.
- Update any `use` imports inside `store.rs` that referenced the moved structs.

**Lines moved:** ~1,500 (definitions + instances are the bulk of the file).

**Risk:** Medium — some private helpers might be reachable from multiple subsystems. Strategy: when in doubt, leave the helper in `store.rs` as `pub(super)` and let it be referenced via `super::helper_name()`. Move ownership only when it's clearly subsystem-local.

**Acceptance:** all tests pass; nothing else changes.

### R.2 — Extract `identities.rs` (1 PR)

`bundle_identity_*` + `agent_identity_*` + `IdentityAccount` + `AgentIdentityLink` + `Identity` + `IdentityBinding` structs.

**Lines moved:** ~600.

### R.3 — Extract `memory_bundles.rs` (1 PR)

`bundle_memory_*` + the memory-bundle struct.

**Lines moved:** ~300.

### R.4 — Extract `content.rs`, `skills.rs`, `history.rs` (1 PR or 3 PRs)

`agent_content_*` + `agent_skill_*` + `agent_history_*` plus their structs. Small enough to fit in one PR if the carving is clean; split into three if review prefers atomic moves.

**Lines moved:** ~700 combined.

### R.5 — Extract `dual_write.rs` (1 PR)

Phase 3a `db_agents` mirror helpers (currently in a private `impl Store {}` block in `wstore.rs` around lines 2372–2881).

**Lines moved:** ~400.

**Note:** When Phase 3c lands (drop old tables per `SPEC_AGENT_ARCHITECTURE_2026_05_27.md`), this file is **deleted**. The split should keep that future deletion clean — no other subsystem code should creep in.

### R.6 — Extract `registry_mirror.rs` (1 PR)

`registry_upsert_if_named` and friends. Same future-deletion note: this file retires in Phase R (registry sunset).

**Lines moved:** ~200.

### R.7 — Audit + cleanup (1 PR)

After all extractions:
- `store.rs` should be ~2,000 LOC of generic CRUD + connection + transactions.
- Each subsystem file should be self-contained except for shared helpers in `store.rs`.
- Move any leftover dead code, consolidate `use` blocks, run `cargo clippy` and address suggestions on the smaller files.

---

## Cross-cutting concerns

### Tests

Today's `wstore.rs` has ~1,200 LOC of tests under `mod tests {}` at the bottom. Most tests cluster by subsystem (`instance_list_named_*`, `bundle_identity_*`, etc.). When a subsystem extracts to its own file, its tests move with it.

The `mod tests {}` per subsystem file pattern is idiomatic Rust and gives:
- Faster targeted runs (`cargo test -p agentmux-srv --release agents::tests::`).
- Closer code-to-test colocation.

### Migrations module

`agents_consolidate.rs` and friends already live as sibling files. They reference `Store::open` etc. and will continue to work unchanged — just update imports to match new paths.

### Public re-exports

`agentmux-srv/src/backend/storage/mod.rs` is the boundary. After the rename, it should export:

```rust
pub use store::{Store, StoreError, StoreTx};
pub use agents::{AgentDefinition, AgentInstance, /* … */};
pub use identities::{IdentityAccount, AgentIdentityLink, /* … */};
// etc.
```

Code outside `storage/` keeps using `crate::backend::storage::Store` exactly as it does today (modulo the `WaveStore` → `Store` rename). The internal split is invisible.

---

## Sequencing — relative to the agent-architecture migration

The agent-data-model migration (`SPEC_AGENT_ARCHITECTURE_2026_05_27.md`) is in flight in parallel. Two phases of that migration **delete** subsystems this spec would extract:

- Phase 3c deletes the dual-write helpers (would-be R.5).
- Phase R deletes the registry mirror (would-be R.6).

Extracting either subsystem before the migration retires it = wasted scaffolding. So R.5 and R.6 are **skipped entirely**; the migration deletes those code blocks directly from the file.

### Locked order

1. **R.0 — rename only.** Land now. Mechanical, low risk. Establishes the clean name (`Store`) so every subsequent migration PR uses it from day one instead of perpetuating the `w` relic. Bundles this spec with the rename code (no-doc-only-PRs rule).
2. **Agent migration: Phase 3b sub-PRs** — read-flips land in the single file (now `store.rs`). Mild review overhead reviewing against the monolith, but cheaper than building R.1 only to immediately revise it during the migration.
3. **Agent migration: Phase 3c** — drops legacy tables, deletes the dual-write block from `store.rs`.
4. **Agent migration: Phase R** — sunsets the JSON registry, deletes the registry-mirror block from `store.rs`.
5. **R.1, R.2, R.3, R.4, R.7** — extract the surviving subsystems (`agents.rs`, `identities.rs`, `memory_bundles.rs`, `content.rs`/`skills.rs`/`history.rs`, then cleanup) against a leaner ~4,400-LOC file. **5 PRs instead of 7.**

This minimizes total work: skips ~600 LOC of pointless extract-then-delete churn and lets the modularization happen on a simpler post-migration codebase.

Pace within each PR: structural-only moves + full test suite. Don't bundle behavior changes with structural moves.

---

## Open questions

- **Rename `WaveObj` trait?** The trait is the marker that says "this type is persistable by the generic CRUD." Renaming it (`WaveObj` → `Storable` or `StoreEntity`) touches ~30+ derive-impl sites across the codebase. Could be batched with R.0 or deferred to a follow-up PR. **Preference:** include in R.0 so we do all renames at once.
- **Keep the `Store as WaveStore` deprecated alias?** Pros: in-flight branches keep compiling. Cons: noise. **Preference:** keep for one release cycle, then delete in a "post-rename cleanup" PR.
- **Sibling files vs `store/` subdir?** The proposed layout puts subsystem files as siblings of `store.rs`. Alternative: `store/mod.rs` + `store/agents.rs`. The sibling layout reads better (`storage/agents.rs` is more discoverable than `storage/store/agents.rs`); the subdir version groups things more visibly. **Preference:** siblings (flatter).
- **Should `dual_write.rs` and `registry_mirror.rs` be a single `legacy_mirror.rs`?** They both retire at the end of the agent-architecture migration. Combining them puts the deletion in one PR. **Preference:** keep separate — the registry mirror retires in Phase R; the dual-write mirror retires in Phase 3c. Different timelines, different reviewers care about each.

---

## Acceptance criteria for "modularization complete"

- [ ] `wstore` and `WaveStore` removed from the codebase (only `Store` remains).
- [ ] `agentmux-srv/src/backend/storage/store.rs` is ≤ 2,500 LOC (the generic CRUD + connection scaffold).
- [ ] Each subsystem file is ≤ 1,500 LOC.
- [ ] All tests pass at each PR boundary.
- [ ] No call sites outside `storage/` had to change (besides the `WaveStore` → `Store` rename).
- [ ] `cargo clippy` passes on the new layout.

---

## References

- `agentmux-srv/src/backend/storage/store.rs` — current target file (5,530 lines).
- `docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md` — the Phase 3b work that lands easier once `agents.rs` is its own file.
- Wave-prefix relic context: the `w` prefix dates to the original Wave Terminal fork; AgentMux replaced the host, the launcher, the UI shell, and most of the backend. The `w` is now noise.
