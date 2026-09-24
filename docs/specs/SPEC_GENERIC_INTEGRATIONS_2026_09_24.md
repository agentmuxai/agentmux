# SPEC: Generic integrations — external services talk to agents through one authenticated interface, nothing hard-coded

**Date:** 2026-09-24
**Status:** proposed; nothing here is built. The research (§1) is measured
against `agentmux` `main` @ `236f5cab4`, `agentmux-cloud` `main` @
`fb93159`, and `a5af/reagent` `main`, all read on 2026-09-24.

**Revision history.** Revised on 2026-09-24 after an adversarial review.
It found three P1s, nine P2s and four P3s; all were verified against the
code and all were accepted. The main changes:
- **The relay no longer resolves agents.** Integration messages go into an
  **account inbox**. Each desktop resolves the target locally, using its
  own alias map and the GitHub links in the Armory (§2.4, §2.5). This
  removes a cloud table that any agent holding the user token could have
  rewritten to redirect notifications.
- **Integration tokens are classified first** and accepted only on
  `/integrations/v1/*` (§2.2).
- **The `trusted` flag is desktop-local and host-gated.** Grants need a
  fresh human login (§2.2, §2.8).
- **Repo scopes must be proven and cover the head repo**, so the fork gate
  is kept (§2.6).
- Also: integration sources can't collide with agent ids; dedupe includes
  the recipient; ReAgent's notify step can't fail the review; the stamp
  survives held and forwarded delivery; quota is per integration and
  account; the migration preconditions are listed.

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
  - It may send only to accounts that granted it, and only about
    resources whose scope those accounts have proven.
  - The relay **stamps** every message with the authenticated
    integration's identity and stores it in that account's **inbox**.
  - The account's desktops resolve the recipient locally and render the
    stamp generically.
  - Whether an integration's messages count as trusted is a local,
    host-gated decision.
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
   its own identity, its trust level, or the account it acts for.
3. **Routing and trust decisions that agents could tamper with in the
   cloud move to the desktop.** Every agent holds the account's cloud user
   token (`MUXBUS_TOKEN`, `server/muxbus_handlers.rs:339`), so the relay
   can't tell the account's srv from its agents. The cloud therefore only
   authenticates senders and enforces grants. The recipient and the trust
   level are decided on the desktop, behind the host-gated boundary from
   the WAN spec (§2.6 there).
4. **Accounts grant; integrations don't self-enroll.**
5. **Services own their logic.** Deciding *what* to notify, and *whom*,
   belongs to the service.
6. **Nothing compiled in.** Adding, rotating, or revoking an integration
   is data, not a release.

### 2.2 Registry, credentials and grants (cloud)

**Registry.** A new table, `muxbus-integrations-<env>`, keyed by
`integration_id`. Each entry holds:
- a display name;
- its Cognito app client id;
- the scopes it may request;
- a `link_hosts` allow-list for links it may include (§2.3);
- a status (`active` or `suspended`).

Registration is an operator action (open question 1).

**Credentials.** Each integration gets its own Cognito app client,
authenticated with `client_credentials`. It is issued scopes on a new
resource server, `muxbus-integrations`: `notify`, and `grants.read`.

Cognito access tokens carry no audience, and `verifyRequest`
(`auth.ts:136-164`) currently accepts any pool token without checking
`client_id` or scope. So:
- **Classification comes first.** A token whose `client_id` is in the
  integration registry **and** that carries a `muxbus-integrations/*`
  scope becomes `{mode: 'integration', integrationId, scopes}`, before any
  M2M or user handling.
- **Route fencing.** The global `onRequest` hook (`index.ts:169-179`)
  rejects `mode: 'integration'` on every route outside
  `/integrations/v1/*`. The `/integrations/v1/*` routes reject every other
  mode.
- **Scopes on the existing M2M path.** The per-account M2M path (clients
  created in `agent-provisioning.ts`) must present its own
  `https://muxbus.agentmux.ai/read|write` scopes, so an integration client
  can never pass as an account client.

The integration keeps its client secret in its own infrastructure;
ReAgent stores it in its own secret, not `services/infra`.

**Grants.** A new table, `muxbus-integration-grants-<env>`:
- **PK** `account_user_id`, **SK** `integration_id`.
- **Attributes:**
  - `grant_id`, a random handle;
  - `scopes`;
  - `resource_scopes`: proven resource patterns, e.g. `github:repo:123456`
    (§2.6);
  - `created_at`, `revoked_at`.

