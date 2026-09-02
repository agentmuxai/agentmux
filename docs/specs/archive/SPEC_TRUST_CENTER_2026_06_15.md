# SPEC: Trust Center — Unified Memory · Identity · Accounts Hub

> **Archived 2026-07-12.** Historical — describes the original Trust Center pane exactly as it shipped. The pane was renamed Armory (see `docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md`) and the account/identity model has since moved to direct links (`docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md`). Consolidated tracking: issue #2024.

**Date:** 2026-06-15
**Status:** Draft — design / plan
**Author:** Smark
**Note:** Folds in and replaces an earlier app-wide-Accounts-only draft; this is the cohesive plan.
**Affects:** `frontend/app/view/trust-center/` (new), `frontend/app/view/identity/`, `frontend/app/view/memory/`, `agentmux-srv/src/identity/`, `agentmux-srv/src/backend/storage/`

---

## 0. TL;DR

Bring the three reusable building blocks an agent is composed from — **Memory** (personality/knowledge), **Identity** (who it acts as), and **Accounts** (the credentials behind an identity) — into **one app-wide hub**: the **Trust Center**.

Most of the data model already exists (see §2). The new work is:
1. **Reuse the existing app-wide modal** — rename the "Identity & Memory" hamburger entry to **"Trust Center"** and add an **Accounts** tab to the modal it already opens (`bundle-manager-modal.tsx`). Identity + Memory tabs stay untouched; singleton + cross-window coordination are inherited for free.
2. **App-wide Accounts** with a broad service catalog and **two auth paths: OAuth and keys/tokens**.
3. A **secure key lifecycle**: enter → **Validate** (user-initiated; only the click triggers the single outbound probe, with an inline egress help note) → backend validates → displays metadata → key is **masked and non-recoverable** (`••••••••3f9a`), stored via OS keychain, never returned to the UI again.

---

## 1. Naming

The hub holds everything that defines *who an agent is and what it may touch* — its trust posture. Candidate names:

| Name | Fit | Notes |
|------|-----|-------|
| **Trust Center** ✅ (recommended) | Strong | Security/governance framing; "memory" reads as *trusted knowledge*, identity + accounts as *trusted credentials*. Familiar enterprise term (Salesforce/Atlassian use it). |
| Identity Center | Good | Undersells memory. |
| Vault | Good for accounts | Too credential-narrow for memory. |
| Loadout / Armory | Playful | "Equip an agent" metaphor; less serious for credentials. |
| Foundry | Decent | Collides with existing **Forge** terminology in the codebase. |

**Recommendation: "Trust Center"**, three sections — **Accounts**, **Identities**, **Memory** — in that order (you set up accounts first, compose them into identities, then memory is the parallel personality track). Decision in §10 Q1.

---

## 2. What Already Exists (verified architecture)

The user's mental model — *account → identity → agent*, plus *memory* — **is already the schema.** This is extend-and-unify, not greenfield.

```
AgentInstance (db_agent_instances)            ← runtime record
  ├─ definition_id → AgentDefinition
  ├─ identity_id   → IdentityBundle           ← "identity"  (empty = blank singleton)
  └─ memory_id     → Memory                   ← "memory"    (empty = blank singleton)

IdentityBundle (db_identity_bundles)           ← THE "identity"
  └─ id, name, description, is_blank
       │  IdentityBinding (db_identity_bindings): (identity_id, provider, account_id)
       ▼
IdentityAccount (db_identity_accounts)         ← THE "account"
  └─ id, name, provider, kind, secret_ref, context, status, timestamps

Memory (db_memory_bundles)                     ← THE "memory"
  └─ id, name, provider, model, instructions, context_files, mcp_servers, skills
```

