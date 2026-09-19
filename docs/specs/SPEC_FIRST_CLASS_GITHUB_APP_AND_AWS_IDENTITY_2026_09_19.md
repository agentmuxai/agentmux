# SPEC: GitHub App and AWS as first-class AgentMux identities

**Date:** 2026-09-19
**Status:** proposed — exploratory design, nothing implemented.
**Author:** Agenty (agent), at the repo owner's request.
**Related:** `docs/specs/SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md` (the
login pattern this generalises), `docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md`,
`docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md`,
`docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`.

---

## 1. Motivation

An agent that needs GitHub or cloud access today gets it through tooling
that lives *outside* AgentMux: wrapper scripts, a separate secrets CLI,
certificates placed on each host by hand, and per-repo CI secrets. All of it
is hand-rolled, per-machine, and invisible to AgentMux itself.

Meanwhile AgentMux already solves this exact problem for Anthropic: click a
button in Armory, complete a login, and every agent bound to that account
gets working credentials at spawn with nothing written into the workspace.

**The proposal is to make GitHub App and AWS identities work the same way as
that Anthropic login.** The goal is not to serve this repo's own
infrastructure — it is that "log in to GitHub as an App" and "log in to AWS
as a role" become product capabilities any AgentMux user gets, which is what
turns AgentMux from a place agents run into a generalised operating
environment for agents that act on real systems.

## 2. What already exists (verified against the tree, not assumed)

### 2.1 The identity model is deliberately loose

`IdentityAccount` (`agentmux-srv/src/backend/storage/identities.rs:119-155`)
carries `provider` and `kind` as **free-form strings**, not enums. The real
discriminated union is `SecretRef`
(`identities.rs:43-112`): `Env`, `SecretsManager` (unimplemented),
`PlaintextDev`, `OAuthConfigDir`, `Keychain`.

Credential resolution flows through exactly one path:
`db_agent_identity_links` (`identities.rs:193-199`) — the older
bundle/binding layer was already removed.

### 2.2 There are only two credential *shapes*

`ProviderClass` (`agentmux-srv/src/identity/resolver/provider.rs:13-25`):

```rust
enum ProviderClass {
    ApiKey { env_vars: &[&str] },        // static secret -> env var(s)
    OAuth  { config_dir_env_var: &str }, // CLI owns tokens in a directory
}
```

`provider_class()` (`provider.rs:45-98`) is the authoritative dispatch, and
it gates spawn-time injection, the OAuth spawn gate, and per-account
isolation-dir minting. Injection is `inject.rs:651-770`.

### 2.3 The Anthropic login pattern

`ClaudeLoginPanel.tsx` → `flows/run-provider-login.ts` (the sole legal caller
of `runCliLogin`) → `server/identity_handlers.rs` (`auth.start` / `auth.poll`
/ `auth.submitcallback`) → `identity_auth_spawn.rs` → on success
`identity_auth_persist.rs` writes `SecretRef::OAuthConfigDir`.

The crucial property: **AgentMux never holds the Anthropic credential.** The
provider CLI writes and refreshes its own tokens inside a per-account
directory; the database stores only a pointer. Directory layout and channel
scoping: `agentmux-common/src/data_paths.rs:370-446`.

### 2.4 A generic service-OAuth stack exists, and GitHub is already in it

`agentmux-srv/src/identity/oauth_client.rs` has `ServiceOAuthConfig`
(`:48-62`), `config_for()` (`:66-110`) with a `github` entry, both
`AuthCodePkce` and `Device` flows, PKCE helpers, and
`persist_oauth_account()` (`:293-325`) writing a token blob to the OS
keychain as `SecretRef::Keychain`.

**But every `client_id` is `None`** (`:71/:81/:91/:101`) — the stack works
only in bring-your-own-credentials mode. This is the closest thing the
codebase has to a real extension point.

### 2.5 AWS is a stub

- `ProviderClass::ApiKey { env_vars: ["AWS_ACCESS_KEY_ID"] }`
  (`provider.rs:64-66`) injects **only the key id and not the secret**, so it
  cannot produce working AWS credentials as written.
- No `key_validator` arm for `aws` (`key_validator.rs:73-82`), so an AWS
  account can never validate.
