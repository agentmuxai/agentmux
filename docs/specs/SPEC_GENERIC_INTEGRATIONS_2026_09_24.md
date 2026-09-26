# SPEC: Generic integrations — external services talk to agents through one authenticated interface, nothing hard-coded

**Date:** 2026-09-24
**Status:** proposed; nothing here is built. The research (§1) is measured
against `agentmux` `main` @ `236f5cab4`, `agentmux-cloud` `main` @
`fb93159`, and `a5af/reagent` `main`, all read on 2026-09-24.

**Revision history.** Two adversarial reviews on 2026-09-24; every finding
was verified against code and accepted.
- **Review 1** (3 P1, 9 P2, 4 P3):
  - the relay stops resolving agents; messages go to an **account inbox**
    and each desktop resolves them locally (§2.4, §2.5);
  - integration tokens are classified first and fenced (§2.2);
  - trust became desktop-local (§2.8);
  - scopes cover the head repo (§2.6).
- **Review 2** (2 P1, 7 P2, 13 P3):
  - a fresh login time can't prove a human, so grants now go through a
    **separate consent client and a relay-hosted consent page** (§2.2);
  - the desktop has no GitHub token to prove repo scopes, so v1
    integrations serve only **operator-approved accounts, with exact repo
    ids** (§2.6);
  - account routes moved to `/account/v1` (§2.4);
  - one notification carries a **list of candidate targets** and is
    delivered once per agent (§2.3, §2.4);
  - claims are atomic, and a desktop where the agent is live wins (§2.4);
  - integration rows are **never forwarded** (§2.4);
  - identity links are written only by a host-channel command (§2.5);
  - the relay rejects sender ids that aren't agent ids (§2.3);
  - residuals are listed (§4).
- **Review 3** (1 P1, 4 P2, 3 P3):
  - the host-gated window and the host channel protect against MCP tools,
    not against a same-user process, until GHSA-6726-q276-g6f6 is fixed.
    The trust toggle therefore ships only in builds that include that fix
    (§2.1, §2.5, §2.8, §4);
  - the consent flow was made buildable on Cognito: a confidential
    client, relay-side code exchange, relay-checked `auth_time` and `sub`,
    and the system browser only (§2.2);
  - holding integration rows gets its own entry point (§2.4);
  - claims live in a claims table keyed by canonical slug (§2.4);
  - the rollout gains a data source and real shadow rows (§3);
  - the source-id check covers every send route (§2.3);
  - `grants.write` is on its own resource server (§2.2);
  - revokes are announced (§2.2).

**Trigger:** the repo owner, 2026-09-24:
> "we need generic github integrations. nothing hard coded … ideally the
> agentmux interface is generic and reagent has the consumer/API rights
> with auth, so any service could come in later too. keep in mind
> 'connect to github' work that is in progress."

**Scope:**
- **`agentmux`:** the desktop srv, the MCP, and the frontend.
- **`agentmux-cloud`:** the muxbus relay, plus the GitHub consumer that
  currently lives in it.
- **External services, as clients:** ReAgent (`a5af/reagent`) first; the
  generic GitHub notifier; any later service.