Sources:
- `IdentityAccount` + `SecretRef` (TS): `frontend/app/view/identity/identity-model.ts:18-61`
- `IdentityBundle` / `IdentityBinding` (TS): `frontend/types/gotypes.d.ts:304-319`
- Rust storage: `agentmux-srv/src/backend/storage/identities.rs:71-127`, `memory_bundles.rs:20-50`
- Composition (startup payload): `frontend/app/view/agent/startup/buildStartupPayload.ts:129-150`
- Existing entry point (to absorb): hamburger **"Identity & Memory"** → `bundle-manager-modal.tsx` (singleton modal)

### Two distinct "account-like" concepts (do NOT merge)
- **A. CLI-provider auth** — OAuth that logs the *agent CLI* into its backend (Claude→Anthropic, Codex→OpenAI). Machinery: `auth_session.rs`, `auth_patterns.rs`, the `auth.*` RPCs, on-disk tokens via `SecretRef::OAuthConfigDir`. **Leave in place.** May surface read-only in Trust Center later ("everything I'm logged into").
- **B. Service accounts** — credentials *injected into* agents (GitHub PAT, AWS role, OpenAI key). Model: `IdentityAccount`. **This is the Accounts section**, expanded.

### What's missing
- Unified hub (today: Memory + Identity in one modal, Accounts buried in the per-agent settings overlay `AgentIdentityPanel.tsx`).
- Broad service catalog (today only `github | aws | anthropic | custom`).
- Real **OAuth for non-CLI services** (Google/Slack/Notion have no CLI to scrape).
- **Secure key lifecycle** with validation + metadata + non-recoverable masking.
- Production secret storage (today `plaintext_dev`; `SecretsManager` defined but unimplemented; no OS keychain).

---

## 3. Trust Center — Surface & Layout

**Decision: reuse the existing modal.** No new modal file. The current hamburger entry **"Identity & Memory"** already opens `bundle-manager-modal.tsx` as an app-wide singleton with cross-window coordination and a tabbed left rail. We:

1. **Rename** the hamburger entry **"Identity & Memory" → "Trust Center"** (`frontend/app/window/hamburger-menu.tsx` ~L130). The label string is the only menu change.
2. **Build Accounts out inside that existing modal** — add an **Accounts** tab alongside the existing Identity and Memory tabs in `bundle-manager-modal.tsx`. Everything else (singleton claim, "open elsewhere" banner, WPS coordination) is inherited unchanged.

This keeps the surface, coordination, and persistence we already have and confines new work to a new tab + the Accounts UI/backend. (A standalone `view: "trust-center"` block pane remains a possible Phase-4 add-on; not needed for the core feature.)

```
┌─ Trust Center ───────────────────────────────────────────────────┐
│  [ Accounts ]  [ Identities ]  [ Memory ]                         │
├───────────────────────────────────────────────────────────────────┤
│  ACCOUNTS                                  [ + Add account ]  🔍   │
│                                                                   │
│  CONNECTED                                                        │
│   ●  GitHub      asafebgi         valid       used by 2 ⋯         │
│   ●  OpenAI      org-acme         valid       used by 1 ⋯         │
│   ◐  AWS         prod-deploy      expires 3d   [Reauth]  ⋯        │
│   ●  Anthropic   ••••••••3f9a     valid        used by 3 ⋯        │
│                                                                   │
│  AVAILABLE                                                        │
│   +Google  +Microsoft  +Slack  +Notion  +Linear  +Vercel  …       │
└───────────────────────────────────────────────────────────────────┘
```

- **Accounts** — manage credentials (§4–§6). NEW primary deliverable.
- **Identities** — compose accounts into named bundles (existing bundle manager, moved under this tab).
- **Memory** — personality/capability bundles (existing memory manager, moved under this tab).

Each account row shows status dot + masked credential hint + "used by N identities" (reverse index generalized from `agentsAssignedToAccount`, `identity-model.ts:89-96`).

---

## 4. Accounts — Two Auth Paths

`AccountKind` extends to `pat | api_key | role | env_ref | oauth` (`identity-model.ts:19`). Each service in the catalog declares which it supports.