There is no `trusted` flag in the cloud (§2.8).

**Who can write grants.**
- A grant is created or revoked only by a **step-up** request: a user
  token of that account whose `auth_time` is under 5 minutes old. The
  desktop obtains it through an interactive login it starts from the
  host-gated Integrations window (the WAN spec's `credential_broker`
  pattern), and never injects or persists it.
- The ordinary `MUXBUS_TOKEN` that agents hold is refused.
- An agent that can read the srv's files can still do anything the srv
  can. That is machine compromise, as in the WAN spec §4.

**How an integration learns its grants.** `GET /integrations/v1/grants`
returns, for the calling integration only, each active grant's
`grant_id` and `resource_scopes`, and nothing about the account. An
integration fans an event out to every grant whose scopes cover the
event's resources.

### 2.3 Notify (cloud)

`POST /integrations/v1/notify`, authenticated with an integration token:

```json
{
  "grant_id": "g_…",
  "target": { "agent_id": "lark" } | { "external": { "provider": "github", "id": "583231", "login": "octocat" } },
  "message": "…",
  "kind": "review.completed",
  "priority": "normal" | "urgent",
  "subject": { "provider": "github", "resources": ["github:repo:123456", "github:repo:789012"], "ref": "pull/3664" },
  "links": [{ "label": "Review", "url": "https://github.com/…" }],
  "dedupe_key": "review:5304786248",
  "ttl_seconds": 1800
}
```

**The relay:**
1. Authenticates the token: mode `integration`, scope `notify`.
2. Loads the grant by `grant_id`. It rejects the request unless:
   - the grant is active;
   - its `integration_id` equals the token's;
   - it has the `notify` scope.
3. **Checks resources.** `subject` is required, and **every** entry in
   `subject.resources` must match the grant's `resource_scopes`.
   - For GitHub pull requests the notifier and ReAgent list **both the
     base and the head repository**, which keeps today's fork gate
     (`agent-mapping.ts:135-158`) as data.
   - Scopes are matched as opaque patterns; `kind` is opaque too. The
     relay knows nothing about GitHub.
4. **Validates everything, as data:**
   - `target` is size-capped; an `agent_id` target passes
     `validate_agent_id`, and an `external` target has `provider` and `id`
     from a safe charset.
   - `links` must be `https` URLs whose host is in the registry entry's
     `link_hosts`.
   - `kind`, `subject.ref` and the labels are charset- and size-capped.
   - The message is capped at 10 KB.
5. **Deduplicates** on `(integration_id, grant_id, target, dedupe_key)`
   with a conditional write, answering 200 with the existing id. A
   service that sends one request per recipient therefore loses none of
   them.
6. **Charges quota per `(integration_id, account)` pair**, with its own
   unbilled rate limit and monthly cap. One integration can't exhaust an
   account's own jekt quota, and one account's traffic can't exhaust an
   integration's allowance for another account.
7. **Stores the row in the account inbox:**
   - `inbox_account` = the grant's `account_user_id`;
   - a new GSI `inbox_account-created_at-index`.
   - `target_agent` is **not** set, so the row never appears in the global
     `target_agent-created_at-index` or in `/reactive/pending/:agent_id`.
   - The target selector is stored as data.
8. **Stamps the row.** The stamp is `source_kind = integration`,
   `integration_id` and `integration_name`, all taken from the registry;
   the request can't supply any of them.

### 2.4 Delivery: account inbox, resolved on the desktop

**Pulling.** `GET /integrations/v1/inbox` is authenticated with the
desktop's **account** credential (a user token or a per-agent M2M token).
- The caller's account is `accountUserId ?? userId`. An M2M client with no
  owner is refused.
- It returns pending, unexpired rows for that account.
- `POST /integrations/v1/inbox/ack` and `/release` check `inbox_account`
  against the caller.

**The wake.** Stays the existing unscoped broadcast: every desktop polls
its own inbox. An account-scoped wake would reach no desktop today,
because the desktop's WebSocket runs on the user token, and `ws-connect`
stores no account for it (`cloud_subscriber.rs:454`, `ws-connect.ts:81`).

**Resolution on the desktop.** For each row, the srv resolves the target
**locally** before claiming it:
- **`agent_id`:** matched against this srv's agents by slug or alias,
  through the alias map (identity spec §4.3), and the identity spec's
  permanence rule for PR-body tags.
