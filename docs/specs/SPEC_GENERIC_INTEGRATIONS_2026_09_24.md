# SPEC: Generic integrations — external services talk to agents through one authenticated interface, nothing hard-coded

**Date:** 2026-09-24
**Status:** proposed; nothing here is built. The research (§1) is measured
against `agentmux` `main` @ `236f5cab4`, `agentmux-cloud` `main` @
`fb93159`, and `a5af/reagent` `main`, all read on 2026-09-24. An
adversarial review is pending.
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
  the Armory's GitHub App identity (`ProviderClass::Minted`).
- `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` (#3649) — the WAN trust
  model that integrations sit beside.
- `agentmux-cloud/docs/PLAN_REAGENT_CLOUD_SERVICE_2026_08_27.md` — linking
  a GitHub installation to a tenant.
- `SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md` — the shared legacy
  key and the flat agent namespace.
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6 — PR-body
  `agentmux:agent_id=` tags must keep resolving.
- `SPEC_CONNECT_WITH_GITHUB_ARMORY_2026_09_23` is cited by
  `dev-tools/docs/specs/SPEC_GH_AGENT_CLI_2026_09_23.md:8-9` as "not yet
  merged", but could not be found on this machine, on any remote branch,
  or in any PR. I have asked its author (camper). §2.7 and §4 must be
  reconciled with it before this spec is accepted.

---

## 0. Summary

- **Today, a ReAgent- and GitHub-specific notification path is compiled
  into both products** (§1.4). It includes:
  - a GitHub consumer Lambda *inside* the relay's stack, holding the
    shared legacy relay key and a `reagent-v1` signing key;
  - `reagent_*` fields stored on relay rows;
  - ReAgent public keys pinned in the desktop binary;
  - a ReAgent-only `SIG=` marker and tier rule.

  ReAgent itself never talks to AgentMux. Agents hear about its reviews
  only because the consumer reacts to the GitHub events those reviews
  produce.
- **Target:** AgentMux exposes one **integration interface** that names no
  service. An *integration* is a registered client with its own
  credential. It sends notifications only to accounts that granted it,
  within the scopes they granted. The relay **stamps** every row with the
  authenticated integration's identity; nothing is taken from the
  request. The desktop renders that stamp generically.
- **Services own their logic.** ReAgent notifies about its own reviews
  with its own credential. Generic GitHub events (CI, merges, human
  reviews) come from a GitHub notifier, which is just another integration
  on the same public interface, with no privileged path. Nothing about
  any service is compiled into the desktop or the relay.
- **"Connect to GitHub" fits as identity linking.** When a user connects
  GitHub in the Armory, the account learns which GitHub logins (the
  user's, and each agent's App bot login) belong to which agents. That
  mapping replaces the consumer's hard-coded login map (§2.5).

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
   - It signs with the Ed25519 `reagent-jekt-signing-key` under key id
     `reagent-v1` and sends four `reagent_*` fields (`:158-235`).
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

**Constraint:** PR-body `agentmux:agent_id=` tags already in GitHub must
keep resolving (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED` §6). Under this
design they resolve in the integration's request (§2.4), not in any
hard-coded code.

---

## 2. Design

### 2.1 Principles

1. **One interface; no service names in either product.** Neither the
   relay nor the desktop has a code path, field, key, or string for any
   particular service.
2. **Identity comes from the credential.** The relay stamps who sent a
   row from the authenticated credential. A client can never assert its
   own identity, its trust level, or the account it is acting for.
3. **Accounts grant; integrations don't self-enroll.** An integration can
   reach an account only after the account owner grants it, with scopes.
4. **Services own their logic.** Deciding *what* to notify, and *whom*
   (a PR author, a reviewer), belongs to the service. AgentMux offers
   generic ways to address a target (§2.4).
5. **Nothing compiled in.** Adding, rotating, or revoking an integration
   is data, not a release.

### 2.2 Integration registry and credentials (cloud)

**Registry.** A new table, `muxbus-integrations-<env>`, keyed by
`integration_id`, a slug such as `reagent` or `github-notifier`. Each
entry holds:
- a display name and a homepage;
- the owner account;
- the scopes it may request;
- its Cognito app client id;
- a status (`active` or `suspended`).

Registration is an operator action for now (§4, open question 1). A
self-serve developer console is out of scope.

**Credentials.** Each integration gets its **own Cognito app client**,
authenticated with `client_credentials`, issuing scopes on a new resource
server `muxbus-integrations`:
- `notify` — send notifications;
- `identities.read` — resolve external identities (§2.5), optional.

This is the same machinery as the existing per-account M2M clients
(`auth.ts:111-124`), with a new token audience and scope. There is no new
secret type. The integration keeps its client secret in its own
infrastructure; ReAgent would store it in its own secret, not
`services/infra`. `verifyRequest` gains a third result shape,
`{mode: 'integration', integrationId, scopes}`. A user token or an agent's
M2M token can never produce it.

**Grants.** A new table, `muxbus-integration-grants-<env>`:
- **PK** `account_user_id`, **SK** `integration_id`.
- **Attributes:**
  - `scopes`;
  - `targets` — which agents it may reach: `all`, or an allow-list;
  - `external_scopes` — for GitHub, the repos or owners it may notify
    about (§2.6);
  - `trusted` — see §2.8;
  - `created_at`, `revoked_at`.

A grant is created by the account owner in AgentMux Settings →
Integrations, through a consent screen listing the scopes. It is revoked
in the same place. Only a **user token** of that account may create or
revoke a grant, never an agent M2M token and never an integration. The
step-up question from the WAN spec (agents hold the user token) applies;
see open question 4.

### 2.3 The notify interface (cloud)

`POST /integrations/v1/notify`, authenticated with an integration token:

```json
{
  "account": "<grant handle — see below>",
  "target": { "agent_id": "lark" } | { "external": { "provider": "github", "login": "octocat" } },
  "message": "…",
  "kind": "review.completed",
  "priority": "normal" | "urgent",
  "subject": { "provider": "github", "repo": "agentmuxai/agentmux", "ref": "pull/3664" },
  "links": [{ "label": "Review", "url": "https://github.com/…" }],
  "dedupe_key": "reagent:review:5304786248",
  "ttl_seconds": 1800
}
```

**The relay:**
1. Authenticates the token as an integration with scope `notify`.
2. Resolves the grant for `(account, integration)`. The `account` field is
   an opaque **grant handle** issued at consent time, not a Cognito
   `sub`. It rejects the request if there is no active grant.
3. Enforces the grant's `targets` and `external_scopes`.
4. Resolves `target` within that account (§2.4). No match means 404, and
   nothing is stored.
5. Deduplicates on `(integration_id, dedupe_key)` with a conditional
   write, answering 200 with the existing id.
6. Stores the row **account-scoped**: `target_account` plus a GSI on
   `(target_account, target_agent)`. Integration rows never enter the flat
   global namespace, and only a caller of that account can pull them
   (§2.7).
7. **Stamps** the row with `source_kind = integration`, `integration_id`,
   `integration_name` (from the registry), and the grant's `trusted` flag.
   `kind`, `subject` and `links` are stored as data, size-capped and
   validated. The message is capped at 10 KB like any jekt.

The existing `/reactive/inject` keeps serving agent-to-agent traffic.
**The `reagent_*` fields and the legacy `github-consumer` rule are
removed** (§3).

### 2.4 Addressing

The target of a notification is always an agent **in the granting
account**. There are two forms:
- **`agent_id`** — an explicit address, used when the service knows it,
  e.g. from a PR-body `agentmux:agent_id=` tag. Validated as an agent id
  (`validate_agent_id`) and resolved within the account's agents, by slug
  or by alias (per the identity spec's alias map).
- **`external`** — an identity in another system, e.g. `{provider:
  "github", login}`. Resolved through the account's **identity links**
  (§2.5). An unlinked login matches nothing.

The service decides whom to notify. ReAgent, for instance, sends one
request per recipient: the PR author, and the head commit's author when
they differ. That is ReAgent's logic, not the relay's.

### 2.5 Identity links — where "connect to GitHub" comes in

A new table, `muxbus-identity-links-<env>`:
- **PK** `account_user_id`, **SK** `<provider>#<external_id>`.
- **Attributes:** `agent_id`, or `*` for "the human operator" (see below);
  `display`; `source` (`armory`, `manual`); `created_at`.

**Who writes links:** only the account's own srv, using the user token,
from two sources:
- **Armory, when the user connects GitHub** (§1.3). The srv publishes the
  link for:
  - the user's own GitHub login;
  - each agent's GitHub App bot login (`<app-slug>[bot]`), taken from the
    agent's Minted GitHub App account in #3413, or from its fleet App.

  This replaces `agent-mapping.ts`'s hard-coded map and regexes.
- **Manual:** Settings → Integrations → "GitHub logins", for identities the
  Armory doesn't manage.

A link names an agent only within the account, so two accounts can link
the same GitHub login to their own agents. Each integration resolves only
within the account that granted it. **The human operator's own login**
maps to `*`, meaning "this account's default agent" or "every agent
subscribed to that subject" (open question 5).

### 2.6 Tenancy for GitHub-sourced integrations

A GitHub event names a repo, not an AgentMux account. Before an
integration notifies an account about repo R, the account must have
granted it R. That is the grant's `external_scopes`, e.g.
`github:agentmuxai/*` or `github:agentmuxai/agentmux`, and the relay
enforces it against `subject.repo`.

**How R gets into the grant.**
- **v1 (our own deployment):** the account owner lists the repos or owners
  on the consent screen.
- **Later (a published, installable ReAgent):** the GitHub App
  installation-to-tenant link from `PLAN_REAGENT_CLOUD_SERVICE`, via a
  signed `state` through GitHub's setup redirect. It populates
  `external_scopes` from the installation's repositories.

This replaces the hard-coded `TRUSTED_REPO_OWNERS`.

### 2.7 Delivery (cloud → desktop)

`GET /reactive/pending/:agent_id` returns integration rows only when
**the caller's account equals the row's `target_account`**. Each such row
carries the stamp: `source_kind`, `integration_id`, `integration_name`,
`trusted`, `kind`, `subject`, and `links`. The stamp is computed by the
relay; a client can't write it. Agent-to-agent rows keep today's
behaviour, and 09-17's W2 stays their fix.

### 2.8 Desktop rendering and trust

- `PendingInj` gains the stamp as optional fields. **The `reagent_*`
  fields go.**
- `InjectionRequest` gains `integration: Option<IntegrationStamp>`,
  `#[serde(skip_deserializing)]`, so it can only ever be set from a
  relay-pulled row.
- **Marker.** When the stamp is present, the marker renders
  `FROM=<integration_id> VIA=integration DELIVERY=wan
  TRUST=integration`.
  - `FROM` is the registry slug, which cannot collide with an agent,
    because `VIA` distinguishes it. It still passes through
    `marker_field`.
  - `kind` renders as `KIND=`, and the subject and links go in the body
    header.
  - The reply hint reads `Reply: not available — integration`. Replying
    to an integration is out of scope (open question 6).
- **Trust.** The relay authenticated the integration, so identity is
  **cloud-attested**, not signed end to end. That is the same trust
  anchor the pinned ReAgent key really had, because the signing key lived
  in the cloud's own secrets (§1.1).
  - The relaxation that `SIG=verified` gave today becomes a
    **per-grant, human-set `trusted` flag**. It is off by default.
  - When `trusted` is on, a sensitive-tier message from that integration
    gets the verified-sender treatment (`ESCALATE=none`), except for
    transcript requests, whose rules are unchanged.
  - When it is off, the message is treated like any unverified WAN
    sender.
- **Removed:** the pinned keys, `verify_*reagent*`, `REAGENT_SIG_MAX_AGE_SECS`,
  the `SIG=` field, the held-row `reagent_verified` column (additive
  migration: stop reading and writing it), the handler downgrade, and
  forwarding `reagent_verified`.

### 2.9 Where the services live

- **ReAgent** adds a notify step after it posts a review, a reply, a
  failure, or a Codex request, using its own credential. It sends:
  - `kind`: `review.completed`, `review.failed`, `reply.posted`, or
    `codex.requested`;
  - `dedupe_key`: the review id, the comment id, or `codex:<sha>`;
  - one `external` or `agent_id` target per recipient.

  It stores its client secret in its own secret. The tenancy of a
  published ReAgent is its own concern, following §2.6.
- **The GitHub notifier.** The generic parts of today's consumer — merges,
  CI failure and completion, human reviews, and Codex reviews until Codex
  has a notifier of its own — become an integration, `github-notifier`,
  with its own credential. It **leaves the relay's CDK stack**. It could
  sit next to `github-router` in `shared-infrastructure`, or in its own
  repo (open question 7). It uses only the public interface.
  - It keeps its GitHub-side logic: event parsing, CI aggregation, and
    GitHub dedupe.
  - It drops the hard-coded login map and trusted owners; identity links
    and grants replace them.
  - It drops the ReAgent-specific branches (`[ReAgent]` formatting,
    `issue-comment.ts`), because ReAgent now notifies for itself.
- **Local messaging bridges** (Discord and the others) stay desktop-local.
  They could later share the same marker rendering (`VIA=integration`),
  but they need no cloud credential (open question 8).

---

## 3. Migration and rollout

Each step keeps notifications flowing; no step needs a flag day.

| Step | Repo | What ships | During the transition |
|---|---|---|---|
| **I1** | cloud | registry, grants and identity-links tables; the integration auth mode; `POST /integrations/v1/notify`; account-scoped rows and the pending filter; the stamp in pending | the consumer still runs; nothing changes for desktops |
| **I2** | agentmux | desktop renders the stamp (`VIA=integration`, `TRUST=integration`, per-grant `trusted`); Settings → Integrations (grants, consent, trusted toggle, GitHub logins); Armory publishes identity links when GitHub is connected | older desktops ignore the stamp and render these rows as `FROM=<slug>`, network-claimed and unsigned: degraded, not broken |
| **I3** | reagent | notify step, with its own credential | the consumer's ReAgent branches are switched off at the same time (an env flag in the consumer), so nothing is sent twice; the dedupe keys differ, so the two paths must not both run |
| **I4** | shared-infrastructure (or a new repo) + cloud | `github-notifier` as an integration; the consumer Lambda and its SNS subscription are removed from the relay stack | a short overlap is covered by the notifier's `dedupe_key` = the GitHub delivery id |
| **I5** | agentmux + cloud | removal: the `reagent_*` fields, the pinned keys, the `SIG=` rule, `reagent-fields.ts`, and muxbus-client forwarding; CLAUDE.md updated; the legacy key retired once nothing uses it (`DISABLE_LEGACY_AUTH`) | desktops older than I2 lose `SIG=verified` on review notices; they become network-claimed, which is safe |

**Ordering constraints:**
- **I5 comes after I3 and I4.** Removing signing before ReAgent and the
  notifier move would leave review notices unsigned but still flowing.
  That is acceptable, but pointless to do early.
- **Grants for our own account must exist before I3 and I4.** They are
  created by hand in Settings once I2 ships, or seeded by an operator
  script during I1.

---

## 4. Open questions

1. **Registration.** Operator-only registration now; a developer console
   later? (Recommended: operator-only.)
2. **User-owned App vs one published App**, for "connect to GitHub" (§1.3
   conflict). This spec needs only identity links, so either works, but
   the Armory spec must decide.
3. **`SPEC_CONNECT_WITH_GITHUB_ARMORY_2026_09_23`**: find it and reconcile
   §2.5 with its bot-identity mode.
4. **Grant consent and agents.** Every agent holds the account's user
   token (`MUXBUS_TOKEN`), so an agent could create a grant. Options: gate
   grant writes behind step-up auth, or through the host-gated approval
   window from the WAN spec (§2.6 there). (Recommended: the host-gated
   window, the same pattern as instance approval.)
5. **Where a human-login target goes:** the default agent, every agent
   watching the subject, or the human's mailbox?
6. **Replies to integrations** (bidirectional), e.g. an agent answering
   ReAgent. Out of scope for v1.
7. **Home for `github-notifier`**: `shared-infrastructure` next to
   `github-router`, or its own repo?
8. **Local bridges:** fold Discord, Telegram, Slack and WhatsApp into the
   same model (a local registry, the `VIA=integration` marker)?
9. **Codex notifications.** Keep them in `github-notifier` (they come from
   Codex's GitHub reviews), or move them into ReAgent's Codex worker once
   `codex-review-gate-in-reagent` ships?

## 5. Tests (summary)

**Cloud:**
- Auth:
  - integration tokens produce `mode: 'integration'`, and user or M2M
    tokens never do;
  - notify without a grant → 403;
  - a target outside the grant → 403;
  - a repo outside `external_scopes` → 403;
  - an unlinked external login → 404, with nothing stored.
- Behaviour:
  - a duplicate `dedupe_key` → the same id;
  - pending returns integration rows only to the target account;
  - the stamp can't be set by a client.
- Removal: `reagent_*` fields are ignored after I5.

**Desktop:**
- Marker rendering of the stamp, through `marker_field`.
- The `trusted` flag:
  - on → a sensitive message gets `ESCALATE=none`;
  - off → escalates;
  - transcript requests always escalate.
- `integration` can't be deserialised from HTTP.
- Settings grant flow; the Armory publishes identity links.
- After I5, no `reagent`, `REAGENT_` or `SIG=` identifiers are left in
  code: `git grep` as a CI check.

**ReAgent:**
- A notify-client contract test.
- The dedupe key survives SQS retries.
- One request per recipient.

**End to end:** a review on a PR in a granted repo reaches the linked
agent with `VIA=integration TRUST=integration`. The same review on a repo
outside the grant produces no row.