### 4.1 Service Catalog
New `frontend/app/view/trust-center/catalog.ts` — generalizes per-provider knowledge currently hard-coded across `providers/index.ts`, `auth_patterns.rs`, and `AgentIdentityPanel.tsx`.

```ts
export interface ServiceDescriptor {
    id: string;                 // "github" | "google" | "openai" | …
    displayName: string;
    icon: string;
    authModes: AccountKind[];   // ["oauth", "api_key"]
    oauth?: { flow: "pkce" | "device" | "authcode"; scopes: string[]; authPatternKey: string };
    keyValidation?: {           // how the backend tests a pasted key (§6)
        probe: "github_user" | "openai_models" | "aws_sts" | "anthropic_models" | "slack_authtest" | "generic_bearer";
        metadataFields: string[]; // which metadata to surface
    };
    injects: { envVars: string[]; configDir?: string };
    contextFields: ContextFieldSpec[];
}
```

**Tier-1 catalog:** GitHub, Google, AWS, Microsoft/Azure, OpenAI, Anthropic, Slack, Atlassian, Notion, Linear, Vercel, Cloudflare — plus `custom` (free-form env var). v1 subset in §10 Q4.

### 4.2 OAuth path (services without a CLI)
CLI-provider OAuth scrapes a subprocess's stdout — services like Google/Slack have no such CLI. New backend OAuth 2.0 client:

- **NEW** `agentmux-srv/src/identity/oauth_client.rs` — Authorization Code + **PKCE** (and **Device Flow** for GitHub). Opens system browser, runs a transient `localhost` redirect listener (or device-code poll), exchanges code → tokens, persists via `SecretRef::OAuthConfigDir` (resolver + expiry probe work unchanged), handles refresh → "Reauth" CTA.
- **NEW** RPCs `account.oauth.start | .poll | .cancel`, modeled on the existing `auth.*` set; driven by the **reused** provider-agnostic `AuthFlowController` / `AuthState` (`auth-flow-controller.ts`, `auth-state.ts`).
- **Client-secret problem:** desktop apps can't ship a confidential `client_secret`. Strategy: **PKCE/device-first** (covers GitHub, Google, Microsoft, Slack with the right app type); **BYO** (user pastes their own OAuth app credentials) for services that mandate a secret. No hosted broker. Decision §10 Q2.

### 4.3 Key/token path
The genuinely new security-critical flow — §5 and §6.

---

## 5. Secure Key Lifecycle (the core new flow)

A pasted key moves through three states. The key is plaintext **only** in the entry state and **only** in transit to the backend; it is never persisted in the DB, never returned to the UI, never logged, and not recoverable.

```
   ┌──────────────┐   Apply    ┌──────────────┐  validate ok   ┌──────────────┐
   │  1. ENTRY    │ ─────────► │ 2. VERIFYING │ ─────────────► │  3. LOCKED   │
   │ password     │            │ backend tests│   metadata     │ ••••••••3f9a │
   │ input, plain │ ◄───────── │ the key live │   + masked     │ + metadata   │
   └──────────────┘  validate  └──────────────┘                │ [Replace]    │
        ▲             fail (error, stay in entry)               └──────┬───────┘
        └───────────────────────────  Replace  ──────────────────────┘
```

1. **Entry** — `<input type="password">`, autocomplete off, no spellcheck, not in any form that could be auto-saved by the browser. Plaintext lives only in this field. The egress help note (§5.1) is shown here, next to the **Validate** button.
2. **Validate (user-initiated)** — the user explicitly clicks **Validate** to proceed. This click is the *only* thing that triggers the outbound test call — nothing leaves the machine on keystroke, paste, blur, or focus change. On click, the frontend POSTs the key over the loopback RPC to the backend (never a query string; never logged), then **drops its reference immediately** (clears the input; the value is not retained in any signal/store).
3. **Verifying** — backend makes the **live test call** to the service (§6) — the single outbound request described in the help note — extracting metadata and confirming validity. The plaintext **never round-trips back to the UI**; validation is entirely backend-side.
4. **Locked** — on success the backend:
   - stores the secret via the **OS keychain** backend (§7), DB holds only a pointer + metadata + a **masked hint** (`maskedTail` = last 4 chars, `length`);
   - returns **metadata + masked hint only** — never the key.
   The UI renders `••••••••3f9a` + metadata (account, scopes, expiry, org…). **No reveal affordance** — non-recoverable by design.
