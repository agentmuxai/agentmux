# SPEC: Per-Agent Identity Provisioning (separate Claude account per agent)

> **Archived 2026-07-12.** Superseded by `docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md` — the identity-bundle provisioning approach here was replaced by direct Account-to-Agent links. Consolidated tracking: issue #2024.

**Date:** 2026-06-30
**Status:** Draft — design for the "close the code gap first" path
**Author:** AgentX
**Decisions (from product owner, 2026-06-30):** separate Anthropic **account per agent** (own login, own rate-limit pool, own billing); **pilot AgentX first**; **close the code gap** before rolling out.
**Related:** `agentmux-srv/src/identity/resolver.rs`, `identity/auth_session.rs`, `identity/migration.rs`, `backend/storage/identities.rs`, `server/agent_handlers.rs`, `backend/providers.rs`; specs `docs/specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md`, `specs/archive/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md`, `docs/specs/agentmux-isolated-auth.md`.

---

## 1. Problem

Agents spawn the Claude CLI inheriting the **shared** provider config
(`~/.agentmux/shared/providers/claude`) — one account, one rate-limit pool, one
billing identity for every agent. Verified live: the running "AgentX" agent has
`AGENTMUX_AGENT_ID=AgentX` but `CLAUDE_CONFIG_DIR=…/shared/providers/claude` and
its `AgentInstance.identity_id` is empty, so the resolver short-circuits to ambient
creds (`resolver.rs:441–450`). We want each agent to run under its **own** Claude
account in an isolated config dir.

## 2. What already works (do NOT rebuild)

The end-to-end machinery is largely present:

| Capability | Where | Status |
|---|---|---|
| Spawn injects `CLAUDE_CONFIG_DIR` from the instance's identity binding | `resolver.rs:548–575` (`inject_identity_env_*`), called at `agent_handlers.rs:3148` | ✅ Wired |
| Provider registry maps Claude → `CLAUDE_CONFIG_DIR`, OAuth class | `providers.rs` (`auth_config_dir_env_var`) | ✅ |
| `SecretRef::OAuthConfigDir { dir }` — per-bundle config dir as the credential | `storage/identities.rs:59` | ✅ |
| Instance carries `identity_id` → bundle; resolver reads it | `storage/agents.rs:238`, `resolver.rs:425–498` | ✅ |
| **Bundle-aware OAuth flow** — `begin_session(provider, into_bundle_id)` / `finish_success(session_id, bundle_id)` | `auth_session.rs:161,232` | ✅ Already targets a named bundle |
| Provisioning prior art — create dir, import creds, probe status, persist `account(OAuthConfigDir)+binding` | `migration.rs:239–289` (Default bundle) | ✅ Pattern exists, but Default-only and imports **ambient** creds |
| Per-identity history/session/memory roots resolve from `identities/<id>/…` | `claude_adapter.rs`, `session_backfill.rs`, `native_memory_handlers.rs:155` | ✅ |

**Implication:** this is far smaller than a from-scratch build. The injection and
data model are done; the OAuth flow is already bundle-aware.

## 3. The genuine gap

For **separate account per agent**, three things are missing or unverified:

**G1 — No entry point to stand up a *new named* bundle with a *fresh* login.**
`migration.rs` provisioning is Default-bundle-only and **imports ambient creds**
(same account, isolated dir) — the opposite of "separate account." We need an
operation that creates a new `db_identity_bundles` row (e.g. "AgentX") and drives
a **fresh OAuth login** into its own dir via the existing bundle-aware
`AuthSessionManager`, yielding a distinct account.

**G2 — Bundle-dir provisioning on the fresh-login path (VERIFY).** `migration.rs`
explicitly `mkdir`s `identities/<id>/claude/` before import. It must be confirmed
that the `finish_success` / OAuth-bind path **also** creates that dir and persists
`SecretRef::OAuthConfigDir { dir: …/identities/<bundle>/claude }` for a *new*
bundle (not just Default). **This is the one make-or-break detail to confirm before
coding** — read `finish_success` (`auth_session.rs:232+`) and its caller in
`install_handlers.rs` / the OAuth completion handler. If it already provisions the
dir + persists the binding for an arbitrary `into_bundle_id`, G1 is nearly free.

**G3 — Launch must bind the instance to the bundle.** An agent launched without
picking an identity gets `identity_id = ""`. For the pilot, the AgentX agent must
be (re)launched with its instance's `identity_id` set to the AgentX bundle — either
via the launch modal's identity picker or a direct instance-bind.

## 4. Design — minimal, reuse-first

### 4.1 Provisioning operation (closes G1, leans on the existing OAuth flow)

