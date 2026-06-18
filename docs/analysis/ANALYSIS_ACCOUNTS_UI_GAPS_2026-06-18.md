# Analysis: Trust Center Accounts UI — Gaps & Fixes

**Date:** 2026-06-18
**Author:** Parko
**Status:** Bugs fixed in this session (see §3)

---

## 1. Context

As part of populating AgentMux `db_identity_accounts` with credentials from AWS
Secrets Manager (`services/infra`), we audited the Trust Center Accounts UI and
found three bugs causing the view to feel "create-only": accounts we added were
either invisible or showed stale/wrong metadata.

Secrets were added using `@a5af/secrets` CLI (v1.1.6) with `backend:
secrets_manager` refs only — no plaintext values were stored in the DB or repo.

---

## 2. Bugs Found

### Bug 1 — `agentmux` provider silently dropped from account list

**Files:** `identity-model.ts:378`, `identity-view.tsx:286`

`accountsByProvider()` iterates a hardcoded provider order that omits `"agentmux"`:

```ts
// BEFORE — agentmux missing
const order: AccountProvider[] = ["github", "google", "aws", "openai", "anthropic", "slack", "custom"];
```

Any account with `provider: "agentmux"` (e.g. our `AgentMux Bus` account) would
never render in the AccountsTab list or AssignmentsTab. `AssignmentsTab` had the
same hardcoded list at `identity-view.tsx:286`.

**Fix:** Add `"agentmux"` to both ordered lists.

---

### Bug 2 — Deprecated `assigned_agents` field drives all agent-count UI

**Files:** `identity-view.tsx:157–159, 241–249, 289–299`

`Account.assigned_agents` is annotated `@deprecated` in `identity-model.ts:67`.
`backendToAccount()` always synthesizes it as `[]`. Despite this, six call sites
in the view read it:

- `AccountRow` line 157–159: shows "N agents" badge → always 0
- `AccountDetail` lines 241–249: "Agents" section → always "No agents assigned"
- `AssignmentsTab` lines 289–299: entire matrix built from deprecated field → always empty

The model exports `agentsAssignedToAccount(accountId, agents)` as the correct
reverse-index path, but calling it requires a live `AgentDefinition[]` list that
isn't available in the identity view. The spec (§12.4) also calls for a
`ListAccountUsageCommand` RPC (not yet implemented) to do this properly.

**Fix applied:** Remove the always-wrong agent-count badge from `AccountRow`.
Replace the Agents section in `AccountDetail` with a note directing users to the
Identities tab (where the live binding data lives). The `AssignmentsTab` matrix
is left as-is pending `ListAccountUsageCommand` — it's a known incomplete
feature, not a regression.

---

### Bug 3 — No actionable CTA for `unknown` / `expired` status

**File:** `identity-view.tsx:254–268`

The detail panel renders a status dot + text but no action button. All 11
accounts we added land in `status: "unknown"` with no path forward visible in
the UI. The spec (§5) calls for:

- `unknown` → **"Validate"** (triggers `AccountKeyVerifyCommand`)
- `expired` → **"Reauth"** (re-opens the edit/entry form)

For `secrets_manager`-backend accounts the key never transits the frontend, so
the Validate button is not applicable from the UI side — those accounts will be
probed at agent launch by the Rust resolver. We therefore scope the Validate CTA
to `keychain` and `plaintext_dev` backends only, and show an informational note
for SM accounts instead.

**Fix applied:** Add status-contextual actions to `AccountDetail` footer.

---

## 3. Changes Made

| File | Lines | Change |
|------|-------|--------|
| `frontend/app/view/identity/identity-model.ts` | 378 | Added `"agentmux"` to `accountsByProvider()` order |
| `frontend/app/view/identity/identity-view.tsx` | 157–159 | Removed deprecated `assigned_agents` count badge from `AccountRow` |
| `frontend/app/view/identity/identity-view.tsx` | 238–249 | Replaced deprecated Agents section with Identities note |
| `frontend/app/view/identity/identity-view.tsx` | 254–268 | Added Validate/Reauth CTAs scoped by backend type |
| `frontend/app/view/identity/identity-view.tsx` | 286 | Added `"agentmux"` to `AssignmentsTab` providers list |

---

## 4. Not Fixed (requires backend work)

| Item | Reason | Spec ref |
|------|--------|----------|
| `AssignmentsTab` matrix (agent↔account grid) | Needs `ListAccountUsageCommand` RPC | §12.4 |
| Status validation for `secrets_manager` accounts | Backend must probe SM refs at startup | §6 |
| `claude` provider in DB (`claude-oauth`) | Not in `AccountProvider` type; rendered specially in `accounts-manager.tsx` as AgentMux Cloud singleton | §2 §12.5 |

---

## 5. Accounts Added to DB

All use `backend: "secrets_manager"` — only path refs stored, no values on disk.

| Name | Provider | Kind | SM key |
|------|----------|------|--------|
| Anthropic API | anthropic | api_key | `claude-api-token` |
| Kimi API | custom | api_key | `kimi-api-key` |
| GitHub (admin) | github | pat | `gh-admin-pat` |
| GitHub (agent1–5, agentx, agenty) | github | pat | `gh-token-agent*` |
| AgentMux Bus | agentmux | api_key | `agentmux-api-key` |