5. **Replace** — the only way to change a key: clears to Entry, requires full re-paste + re-validation. Old secret is overwritten/zeroized.

**Failure** — validation fail returns a structured error (invalid / network / insufficient-scope) and stays in Entry; nothing is stored.

### 5.1 Egress Transparency (UI)

Validating a key requires **one outbound network request from the local backend to the service's API** — this is the only way to confirm a key works and read its metadata. The user must understand and consent to this *before* they click. Surface it in the UI, placed at the point of action:

- **Inline help next to the Validate button** — a small `ⓘ` affordance with hover/click help. Copy, dynamically naming the service and endpoint from the catalog:

  > **ⓘ How validation works**
  > Clicking **Validate** sends this key once, over HTTPS, from the AgentMux backend on your machine directly to **{service}** (`{probe endpoint, e.g. api.github.com/user}`) to confirm it works and read its details (account, scopes, expiry).
  > The key is **never** stored in plaintext, never logged, and never sent anywhere else. After validation it is locked into your OS keychain and can't be viewed again.

- **Placement** — directly adjacent to the Validate button so it's read at the moment of decision, not buried in a tooltip elsewhere. On first use, consider expanding it inline (not collapsed) so it isn't missed.
- **Per-service endpoint** — the help text interpolates `keyValidation.probe`'s target host from the catalog, so the user sees exactly where their key goes.
- **No silent egress** — reinforce in copy that nothing is sent until they click; keystrokes/paste never leave the machine. (Resolves §11 Q8.)
- **Air-gapped escape hatch** — for users who cannot allow egress, offer **"Save without validating"** (secondary action). The account is stored with `status: "unknown"` and no metadata; a "Validate now" CTA stays available later. This keeps validation strictly user-initiated and optional.

---

## 6. Backend Key Validation & Metadata

Validation is per-service (`keyValidation.probe` in the catalog). Each probe makes a minimal authenticated request and maps the response to metadata.

| Service | Probe call | Metadata surfaced |
|---------|-----------|-------------------|
| GitHub | `GET /user` (+ scopes header) | login, scopes, token expiry, SSO orgs |
| OpenAI | `GET /v1/models` | org id, key prefix, model access |
| AWS | STS `GetCallerIdentity` | account id, ARN, user/role |
| Anthropic | `GET /v1/models` | org, key type, tier |
| Google | `tokeninfo` | email, granted scopes |
| Slack | `auth.test` | team, bot user id, scopes |
| Microsoft | Graph `/me` | upn, tenant, scopes |
| custom / generic | configurable `GET` with `Authorization: Bearer` | http status, optional JSON path → display name |

Metadata is stored alongside the account (`context` JSON) so the panel can show it without the key. `status` + `last_validated_at` + (where the service exposes it) `expires_at` drive the status dot and Reauth CTA — reusing the existing `probe_oauth_status` status vocabulary (`resolver.rs`).

---

## 7. Secret Storage — Best Practices

Today: `SecretRef { env | secrets_manager | plaintext_dev | oauth_config_dir }` (`identity-model.ts:23-29`, `identities.rs`). `plaintext_dev` is dev-only and inadequate for keys.

**Add `SecretRef::Keychain { service, account }`** — a pointer into the OS-native secret store:
- **macOS** — Keychain Services
- **Windows** — Credential Manager (DPAPI-backed)
- **Linux** — Secret Service / libsecret (with an encrypted-file fallback when no agent is present)

