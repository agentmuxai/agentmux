# Spec: Completing Armory Phase 4 — the Storage Rename

**Date:** 2026-07-12
**Author:** AgentY
**Type:** Verified audit + implementation-ready completion plan. No code shipped yet.
**Purpose:** `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`'s Phase 4 — drop `db_identity_bundles`/`db_identity_bindings`, rename `db_identity_accounts`→`db_accounts` and `db_memory_bundles`→`db_bundles` — has been "gated on a backfill soak period" since 2026-07-10 with no concrete blocker list. This spec replaces that vague gate with a verified, file:line-level account of exactly what's blocking each table and what has to happen first. Tracked at issue #2024.

---

## 1. Verified current state (2026-07-12)

Confirmed directly against `agentmux-srv/src/backend/storage/migrations.rs` — none of Phase 4's renames/drops have happened. All four tables still exist under their original names.

| Table | Target | Classification |
|---|---|---|
| `db_identity_accounts` | rename → `db_accounts` | **Rename-safe** |
| `db_memory_bundles` | rename → `db_bundles` | **Rename-safe** |
| `db_identity_bundles` | drop | **Drop-blocked** — 3–4 live read dependents |
| `db_identity_bindings` | drop | **Drop-ready** — no independent live readers (child of the table above) |

This 2-vs-2 split matters: Phase 4 is not one uniform migration. Half of it is a low-risk mechanical rename that could ship almost immediately; the other half needs real cleanup work first.

---

## 2. `db_identity_accounts` → `db_accounts` — rename-safe

This is the live, correctly-shaped table backing the Armory's **Accounts** tab today, not a legacy leftover. All CRUD lives in `backend/storage/identities.rs:148-284`. Live RPC entry points: `identity.account.upsert`/`identity.self.unlink` (`app_api/identity.rs:28-203`, the Accounts-tab write path), `deleteidentityaccount` (`identity_handlers.rs:400-440`), `auth.start` in direct-account mode (`identity_handlers.rs:1345-1408` — confirmed the *only* reachable mode from the frontend: `auth-flow-controller.ts:37-46` hardcodes `directAccount: true`). Frontend consumers confirmed live: `identity-model.ts`, `AgentLaunchModal.tsx:171-188`, `agent-identities-panel.tsx` (the Armory Identities tab).

**What the rename actually touches:** the `CREATE TABLE` in both `objects.db` and shared `store.db` migration blocks (`migrations.rs:205-229`, `620-643`), the FK clause on `db_agent_identity_links.account_id` (both blocks), every query string in `identities.rs`, and the index names. No consumer needs to change — same shape, same semantics, new name.

---

## 3. `db_memory_bundles` → `db_bundles` — rename-safe

The live, current storage for the Bundle primitive exactly as `CLAUDE.md` already documents ("Backend names stay `db_memory_bundles`"). All CRUD in `backend/storage/memory_bundles.rs:95-216`. Live RPC entry points: `listmemories`/`getmemory`/`upsertmemory`/`deletememory`/`reorderglobalbrain` (`agent_handlers/memory.rs:208+`). Frontend consumers confirmed live and heavily used: `memory-model.ts` (Armory Bundles tab), `global-brain-model.ts` (Brain/global-tier), `AgentLaunchModal.tsx:189-197`, `AgentCreateFromTemplateModal.tsx:127`, `drone-view.tsx:829-830` (Drone node Memory picker — genuinely wired, unlike the Drone Identity picker in §4).

**No FK complications:** `db_agent_instances.memory_id` is a plain `TEXT` column with no FK constraint (`migrations.rs:281`) — an app-level reference only, no cascade behavior to preserve.

**Recommendation: ship §2 and §3 together as a standalone PR now.** Nothing blocks either rename; they're independent of the drop work in §4/§5 below, and there's no reason to keep holding them hostage to the harder half of Phase 4.

---

## 4. `db_identity_bundles` — drop-blocked, not by credential resolution

**The good news, verified directly:** `resolve_bindings_for_instance` (`identity/resolver.rs:450-510`, the actual spawn-time credential-resolution function) queries **only** `db_agent_identity_links` via `agent_identity_list_for_agent`. There is no fallback read of `db_identity_bundles`/`db_identity_bindings` — the function's own doc comment names this explicitly: a bundle-only binding with no direct link produces "nothing to inject," with a pointer to file it under #1624 PR-C if that's ever a real complaint. Test `inject_bundle_only_binding_no_longer_injects` (`resolver.rs:1355-1427`) pins this. **Credential resolution is fully migrated off this table already — that axis is not what's blocking the drop.**