- **`external`:** matched against local **identity links** (§2.5) on
  `(provider, id)`, the immutable numeric id. The login is display only.

Then:
- **The row resolves to an agent on this desktop:** the srv claims it with
  `ack` and delivers it. If the agent is absent, it holds the message as a
  durable jekt (Phase 1), with the stamp (§2.8).
- **It doesn't resolve here:** the srv leaves the row pending for another
  of the account's desktops. A desktop records which rows it has already
  evaluated, so it doesn't re-evaluate them on every wake. A row no
  desktop claims expires at its TTL. The relay counts these as
  `inbox.expired_unclaimed`, per integration, for §3's migration check.

A target the relay doesn't understand is harmless, because the relay never
resolves one. It also no longer needs an alias map or complete ownership
data: the account's own desktops decide.

### 2.5 Identity links — where "connect to GitHub" comes in

Identity links live **on the desktop**, in the srv's store, as Armory
data: `(provider, external_id) → agent_id | operator`, plus a display
login and a `source` (`armory`, or `manual`).

**Where links come from:**
- **"Connect to GitHub" in the Armory** (§1.3). The srv records:
  - the user's own GitHub account, as `operator`;
  - each agent's GitHub App bot account, from the agent's Minted GitHub
    App identity in #3413, or from its fleet App.

  Both are keyed by the numeric GitHub id, which survives renames and
  covers the `login` vs `login[bot]` forms (`agent-mapping.ts:171`).
- **Manual:** Settings → Integrations → *Linked identities*, for PAT peer
  logins such as `lark-asaf`, which the Armory doesn't manage.

**How they are protected.** Links are written only through the
host-gated Settings or Armory window, never through an `X-AuthKey` RPC
that agents can reach. They are audited, and every change is announced
to the operator. The cloud holds no copy, so nothing that holds
`MUXBUS_TOKEN` can redirect notifications.

**Where `operator` targets go** is open question 5.

**Seeding** (§3 precondition). The desktops that run today's fleet import
the consumer's hard-coded `GITHUB_TO_AGENT_MAP` once, as `manual` links.
The fleet's `NUMBERED_PAT_PATTERN` (`agent<slot>-<host>`) can't be listed,
so it is expanded into concrete links for the slots and hosts that exist.

### 2.6 Tenancy: proven resource scopes

A GitHub event names repositories, not an AgentMux account. A grant's
`resource_scopes` must be **proven**, never just typed in. Otherwise any
account could claim `github:repo:*`, and once one notifier serves several
accounts, review and CI content about private repos would leak between
them.

**v1 (our own deployment, operator-registered integrations).** The
account owner connects GitHub. At grant time the desktop proposes the
repositories that the owner's linked GitHub identity can access, fetched
with the owner's own GitHub token from the Armory. The step-up request
carries that list, and the relay stores it as the proven scopes.

**Later (a published, installable integration).** The GitHub App
installation-to-tenant link from `PLAN_REAGENT_CLOUD_SERVICE` provides
scopes, via a signed `state` round-tripped through GitHub's setup
redirect. It populates `resource_scopes` from the installation's
repositories.

Either way the check is generic: opaque patterns on both sides. This
replaces the hard-coded `TRUSTED_REPO_OWNERS`, and because the head repo
is checked too (§2.3 step 3), a fork PR from an ungranted owner produces
no row.

### 2.7 Stamp, marker, and source naming

**Source naming.** An integration row's `source_agent` is
`integration:<integration_id>`.
- `:` is not allowed in agent ids, so it can never collide with or
  impersonate an agent.
- Desktops that include #3664 render an unrecognised source as
  `?integration:<id>` with no reply hint. Older desktops render it
  plainly, and a reply to it fails `validate_agent_id` at send time.
  Integration traffic is therefore never mistaken for an agent,
  regardless of desktop version.

**The desktop's stamp.** `PendingInboxRow` carries the stamp.
`InjectionRequest` gains `integration: Option<IntegrationStamp>`, marked
`#[serde(skip_deserializing)]`; it is set only from a pulled inbox row.

**Marker, when the stamp is present:**

```
[JEKT:FROM=integration:reagent VIA=integration TO=lark KIND=review.completed DELIVERY=wan TRUST=integration …]
```