Rust crate: `keyring` (cross-platform) for the common path; encrypted-file fallback uses an OS-keychain-derived master key, never a hardcoded one.

**Hardening checklist:**
- **No plaintext at rest** in the SQLite DB — DB holds only the keychain pointer + metadata + `maskedTail` + `length`.
- **No plaintext in transit beyond loopback** — sidecar is local; treat the RPC as sensitive (POST body only, `Cache-Control: no-store`).
- **No secrets in logs** — scrub key fields from RPC logging, transcripts (`docs/analysis/*`), and crash dumps; add a redaction guard on the identity RPC path.
- **Memory hygiene** — backend zeroes plaintext buffers after use (`zeroize` crate); frontend minimizes plaintext lifetime (clear input on Apply; never store in a signal/localStorage).
- **One-way display** — store only `maskedTail`+`length` for rendering; the full value is never read back into the renderer. **No reveal button** (matches the non-recoverable requirement).
- **Rotation** — Replace requires full re-entry + re-validation; old secret overwritten.
- **Injection at spawn only** — resolver reads the keychain at agent launch and injects env vars into the child process; the value never transits the UI layer.
- **Audit** — `created_at`, `updated_at`, `last_validated_at`, `last_used_at`.
- **Scope transparency** — show granted scopes/permissions so the user understands blast radius before assigning the account to an identity.

---

## 8. Architecture / Data Flow

```
┌────────── bundle-manager-modal.tsx (existing singleton, + Accounts tab) ─────┐
│  Accounts │ Identities │ Memory                                              │
│                                                                              │
│  Add account → service picker (catalog.ts)                                   │
│     ├─ OAuth ─► AuthFlowController (reused) ─► account.oauth.start/.poll      │
│     └─ Key   ─► [Entry] ──Validate (user click)──► account.key.verify        │
│                    │ frontend drops plaintext immediately (loopback POST)     │
│                    ▼                                                          │
└────────────────────┼──────────────────────────────────────────────────────────┘
                     ▼
   agentmux-srv
     ├─ oauth_client.rs (NEW)  — PKCE/device, token exchange + refresh
     ├─ key_validator.rs (NEW) — live probe per service → metadata
     └─ secret_store.rs (NEW)  — SecretRef::Keychain read/write (zeroize)
                     │ returns metadata + maskedTail ONLY (never the secret)
                     ▼
   db_identity_accounts (pointer + metadata + maskedTail)  ── status via probe
                     │ IdentityBinding
                     ▼
   IdentityBundle ── AgentInstance.identity_id ── spawn (resolver injects env)
```

---

## 9. Implementation Plan (phased)

### Phase 1 — Rename + Accounts tab in the existing modal
- **Rename** hamburger entry "Identity & Memory" → "Trust Center" (label string only).
- **Add an Accounts tab** to the existing `bundle-manager-modal.tsx` left rail (Identity + Memory tabs stay as-is; no re-parenting/rewrite).
- New `AccountsSection.tsx` + `catalog.ts` (seed: current 4 providers, key/token modes only).
- Wire Accounts CRUD to existing `ListIdentityAccountsCommand` / `Upsert…` / `Delete…`.
- **Outcome:** Accounts live in the same app-wide modal as Identity + Memory, with all singleton/coordination inherited for free.

### Phase 2 — Secure key lifecycle + validation + keychain
- `AccountKind += "oauth"`; Entry→Validate→Verifying→Locked state machine in the Accounts UI, with the §5.1 egress help inline next to **Validate** and a "Save without validating" escape hatch.
- **NEW** backend: `key_validator.rs` (per-service probes), `secret_store.rs` (`SecretRef::Keychain`), redaction guard.
- New RPCs `account.key.verify` (validate + store, returns metadata + maskedTail), `account.key.replace`.
- Masked, non-recoverable display + metadata + scope panel.
- **Outcome:** keys are validated, locked, masked, non-recoverable, keychain-backed.