- `AccountContext.aws_profile / aws_role_arn / aws_region`
  (`frontend/app/view/identity/identity-model.ts:45-47`) are display-only.
- The AWS tile carries an explicit comment that IAM Identity Center "isn't
  wired in the backend yet" (`accounts-catalog.ts:40`).
- **`credential_process` appears nowhere in the codebase.**

## 3. The actual gap

Neither existing credential shape fits either target, and this is the whole
design problem:

| | static secret? | CLI owns tokens on disk? | fits today? |
|---|---|---|---|
| **GitHub App** | no — a private key that *mints* 1-hour tokens | no | **neither** |
| **AWS Roles Anywhere** | no — a certificate that *exchanges* for 1-hour STS credentials | no | **neither** |

Both are **minted credentials**: a long-lived *authenticator* (private key,
certificate) that produces a *short-lived* credential on demand. Forcing
either into `ApiKey` means storing a long-lived secret and handing it to
agents — precisely what these mechanisms exist to avoid. Forcing either into
`OAuth` means pretending a provider CLI owns a token directory, which no
GitHub App or Roles Anywhere flow does.

### 3.1 Proposal: a third `ProviderClass`

```rust
enum ProviderClass {
    ApiKey { env_vars: &[&str] },
    OAuth  { config_dir_env_var: &str },
    Minted(MintedSpec),   // NEW
}
```

A `Minted` account stores an authenticator (via the existing `SecretRef`, so
keychain storage is reused unchanged) plus a **minting strategy**. At spawn,
instead of reading a secret and injecting it, AgentMux *calls* the strategy
and injects the short-lived result.

Two properties fall out, both of which the current model cannot express:

- **Expiry is first-class.** A minted credential has a known lifetime, so
  `IdentityAccount.status` can mean "the authenticator is valid" rather than
  "some secret exists". Re-minting on long sessions becomes possible.
- **Nothing long-lived reaches the agent.** The agent receives a token that
  expires, and never the key that produced it. This is strictly better than
  the `ApiKey` path and is the security argument for the whole feature.

## 4. GitHub App identity

**Login flow.** Unlike Anthropic, there is no CLI to scrape and no OAuth
dance to complete — a GitHub App is *configured*, not logged into. The
realistic flow is GitHub's **App Manifest**: AgentMux opens a
pre-filled creation page, the user confirms on GitHub, GitHub redirects back
with a temporary code, and AgentMux exchanges it for the App's id, private
key and webhook secret in one call. The user never copy-pastes a private
key. An "I already have an App" path accepts an existing id + key.

**Storage.** Private key → OS keychain via the existing
`secret_store.rs` (`SecretRef::Keychain`). App id and installation ids →
`AccountContext`, which is already a free-form JSON blob.

**Minting.** Sign a short JWT with the private key, exchange it for an
installation token, inject as `GITHUB_TOKEN`/`GH_TOKEN` — the same env vars
the existing `github` provider already declares
(`provider.rs:52-54`), so every tool that reads them works unchanged.

**The constraint that must be modelled explicitly:** installation tokens are
**per-account**. One token cannot span two GitHub accounts or orgs. This is
not a theoretical concern — it was confirmed empirically while migrating
this repo's own CI, where a token minted for one account failed against
another with `403 permission_denied: The requested installation does not
exist`. A GitHub App identity is therefore `(app, installation)`, not
`(app)`, and the UI must let a user pick the installation when an App is
installed in more than one place.

**Token scoping.** GitHub allows minting a token restricted to named
repositories and a permission subset. An agent should get the narrowest
token that satisfies its binding, not the App's full installation.

## 5. AWS identity

Three mechanisms, increasing in value:

1. **Static access keys** — finish what `provider.rs:64-66` starts by
   injecting the secret alongside the key id. Cheap, unblocks the existing
   broken tile, but is the thing worth moving away from.
2. **Assume-role** — store a base credential, call `sts:AssumeRole`, inject
   the returned triple. Naturally `Minted`.
3. **IAM Roles Anywhere** — a certificate exchanged for STS credentials with
   no static key anywhere. The strongest option and the natural fit for
   `Minted`.