- Every field goes through `marker_field`.
- Subject and links render as **data lines** in the body, never in the
  header, and links are `https` only (§2.3).
- The reply hint reads `Reply: not available — integration` (open
  question 6).

**Frontend.** `stream-parser.ts` learns `VIA` and `KIND`, and `JektTrust`
gains `integration`. `stripJektEnvelope` stops relying on fixed header
offsets, so the extra data lines don't break it.

### 2.8 Trust, held and forwarded delivery, and removal

**Trust.** The relay authenticated the integration, so its identity is
**cloud-attested**, not signed end to end. That is the trust anchor the
pinned ReAgent key really had, because the signing key lived in the
cloud's own secrets (§1.1).

Whether an integration's messages get the verified-sender treatment
(`ESCALATE=none` for sensitive-tier content, except where
`transcript_request_escalate_forced` applies, `handler.rs:1292-1293`) is
a **desktop-local** setting, `trusted_integrations`, keyed by
`integration_id`:
- It is set only through the host-gated window.
- It is **off by default**.
- The relay has no say in it.

Removing `reagent-v1` signing (I5) drops the relaxation for **every**
GitHub notice, because the consumer signs them all (§1.1). An operator
who wants it back marks `github-notifier` or `reagent` as trusted.

**Held and forwarded delivery.**
- `db_jekt_held` gains the stamp columns (`integration_id`,
  `integration_name`, `kind`, subject and links JSON; additive).
- The forwarding verdict struct (`server/reactive.rs:44-61`) carries the
  stamp.
- Held replay and forwards keep `VIA=integration`. The trust setting is
  re-read at delivery time.

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
`kimi-reviewer`). It fans out to each grant whose scopes cover the PR's
base and head repos, with one request per recipient:
- the PR author;
- the head commit's author, when different;
- the PR-body `agentmux:agent_id=` tag, sent as an `agent_id` target.

`dedupe_key` is the review id, the comment id, or `codex:<sha>`.

The notify step must never change the review's outcome:
- It runs in **its own `try`**, after the review is posted, with
  in-process retries and backoff. It never raises. Today an exception
  after posting reaches the `except` that posts a failure review
  (`reviewer_handler.py:1916-1952`), so one run could produce both a
  success and a failure.
- The queues have `maxReceiveCount: 1`, so SQS will not retry. A notify
  that still fails is logged with a metric, and is lost like any
  best-effort notice.
- It ships **off by default** behind a config flag.

**The GitHub notifier.** The generic parts of today's consumer become an
integration, `github-notifier`:
- merges, CI failure and completion, human reviews;
- Codex reviews, until Codex has a notifier of its own (open question 9).

It has its own credential, and it **leaves the relay's CDK stack**, with
its dedupe table (open question 7). It uses only the public interface:
- It keeps its GitHub logic: event parsing, CI aggregation, and GitHub
  dedupe.
- It drops the hard-coded login map and trusted owners. Identity links on
  the desktop and proven scopes replace them.
- It sends `external` targets by numeric GitHub id.
- It drops its ReAgent-specific branches (`[ReAgent]` formatting,
  `issue-comment.ts`), because ReAgent notifies for itself.

**Local messaging bridges** (Discord and the others) stay desktop-local
(open question 8).

---

## 3. Migration and rollout

Each step keeps notifications flowing; no step needs a flag day.

| Step | Repo | What ships | During the transition |
|---|---|---|---|
| **I1** | cloud | integration token classification and route fencing; scope checks on the M2M path; registry and grants tables, with step-up grant writes; `notify`, `grants`, and the inbox routes; quota per `(integration, account)`; `inbox.expired_unclaimed` and no-grant counters | the consumer still runs; nothing changes for desktops |
| **I2** | agentmux | inbox pull and local resolution; identity links (Armory and manual) and seeding; the host-gated Integrations window (grants with step-up, trusted integrations, linked identities); stamp rendering and parser; held and forward stamp columns | desktops that haven't upgraded don't pull the inbox, so they get nothing from integrations. They still get consumer notices until I3 and I4 |
| **I3** | reagent | the notify step, off by default | the consumer's ReAgent branches and ReAgent's notify step are two **config flags, flipped together**. A stack deploy can't be atomic, so the switch is a config change on both sides |
| **I4** | notifier's new home + cloud | `github-notifier` as an integration, off by default; then the consumer Lambda, its SNS subscription and its dedupe table are removed from the relay stack | the same flag-flip approach |
| **I5** | agentmux + cloud | removal (§2.8); the legacy key retired once `auth.ts` sees no legacy callers (`DISABLE_LEGACY_AUTH`) | desktops older than I2 get no GitHub notices at all from here on: an announced break, see below |