### Phase 3 — Service OAuth + breadth
- `oauth_client.rs` (PKCE + device); `account.oauth.*` RPCs driven by reused `AuthFlowController`.
- Expand catalog to tier-1 services; refresh + Reauth CTAs.
- **Outcome:** "log into Google/GitHub/Amazon/…" end-to-end.

### Phase 4 — Polish & integration
- Account ↔ identity quick-bind ("Add to identity…", "used by N").
- Read-only `view: "trust-center"` block pane (register `block.tsx:47-61`, label `blockutil.tsx:33-54`) deferring to the modal.
- Optional status-bar badge when something needs re-auth.
- Optional: surface CLI-provider auth (concept A) read-only in Accounts for a unified "everything I'm logged into" view.

---

## 10. File-Level Change Map

| File | Change | Phase |
|------|--------|-------|
| `frontend/app/window/hamburger-menu.tsx` | rename entry "Identity & Memory" → "Trust Center" (label only) | 1 |
| `frontend/app/modals/bundle-manager-modal.tsx` | add **Accounts** tab to left rail; render `AccountsSection` | 1 |
| `frontend/app/view/accounts/AccountsSection.tsx` | NEW — accounts UI + key Entry→Validate→Locked state machine + §5.1 egress help | 1/2 |
| `frontend/app/view/accounts/catalog.ts` | NEW — service descriptors | 1/3 |
| `frontend/app/view/accounts/accounts-model.ts` | NEW — view model, CRUD, verify flow | 1/2 |
| `frontend/app/view/identity/identity-model.ts` | extend `AccountProvider`; `AccountKind += "oauth"`; add `maskedTail`,`length`,`last_validated_at` | 2 |
| `frontend/app/store/rpc-api.ts` | add `account.key.*`, `account.oauth.*` bindings | 2/3 |
| `agentmux-srv/src/identity/key_validator.rs` | NEW — per-service validation probes | 2 |
| `agentmux-srv/src/identity/secret_store.rs` | NEW — `SecretRef::Keychain`, zeroize | 2 |
| `agentmux-srv/src/identity/oauth_client.rs` | NEW — OAuth 2.0 PKCE/device client | 3 |
| `agentmux-srv/src/server/identity_handlers.rs` | add `account.key.*` / `account.oauth.*` handlers + log redaction | 2/3 |
| `agentmux-srv/src/backend/storage/identities.rs` | `Keychain` variant; metadata columns; catalog-driven validation | 2 |

Reused as-is: existing `bundle-manager-modal.tsx` singleton + cross-window coordination, Identity + Memory tabs (untouched), `auth-state.ts`, `auth-flow-controller.ts`, `resolver.rs::probe_oauth_status`, `IdentityBinding`, identity/memory SCSS.

---

## 11. Decisions (resolved)

All resolved; implementable designs in §12.

1. ~~**Name + structure**~~ **Decided:** "Trust Center" — rename the existing "Identity & Memory" hamburger entry; add an Accounts tab to the existing `bundle-manager-modal.tsx` (Accounts / Identities / Memory). §3.
2. ~~**OAuth client-secret strategy**~~ **Decided:** PKCE/device-first + BYO-credentials fallback for services that mandate a secret. No hosted credential broker.
3. ~~**Keychain library + Linux fallback**~~ **Decided:** `keyring` crate cross-platform; encrypted-file fallback (OS-keychain-derived master key) where no Secret Service agent is present.
4. ~~**v1 service catalog**~~ **Decided:** start with GitHub / Google / AWS / OpenAI / Anthropic / Slack (Phase 2–3); remaining tier-1 services follow.
5. ~~**"used by" granularity**~~ **Decided:** show **identities** per account in v1 (agents derivable later via the bundle→instance link).
6. ~~**CLI-provider auth (concept A) in Accounts**~~ **Decided:** keep fully separate for now; optional read-only "everything I'm logged into" surface deferred to Phase 4.
7. ~~**Migration**~~ **Decided:** leave the deprecated `AgentDefinition.accounts` blob untouched (already superseded by bindings); no migration in this work.
8. ~~**Validation network egress**~~ **Decided:** egress is strictly **user-initiated** — only the explicit **Validate** click sends the one outbound probe; an inline egress help note (§5.1) names the exact destination, and "Save without validating" covers air-gapped users. No always-on toggle needed.

