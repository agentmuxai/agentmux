# Spec: Identity direct-links, PR-C (Armory read-only view + bundle-free agent creation)

**Date:** 2026-07-10
**Author:** AgentX
**Governing decisions:** `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` §Phase 3 (target: `instance/bundle → account` directly, ≤1 account per provider enforced at resolve time). Tracking issue: **#1624** ("Reconcile per-agent keychain with the live identity-bundle system").

---

## Where this picks up

PR-A (#1927, dual-read resolver + `db_agent_identity_links` backfill migration) and PR-B (#1952/#2041, resolver flips to reading direct links exclusively) are merged. `agentmux-srv/src/identity/resolver.rs`'s `resolve_bindings_for_instance` now reads **only** `db_agent_identity_links` at spawn time — it has no bundle fallback.

What's left, per #1624's 2026-07-03 resolution comment, is **PR-C**: stop the two remaining write paths that only write bundles, since a binding created through either is invisible to the resolver until the launch-flow's write-through reconcile (`agent-model.ts`) happens to also copy it into a direct link.

1. The Armory "Identities" tab (full `db_identity_bundles`/`db_identity_bindings` CRUD UI).
2. `AgentLaunchModal`'s "+ New identity" button and OAuth Connect flow — discovered mid-investigation to be the *actual* creation UI for new agent identities (the write-through reconcile only works because it reads back the bundle's bindings after the fact; it was never an independent write path). Scope was expanded accordingly.

**Explicit non-goals for this phase:** `resolver.rs`, the m0013/m0014 migrations, and every `bundle_*` RPC handler are untouched — this is a frontend-write-surface migration, not a backend API deletion. The agent-pane's own `view: "identity"` settings tab (cog → settings → Identity) already renders a read-only `<BundleSummaryPanel/>` (docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md PR 5) and is unaffected.

**Relationship to §3.4's longer-term decision**: `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` §3.4 resolves that "Identity" should ultimately fold into the **Accounts** tab entirely (no standalone Identities concept at all). This phase does **not** do that — it implements the interim step #1624 itself specifies ("Identities tab → derived view"), which keeps a distinct, read-only Identities view rather than folding it away. The full Accounts-fold is out of scope here and remains open (tracked loosely under issue #2024).

## Sequencing

Three PRs, each independently reviewable:

1. **PR 1 — Armory Identities tab → read-only.** No dependency on anything else.
2. **PR 2 — Backend OAuth-direct-account primitive.** New Rust function, no frontend caller yet — isolates the highest-risk change (OAuth session state machine) for standalone review.
3. **PR 3 — AgentLaunchModal + AgentCreateFromTemplateModal frontend redesign.** Consumes PR 2.

## PR 1 — Armory Identities tab (read-only, per-agent)

**Design decision — new `listallagentidentities` RPC vs. N+1 `ListAgentIdentitiesCommand` calls**: chose the new RPC. `Store::agent_identity_list_all()` (`agentmux-srv/src/backend/storage/identities.rs:325`) already existed, already tested (m0011/m0012/m0013 backfill migrations), and returns exactly the shape the rail needs. Wrapping it in `COMMAND_LIST_ALL_AGENT_IDENTITIES = "listallagentidentities"` (mirrors the existing `listagentidentities` handler, no `agent_id` filter) avoids issuing one RPC per rail row.

**Shape**: rail = every `AgentDefinition` (`useAgentDefinitions()`, already exported and live-refreshing). Detail = the selected agent's provider→account rows, sourced from one `ListAllAgentIdentitiesCommand` call filtered client-side (`joinAgentIdentityRows`, `frontend/app/view/identity/agent-identities-model.ts`), joined against the existing account cache (`loadAccounts()`/`subscribeAccountChanges()`). No create/edit/delete/bind/unbind — this view only shows what's already linked.

New files: `frontend/app/view/identity/agent-identities-model.ts` (pure join logic, unit-tested), `frontend/app/view/identity/agent-identities-panel.tsx` (the component, mounted at the Armory "Identities" rail slot in place of the old `<IdentityManager/>`). Reuses the `.identity-pane-readonly` table markup + `statusBadge()` helper from `identity-manager.tsx` (exported for this purpose) rather than duplicating it.

