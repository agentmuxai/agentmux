# SPEC: `updateagentinstance` partial-update refactor (2026-05-29)

**Author:** AgentA
**Status:** Scope / plan — ready to implement.
**Why:** Single highest-leverage unblock for Phase 3b. It removes the `instance_get` fetch-and-merge in the update path, which is the *only* production code that needs transient per-launch fields out of `instance_get` — clearing the blocker on **3b.3c** (`instance_get` → db_agents) and, bundled, **3b.3b** (`instance_list` status-filter).
**Refs:** `docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md` (Phase 3b status table); `project_agent_migration_2026_05_28` memory.

---

## 1. Current shape (the problem)

`updateagentinstance` handler (`agent_handlers.rs:1396`) does **fetch-and-merge**:

```rust
let existing = wstore.instance_get(&cmd.id)?...;        // FULL row incl. transient fields
let merged = AgentInstance {
    block_id:  cmd.block_id.unwrap_or(existing.block_id),
    session_id: cmd.session_id.unwrap_or(existing.session_id),
    status:    cmd.status.unwrap_or(existing.status),
    github_context: cmd.github_context.unwrap_or(existing.github_context),
    ended_at:  cmd.ended_at.unwrap_or(existing.ended_at),
    ...all other fields copied from `existing`...
};
wstore.instance_update(&merged)?;                       // writes back the whole struct
Ok(Some(serde_json::to_value(&merged)))                 // returns merged to frontend
```

`instance_update` (`agents.rs:1256`) is **already a partial SQL UPDATE** — it only writes 5 columns (`block_id, session_id, status, github_context, ended_at`). The struct's other fields are ignored. Then it re-reads via `instance_get` to refresh the registry mirror + Phase-3a dual-write.

So the only reason the handler reads the full row is to **fill the 4 non-provided of those 5 fields** before handing a whole `AgentInstance` to `instance_update`. That read is what pins `instance_get` to the legacy `db_agent_instances` table (transient `block_id`/`session_id`/`status`/`started_at`/`ended_at` have no home on `db_agents`).

## 2. Findings that make this safe (verified 2026-05-29)

- **`instance_get` has exactly one production caller that needs transient fields: the update handler.** The only other production call is `instance_update`'s own internal post-write reload — and its consumers (`registry_upsert_if_named`, `agents_dual_write_instance_update`) read **only non-transient fields**: `instance_name`, `parent_instance_id`, `definition_id`, `id`, `github_context`. The dual-write code comments confirm `block_id`/`session_id`/`status`/`ended_at` are *not* modelled on `db_agents`. Everything else calling `instance_get` is a test.
- **No live caller passes a status filter to `instance_list`.** Only store-tests call `instance_list(_, Some(status))`. The `instance_list_legacy` fallback exists solely to serve that test-only branch. So **3b.3b is largely a delete**, not a migration.
- **`status` (and block/session/ended_at) remain transient** — they still live in `db_agent_instances` and are written by `instance_update`. Phase 3c retires that table; until then transient state stays there. This refactor does *not* move transient writes.

## 3. Open decision (must resolve before coding)

**What does the handler return?** Today it returns the `merged` `AgentInstance` (full struct, incl. transient). With a partial update we no longer build `merged`. Options:

| Option | Behavior | Cost |
|---|---|---|
| **A. Return the partial echo** | Return `{ id, ...the fields that were set }` (or just `{ id }`). | Frontend must not depend on a full instance back. **Verify consumers first.** |
| **B. Re-read + return full** | After the partial write, `instance_get` the row and return it. | Keeps the response shape, but re-introduces an `instance_get` call — which is *fine* as long as it's a read-only response that doesn't pin the migration (the response can read legacy until 3c; or read db_agents post-3b.3c and return empty transient fields). |

**Action item:** grep the frontend for the `updateagentinstance` RPC response consumer. If it's fire-and-forget (likely — most instance updates are status/session writes), choose **A**. If it reads the returned instance, choose **B** and decide whether the returned transient fields matter.

**RESOLVED (2026-05-29): choose A.** `UpdateAgentInstanceCommand` has **zero callers** in the frontend — only the generated RPC wrapper (`rpc-api.ts:1026`) exists, nothing invokes it. No consumer reads the response, so returning a minimal `{ id }` (or `null`) is risk-free. No need to re-read for the response.