---

## 12. Resolved-Decision Designs

The §11 decisions, designed to an implementable level.

### 12.1 OAuth client-secret strategy (Q2) — PKCE/device-first + BYO fallback

Desktop apps cannot ship a confidential `client_secret`. Each tier-1 service is classified by the flow it supports as a **public client**:

| Service | Flow (public client) | Needs `client_secret`? | Strategy |
|---------|----------------------|------------------------|----------|
| GitHub | Device Authorization Grant | No | Built-in app (device flow) |
| Google | Auth Code + PKCE (loopback redirect) | No (PKCE) | Built-in app |
| Microsoft/Azure | Auth Code + PKCE | No (PKCE) | Built-in app |
| Slack | Auth Code | Yes (Slack requires secret) | **BYO** |
| AWS | IAM Identity Center (OIDC device) | No | Built-in app; else key/role |
| Anthropic / OpenAI | (API key only — no OAuth) | — | key path (§5) |

**Built-in path** — AgentMux ships a registered **public** OAuth client id per service (no secret). PKCE: backend generates `code_verifier`/`code_challenge`, opens the system browser, runs a transient `127.0.0.1:<rand>` redirect listener, captures `code`, exchanges with the verifier. Device flow (GitHub, AWS IC): backend requests a device+user code, UI shows the user code + verification URL, backend polls the token endpoint.