**What's actually blocking it: three live, non-credential read dependents.**

1. **`listrecentsessions`** (`agent_handlers/session.rs:208,221-226`) — calls `bundle_identity_list()` and matches `instance.identity_id` against it to show a human-readable identity name in the recent-sessions/"Continue agent" list. Verified live (spot-checked directly).
2. **`listnamedagents`** (`identity_handlers.rs:551-556`) — same `bundle_identity_list()` pattern, for the named-agents picker.
3. **`IdentityPaneViewModel`'s constructor** (`identity-pane-model.ts:179,185`) — unconditionally calls `refreshBundles()` on every agent-pane Identity-tab open. The result is fetched but never rendered: `identity-pane-view.tsx:9-15` confirms the CRUD UI that used to consume it (`identity-manager.tsx`) was deleted as dead code, and the view now renders only the read-only `<BundleSummaryPanel/>`, which never touches this data. **Live but functionally vestigial** — safe to just delete the call, not migrate it to anything.
4. **`drone-view.tsx:826-847`** (`AgentRefEditor`) — still renders a live "Identity" `<select>` populated from `ListIdentityBundlesCommand`, persisting the choice into `node.data.agent_ref.identityId`. Verified separately: **the Drone runner never reads `identity_id` for credential injection at all** — a direct grep for `identity_id`/`inject_identity_env` across `agentmux-srv/src/drone/` returned zero hits. This picker is already non-functional for its stated purpose, independent of this migration. It's a live UI+read dependency (blocks the drop) but not a resolution dependency (doesn't block on correctness grounds — it's already broken).

**A data-shape wrinkle worth fixing regardless of drop timing:** `agent-model.ts:556-567` now populates new instances' `identity_id` with an **account_id**, not a bundle id (comment: "`accountId` replaces the old bundle-id `identityId`"). So dependents #1 and #2 above are already silently missing matches for every new-style instance today — the live read executes, but increasingly returns nothing useful even before the table is dropped. This is an existing, independent bug worth its own small fix.

---

## 5. `db_identity_bindings` — drop-ready

Verified: writes are unreachable (`bundle_identity_bind`/`bundle_identity_unbind` in `identities.rs:463-492` are only called from the same unreachable OAuth-bundle branch and the orphaned pane model already covered in §4). Reads are `identity_handlers.rs:1261` (inside that same unreachable branch), `listidentitybindings` RPC (`agent_handlers/memory.rs:200-206`, whose only frontend caller is the orphaned `identity-pane-model.ts:291`), and the `m0013`/`m0014` backfill migrations (by design — see §6). **No independent live dependents.** FK: `identity_id`→`db_identity_bundles(id)`, `account_id`→`db_identity_accounts(id)`, both `ON DELETE CASCADE` — drop this table alongside or immediately before its parent.

---

## 6. The backfill already ran, is idempotent, and can be re-run as a safety net