**Related:**
- `SPEC_FIRST_CLASS_GITHUB_APP_AND_AWS_IDENTITY_2026_09_19.md` (#3413) —
  the Armory's GitHub App identity.
- `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` (#3649) — the WAN trust
  model, and the host-gated approval pattern reused in §2.2 and §2.8.
- `agentmux-cloud/docs/PLAN_REAGENT_CLOUD_SERVICE_2026_08_27.md` — linking
  a GitHub installation to a tenant.
- `SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md` — the shared legacy
  key and the flat agent namespace.
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §4.3 and §6 —
  the alias map, and PR-body `agentmux:agent_id=` tags, which must keep
  resolving.
- `SPEC_CONNECT_WITH_GITHUB_ARMORY_2026_09_23` is cited by
  `dev-tools/docs/specs/SPEC_GH_AGENT_CLI_2026_09_23.md:8-9` as "not yet
  merged", but could not be found on this machine, on any remote branch,
  or in any PR. I have asked its author (camper). §2.5 must be reconciled
  with it before this spec is accepted.

---

## 0. Summary

- **Today, a ReAgent- and GitHub-specific notification path is compiled
  into both products** (§1.4). It includes:
  - a GitHub consumer Lambda *inside* the relay's stack, holding the
    shared legacy relay key and a `reagent-v1` signing key it uses on every
    GitHub notice;
  - `reagent_*` fields stored on relay rows;
  - ReAgent public keys pinned in the desktop binary;
  - a ReAgent-only `SIG=` marker and tier rule.

  ReAgent itself never talks to AgentMux.
- **Target:** AgentMux exposes one **integration interface** that names no
  service.
  - An *integration* is a registered client with its own credential.
  - It may send only to accounts that granted it, through a human-only
    consent step, and that the operator approved for it in v1.
  - It may send only about the exact resources in that grant.
  - The relay **stamps** every message with the authenticated
    integration's identity and stores it in that account's **inbox**.
  - The account's desktops resolve the recipient locally and render the
    stamp generically.
  - Whether an integration's messages count as trusted is a local
    decision, made in a host window that MCP tools can't reach. Full
    isolation from agent processes depends on GHSA-6726-q276-g6f6 (§4).
- **Services own their logic.**
  - ReAgent notifies about its own reviews with its own credential.
  - Generic GitHub events (CI, merges, human reviews) come from a GitHub
    notifier: just another integration on the same public interface, with
    no privileged path.
  - Nothing about any service is compiled into the desktop or the relay.
- **"Connect to GitHub" supplies identities.** When a user connects GitHub
  in the Armory, the desktop learns which GitHub accounts (the user's, and
  each agent's App bot) belong to which agents. That local mapping
  replaces the consumer's hard-coded login map (§2.5).

---

## 1. Research

### 1.1 The current GitHub → agent notification path

1. **Ingress.** A GitHub webhook hits `github-router.asaf.cc`, which lives
   in `a5af/shared-infrastructure`, not the product. It checks the HMAC
   (`router.py:135-146`) and publishes `{event_type, delivery_id,
   payload}` to the SNS topic `github-webhooks` (`:51-104`). Webhooks are
   configured per repo or org and point at the router
   (`reagent/ADDING_REPOS.md:9-37`). Nobody has confirmed exactly which
   webhooks are configured
   (`SPEC_CI_COMPLETION_NOTIFICATIONS:150-155`; retro 2026-09-20).
2. **Consumer.** The Lambda `muxbus-github-consumer` is defined in the
   **relay's own CDK stack** (`agentmux-cloud/muxbus/infrastructure/lib/muxbus-stack.ts:240-305`)
   and subscribes to that topic. It handles five kinds of event
   (`consumers/github/handler.ts:631-772`):
   - merges (`events/merge.ts`);
   - reviews (`events/review.ts`): `[ReAgent] …`, `[Codex] …`, and human
     reviews;
   - CI failure (`ci-failure.ts`) and CI completion (`ci-complete.ts`),
     including an all-required-checks aggregate;
   - ReAgent replies (`issue-comment.ts`).

   It dedupes on the GitHub delivery id in DynamoDB (`:44-88`) and drops
   notifications older than 600 s (`:247-258`). It reads the GitHub API
   with `services/infra` → `github.token`, the root `a5af` PAT
   (`:110-116`).
3. **Addressing** (`agent-mapping.ts`). Resolution tries, in order:
   - a **hard-coded** login → agent map (`:30-59`);
   - regexes for fleet App logins (`agent{x,y,a-g,1-5}-workflow[bot]`,
     `agent{slot}-<host>`, `:69-125`);
   - the PR-body tag `<!-- agentmux:agent_id=X -->` (`:202, :221-238`).

   All three are gated on a **hard-coded** trusted-owner set, `{agentmuxai,
   agentmuxhq, a5af}` (`:148-153`). Reviews also notify the head commit's
   author (`handler.ts:349-374`).
4. **Relay.** The consumer calls `POST /reactive/inject` with the **shared
   legacy key** (`services/infra` → `muxbus-api-key`), `X-Agent-ID:
   github-consumer` and `priority: urgent`. It retries three times, then
   falls back to `/api/messages` (`handler.ts:260-347`).
   - It signs **every** notification it sends — CI, merges, human reviews
     and Codex, not only ReAgent's — with the Ed25519
     `reagent-jekt-signing-key` under key id `reagent-v1`, and sends four
     `reagent_*` fields (`:158-235`, `:323`). Removing the signature
     therefore affects every GitHub notice, not just ReAgent's (§2.8).
   - The relay stores those fields, and accepts them only from `legacy` +
     `github-consumer` (`server/src/reagent-fields.ts`, since
     agentmux-cloud#89).
   - Legacy auth has no account, so these rows sit in the **flat global
     agent namespace**.
5. **Desktop.** `cloud_subscriber.rs` pulls the row and verifies the
   `reagent_*` fields against public keys **pinned in the binary**
   (`agentmux-common/src/jekt_sign.rs`, `verify_trusted_reagent_jekt`
   since #3657). It sets `reagent_verified` and renders `SIG=verified` or
   `SIG=invalid`. A verified sender relaxes `ESCALATE` for
   sensitive-tier messages (`handler.rs` `is_cryptographically_verified`).

### 1.2 ReAgent (`a5af/reagent`)

- **Infrastructure.** AWS CDK (`infrastructure/lib/reagent-stack.ts`):
  - SNS consumers subscribed to the same `github-webhooks` topic, filtered
    to `pull_request`, `issues` and `issue_comment`;
  - an SQS queue (`maxReceiveCount: 1`);
  - a Docker Lambda, `claude-reviewer`, which runs the `claude` CLI
    against the PR (`lambdas/providers/claude.py:283-299`; the CLI runs
    with `--dangerously-skip-permissions` inside the Lambda);
  - a second reviewer Lambda, `kimi-reviewer`, which also posts reviews.
    Its event source is currently disabled (`reagent-stack.ts:225-347`);
  - a DynamoDB table `reagent-state` that is provisioned but unused.
- **It posts as its own GitHub App**, `reagentx-workflow[bot]`: it mints
  the installation token per repo (`utils.py:297-388`) and posts the
  review via `POST /pulls/{n}/reviews` (`reviewer_handler.py:975-1061`).
- **Codex.** It triggers Codex by posting `@codex review` with an **a5af
  PAT**, because Codex ignores bots (`reviewer_handler.py:1124-1158`,
  `codex_policy.py`).
- **Secrets.** Everything lives in the shared AWS secret `services/infra`.
- **ReAgent never calls muxbus.** It has no AgentMux credential, no jekt
  code, and no agent-id resolution. `specs/comment-reply-mode-2026-08-16.md:130-135`
  says outright that the muxbus consumer owns notifications.
- **What it already has at notify time.** In process it knows the repo,
  PR, head SHA, author, PR body, the verdict and structured findings
  before posting, the review URL, failure classifications, replies, and
  Codex decisions. That is richer than what the consumer rebuilds from
  webhooks.
- **What it lacks:**
  - author → agent resolution;
  - a relay client;
  - its own credential;
  - notification idempotency, since SQS retries plus the `finally` path
    can double-notify;
  - any concept of tenancy (hard-coded owners, a5af PATs).

### 1.3 In-progress "connect to GitHub" and related work

| Item | Status | What it says |
|---|---|---|
| `SPEC_FIRST_CLASS_GITHUB_APP_AND_AWS_IDENTITY_2026_09_19` (#3413) | proposed, nothing built | A GitHub App as a `ProviderClass::Minted` credential. Created by manifest flow; private key in the keychain; `(app, installation)` in `AccountContext`; the srv mints installation tokens at spawn as `GH_TOKEN`. Assumes **user-owned Apps**. **No webhooks or notifications.** |
| `SPEC_CONNECT_WITH_GITHUB_ARMORY_2026_09_23` | cited as "not yet merged"; **not found** | The Armory "Connect with GitHub" design, including a "bot-identity mode" that would hold the fleet's per-agent Apps (per the gh-agent spec). |
| `oauth_client.rs:88-97`, `oauth-catalog.ts:39-48`, issue #2115 | code present, no client id | A per-user GitHub OAuth-App device flow, token in the keychain; `builtIn: false`. |
| `SPEC_GITHUB_APP_IDENTITY_MIGRATION_2026_09_18` (shared-infrastructure) | Phases 0–1 done | Fleet per-agent Apps `agentN-workflow`, PEMs in `services/infra`, tokens minted by external tooling (gh-agent). |
| `SPEC_GH_AGENT_CLI_2026_09_23` (dev-tools) | built, interim | `gh-agent` mints App tokens for agents until the Armory design ships. |
| `PLAN_REAGENT_CLOUD_SERVICE_2026_08_27`, `PLAN_GROUNDSKEEPER_CLOUD_SERVICE_2026_08_27` (cloud) | not started | An installation id is not tenant isolation. Linking an installation to a tenant needs a signed state parameter through GitHub's setup redirect, plus a per-webhook ownership check. The consumer "is an internal-only relay, not an installable integration". |
| `MessagingBridge` (`agentmux-srv/src/messaging/mod.rs:132`) | shipped | Discord, Telegram, Slack and WhatsApp bridges inject locally as `source_agent: "discord"` etc. They are an integration-like concept, but desktop-local with no cloud credential. |

**Conflicts to settle:**
- **User-owned Apps vs one published App.** #3413 prefers user-owned Apps;
  the cloud plans want one installable App plus tenant linking.
- **OAuth App vs GitHub App.** #2115 and `oauth_client.rs` use an OAuth
  App; #3413 uses a GitHub App.
- **Where webhooks are configured** has never been measured.

This spec takes no side on how agents *act* on GitHub; that stays with
#3413 and the Armory spec. It needs only one thing from "connect to
GitHub": the **identities** it establishes (§2.5).

### 1.4 Inventory: everything ReAgent- or GitHub-specific (to remove)

**agentmux** (real identifiers, not comments):

| Where | What |
|---|---|
| `agentmux-common/src/jekt_sign.rs` | `reagent_public_key` (two pinned keys); `verify_reagent_jekt`, `verify_trusted_reagent_jekt`, `is_reagent_trusted_signing_key`; `signed_material` (the ReAgent format) |
| `agentmux-srv/src/muxbus/cloud_subscriber.rs` | `PendingInj.reagent_*`; `REAGENT_SIG_MAX_AGE_SECS`; the verification block |
| `agentmux-srv/src/server/reactive.rs` | `verify_reagent_signature` (HTTP path) and its tests; copies `reagent_verified` when forwarding |
| `agentmux-srv/src/server/websocket.rs:618, 665`, `server/jekt_held.rs:139` | `reagent_verified` passed through |
| `agentmux-srv/src/backend/reactive/types.rs` | `reagent_sig/key_id/msg_id/ts_secs`, `reagent_verified` |
| `agentmux-srv/src/backend/reactive/handler.rs` | the downgrade in `deliver_audited`; `reagent_verified` in the forcing rules and in `is_cryptographically_verified` |
| `agentmux-srv/src/backend/reactive/sanitize.rs` | the `SIG=` field |
| `agentmux-srv/src/backend/storage/jekt_held.rs`, `migrations.rs` | the stored `reagent_verified` column |
| `CLAUDE.md` (repo and workspace copies) | `SIG=verified` wording, "ReAgent-signed" |

**agentmux-cloud:**

| Where | What |
|---|---|
| `muxbus/server/src/index.ts`, `store.ts`, `reagent-fields.ts` | accepting and storing `reagent_*`; the `github-consumer` + legacy rule |
| `muxbus/packages/muxbus-client/src/client.ts` | forwarding `reagent_*` |
| `muxbus/consumers/github/**` | the whole consumer: ReAgent and Codex logins, `[ReAgent]`/`[Codex]` formatters, `issue-comment.ts`, signing, the hard-coded login map and trusted owners, the legacy key |
| `muxbus/infrastructure/lib/muxbus-stack.ts:240-305` | the consumer Lambda and its SNS subscription inside the relay stack |
| `muxbus/packages/muxbus-jekt` | `wrapJektMessage`, test-only; an unescaped marker builder |
| `muxbus/server/src/index.ts:385, 600` | the `startsWith('github')` special case that skips source normalisation |
| `muxbus/infrastructure/lib/constructs/muxbus-tables.ts:131` | the consumer-owned `muxbus-processed-github-events` dedupe table (moves with the notifier in I4) |

**Constraint:** PR-body `agentmux:agent_id=` tags already in GitHub must
keep resolving (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED` §6). Under this
design the service sends a tag's value as an `agent_id` target, and the
desktop resolves it through the alias map (§2.4); no hard-coded code is
involved.

---

## 2. Design

### 2.1 Principles

1. **One interface; no service names in either product.** Neither the
   relay nor the desktop has a code path, field, key, or string for any
   particular service, and none for GitHub either: resource scopes and
   identities are `<provider>:<…>` data.
2. **Identity comes from the credential.** The relay stamps who sent a
   message from the authenticated credential. A client can never assert
   its own identity, trust level, or the account it acts for.
3. **Agents hold the account's everyday credentials.** Every agent gets
   the user's cloud access token (`MUXBUS_TOKEN`,
   `server/muxbus_handlers.rs:339`) and the srv's `AGENTMUX_AUTH_KEY`,
   which reaches every `/ws` RPC. (Update 2026-09-26: #3881 stopped
   injecting `MUXBUS_TOKEN`. A same-user agent can still read the srv's
   stored login from disk, so the human-only paths below still apply.)
   - So anything that grants access, links identities, or sets trust needs
     a **human-only path**: a separate login client plus a consent page on
     the relay (§2.2), or a CEF-host window behind the host secret (§2.5,
     §2.8).
   - The routing decision — which agent receives a message — is made on
     the desktop (§2.4).
   - **Limit:** until GHSA-6726-q276-g6f6 is fixed, the host-gated
     window protects against MCP tools, not against a same-user process
     (§4). Anything whose misuse would relax a safety stop waits for that
     fix (§2.8).
4. **Accounts grant; integrations don't self-enroll.** In v1 the
   operator must also approve every account an integration may serve
   (§2.6).
5. **Services own their logic.** Deciding *what* to notify, and *whom*,
   belongs to the service.
6. **Nothing compiled in.** Adding, rotating, or revoking an integration
   is data, not a release.

### 2.2 Registry, credentials, grants, and the consent flow (cloud)

**Registry.** A new table, `muxbus-integrations-<env>`, keyed by
`integration_id`. Each entry holds:
- a display name;
- its Cognito app client id;
- the scopes it may request;
- `link_hosts`, an allow-list for links (§2.3);
- `allowed_accounts`, the accounts the operator approved for it (§2.6);
- `skip_actor_ids`, actors it must not report on (§2.9);
- a status (`active` or `suspended`).

Registration and `allowed_accounts` are operator actions.

**Integration credentials.**
- Each integration gets its own Cognito app client, authenticated with
  `client_credentials`, with scopes on a new resource server
  `muxbus-integrations`: `notify` and `grants.read`.
- Cognito access tokens carry no audience, and `verifyRequest`
  (`auth.ts:136-164`) accepts any token from the pool.
- **Classification is first and fails closed.** A token whose `client_id`
  is in the integration registry becomes `{mode: 'integration',
  integrationId, scopes}`, whatever its scopes, and never falls through
  to the M2M or user path.
- **Fences:**
  - The global `onRequest` hook (`index.ts:169-179`) accepts
    `mode: 'integration'` only on `/integrations/v1/*`, and only
    integration tokens are accepted there.
  - The WebSocket `$connect` Lambda (`ws-connect.ts:61`) doesn't go
    through `onRequest`, so it applies the same classification and
    refuses integration tokens.
  - The per-account M2M path (clients created in `agent-provisioning.ts`)
    must carry its own `https://muxbus.agentmux.ai/read|write` scopes.

The integration keeps its client secret in its own infrastructure;
ReAgent stores it in its own secret, not `services/infra`.

**Grants.** A new table, `muxbus-integration-grants-<env>`:
- **PK** `account_user_id`, **SK** `integration_id`.
- **Attributes:**
  - `grant_id`, a random handle;
  - `scopes`;
  - `resource_scopes`: **exact** resource ids such as `github:repo:123456`,
    with no wildcards;
  - `created_at`, `revoked_at`.

There is no `trusted` flag in the cloud (§2.8).

**The consent flow, human-only.** An agent's `MUXBUS_TOKEN` comes from the
same public `DesktopClient` (`muxbus-cognito.ts:272-330`) that a desktop
login uses. Refreshing keeps the original `auth_time`, and
`verifyRequest`'s user path ignores `client_id` (`auth.ts:170-182`). So
nothing about an ordinary account token proves a human is present.
Instead:

1. **The desktop creates a pending grant request** with its ordinary
   account token: `POST /account/v1/integration-grants/requests`, with
   `{integration_id, scopes, resource_scopes}`.
   - The relay stores the request with `created_at` and the requesting
     account.
   - It returns only an opaque `request_id`.
   - A pending request grants nothing.
2. **The desktop opens the relay's consent URL in the system browser.**
   Never in an AgentMux pane: panes share one cookie store
   (`browser_pane/creation.rs:192`) that agent tooling can drive.
   - **The relay generates the Cognito `state`** and keeps the mapping
     from `state` to `request_id` on the server.
   - **The consent app client is confidential** (`generateSecret: true`),
     and its only callback is on the relay. The relay exchanges the code
     server-side and **revokes the refresh token immediately**
     (`enableTokenRevocation`). Cognito always issues a refresh token for
     the authorization-code grant, so revocation is the control.
   - The client's only scope is `grants.write`, on its **own resource
     server**, which no integration or desktop client can ever be given.
   - Nothing parks a code or token where anyone can poll for it. The
     unauthenticated `/api/login-relay` park-and-poll is never used for
     consent.
3. **The relay checks the consent login itself:**
   - `auth_time ≥ request.created_at`, so the human logged in *for this
     request*, whatever the hosted UI's `prompt` handling does;
   - the consent token's `sub` equals the request's account. A Google
     identity and a native identity with the same email have different
     subs.
4. **The human confirms on the relay-hosted page.** It shows the
   integration and the exact repositories.
   - The confirm route is pre-auth, like `/desktop-callback`
     (`index.ts:176`), and protected by an HttpOnly session the relay set
     after the code exchange, plus a CSRF token.
   - The grant commits only on Confirm.
5. **Consent tokens never work as Bearer credentials.** The consent
   `client_id` is classified first and refused as a Bearer token on every
   route, including `$connect`.

**Re-authentication strength.** Both Cognito domains use the default
(classic) hosted UI (`muxbus-cognito.ts:218-239`). Moving to managed login
v2 is domain-wide, so it would also change the desktop login pages.
- **Native Cognito sign-in:** the relay's `auth_time` check forces a real
  login; passkeys are available on the ESSENTIALS tier (line 164).
- **Google-only owners:** re-authentication happens at Google, and is
  silent if the human is already signed in there.
- Per-client MFA isn't possible, because Cognito MFA is set per pool and
  doesn't apply to federated users.

The resulting residual is recorded in §4, and the choice is open question
4.

**Revoking** accepts any account token, because it only reduces access.
But an agent could use it to silently drop grants and so suppress
notices, so every revoke is audited and announced to the operator, and
§4 records it.

**How an integration learns its grants.** `GET /integrations/v1/grants`
returns, for the calling integration only, each active grant's
`grant_id` and `resource_scopes`, and nothing about the account.

### 2.3 Notify (cloud)

`POST /integrations/v1/notify`, authenticated with an integration token:

```json
{
  "grant_id": "g_…",
  "targets": [
    { "external": { "provider": "github", "id": "583231" }, "display": "octocat" },
    { "agent_id": "lark" }
  ],
  "message": "…",
  "kind": "review.completed",
  "priority": "normal" | "urgent",
  "subject": { "provider": "github", "resources": ["github:repo:123456", "github:repo:789012"], "ref": "pull/3664" },
  "links": [{ "label": "Review", "url": "https://github.com/…" }],
  "dedupe_key": "review:5304786248",
  "ttl_seconds": 1800
}
```

`targets` is an ordered list of **candidates** for one logical
notification: for a PR, its author, the head commit's author, and the
PR-body tag. The desktop delivers once per distinct agent the candidates
resolve to (§2.4), so one agent never receives the same notice twice.

**The relay:**
1. Authenticates the token: mode `integration`, scope `notify`.
2. **Checks the grant.** It loads the grant by `grant_id` and rejects the
   request unless:
   - the grant is active;
   - its `integration_id` equals the token's;
   - it has the `notify` scope;
   - its account is in the registry entry's `allowed_accounts`.
3. **Checks resources.** `subject` is required, and every entry in
   `subject.resources` must be **exactly equal** to one of the grant's
   `resource_scopes`.
   - For pull requests the integration lists both the base and head
     repositories, which keeps the fork gate
     (`agent-mapping.ts:135-158`) as data.
   - Listing the head repo is the integration's job; §4 notes that the
     integration is trusted to be honest about it.
4. **Validates everything, as data:**
   - `targets`: at most 8; each `agent_id` passes `validate_agent_id`,
     each `external` has `provider` and `id` from a safe charset;
     `display` is capped.
   - `links`: `https` only, host in `link_hosts`.
   - `kind`, `subject.ref` and the labels are charset- and size-capped.
   - The message is at most 10 KB.
5. **Deduplicates** on `(integration_id, grant_id, dedupe_key)`, answering
   200 with the existing id. The recipient list is part of the one row,
   so dedupe no longer needs the target (and never the display login).
6. **Charges quota per `(integration_id, account)` pair**, with an
   unbilled rate limit and monthly cap.
7. **Stores the row in the account inbox:** `inbox_account` = the grant's
   account, plus a GSI `inbox_account-created_at-index`. `target_agent`
   is not set, so the row never appears in `/reactive/pending/:agent_id`.
8. **Stamps the row** with `source_kind = integration`, `integration_id`
   and `integration_name`, all from the registry.
9. **Wakes only that account.** `ws-connect.ts:81` is changed to store
   `accountUserId ?? userId` for user tokens too, so an account-scoped
   wake (`broadcast.ts`) reaches the account's desktops.

**Also in I1: a source-id check on the relay.** Every route that creates
an injection from `X-Agent-ID` rejects a source that fails the agent-id
rule after the relay's own normalisation (lowercase, then
`validate_agent_id`'s charset). That covers `/reactive/inject`,
`/api/messages` (`index.ts:286-334`) and `/mcp` send (`index.ts:780`).
`github-consumer` is kept as-is until I5. With the check in place, no
agent-to-agent message can carry an `integration:` source (§2.7).

### 2.4 Delivery: account inbox, resolved and claimed on the desktop

**Pulling.**
- `GET /account/v1/integration-inbox?since=<cursor>` is authenticated with
  the desktop's account credential (a user token, or a per-agent M2M token
  with an owner). The caller's account is `accountUserId ?? userId`; an
  M2M client with no owner is refused.
- It returns rows created after the cursor, plus rows still pending.
- `POST /account/v1/integration-inbox/claim` and `/release` check
  `inbox_account`.
- **Claims.** `/reactive/ack` is a single-status update
  (`store.ts:365-372`), so it doesn't fit. Claims live in a
  `muxbus-integration-claims-<env>` table:
  - **PK** row id, **SK** the **canonical slug** of the resolved agent
    (after alias resolution, never a raw alias);
  - written with `attribute_not_exists`, which makes each claim atomic and
    lets one row be claimed once per distinct agent;
  - two desktops resolving through different aliases therefore collide on
    the same key.
- **Release** carries the claimant and the claim time, like
  `releaseInjection`.
- **"Pending"** means every row within its TTL: the relay never knows how
  many agents a row resolves to.

**Resolution.** For each row, each desktop resolves the candidates
**locally**, in order:
- **`agent_id`:** by slug or alias (identity spec §4.3), which keeps
  PR-body tags resolving permanently.
- **`external`:** through local external-identity links (§2.5), on
  `(provider, id)`.

It collects the distinct agents the candidates resolve to on this
desktop.

**Claiming, when agents are on several desktops.** For each resolved
agent:
- **Live here** (registered and present): claim at once and deliver.
- **Known here but not live:** wait a **grace period** (60 s) so a
  desktop where the agent is live can claim first. Then claim and hold it
  through a **separate hold entry point** for inbox rows (§2.8). Today's
  `hold_for_absent_target` only runs for full-key host-tier HTTP
  injections (`reactive.rs:1346-1358`).
- **Two desktops each have a live agent of that name** (two seats or two
  hosts): the atomic claim picks one. This is the same "one row, one
  delivery" behaviour as agent-to-agent WAN delivery today.

**Re-evaluation.** A desktop caches "no local match" for a row under the
current **resolution generation**, a counter that bumps whenever an
agent registers, an alias changes, or a link is added. A bump
re-evaluates every pending row, so a new link or agent picks up
notifications that are still pending.

**Unclaimed rows** expire at their TTL. A row with **no claims at
expiry** counts as `inbox.expired_unclaimed`, per integration.

**Integration rows are never forwarded** between srvs (channels, LAN
peers, same-host siblings). Each srv of the account pulls the inbox
itself, and a srv that can't resolve a row leaves it for the others. The
stamp therefore never has to cross an HTTP hop, where
`skip_deserializing` would drop it.

**Residual.** Any agent holding the account's token can pull, claim, or
release the whole account inbox. It could read other agents' notices, or
suppress them. This matches today's flat `/reactive/pending` exposure,
and is recorded in §4.

### 2.5 External-identity links — where "connect to GitHub" comes in

The desktop keeps a new table, **`db_external_identity_links`**
(`db_agent_identity_links` already exists, `migrations.rs:619`, for
provider accounts):
- `(provider, external_id)` → `agent_id` or `operator`;
- plus `display` and `source` (`armory`, `manual`, `seed`).

**Who writes links.** The Armory and Settings panes write over
`X-AuthKey` RPCs, which agents can reach. So:
- panes and agents can only **propose** a link;
- a link is written only by a **host-channel command**, gated on
  `AGENTMUX_HOST_REG_SECRET` (`server/service/credential.rs`,
  `host_ipc.rs`);
- that command runs after the human confirms in a CEF-host window.
  `UIClick` is limited to the caller's own pane
  (`ui_handlers.rs:295-298`), so MCP tools can't reach that window.
  Isolation from a same-user process depends on GHSA-6726-q276-g6f6
  (§4). Until that fix ships, a misused link can reroute notices but
  can't relax a stop.

Every change is audited and announced to the operator.

**Where links come from:**
- **"Connect to GitHub" in the Armory**, once #3413 or the Armory spec
  ships:
  - the user's own GitHub account, as `operator`;
  - each agent's GitHub App bot account.

  They are keyed by numeric id, which survives renames and covers the
  `login` vs `login[bot]` forms. Until those identities exist, links are
  manual or seeded.
- **Manual:** Settings → Integrations → *Linked identities*, through the
  same confirm window.
- **Seed** (§3 precondition): an operator tool resolves every login in the
  consumer's `GITHUB_TO_AGENT_MAP` (`agent-mapping.ts:30-59`), and the
  concrete `agent<slot>-<host>` logins the fleet uses, to numeric ids
  through the GitHub users API. The human then confirms the import once
  in the host window.

**Where `operator` targets go** is open question 5.

### 2.6 Tenancy: operator-approved accounts, exact scopes

**v1: the desktop's scope list can't be proven.** The desktop has no
owner GitHub token to prove repository access with: the OAuth client id
is `None` (`oauth_client.rs:91`, #2115), GitHub is an API-key provider in
the Armory (`provider.rs:52`), and #3413 isn't built. Cognito sign-up is
also open. So:
- An integration serves **only accounts in its registry
  `allowed_accounts`**, which the operator sets (our own account for
  ReAgent and `github-notifier`).
- Grants carry **exact repository ids**, no wildcards, confirmed by the
  human on the consent page.
- A stranger's account can't receive anything from an integration the
  operator hasn't approved for it.

**Later: the relay proves scopes itself.** Either:
- the installation-to-tenant link from `PLAN_REAGENT_CLOUD_SERVICE`: a
  signed `state` through GitHub's setup redirect, then the installation's
  repositories fetched server-side; or
- a GitHub user token the relay checks against the repositories.

Only then can `allowed_accounts` be dropped for that integration.

**Stale scopes.** A repository added after the grant isn't covered. The
relay counts rejected notifies per grant and resource, and the desktop's
Integrations window shows "N notices for repositories outside this grant",
with a button to extend it through the consent flow.

**PR-body tags** name an agent. Delivery happens only in accounts whose
grant covers the PR's repositories, and each account resolves the tag
against its own agents. That is correct, because each such account has
proven access to the repo (later) or was approved by the operator (v1).

### 2.7 Stamp, marker, and source naming

**Source naming.**
- An integration row's `source_agent` is `integration:<integration_id>`.
- `:` is not allowed in agent ids.
- The relay's source-id check (§2.3) stops agent-to-agent traffic from
  carrying it.
- **The desktop renders a bare `integration:<id>` only when the stamp is
  present.** Otherwise any `integration:`-shaped source renders as
  `?integration:<id>` (#3664 behaviour) with no reply hint.

**The desktop's stamp.** `PendingInboxRow` carries the stamp.
`InjectionRequest` gains `integration: Option<IntegrationStamp>`, marked
`#[serde(skip_deserializing)]`; it is set only from a pulled inbox row.

**Marker:**

```
[JEKT:FROM=integration:reagent VIA=integration TO=lark KIND=review.completed DELIVERY=wan TRUST=integration …]
```

- Every field goes through `marker_field`.
- Subject and links render as data lines in the body.
- The reply hint reads `Reply: not available — integration`.

**Frontend.** `stream-parser.ts` learns `VIA` and `KIND`, and `JektTrust`
gains `integration`. `stripJektEnvelope` stops relying on fixed header
offsets.

**CLAUDE.md** gains the `VIA=integration` and `TRUST=integration` wording
in I2.

### 2.8 Trust, held delivery, and removal

**Trust.** Integration identity is **cloud-attested**. That is the trust
anchor the pinned ReAgent key really had, because its signing key lived
in the cloud's own secrets (§1.1).

Whether an integration's messages get the verified-sender treatment
(`ESCALATE=none` for sensitive-tier content, except where
`transcript_request_escalate_forced` applies, `handler.rs:1292-1293`) is
a **desktop-local** setting, `trusted_integrations`:
- It is written only by the host-channel command after confirmation in
  the CEF-host window (§2.5).
- It is **off by default**.
- The relay has no say in it.
- **It ships enabled only in builds that include the fix for
  GHSA-6726-q276-g6f6.** Until then the host-gated path doesn't isolate
  it from a same-user process (§4), and turning it on would relax a
  safety stop. In earlier builds no integration is ever trusted.

**Sender trust is not content trust.** A trusted integration's notice
can quote attacker-written PR text. Trusting an integration relaxes the
*stop*; it does not make the content safe to act on. The Integrations
window says so next to the toggle.

Removing `reagent-v1` signing (I5) drops the relaxation for **every**
GitHub notice, because the consumer signs them all (§1.1). An operator who
wants it back marks `github-notifier` or `reagent` as trusted.

**Held delivery.** Inbox rows get their own hold entry point. `db_jekt_held`
gains, as additive columns:
- the **stored delivery tier**;
- the stamp: `integration_id`, `integration_name`, `kind`, and the
  subject and links as JSON.

`request_from_held` restores the stored tier instead of hard-coding
`"host"` (`jekt_held.rs:131`), so a replayed notice is never treated as
more trusted than WAN. Replay keeps `VIA=integration` and re-reads the
trust setting at delivery time.

A held row expires at the earlier of the row's own expiry and
`HELD_TTL_MS`.

**Known limit:** once a desktop has claimed and held a notice, it stays
there. If the agent comes back on a different desktop, it won't get the
notice.

There is no forwarding (§2.4).

**Removed in I5:**
- the pinned keys and the `verify_*reagent*` functions;
- `REAGENT_SIG_MAX_AGE_SECS` and the `SIG=` field;
- the handler downgrade;
- the `reagent_*` fields in `PendingInj`, `InjectionRequest`, websocket
  and forwarding;
- the held `reagent_verified` column (no longer read or written);
- `reagent-fields.ts` and the relay's `reagent_*` storage;
- muxbus-client forwarding;
- the `startsWith('github')` special case;
- the CLAUDE.md `SIG=` wording.

### 2.9 Where the services live

**ReAgent.** A notify step after it posts a review, a reply, a failure, or
a Codex request, in **both** reviewer Lambdas (`claude-reviewer` and
`kimi-reviewer`).
- **Fan-out:** one request per grant whose scopes cover the PR's base and
  head repositories.
- **Candidates,** in order: the PR-body tag as `agent_id`, the PR author,
  and the head commit's author.
- **`dedupe_key`:** the review id, the comment id, or `codex:<sha>`.

It must never change the review's outcome:
- It runs in **its own `try`** after posting, and never raises.
  Otherwise an exception after posting reaches the `except` that posts a
  failure review (`reviewer_handler.py:1916-1952`).
- Its retries fit a **time budget** taken from the Lambda's remaining
  time (`context.get_remaining_time_in_millis()`), leaving a margin. A
  timeout would fail the invocation, and with `maxReceiveCount: 1` send it
  to the DLQ.
- It ships **off by default** behind a config flag.

**The GitHub notifier.** An integration, `github-notifier`, built from the
generic parts of today's consumer:
- merges, CI failure and completion, human reviews;
- Codex reviews, until open question 9 decides.

It has its own credential, and it **leaves the relay's CDK stack**, with
its dedupe table (open question 7). It:
- keeps its GitHub logic;
- drops the hard-coded login map and trusted owners;
- sends numeric-id candidates;
- drops its ReAgent-specific branches.

**Not double-reporting ReAgent.** Without those branches, ReAgent's
reviews would look like human reviews to `review.ts:96-103`. So the
notifier skips actors listed in its registry entry's `skip_actor_ids`
(ReAgent's bot id): data, not code.

**The Codex re-review hint** (`review.ts:220`) moves with whichever
integration sends Codex notices.

**Local messaging bridges** stay desktop-local (open question 8).

---

## 3. Migration and rollout

Each step keeps notifications flowing; no step needs a flag day.

| Step | Repo | What ships | During the transition |
|---|---|---|---|
| **I1** | cloud | fail-closed integration classification; route and `$connect` fences; scope checks on the M2M path; the relay's source-id check; registry, grants, pending requests and the consent client and page; `notify`, `grants`, and the `/account/v1` inbox routes with atomic claim and `since` cursor; account-scoped wake (`ws-connect` stores the user token's account); quota per `(integration, account)`; `inbox.expired_unclaimed`, no-grant and out-of-scope counters | the consumer still runs; nothing changes for desktops |
| **I2** | agentmux | inbox pull; local resolution with generations; grace-period claiming; external-identity links (proposal plus host-gated confirm, seeding import); the Integrations CEF-host window (grant requests, trusted integrations, linked identities, out-of-scope notices); stamp rendering, parser, CLAUDE.md; held stamp columns | desktops that haven't upgraded don't pull the inbox; they still get consumer notices until I3 and I4 |
| **I3** | reagent | the notify step (both Lambdas), off by default | ReAgent's notify flag and the consumer's ReAgent-branch flag are **flipped together as config** |
| **I4** | notifier's new home + cloud | `github-notifier` (with `skip_actor_ids`), off by default; then the consumer Lambda, its SNS subscription and its dedupe table are removed from the relay stack | the same flag-flip approach |
| **I5** | agentmux + cloud | removal (§2.8); the legacy key retired once no legacy callers remain (`DISABLE_LEGACY_AUTH`) | desktops older than I2 get no GitHub notices: announced (below) |

**Preconditions for I3 and I4.** These have to be true before either
flag is flipped:
- **Grants.** Identify the accounts that receive these notices today.
  Existing data can't answer that: desktops pull with user tokens,
  `checkAgentBinding` returns early without `accountUserId`
  (`agent-binding.ts:31-33`), and the pending route logs nothing.
  - **I1 therefore counts `(sub, agent_id, auth mode)` on
    `/reactive/pending`,** for at least one release.
  - From those counts, list the subs, including Google and native
    duplicates. Move any legacy-key pullers to Cognito, since they have no
    account.
  - Add the accounts to `allowed_accounts`, and grant through the consent
    flow.
- **Desktops.** Every desktop running a notified agent is on I2 or later,
  with its links seeded and confirmed.
- **Dry run.** Integrations send **shadow rows**, flagged as such. Desktops
  resolve them and report "would deliver" without claiming or delivering.
  Those reports must match the consumer's deliveries for the same events.
- **Rollback.** Flip the flags back.

**Old desktops.** After I4 a desktop older than I2 receives no GitHub
notices. This is announced in the I2 release notes, and I4 waits one
release cycle after I2.

---

## 4. Residuals and open questions

**Residuals:**
- **Inbox exposure.** Any agent holding the account's token can read,
  claim, or release the account inbox (§2.4). This is the same as today's
  flat pending exposure. Narrowing it needs per-agent inbox credentials,
  which is future work.
- **The fork gate relies on the integration.** The relay can only check
  the resources the integration lists (§2.3). A dishonest or buggy
  integration could omit a fork's head repo. v1 integrations are
  operator-approved, so this is accepted.
- **Re-authentication strength.** The consent flow needs the human's own
  login, but for Google-only owners that login is silent if they are
  already signed in at Google (§2.2).
- **Same-user processes and GHSA-6726-q276-g6f6.** Until that advisory's
  fix ships:
  - the host-gated window and host channel protect against MCP tools, not
    against a process running as the same OS user;
  - external-identity links can be rewritten by such a process, which
    reroutes notices;
  - `trusted_integrations` stays disabled (§2.8), so no safety stop can be
    relaxed this way.
- **Revoke as suppression.** Any account token can revoke a grant, and so
  suppress an integration's notices. Revokes are audited and announced
  (§2.2).

**Open questions:**
1. **Registration:** operator-only now, a developer console later?
   (Recommended: operator-only.)
2. **User-owned App vs one published App**, for "connect to GitHub" (§1.3
   conflict).
3. **`SPEC_CONNECT_WITH_GITHUB_ARMORY_2026_09_23`**: find it and reconcile
   §2.5.
4. **Consent re-authentication:** move the domains to managed login v2,
   which changes the desktop login pages too? And restrict the consent
   client to native sign-in with passkeys, or accept weak re-auth for
   Google-only owners?
5. **Where an `operator` target goes:** the default agent, every agent
   watching the subject, or a desktop notification (#3662)?
6. **Replies to integrations.** Out of scope for v1.
7. **Home for `github-notifier`**: `shared-infrastructure` next to
   `github-router`, or its own repo?
8. **Local bridges:** fold Discord, Telegram, Slack and WhatsApp into the
   same model?
9. **Codex notifications:** `github-notifier`, or ReAgent's Codex worker
   once `codex-review-gate-in-reagent` ships?

## 5. Tests (summary)

**Cloud:**
- Classification and fencing:
  - a registry `client_id` is classified as an integration even without a
    scope;
  - integration tokens are rejected on every non-`/integrations/v1`
    route and on `$connect`;
  - account tokens are rejected on `/integrations/v1`;
  - an M2M token without its read or write scope is rejected;
  - a non-agent-id source is rejected on `/reactive/inject`,
    `/api/messages` and `/mcp` send.
- Grants and consent:
  - a pending request grants nothing;
  - the consent `client_id` is refused as a Bearer token on every route,
    including `$connect`;
  - the refresh token is revoked after the code exchange;
  - the confirm route needs the relay session plus CSRF;
  - a commit is refused when `auth_time` is older than the request, or when
    `sub` differs from the requesting account;
  - a grant commits only on Confirm;
  - revokes are announced;
  - notify is rejected for an account not in `allowed_accounts`.
- Notify:
  - a resource that isn't an exact scope match (including a fork head
    repo) → 403;
  - a non-`https` link, or a host not in `link_hosts` → 400;
  - more than 8 targets → 400;
  - a duplicate `dedupe_key` → the same id.
- Inbox:
  - a claim is atomic, per `(row, canonical slug)`, and two aliases of one
    agent collide;
  - only the owning account can pull, claim or release;
  - inbox rows never appear in `/reactive/pending`;
  - the account-scoped wake reaches the account's desktops.

**Desktop:**
- Resolution and claiming:
  - candidates resolve by tag, alias, or numeric-id link;
  - two candidates that resolve to the same agent deliver once;
  - a live-agent desktop claims before a holding desktop within the grace
    period;
  - a generation bump re-evaluates pending rows;
  - integration rows are never forwarded.
- Protection:
  - links and trusted integrations reject every caller except the host
    channel;
  - `integration` can't be deserialised from HTTP.
- Rendering:
  - a bare `integration:` source renders only with the stamp;
  - the marker and data lines go through `marker_field`.
- Trust:
  - trusted plus a sensitive message → `ESCALATE=none`;
  - a forced transcript request escalates.
- Held replay keeps the stamp and the stored tier (never `host`), and
  expires at the earlier of the row's expiry and the held TTL.
- Before the GHSA-6726-q276-g6f6 fix, `trusted_integrations` can't be
  enabled.
- After I5, `git grep` finds no ReAgent identifiers in code, as a CI
  check.

**ReAgent:**
- A notify failure or timeout never produces a failure review or a DLQ
  entry.
- Both Lambdas notify.
- Fan-out follows grants; candidate order is tag, author, commit author.

**End to end:**
- A review on a granted repo reaches the linked agent once, with
  `VIA=integration`.
- A fork PR from an ungranted owner produces no row.
- An account outside `allowed_accounts` receives nothing.
- A second account's desktop never sees the row.