**BYO path** — for services that mandate a confidential secret (Slack), the Add-account flow offers **"Use my own OAuth app"**: the user pastes `client_id` + `client_secret` (obtained from the service's developer console; link provided inline). These BYO app credentials are themselves stored via the keychain backend (§12.2) — never in the DB plaintext — and used only to drive that account's token exchange. This keeps AgentMux free of any shipped secret while still supporting secret-mandatory services.

**Config shape** (extends `catalog.ts`):
```ts
oauth?: {
    flow: "pkce" | "device";
    clientId?: string;          // built-in public client id (omitted ⇒ BYO required)
    requiresBYOSecret?: boolean; // true ⇒ prompt for client_id + client_secret
    authUrl: string; tokenUrl: string; deviceUrl?: string;
    scopes: string[];
    redirect?: "loopback";       // pkce only
};
```
RPCs (`account.oauth.*`) carry an optional `byo: { clientId, clientSecretRef }`. The reused `AuthFlowController`/`AuthState` need no change — only the start payload differs.

### 12.2 Secret store: keychain + encrypted-file fallback (Q3)

`secret_store.rs` exposes `put(account_id, secret) -> SecretRef` / `get(SecretRef) -> Zeroizing<String>` / `delete`.

- **Primary** — `keyring` crate. Service string `"agentmux"`, account string `"acct:<account_id>"`. Maps to macOS Keychain, Windows Credential Manager (DPAPI), Linux Secret Service. Produces `SecretRef::Keychain { service, account }`.
- **Fallback** (Linux headless / CI / no Secret Service agent) — `SecretRef::EncryptedFile { path }`. AEAD (XChaCha20-Poly1305 via `ring`/`age`) over the secret; the file-encryption master key is itself stored in the keychain when available, else derived from a machine-bound seed (e.g. `machine-id` + install salt) — **never a hardcoded key**. Detection: probe Secret Service at startup; if absent, default new writes to `EncryptedFile` and log the downgrade once.
- **Migration of existing `plaintext_dev` rows** — on first read, opportunistically re-`put` through the keychain and rewrite the `SecretRef` (best-effort, behind a feature check).
- Reads return `Zeroizing<String>`; buffers zeroed on drop (`zeroize`).

### 12.3 v1 service catalog (Q4) — locked

Phase 2 (keys) + Phase 3 (OAuth) ship exactly these six. `catalog.ts` entries:

| id | displayName | authModes | OAuth | key validation probe | injects |
|----|-------------|-----------|-------|----------------------|---------|
| `github` | GitHub | `oauth`, `pat` | device, built-in | `GET api.github.com/user` (+ `X-OAuth-Scopes`) | `GITHUB_TOKEN`, `GH_TOKEN` |
| `google` | Google | `oauth` | pkce, built-in | `oauth2.googleapis.com/tokeninfo` | ADC file / `GOOGLE_OAUTH_TOKEN` |
| `aws` | AWS | `oauth`, `role`, `api_key` | device (Identity Center), built-in | STS `GetCallerIdentity` | `AWS_*` / profile |
| `openai` | OpenAI | `api_key` | — | `GET api.openai.com/v1/models` | `OPENAI_API_KEY` |
| `anthropic` | Anthropic | `api_key` | — | `GET api.anthropic.com/v1/models` | `ANTHROPIC_API_KEY` |
| `slack` | Slack | `oauth` (BYO) | authcode, BYO secret | `slack.com/api/auth.test` | `SLACK_BOT_TOKEN` |

Plus `custom` (free-form env var + optional generic bearer probe). Remaining tier-1 (Microsoft, Atlassian, Notion, Linear, Vercel, Cloudflare) are catalog-only stubs until Phase 4+.

### 12.4 "Used by" reverse index (Q5) — identities per account

Per account row, show **which identity bundles bind it**. Query the existing `db_identity_bindings` (`account_id → identity_id`). New RPC `ListAccountUsageCommand(account_id) -> { identities: {id, name}[] }` (or extend `ListIdentityAccountsCommand` to include a `used_by_identity_ids` array to avoid N calls). UI: "used by 2 identities" → hover/click lists them, each linking to the Identities tab. Generalizes `agentsAssignedToAccount` (`identity-model.ts:89-96`) from agents to bundles. Agent-level usage is derivable later via bundle→`AgentInstance.identity_id` but is out of scope for v1.

### 12.5 CLI-provider auth separation (Q6) & migration (Q7)

- **Q6** — concept A (CLI-provider login) stays in the agent pane; no Trust Center coupling in v1. A read-only "Connected CLIs" surface inside the Accounts tab is a Phase-4 nice-to-have, gated so it never lets the user *edit* CLI auth from here (avoids two write paths to the same on-disk tokens).
- **Q7** — `AgentDefinition.accounts` (deprecated JSON blob) is left as-is. Bindings already supersede it; no reader in the new code consumes it. A formal column drop is a separate cleanup, explicitly **not** in this work to avoid a schema migration on the critical path.

---

## 13. References

- Identity model (TS): `frontend/app/view/identity/identity-model.ts`
- Backend identity/memory storage (Rust): `agentmux-srv/src/backend/storage/identities.rs`, `memory_bundles.rs`
- OAuth session manager / patterns: `agentmux-srv/src/identity/auth_session.rs`, `auth_patterns.rs`
- Token expiry probe: `agentmux-srv/src/identity/resolver.rs`
- Auth state machine (reusable): `frontend/app/view/agent/auth/auth-state.ts`, `auth-flow-controller.ts`
- CLI provider catalog: `frontend/app/view/agent/providers/index.ts`
- Startup composition: `frontend/app/view/agent/startup/buildStartupPayload.ts`
- Hamburger menu: `frontend/app/window/hamburger-menu.tsx`
- Singleton modal pattern (to mirror): `frontend/app/modals/bundle-manager-modal.tsx`
- Block registry: `frontend/app/block/block.tsx:47-61`
- Prior specs: `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md`, `docs/specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md`