**Injection has an AWS-specific subtlety.** Some tooling reads
`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN` from the
environment; other tooling expects a profile in `~/.aws/config`, often via
`credential_process`. The env-var route alone cannot express "re-mint when
this expires", because environment variables are fixed at spawn and agent
sessions outlive a one-hour credential.

So an AWS `Minted` account should support **both**: inject env vars for
immediate use, *and* provision a per-account `AWS_CONFIG_FILE` containing a
`credential_process` entry that points back at AgentMux. That makes renewal
automatic and is the same architectural move the Anthropic flow already
makes — a per-account directory pointed at by an env var
(`data_paths.rs:444-446`), except AgentMux supplies the process rather than a
vendor CLI.

**Known trap, already paid for once in this project's own tooling:**
`credential_process` does not expand `~`, and is invoked without a shell.
Paths must be absolute. A setup that works under an interactive shell can
still fail when an SDK invokes it directly.

## 6. Why this generalises AgentMux

Everything above is a special case of one missing primitive: **AgentMux can
hold an authenticator and vend short-lived credentials to agents.** With
`Minted` in place, the same machinery covers GCP workload identity, Azure
service principals, Vault, Kubernetes service accounts, signed-JWT service
auth — none of which fit `ApiKey` or `OAuth` either.

It also closes a real gap in the agent security story. Today an
identity-bound agent receives a long-lived secret in its environment, and
`pane_env.rs` can stop that secret leaking *onward* but cannot make it
expire. A minted credential is bounded by construction.

## 7. Phasing

| Phase | Scope | Value |
|---|---|---|
| **0** | `ProviderClass::Minted` + `resolve_secret` seam + injection dispatch. No UI. | Unblocks everything; nothing user-visible. |
| **1** | GitHub App: manifest flow, keychain storage, installation picker, token minting. | Replaces GitHub PATs for agents. |
| **2** | AWS assume-role, env injection only. | Finishes the broken AWS tile. |
| **3** | `credential_process` provisioning + auto-renew. | Sessions outlive one hour. |
| **4** | Roles Anywhere certificate enrolment. | No static cloud key anywhere. |

Phase 1 is independently valuable and should not wait for AWS.

## 8. Risks and open questions

1. **`provider_class()` is load-bearing** (`provider.rs:45-98`) — it gates
   injection, the OAuth spawn gate, and isolation-dir minting. Adding a
   variant touches every `match class`. This is the main implementation risk
   and argues for Phase 0 landing alone, with tests, before any UI.
2. **Several parallel lists drift silently.** Backend `provider_class` ↔
   frontend `AccountProvider` ↔ `accounts-catalog.ts` ↔ `key_validator.rs`
   are mirrored by hand; only the first pair has even a hand-maintained test
   (`provider.rs:134-161`). A new credential kind should not add a sixth
   copy — consider a single generated source.
3. **Minting needs network at spawn.** Unlike reading a secret, minting can
   fail, rate-limit, or hang. Spawn must fail closed with a clear error, as
   the OAuth gate already does (`inject.rs:685-697`) — never silently
   spawn an agent with no credentials.
4. **Where does minting run?** In `agentmux-srv`, which means srv holds the
   authenticator in memory. Acceptable — it already resolves keychain
   secrets — but it makes srv a higher-value target and should be stated
   rather than discovered.
5. **Does a minted credential cross the `pane_env.rs` boundary correctly?**
   It is not an `AGENTMUX_*` variable, so `PANE_ENV_KEEP` does not apply;
   confirm the intended behaviour for third-party spawns explicitly.
6. **Open question — is the App itself per-user or per-AgentMux?** A single
   AgentMux-published GitHub App that users install is far better UX than
   every user creating their own, but makes AgentMux the App owner and puts
   it in the supply chain for every user's repos. The manifest flow above
   assumes **user-owned Apps** deliberately; revisit only with that
   trade-off stated.

## 9. Non-goals

- Not a migration plan for this repo's own infrastructure. That work is
  tracked privately and would merely become an early consumer.
- No credential material in this repository, and no account, role, or
  installation identifiers — see the public-repo rule in `CONTRIBUTING.md`.
- Not a secrets manager. AgentMux vends credentials to agents; it does not
  become a general store for arbitrary secrets.