## 4. Proposed API

```rust
/// Partial update of an instance's mutable runtime fields. Only `Some`
/// fields are written; `None` leaves the column untouched. Replaces the
/// caller-side fetch-and-merge that pinned `instance_get` to the legacy
/// transient row.
pub struct InstanceUpdate {
    pub block_id: Option<String>,
    pub session_id: Option<String>,
    pub status: Option<String>,
    pub github_context: Option<String>,
    pub ended_at: Option<i64>,
}

impl Store {
    pub fn instance_update_partial(&self, id: &str, upd: &InstanceUpdate)
        -> Result<bool, StoreError>;
}
```

`instance_update_partial` builds a dynamic `UPDATE db_agent_instances SET <only-Some columns> WHERE id = ?` (no-op / `Ok(false)` if all `None`), then runs the same post-write reload → `registry_upsert_if_named` + `agents_dual_write_instance_update`. Because those consumers need only non-transient fields, the reload can flip to a `db_agents`-backed `instance_get` in 3b.3c without breaking them.

`CommandUpdateAgentInstanceData` already mirrors `InstanceUpdate` exactly (`id` + the 5 `Option` fields) — `From<CommandUpdateAgentInstanceData> for InstanceUpdate` is a trivial move.

## 5. Sub-PR sequence

1. **PR α — partial-update API + handler rewrite (the unblocker).**
   - Add `InstanceUpdate` + `instance_update_partial`. Keep the old `instance_update(&AgentInstance)` temporarily (other callers? grep — `instance_repoint_definition` is separate; check) or migrate its callers.
   - Rewrite the handler: drop the `instance_get` fetch-and-merge; call `instance_update_partial(cmd.id, &cmd.into())`. Resolve §3 (response shape).
   - Tests: cover partial writes (set only `status`; set only `session_id`; all-`None` no-op; confirm untouched columns preserved).
   - **No read-flip yet** — `instance_get` still reads legacy. This PR is pure decoupling + must be behavior-identical at the SQL level. Ship + verify.

2. **PR β — 3b.3c: flip `instance_get` → db_agents.**
   - Now that no production caller needs transient fields from it, point `instance_get` at `db_agents WHERE id = ?` (same projection shape as `instance_list`'s no-status case — transient fields returned as empty defaults).
   - Update the store-tests that assert transient fields from `instance_get` (they must move to asserting via the legacy path or be re-pointed).
   - `instance_update`'s internal reload now reads db_agents — verify registry mirror + dual-write still get `instance_name`/`github_context`/etc.

3. **PR γ — 3b.3b: drop the `instance_list` status-filter legacy branch.**
   - Delete the `if status.is_some() → instance_list_legacy` branch and the `instance_list_legacy` helper (no live caller). Either drop the `status` param from `instance_list` entirely or keep it as an accepted-but-ignored arg for signature stability — decide based on the test churn.
   - Update/remove the store-tests that pass `Some(status)`.

(β and γ are independent of each other; both depend on α only for β. γ could even land first since it's pure dead-code removal — but bundling after α keeps the "transient fields" story in one arc.)

## 6. Risks

- **Response-shape regression (§3).** The one real behavioral decision. Mitigated by checking frontend consumers before choosing A vs B.
- **`instance_update` other callers.** Grep for `instance_update(` — if callers other than the handler + tests exist, they need migrating to `instance_update_partial` or the old method kept. (Initial scan: the handler + internal tests; `instance_repoint_definition` is a distinct method.)
- **Test coupling.** Several store-tests assert `instance_get(...).status` and `instance_list(_, Some(...))`. These are the main churn; they're testing the legacy transient surface and must be re-pointed when β/γ land.
- **Dual-write timing.** The reload→dual-write must keep firing; `instance_update_partial` must replicate `instance_update`'s post-write reload exactly (don't drop the `agents_dual_write_instance_update` call — Phase 3b propagates its errors).

## 7. What this unblocks

α alone clears the *only* production dependency on `instance_get`'s transient fields → β (3b.3c) becomes a clean read-flip, and γ (3b.3b) a dead-code delete. After β + γ, the remaining Phase 3b items are 3b.1b (`instance_list_named`, blocked on the `listrecentsessions` per-launch story) and 3b.5 (`agent_def_get` working_directory fold) — then Phase 3c retires the legacy tables.