Add one operation — `identity.bundle.provision` (RPC/App-API, or an internal helper
the launch modal calls) — that, given a bundle **name** and **provider** (`claude`):

1. Create-or-get the named bundle (`bundle_identity_upsert`) → `bundle_id`.
2. Ensure the dir exists: `mkdir -p <shared>/identities/<bundle_id>/claude` (reuse
   the `migration.rs` dir helper — factor it out if inline).
3. Start a **fresh** OAuth session via `AuthSessionManager::begin_session(
   "claude", Some(bundle_id))` with `CLAUDE_CONFIG_DIR` pointed at the bundle dir,
   so the login writes a *new* account's `.credentials.json` there (NOT a copy of
   ambient).
4. On `finish_success`, persist `IdentityAccount { secret_ref: OAuthConfigDir {
   dir }, kind: "oauth", status: <probed> }` + `bundle_identity_bind(bundle_id,
   "claude", account_id)` — exactly the `migration.rs:256–288` shape.

If G2's verification shows `finish_success` already does steps 2+4 for any
`into_bundle_id`, this operation is mostly orchestration (create bundle → kick the
existing flow → done).

### 4.2 Bind at launch (closes G3)

For the pilot, ensure the AgentX agent's instance is created/updated with
`identity_id = <AgentX bundle_id>`. Prefer the existing launch-modal identity
picker; if a programmatic path is needed, a small `instance bind identity`
operation (set `AgentInstance.identity_id`) suffices. No new resolver work — once
`identity_id` is set, spawn injection already does the rest.

### 4.3 Separate-account invariant

The credential source for a per-agent bundle is a **distinct interactive OAuth
login** (a different Anthropic account), never a copy of the shared/ambient creds.
This is the one behavioral difference from the Default migration and the crux of
"separate account per agent." Document it as INV: *a non-Default bundle's
`OAuthConfigDir` is populated only by a login performed into that dir.*

## 5. Pilot: stand up AgentX

1. `identity.bundle.provision(name="AgentX", provider="claude")` → creates bundle +
   dir, opens the OAuth URL.
2. Complete the login with **AgentX's own** Anthropic account.
3. Verify on disk: `~/.agentmux/shared/identities/<id>/claude/.credentials.json`
   exists; account row has `OAuthConfigDir { dir }`; binding links bundle→claude.
4. Relaunch the AgentX agent bound to the bundle (`instance.identity_id = <id>`).
5. **Verify isolation:** in the new agent, `CLAUDE_CONFIG_DIR` resolves to the
   bundle dir (not `shared/providers/claude`), and `identity` resolver logs show
   `injected CLAUDE_CONFIG_DIR for oauth provider claude (identity=<id>…)`.

## 6. Non-goals

- Fleet rollout (deferred until the pilot is proven — product owner's call).
- Auto-provisioning identities for every agent at launch (manual, explicit per
  agent for now).
- Changing the resolver, provider registry, or the spawn injection path — all done.
- Importing/migrating existing shared creds into per-agent bundles (that's the
  *shared-account* model, explicitly not chosen).

## 7. Risks / open questions

- **G2 (linchpin):** confirm `finish_success` provisions the dir + persists the
  binding for an arbitrary new `into_bundle_id`. If it doesn't, the provisioning
  operation must do the `mkdir` + account/binding persistence itself (factor out
  `migration.rs`'s helpers). **Resolve before writing the operation.**
- **OAuth app limits:** AgentMux can't self-register an OAuth app
  (`agentmux-isolated-auth.md`); the login uses Claude's own OAuth. Each separate
  account is a real, separate Claude login the operator must perform — N logins for
  N agents. Acceptable per the chosen model; flag the operational cost.
- **Token refresh per bundle:** confirm the OAuth expiry probe / refresh
  (`resolver.rs` OAuth branch) operates per-`OAuthConfigDir`, so each agent's token
  refreshes independently.
- **Idempotency:** provisioning an already-provisioned bundle must be safe
  (reuse `account_id`, re-probe status) — mirror `migration.rs:247–254`.

## 8. Implementation sequence

1. **Verify G2** (read `finish_success` + OAuth completion handler) — gate.
2. Factor `migration.rs` dir + account/binding helpers into a reusable
   `provision_bundle_*` if not already shared.
3. Add `identity.bundle.provision` operation (orchestrate create-bundle → fresh
   OAuth → persist).
4. Wire the launch path to bind the instance (or use the modal picker for the pilot).
5. Pilot AgentX end-to-end (§5); confirm isolation from logs + on-disk creds.
6. Only then consider fleet rollout.
