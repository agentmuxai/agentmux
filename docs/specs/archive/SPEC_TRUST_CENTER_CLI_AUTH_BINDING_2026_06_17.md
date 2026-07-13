# SPEC: Trust Center ↔ CLI-OAuth two-way binding

> **Archived 2026-07-12.** Historical — shipped as specced. Consolidated tracking: issue #2024.

**Date:** 2026-06-17
**Status:** Draft — implementation (folded into the PR)
**Scope:** Trust Center Accounts (`frontend/app/view/accounts/**`, `frontend/app/view/identity/identity-model.ts`)
**Related:** `specs/archive/SPEC_TRUST_CENTER_2026_06_15.md` (the hub), `identity/migration.rs` (Default-bundle ambient discovery), `identity/resolver.rs::probe_oauth_status`

---

## 1. Problem

A user talking to a running Claude instance is *clearly* authorized with Anthropic — but the Trust Center shows **no** Anthropic connection. Two namespaces never met:

- **Concept A — CLI-provider OAuth:** the Claude CLI's login lives at `~/.claude/.credentials.json`. At startup, `migration.rs` discovers it and creates an `IdentityAccount` with **`provider = "claude"`**, `kind = "oauth"`, `secret_ref = OAuthConfigDir { ~/.claude }`, `status` from `probe_oauth_status` (live), bound to the **Default** bundle.
- **Concept B — Trust Center brands:** the gallery + grouping only know brand ids — `AccountProvider = github|google|aws|openai|anthropic|slack|custom|agentmux`. **`"claude"` is not in the type, not a tile, not in the `accountsByProvider` order.**

So the already-authorized, already-probed `"claude"` OAuth account is **filtered out and invisible**: `accountsByProvider` (the source for both the tile count `AccountsGallery.countFor` and the connected list `AccountsTab`) drops any provider not in its brand `order`.

## 2. Insight — unify the namespace at the grouping key

The fix is a **display-only normalization**: a CLI-OAuth provider *is* a brand authorization. Map CLI provider → brand and group by the brand, **without** changing the account's stored `provider` (the resolver still injects env keyed by the real `"claude"` id at spawn — untouched).

| CLI provider (concept A) | Trust Center brand |
|---|---|
| `claude` | `anthropic` |
| `codex` | `openai` |
| `gemini` | `google` |
| `copilot` | `github` |
| (`openclaw`, others) | passthrough (no brand tile yet) |

`accountsByProvider` already keys the gallery tile count *and* the connected list. Normalizing its grouping key therefore surfaces the detected account **everywhere at once**:
- the **Anthropic tile** shows "1 connected";
- the **Connected accounts** list shows the `claude-oauth` row under the Anthropic group, with its **live status dot** (valid/expired from `probe_oauth_status`, refreshed every spawn + broadcast) — the read side of the two-way binding;
- the per-agent identity panel groups it under Anthropic consistently.

## 3. Two-way binding

- **Read (this PR):** the Trust Center reflects real OS auth state. The account row is the *same* row the resolver re-probes at every agent spawn (`inject_identity_env_with_broker` → `probe_oauth_status` → upsert status → `identitybundlebindings:changed` broadcast), so the status the Trust Center shows is live, not a snapshot.
- **Write (already wired):** the account is bound to the Default bundle, so it composes into identities/agents today; reconnect routes through the existing OAuth login flow (`auth.start` into the bundle). No new write path needed for v1.

## 4. Changes

| File | Change |
|------|--------|
| `frontend/app/view/accounts/provider-brand.ts` *(new)* | `brandForProvider(provider): AccountProvider` — CLI-OAuth id → brand (table above), else passthrough. Single source of the mapping. |
| `frontend/app/view/identity/identity-model.ts` | `accountsByProvider()` groups by `brandForProvider(a.provider)` instead of the raw provider. Account `provider` is unchanged; only the **grouping key** normalizes. |
| (verify) `AccountsGallery.countFor` | unchanged — it reads `accountsByProvider().get(id)`, so it's fixed transitively. |

That's the whole fix: ~1 new tiny module + a one-line key change. No backend change — the account + live status already exist.

## 5. Robustness follow-ups (not in v1)

- **Live ambient probe RPC** (`cli_auth_status`): for the case where the Default-bundle migration hasn't created the account (fresh machine, non-standard login), probe `~/.{authDirName}/.credentials.json` for each OAuth CLI on Trust Center open and surface detected brands even with no stored row. The migration covers the common case today; this hardens it.
- **Brand-native reconnect:** when a brand tile's only connection is a CLI-OAuth account, its OAuth action should route to that CLI's login (`auth.start` provider=`claude`) rather than a brand-OAuth path.
- **Logo/label:** the connected row shows the Claude logo under the Anthropic group (informative — "via Claude CLI"); optionally add a "via Claude CLI (OAuth)" sublabel.

## 6. Verification

- Anthropic tile shows "1 connected" when `~/.claude` OAuth exists; the Connected list shows the `claude-oauth` row under Anthropic with a live status dot.
- Stored `provider` stays `"claude"` (agent env injection unaffected); `npm run build` + identity tests pass.
- No regression to brand-native (concept-B) accounts — they group under their own provider as before.