`m0013_agent_direct_bindings.rs` (PR-A's backfill) copies every non-sentinel instance's bundle bindings into `db_agent_identity_links`. `m0014_agent_direct_bindings_rerun.rs` covers the write-through gap window, restricted to each definition's latest instance, deliberately non-destructive. Both are `MigrationScope::Global`, registered in `migrations/mod.rs:121-122`, **already applied** (tracked once-per-install in `db_migrations`), and both have dedicated idempotency tests (`m0013.rs:357-374`, `m0014.rs:312-328`) backed by `agent_identity_link`'s `ON CONFLICT(agent_id, provider) DO UPDATE`.

Because the migration framework skips already-applied ids, re-running the same logic as a pre-drop safety net means wrapping the same pure backfill functions in a new id (e.g. `m0015`) — cheap, and removes any doubt about whether every legacy binding made it across before the drop.

---

## 7. Recommended phased plan

**Phase 4a — ship now, independently.** The two rename-safe tables (§2, §3). Zero consumer changes needed, no drop-safety analysis required. Standalone PR.

**Phase 4b — clear the three drop-blockers on `db_identity_bundles`.**
1. Delete `IdentityPaneViewModel`'s vestigial `refreshBundles()` call (§4 item 3) — pure deletion, nothing depends on its result.
2. Fix `listrecentsessions`/`listnamedagents` (§4 items 1–2) to resolve identity display names via `db_agent_identity_links`/`db_identity_accounts` instead of `bundle_identity_list()` — this also fixes the data-shape wrinkle from §4 (new-style instances currently resolve to nothing).
3. Remove or repoint the Drone Identity picker (§4 item 4) — since it's already disconnected from real credential injection, removing it is arguably a UX fix in its own right (stop offering a control that silently does nothing), not just a migration prerequisite. Worth a product call: remove entirely, or actually wire it to `db_agent_identity_links` and make it real.

**Phase 4c — run the safety-net backfill (§6) as `m0015`, then drop both tables** (`db_identity_bindings` first or alongside `db_identity_bundles`, per the FK direction in §5) in one migration.

**Sequencing note:** 4a has no dependency on 4b/4c and should not wait for them. 4b's three items are independent of each other and could land as separate small PRs. 4c is the only piece that must come last.

## 8. Open questions — resolved 2026-07-12

1. **Phase 4b item 3 (Drone Identity picker) — remove or wire up?** Resolved: **remove**. It's already non-functional for credential injection (§4 item 4); keeping a control that silently does nothing is worse than not offering it. Actually wiring it up would be new scope, not a migration prerequisite.
2. **Does anything outside this repo reference `db_identity_bundles`/`db_identity_bindings` directly?** Checked — no external exports, integrations, or saved references found. Clear to proceed with the eventual drop in Phase 4c.
3. **Should the `m0015` safety-net backfill be unconditional or gated?** Resolved: **unconditional**. Both `m0013`/`m0014` are proven idempotent and cheap; no meaningful cost to just always re-running them before the Phase 4c drop.

## 8a. Phase 4a implementation note — dual schema locations (discovered 2026-07-12)

The original classification in §2/§3 undersold the mechanical scope: `objects.db` and the shared `~/.agentmux/shared/store.db` each define their **own independent** `CREATE TABLE` block for `db_identity_accounts`/`db_memory_bundles` (`run_object_schema` vs `run_shared_store_schema` in `migrations.rs`), and `id_store` routing (`main.rs`) means identity/memory RPCs can land on either depending on whether `0011_shared_store_backfill` has run. Both blocks needed the rename, both needed their own `idx_ss_*`-prefixed legacy index drops, and — critically — `run_shared_store_schema` did not previously call `adopt_legacy_table_names` at all, so it needed that call added before the rename could take effect there.

A second gotcha: the additive-column `ALTER TABLE db_memory_bundles ADD COLUMN …` statements in `run_object_schema` run *after* `adopt_legacy_table_names` in the same pass, so once the rename entry was added they had to be retargeted to `db_bundles` — otherwise every DB going through the rename would hit "no such table: db_memory_bundles" on the very next startup.

Landed in Phase 4a: `LEGACY_TABLE_RENAMES` + `LEGACY_INDEX_DROPS` entries for both tables, both `CREATE TABLE` blocks (objects.db + shared store.db) renamed with FK updates, `OBJECT_SCHEMA_VERSION` 10→11 and `SHARED_STORE_SCHEMA_VERSION` 2→3, all query strings in `identities.rs`/`memory_bundles.rs` retargeted, and doc-comment references updated across 9 other Rust files + 4 frontend files + `CLAUDE.md`. Verified via `cargo test -p agentmux-srv` (1476 unit + 4 integration + 7 subprocess tests, all passing) and `tsc --noEmit`.

## 9. References

- `agentmux-srv/src/backend/storage/migrations.rs` — current schema, all 4 tables' `CREATE TABLE` blocks (lines cited per-table above).
- `agentmux-srv/src/identity/resolver.rs:450-510` — `resolve_bindings_for_instance`, the verified sole credential-resolution path (direct links only).
- `agentmux-srv/src/migrations/m0013_agent_direct_bindings.rs`, `m0014_agent_direct_bindings_rerun.rs` — the already-applied, idempotent backfill.
- `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` — originating spec, Phase 4 definition.
- `docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md` — the Phase 3 work this Phase 4 builds on.
- Issue #2024 — consolidated tracking for the whole Armory/Identity family.