**Preconditions for I3 and I4.** These have to be true before either
flag is flipped:
- **Grants exist** for every account that receives these notices today.
  Legacy rows have no account, so first find which Cognito account or
  accounts own the fleet's agent ids, using the agent-ownership table and
  the pending callers in logs.
- **Every desktop that runs a notified agent is on I2 or later**, and its
  identity links are seeded (§2.5).
- **A dry run matches:** the notifier runs in shadow mode (resolve and
  count, don't store) alongside the consumer, and its counts match the
  consumer's.
- **Rollback is known:** re-enable the consumer flag.

**Old desktops.** After I4 a desktop older than I2 receives no GitHub
notices. It never had another way to get them. This is announced in the
release notes of the I2 release, and I4 waits one release cycle after I2.

---

## 4. Open questions

1. **Registration.** Operator-only registration now; a developer console
   later? (Recommended: operator-only.)
2. **User-owned App vs one published App**, for "connect to GitHub" (§1.3
   conflict). This spec needs only identities and proven scopes, so either
   works, but the Armory spec must decide.
3. **`SPEC_CONNECT_WITH_GITHUB_ARMORY_2026_09_23`**: find it and reconcile
   §2.5 with its bot-identity mode.
4. **The step-up freshness window** for grant writes (5 minutes proposed),
   and whether to require MFA.
5. **Where an `operator` target goes:** the default agent, every agent
   watching the subject, or the human's desktop notification (the notify
   work in #3662)?
6. **Replies to integrations** (bidirectional), e.g. an agent answering
   ReAgent. Out of scope for v1.
7. **Home for `github-notifier`**: `shared-infrastructure` next to
   `github-router`, or its own repo?
8. **Local bridges:** fold Discord, Telegram, Slack and WhatsApp into the
   same model (local registry, the `VIA=integration` marker, trusted
   integrations)?
9. **Codex notifications:** keep them in `github-notifier`, or move them
   into ReAgent's Codex worker once `codex-review-gate-in-reagent` ships?

## 5. Tests (summary)

**Cloud:**
- Classification and fencing:
  - an integration token becomes `mode: 'integration'`;
  - it is rejected on `/reactive/*`, `/api/*` and every other
    non-integration route;
  - user and M2M tokens are rejected on `/integrations/v1/*`;
  - an M2M token without its read or write scope is rejected.
- Grants:
  - grant writes without a fresh `auth_time` are rejected;
  - `grants` lists only the caller's own grants.
- Notify:
  - a grant belonging to another integration → 403;
  - missing `subject`, or any resource outside the scopes (including a
    fork head repo) → 403;
  - a non-`https` link, or a host not in `link_hosts` → 400;
  - the same `dedupe_key` for two recipients → two rows;
  - quota is counted per `(integration, account)`.
- Inbox:
  - only the owning account can pull, ack or release;
  - an inbox row never appears in `/reactive/pending`.

**Desktop:**
- Resolution:
  - resolve by slug, by alias, and by numeric-id link;
  - an unresolved row is left pending and not claimed;
  - an absent target is held with its stamp.
- Protection:
  - links, trusted integrations and grants reject a caller with only
    `X-AuthKey`;
  - `integration` can't be deserialised from HTTP.
- Rendering:
  - the marker is `FROM=integration:<id> VIA=integration`, through
    `marker_field`;
  - links are data lines only;
  - the reply hint is absent.
- Trust:
  - trusted plus a sensitive message → `ESCALATE=none`;
  - untrusted → escalates;
  - a forced transcript request → escalates.
- Held replay and forwarding keep the stamp.
- After I5, `git grep` for ReAgent identifiers finds none in code, as a
  CI check.

**ReAgent:**
- A notify failure never produces a failure review.
- Both reviewer Lambdas notify.
- One request per recipient; fan-out follows grant scopes; head and base
  repos are both sent.

**End to end:**
- A review on a PR in a granted repo reaches the linked agent with
  `VIA=integration`.
- A fork PR from an ungranted owner produces no row.
- A second account's desktop never sees the row.