**Dropped for v1**: the "Reconnect" action (re-running OAuth on an expired token in place). It needs PR 2's direct-account OAuth primitive to wire against; adding it now against the bundle-based OAuth flow would be throwaway work. Fast-follow once PR 2 lands.

**Discovered dead code, deliberately left alone**: `identity-manager.tsx`'s `IdentityManagerBody`/`IdentityManager` (the full bundle-CRUD component this PR replaces at its only mount point) turns out to have **no remaining consumer anywhere** after this change — the agent-pane settings tab was already demoted to `<BundleSummaryPanel/>` in an earlier PR (docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md PR 5), and Armory was the last live mount. Left in place rather than deleted, to keep this PR's diff scoped to what it set out to do; flagged here as a candidate for a follow-up cleanup PR.

## PR 2 — Backend OAuth-direct-account primitive

**Design decision — account-id round-trip vs. immediate-link**: chose the round-trip (mint the account server-side during OAuth, return its id to the frontend, the actual `agent_identity_link` write happens later once the agent is created). The launch modal picks an identity *before* the agent exists, so there's no `agent_id` available at OAuth-start time to link into immediately — this mirrors today's bundle-id round-trip shape exactly and needs the least new plumbing.

`StartProviderAuthReq` gains `direct_account: bool` + `existing_account_id: String` (both additive, `#[serde(default)]`) — a genuinely new discriminator, not an overload of `into_bundle_id: None` (which already means the distinct legacy "ambient, no persist" case). New sibling functions `compute_and_ensure_account_dir`/`persist_oauth_direct_account` mirror `compute_and_ensure_bundle_dir`/`persist_oauth_binding_or_synthetic` but key by `account_id` and skip `bundle_identity_bind` entirely. The existing `success_transitioned: AtomicBool` double-persist guard (added for #981) is reused unchanged — it already wraps the whole persist-and-finish block for both branches.

`AuthSessionStatus::Success` gains `account_id: Option<String>` (skip-serializing when `None`); `AuthSessionManager::finish_success` gains a matching parameter, `None` at all 4 existing bundle-mode call sites.

## PR 3 — AgentLaunchModal + AgentCreateFromTemplateModal

Renames `identityId`/`IdentityBundle` → `accountId`/`Account` through the launch-flow reducer, auth controller, and the launch-modal component tree specifically (not in unrelated files like the historical `NamedAgentRow.identity_id` DB column).

**The core fix**: `PreLaunchAuthPanel.tsx`'s `handleConnect` no longer refuses to start OAuth when no identity is pre-selected — OAuth starts unconditionally, and the backend (PR 2) mints the account. This removes the "+ New identity bundle" pre-step for OAuth entirely; that button is repurposed for manual/API-key account entry only (`AccountKeyVerifyCommand`, already bundle-free).

The post-launch write-through reconcile (`agent-model.ts`'s `launchAgentDefinition`) collapses from a bundle-bindings diff (`ListIdentityBindingsCommand` → diff → `Link`/`Unlink` loop) to a single `LinkAgentIdentityCommand` call — `agent_identity_link`'s `ON CONFLICT(agent_id, provider) DO UPDATE` (`identities.rs:292-306`) makes a single upsert correct without an explicit unlink-then-link, which was flagged as a P1 risk (losing a link if unlink succeeds but the following link fails) on the unrelated but adjacent open PR #2056.

**Migration note**: old `NamedAgentRow.identity_id` values (from instances launched before this PR) are bundle ids, not account ids. Decision: leave the "Continue"/"Reattach"/"Fork" carry-over paths unchanged — an unresolvable carried id already falls back to "re-pick," via existing defensive code (`AgentLaunchModal.tsx:322-329`). Accepted as a one-time UX rough edge for pre-existing rows, not fixed.

## Known residual risks

- **Overlap with open PR #2056** (`agentmux-srv/src/server/app_api/identity.rs`'s S1-authenticated handlers — different functions/files from what this phase touches, same conceptual area). No hard dependency either direction; new code in this phase is written to avoid the same redundant-unlink bug independently of whether #2056 lands first.
- Two live OAuth-persist code paths (bundle-mode for the agent-pane settings tab's Reconnect affordance if ever re-added, direct-account-mode for the launch modal) coexist after this lands — accepted cost of additive-first staging.
