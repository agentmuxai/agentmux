# Codex Provider Integration: Claude-Parity Lifecycle

**Date:** 2026-08-08
**Status:** active — Slices A (JSONL adapter) + B (provider argv/resume) shipped in PR #2476; Slices C (Docker identity projection), D (Armory/Stash Codex account UX), E (provider-native materializer), F (Docker smoke) not started. Verified 2026-08-10.
**Scope:** Codex provider registration, Armory accounts, Agent Stash bindings,
authentication isolation, Docker projection, provider-native configuration, and
delivery slices
**Companion:** `SPEC_CODEX_JSONL_CONTRACT_2026_08_08.md`
**Reference implementation:** AgentMux Claude provider
**Current AgentMux pin:** `@openai/codex@0.116.0`

---

## 1. Purpose

Define how Codex becomes a first-class AgentMux provider using the mature Claude
integration as the lifecycle reference without copying Claude-specific protocol or
filesystem behavior.

The complete Codex path is:

```text
Armory Codex account
  -> Agent Stash direct binding
  -> isolated CODEX_HOME resolution
  -> host or Docker runtime projection
  -> Codex-native instructions / MCP / skills materialization
  -> codex exec --json
  -> Codex JSONL adapter
  -> provider-neutral agent pane
```

This document answers the parts deliberately excluded from the companion JSONL
contract:

1. What AgentMux should copy from Claude's account and binding lifecycle.
2. How a Codex account is created, refreshed, linked, resolved, and revoked.
3. How a host `CODEX_HOME` becomes a valid container `CODEX_HOME` without leaking
   an unusable host path into Linux.
4. How Armory Bundles, Skills, and MCP Servers become native Codex configuration.
5. Which Claude-only features must remain provider-specific.
6. How to split implementation into reviewable changes.

The goal is not visual provider parity alone. A Codex pane is complete only when the
account visible in Armory and linked in Stash is the account the CLI actually uses,
on both host and Docker runs.

---

## 2. Normative sources and precedence

### 2.1 Upstream Codex sources

- [Codex CLI command reference](https://developers.openai.com/codex/cli/reference/)
- [Codex configuration reference](https://developers.openai.com/codex/config-reference/)
- [Custom instructions with AGENTS.md](https://developers.openai.com/codex/guides/agents-md/)
- [Codex MCP configuration](https://developers.openai.com/codex/mcp/)
- [Codex skills](https://developers.openai.com/codex/skills/)

As of this spec date, the official documentation establishes that:

- `codex exec` is the stable non-interactive command;
- `--json` emits newline-delimited JSON events;
- a prompt may be read from stdin using `-`;
- a saved session resumes with `codex exec resume [SESSION_ID]`;
- `--dangerously-bypass-approvals-and-sandbox` is intended only inside an
  externally isolated runner;
- authentication and user configuration are rooted at `CODEX_HOME`;
- user configuration is stored in `$CODEX_HOME/config.toml`, with selectable
  profile files beside it;
- `developer_instructions` is a supported configuration field;
- MCP servers are configured through `mcp_servers` TOML entries;
- project instructions use `AGENTS.md` / `AGENTS.override.md` discovery;
- repository skills are discovered under `.agents/skills` between the working
  directory and repository root.

### 2.2 AgentMux sources

The current implementation is authoritative for shared lifecycle semantics:

- provider registry: `agentmux-srv/src/backend/providers.rs`;
- frontend provider catalog: `frontend/app/view/agent/providers/catalog.ts`;
- account storage and direct links:
  `agentmux-srv/src/backend/storage/identities.rs`;
- spawn-time account resolution:
  `agentmux-srv/src/identity/resolver/inject.rs`;
- OAuth provider classification:
  `agentmux-srv/src/identity/resolver/provider.rs`;
- shared provider login flow:
  `frontend/app/view/agent/flows/run-provider-login.ts`;
- Claude's Armory/Stash recovery UI:
  `frontend/app/view/accounts/ClaudeLoginPanel.tsx` and
  `frontend/app/view/identity/agent-identity-links-panel.tsx`;
- Stash composition:
  `frontend/app/view/agent/components/AgentStashModal.tsx`;
- current project materializer:
  `agentmux-srv/src/backend/agent_config.rs`;
- container runtime: `agentmux-srv/src/backend/container.rs` and
  `agentmux-srv/src/backend/blockcontroller/subprocess/container_spawn.rs`.

When upstream documentation and a captured pinned-CLI fixture disagree about a
wire field, the fixture governs translation for that pin and the discrepancy is
recorded. When this spec and Claude-specific code disagree about Codex filesystem
behavior, this spec governs Codex.

---

## 3. Vocabulary and provider IDs

AgentMux has two related but distinct credential namespaces:

| ID | Meaning | Credential class | Injection |
|---|---|---|---|
| `codex` | Codex CLI account | OAuth/config-directory | `CODEX_HOME` |
| `openai` | Raw OpenAI API-key account | API key | `OPENAI_API_KEY` |

This spec concerns the canonical CLI provider ID `codex`. A Stash link under
`openai` does not satisfy a Codex OAuth binding, and a `codex` account must not be
renamed to `openai` merely because OpenAI operates both products.

Terms:

- **Armory account:** shared `db_accounts` row describing one reusable credential.
- **Stash binding:** one agent's `db_agent_identity_links` row pointing to an
  Armory account for a provider.
- **Account home:** the host-side directory stored as
  `SecretRef::OAuthConfigDir`; for Codex this is a `CODEX_HOME`.
- **Runtime home:** the path visible to the CLI process. It equals the account home
  for host runs and `/home/agent/.codex` for the phase-1 Linux container.
- **AgentMux profile:** an AgentMux-owned Codex profile layer named for one agent
  definition and selected with `--profile`.
- **Project materialization:** generated provider-native files in the working
  directory, tracked by an AgentMux manifest.

---

## 4. What Codex copies from Claude

Claude is the reference for these lifecycle invariants:

1. Armory owns reusable account records; Stash owns per-agent links.
2. The direct-link table is the only spawn-time binding authority.
3. OAuth accounts use isolated config directories instead of ambient user homes.
4. Re-login refreshes the same account rather than minting an orphan replacement.
5. Successful UI state requires both credential persistence and, for Stash flows,
   verification of the exact canonical agent link.
6. Provider aliases are canonicalized and stale alias rows are removed only after
   the canonical link is confirmed.
7. Missing, deleted, malformed, or unauthenticated OAuth bindings fail closed
   before the provider process is created.
8. Account secrets are not copied into pane metadata, argv, JSONL history, or
   ordinary logs.
9. Shared Armory resources are resolved at launch, not cached forever in a pane.
10. CLI versions and provider metadata remain synchronized across frontend,
    backend, installer, and container-image declarations with contract tests.

Codex does not copy these Claude-specific mechanisms:

- Claude's persistent bidirectional stream controller;
- Claude's stdio control protocol or `AskUserQuestion` response wire;
- `CLAUDE.md`, `.claude/settings.json`, `.claude/commands`, or `.mcp.json`;
- Claude's project-native memory directory convention;
- Claude hook formats or `agentmux-bashwrap` hook injection;
- Claude stream event schemas.

Codex remains a one-subprocess-per-turn provider in this phase. Interactive
mid-turn steering is not claimed until an upstream Codex surface with a stable
bidirectional contract is deliberately adopted.

---

## 5. Target architecture and ownership

```text
                         SHARED CATALOG
                +-----------------------------+
                | Armory                      |
                | db_accounts                 |
                | db_bundles                  |
                | db_skills                   |
                | db_mcp_servers              |
                +--------------+--------------+
                               |
                        per-agent selection
                               |
                +--------------v--------------+
                | Agent Stash                 |
                | db_agent_identity_links     |
                | db_agent_skills_ref         |
                | db_agent_mcp_ref            |
                | startup_bundle_id content   |
                +--------------+--------------+
                               |
                    resolve at launch/turn
                               |
                +--------------v--------------+
                | Codex provider adapter      |
                | account + profile + argv    |
                +-------+-------------+-------+
                        |             |
                     host run      Docker run
                        |             |
            CODEX_HOME=<host>   bind exact account home
                                CODEX_HOME=/home/agent/.codex
                        \             /
                         +-----v-----+
                         | codex exec |
                         | --json     |
                         +-----+-----+
                               |
                         JSONL contract
                               |
                         AgentMux pane
```

Ownership rules:

- `db_accounts` owns account metadata and the host account-home pointer.
- `db_agent_identity_links` owns which account one agent uses for `codex`.
- the provider registry owns CLI command, version, auth env name, controller type,
  container runtime-home path, and argv strategy;
- the Codex materializer owns only AgentMux-prefixed profile/files and its managed
  manifest;
- the repository and user retain ownership of pre-existing `AGENTS.md`,
  `.codex/config.toml`, `$CODEX_HOME/config.toml`, and non-manifest files;
- Codex owns its auth token format and session rollout files inside the selected
  account home.

---

## 6. Provider registry contract

The existing `codex` registry entry is the starting point, not the completed
integration. The final provider definition must express:

| Field | Codex value / behavior |
|---|---|
| canonical ID | `codex` |
| command | `codex` |
| package | `@openai/codex` |
| controller | `subprocess` |
| output format | `codex-json` |
| auth type | OAuth |
| auth env | `CODEX_HOME` |
| account-home name | `codex` |
| container runtime home | `/home/agent/.codex` |
| first turn | provider argv strategy for `exec --json ... -` |
| resume | provider argv strategy for `exec resume ... <thread_id> -` |
| default execution policy | `--dangerously-bypass-approvals-and-sandbox` |

The registry must become the single source of truth for the container auth path and
resume argv shape. Host and Docker call the same pure argv builder; neither runtime
may reconstruct Codex syntax independently.

`resume_flag: null` is acceptable only until the provider argv strategy lands. It
must not be misread as a product decision that Codex sessions are non-resumable.

Pin changes follow the companion JSONL spec's fixture/version gate. The pin must be
identical in the Rust registry, frontend catalog, installer/CEF registry, and Docker
image build input.

---

## 7. Armory account lifecycle

### 7.1 Account shape

A persisted Codex account is an `IdentityAccount` with:

```text
provider   = "codex"
kind       = OAuth-compatible account kind
secret_ref = OAuthConfigDir { dir: <absolute host account home> }
status     = active | expired | invalid | unknown
```

The account row stores a pointer, not OAuth token contents. Account directories are
created beneath AgentMux's shared provider/account area using the existing
`identity.ensureaccountdir` path. Directory names must be stable for the account ID
and must not include display names or email addresses.

### 7.2 Connect and re-login

The login state machine should reuse `runProviderLogin`; it must not fork into a
second Codex-only implementation. A provider-neutral `ProviderLoginPanel` may be
extracted from `ClaudeLoginPanel`, retaining a thin Claude wrapper if useful for
compatibility.

For Codex:

1. Resolve the pinned CLI.
2. Mint a new account home, or resolve the existing account home for re-login.
3. run `codex login` with `CODEX_HOME` pointing directly at that home;
4. poll `codex login status` against the same environment;
5. persist the account only after authentication is confirmed;
6. when launched from Stash or an agent recovery surface, link the canonical
   `codex` provider to the agent;
7. re-read `db_agent_identity_links` and verify that the exact account ID is linked
   under canonical provider `codex` before displaying success;
8. remove a stale alias link only after step 7 succeeds.

Retry after a link-verification failure must reuse the newly persisted account ID.
It must not mint another account and orphan the first.

### 7.3 Armory surface

Armory Accounts must allow a fresh Codex connection without an agent target. A
successful bare Armory connect creates a reusable account but no agent link.

The account card must show at minimum display name, provider, status, and actions
supported by the generic account manager. Raw account-home paths and token filenames
are diagnostic details, not primary UI.

### 7.4 Delete and revoke

Deleting a Codex account must:

- delete the account row;
- cascade/remove all direct agent links to it;
- publish account and affected-agent change events;
- make the next turn for each affected Codex agent fail the OAuth spawn gate;
- offer Connect/Re-login in Stash;
- follow the existing recoverable account-directory deletion policy rather than
  silently falling back to a global `~/.codex` login.

An already-running Codex turn is not killed merely because the row is deleted unless
the general account-revocation policy explicitly requires that behavior. The next
provider spawn is the mandatory enforcement point.

---

## 8. Agent Stash integration

### 8.1 Accounts tab

The Accounts tab continues to read `db_agent_identity_links`, the same table used by
the spawn resolver. For a Codex agent it must provide:

- **Connect** when no canonical Codex link exists;
- **Re-login** when a Codex link exists, even if the local account cache is stale;
- provider/account/status display;
- link verification before success;
- alias cleanup using the raw provider string from the acted-on row.

The tab must not write the legacy `AgentDefinition.accounts` JSON blob.

### 8.2 Skills and MCP tabs

The existing Stash binding tables remain provider-neutral:

- Skills: globals plus `db_agent_skills_ref` for explicit non-global bindings.
- MCP: globals plus `db_agent_mcp_ref` for explicit non-global bindings.

The difference is at materialization time. A Codex agent must receive native Codex
skills and MCP config, not Claude files. Stash UI should not imply a resource is
active until the Codex materializer can represent it; unsupported fields must be
reported rather than silently discarded.

### 8.3 Startup

The Stash-selected `startup_bundle_id` remains provider-neutral. Its resolved Bundle
instructions are included in AgentMux's first-turn Session Context payload. This is
separate from the persistent developer-instruction profile described in section 10:

- persistent identity/rules/global guidance -> AgentMux Codex profile;
- perform-now startup Bundle -> first-turn Session Context payload.

### 8.4 Memory

Claude native memory is not the model for Codex storage paths. Phase 1 does not map
Codex to `$CLAUDE_CONFIG_DIR/projects/.../memory`, nor does it relabel Claude files as
Codex memories.

Until a Codex-native memory adapter has its own verified storage and synchronization
contract, the Stash Memory tab must be provider-aware and either:

- present an explicit "Codex native memory is not integrated yet" state; or
- hide the native-memory browser while leaving Armory Bundles available.

Armory Bundles are reusable instructions. They are not a substitute for provider-
learned native memory, and the two must not silently overwrite one another.

---

## 9. Spawn-time identity resolution

The existing OAuth resolver already classifies `codex` as:

```text
ProviderClass::OAuth { config_dir_env_var: "CODEX_HOME" }
```

For a host turn:

1. resolve the active agent instance and definition;
2. list its direct identity links;
3. canonicalize each provider ID;
4. fetch the linked account;
5. require `SecretRef::OAuthConfigDir` for `codex`;
6. verify/probe account status according to the shared OAuth policy;
7. set `CODEX_HOME` to the absolute host account home;
8. continue to profile materialization and argv construction.

For an OAuth-class Codex definition, all of these conditions are blocking:

- no canonical Codex link;
- link points at a missing account;
- account provider does not canonicalize to Codex;
- secret reference is malformed or is not `OAuthConfigDir`;
- account home cannot be accessed;
- authentication status is invalid under the shared gate.

`use_ambient_login` must not cause a fallthrough to the user's global Codex login.
The account shown in Stash must be the account used by the subprocess.

The resolver returns a structured runtime credential projection, not only a flat env
map. The minimum shape is conceptually:

```text
ResolvedProviderIdentity {
  provider: "codex",
  account_id,
  host_config_dir,
  host_env: { CODEX_HOME: host_config_dir },
  container_env: { CODEX_HOME: "/home/agent/.codex" },
  container_mount: { source: host_config_dir, target: "/home/agent/.codex", rw: true }
}
```

API-key identities may continue using env injection. OAuth config-directory
identities require path projection so host paths cannot accidentally become
container paths.

---

## 10. Codex-native configuration materialization

### 10.1 Why the Claude builder cannot be reused directly

The current `build_config_files()` always creates Claude-native files including
`CLAUDE.md`, `.claude/settings.json`, `.claude/commands`, `.claude/skills`, and
`.mcp.json`. Writing those for a Codex definition produces a successful filesystem
operation but does not configure Codex.

`write_agent_config_files()` must dispatch through a provider materializer. Shared
resolution—effective Bundles, Skills, MCP Servers, template variables—can remain
provider-neutral. Rendering and destination paths are provider-specific.

### 10.2 Instructions: AgentMux-owned profile, user-owned AGENTS.md

AgentMux must not overwrite, merge by string marker into, or claim ownership of an
existing repository `AGENTS.md`. Codex's native project instruction discovery
continues to load user/repository `AGENTS.md` files normally.

AgentMux compiles the agent's persistent managed guidance into
`developer_instructions` in an AgentMux-owned profile:

```text
$CODEX_HOME/agentmux-<agent-definition-id>.config.toml
```

The profile name is deterministic, filesystem-safe, and derived from the stable
definition ID, not the display name. Codex is launched with:

```text
--profile agentmux-<agent-definition-id>
```

The generated `developer_instructions` contains, in stable order:

1. agent Soul content;
2. agent `agentmd` content;
3. global Armory Bundle instructions;
4. any explicitly defined persistent agent instruction content;
5. a concise index of materialized AgentMux-bound skills when useful.

The Stash Startup Bundle is not duplicated here; it remains a first-turn action
payload.

The base `$CODEX_HOME/config.toml` remains user/account-owned. AgentMux may read it
for validation but must never replace it. An existing non-AgentMux profile with the
same name is impossible by namespace rule; if an AgentMux-owned profile is malformed
or unwritable, launch fails visibly rather than falling back to unconfigured Codex.

### 10.3 Skills

Effective AgentMux skills are rendered in native Agent Skills format:

```text
<workspace>/.agents/skills/<stable-slug>/SKILL.md
```

Each generated skill includes valid frontmatter with `name` and `description`, plus
the expanded content. AgentMux records every generated path in a provider-specific
manifest, for example:

```text
<workspace>/.agentmux/managed/codex-skills.json
```

On the next materialization pass, AgentMux removes only stale files named in its
prior manifest. It must never delete a user-authored `.agents/skills` entry that was
not previously managed. Slug collisions with user-owned directories fail or choose
a deterministic AgentMux-prefixed slug; they never overwrite silently.

This intentionally uses Codex's repository skill discovery rather than writing
Claude command files or placing per-agent skills into a shared account-wide user
directory.

### 10.4 MCP servers

Effective AgentMux MCP servers are translated into native Codex
`mcp_servers.<id>` entries inside the AgentMux profile. The materializer supports at
minimum:

- stdio: command, args, cwd, selected environment values, startup timeout;
- streamable HTTP: URL and supported auth indirection;
- the synthetic AgentMux MCP server required for pane/agent integration.

Secrets must remain indirect. If an Armory MCP definition references a bearer token
or sensitive header, the profile uses Codex's environment-variable indirection
fields and the runtime injects the value through the existing secret resolver.
Plaintext secret values must not be serialized into the generated profile.

Unsupported Claude `.mcp.json` fields cause a visible validation warning naming the
server and field. The server is not emitted partially when dropping a field would
change its security or transport semantics.

### 10.5 Host/container path portability

Generated profile content must be valid in the runtime where it is read. Prefer
commands resolved from `PATH`, workspace-relative paths, and environment indirection
over absolute host paths.

If a server or generated setting requires different host and container paths, the
materializer produces runtime-specific projections from one logical config. It must
not persist a Windows path in a profile that a Linux container consumes.

---

## 11. Docker contract

### 11.1 Current gap

The current container runtime is Claude-oriented:

- default image naming is Claude-specific;
- every container receives a named volume at `/home/agent/.claude`;
- the env denylist blocks `CLAUDE_CONFIG_DIR` but not `CODEX_HOME`;
- identity resolution produces a host path before the container branch;
- the container subprocess controller filters env variables but does not project a
  Codex account home.

Therefore the existing code can pass a Windows `CODEX_HOME` into a Linux container,
where it is unusable, while the mounted Claude volume is irrelevant. This must be
fixed before a Docker Codex smoke can be considered evidence of Armory/Stash
integration.

### 11.2 Required projection

For a Docker Codex agent with account `A`:

1. resolve `A` through the same direct-link gate used for a host turn;
2. resolve and validate `A`'s exact host account home;
3. bind-mount only that directory read-write at `/home/agent/.codex`;
4. set container `CODEX_HOME=/home/agent/.codex`;
5. remove/replace the host `CODEX_HOME` before Docker exec environment creation;
6. ensure the generated AgentMux profile is visible inside that mounted home;
7. run the provider CLI from the image, not a host-resolved executable path.

Read-write is required because Codex owns token refresh and session rollout state.
The mount source must be an already-resolved absolute path under the selected
account record. It must never come from user-authored `container_volumes`, an
unresolved environment variable, or display text.

The same Armory account may be linked to multiple agents and mounted into their
containers. That is the intended meaning of a reusable account. AgentMux does not
copy credentials into per-container homes, because copies would diverge after token
refresh and create multiple sources of truth.

### 11.3 Mount conflict and admission rules

- User-supplied volumes may not target `/home/agent/.codex` for a Codex agent.
- A second automatic mount may not shadow the runtime home.
- The host source must exist and be a directory before container creation.
- The container must run as the expected non-root agent user.
- The workspace mount and working directory must be explicit and must not be `/`.
- Bypass mode is admitted only for an AgentMux-managed container satisfying the
  hardened-runner checks; a container request that falls back to host execution is
  a hard error.
- Secrets remain off Docker argv and are supplied through the Docker API/mount
  configuration.

### 11.4 Image contract

The selected Codex image must contain the exact pinned Codex CLI or a version that
passes the fixture gate. Image inspection at launch or build-time pin consistency
tests must prove the expected version.

The provider registry selects the container command and expected auth home. The
container manager must not hardcode Claude paths for all providers. Migrating
Claude's existing named-volume behavior is a separate compatibility decision; Codex
must not depend on that migration to receive its own correct projection.

---

## 12. Runtime, JSONL, and pane behavior

The companion JSONL contract is normative for:

- first-turn and resume argv;
- stdin prompt delivery;
- `thread_id` persistence;
- item lifecycle reduction;
- command/file/MCP/web/plan rendering;
- usage, errors, cancellation, malformed records, and terminal gating;
- fixtures and version upgrades.

This spec adds these lifecycle requirements:

- account resolution and provider materialization finish before the subprocess is
  spawned;
- the runtime records the canonical account ID used for the turn in internal
  diagnostics, but never in assistant text;
- host and Docker turns use the same logical provider config and argv builder;
- resume uses the same account, profile, workspace, and runtime projection as the
  original thread unless the user explicitly rebinds the agent;
- after a rebind, a stored thread belonging to a different account is not resumed
  automatically; the pane requires an explicit new session or a verified migration
  policy.

Codex has no Claude control channel in this phase. AgentMux's bypass default means
ordinary tool approval prompts should not occur inside the managed Docker runner.
If Codex emits a request that requires an unsupported interactive response, the turn
fails visibly rather than hanging indefinitely.

---

## 13. State changes and events

The following mutations publish existing or provider-neutral events:

| Mutation | Required effect |
|---|---|
| Codex account persisted/updated | refresh Armory account cache/status |
| agent linked/relinked | refresh that agent's Stash Accounts table |
| stale alias removed after migration | refresh links without a false "revoked" warning |
| account deleted | refresh Armory and every formerly linked agent |
| Skill/MCP/Startup binding changed | invalidate provider materialization for next launch/turn |
| account probe changes status | refresh Armory/Stash status badges |

A live pane may receive a best-effort environment/profile refresh after re-login,
but durable storage remains authoritative. The next turn always re-resolves the
binding rather than trusting old pane metadata.

No event payload contains token contents, auth file contents, or unrestricted
config-directory listings.

---

## 14. Failure behavior

| Failure | Required behavior |
|---|---|
| Codex CLI missing | installer/resolve error; no pane spawn |
| login succeeds but account persistence fails | report bookkeeping failure; do not claim connected |
| account persists but Stash link fails | account remains in Armory; Stash reports link failure and retry reuses it |
| linked account missing/deleted | spawn gate blocks before CLI creation |
| host account path sent toward container | reject/projection invariant failure |
| automatic auth mount conflicts with user volume | container admission failure naming target |
| generated profile invalid or unwritable | block launch; preserve previous valid profile where possible |
| one MCP server cannot be represented securely | omit that server with visible validation error; do not weaken auth |
| generated skill collides with user-owned file | do not overwrite; report collision |
| container image lacks matching Codex CLI | version/image error; do not fall back to host binary |
| resume ID belongs to stale/different binding | no automatic replay; require explicit new session/recovery |
| unsupported interactive provider request | visible provider error and terminal turn cleanup |

Every failure must leave the pane out of an indefinite `Working` state. Errors are
persisted through the same transcript path used by other provider failures.

---

## 15. Implementation slices

### Slice A — evidence and JSONL adapter

Owned by the companion spec:

- capture real pinned/candidate Codex JSONL fixtures;
- implement the item reducer and terminal semantics;
- add malformed/unknown compatibility tests;
- do not change Armory schema or Claude behavior.

### Slice B — provider argv and session continuity

- introduce the provider-specific pure argv strategy;
- support `exec resume <thread_id>` for host and container paths;
- preserve bypass, JSON, color, profile, model, and stdin flags;
- add session lease and stale-resume tests.

### Slice C — provider-neutral Docker identity projection

- return structured OAuth path projection from identity resolution;
- add provider runtime-home metadata;
- mount the selected Codex account home at `/home/agent/.codex`;
- set container-local `CODEX_HOME` and prohibit host-path forwarding;
- add mount-conflict, missing-dir, host/container parity, and no-host-fallback tests.

This slice is required before the Docker live smoke can prove account correctness.

### Slice D — Armory/Stash Codex account UX

- generalize the Claude-only login panel where practical;
- add Armory Connect and Stash Connect/Re-login for `codex`;
- reuse the existing account mint/persist/link flow;
- verify canonical links before success;
- add delete/revoke/recovery and stale-alias tests.

### Slice E — provider-native materializer

- introduce provider dispatch around shared resource resolution;
- generate the AgentMux Codex profile with `developer_instructions` and MCP;
- generate managed `.agents/skills` entries;
- preserve user `AGENTS.md`, base Codex config, and unmanaged skills;
- make the Stash Memory surface provider-aware;
- add host/container path projection and deterministic-output tests.

### Slice F — integrated Docker smoke and pin decision

- launch a clean Docker Codex agent with a bound Armory test account;
- verify the account identity used by `codex login status` without exposing tokens;
- verify profile instructions, one bound skill, and one bound MCP server;
- run command/file-change/final-message fixture scenarios;
- resume a second turn by `thread_id`;
- revoke/delete the account and verify the next turn fails closed;
- upgrade the pin only if the candidate passes all fixture and integration gates.

Slices should land as separate reviewable changes. A slice may add shared
provider-neutral infrastructure, but it must not opportunistically rewrite Claude's
translator, control protocol, or memory system.

---

## 16. Test matrix

### 16.1 Account and binding

- fresh Armory Codex connect creates one account and no link;
- Stash Connect creates one account and one canonical `codex` link;
- Re-login reuses the existing account ID and directory;
- failed link verification does not show success;
- Retry after link failure does not mint a second account;
- legacy alias is removed only after canonical-link verification;
- delete removes links and blocks next spawn;
- `openai` API-key link does not satisfy `codex` OAuth gate.

### 16.2 Runtime projection

- host env contains the resolved absolute account home;
- container env contains exactly `/home/agent/.codex`;
- container env never contains the host account path;
- exact account home is mounted read-write at the runtime home;
- a conflicting user volume is rejected;
- missing or non-directory mount source is rejected;
- Codex container never falls back to a host process;
- account A and account B produce different mount sources;
- two agents linked to account A resolve the same source intentionally.

### 16.3 Materialization

- deterministic AgentMux profile name and content;
- base `config.toml` and repository `AGENTS.md` remain byte-identical;
- Soul/agent/global Bundle guidance appears in `developer_instructions` once;
- Startup Bundle appears in first-turn payload, not duplicated in profile;
- global + bound skills produce valid `SKILL.md` files;
- stale managed skills are removed; unmanaged files survive;
- global + bound MCP servers become valid TOML entries;
- secret-bearing MCP values use environment indirection;
- unsupported MCP config is visible and not partially weakened;
- generated paths work in both host and container projections.

### 16.4 End-to-end

- bound account shown in Stash equals account used by the host CLI;
- bound account shown in Stash equals account mounted into Docker;
- text/reasoning/tools/files/MCP render through JSONL fixtures;
- second turn resumes the same thread;
- process exit and cancellation clear `Working` exactly once;
- account deletion before the next turn blocks provider spawn;
- no auth token, auth file content, or host path appears in argv or transcript.

---

## 17. Acceptance criteria

Codex provider integration is complete when:

1. Armory can create and display a real isolated Codex account.
2. Stash can connect, re-login, and display the exact direct binding used at spawn.
3. Host Codex turns use the linked account's `CODEX_HOME` and never ambient login.
4. Docker Codex turns mount that same account at `/home/agent/.codex` and never
   receive a host config path.
5. Bypass remains the default only inside the admitted AgentMux container boundary;
   container failure never causes host fallback.
6. AgentMux guidance is delivered through an AgentMux-owned Codex profile without
   overwriting user `AGENTS.md` or base Codex config.
7. Bound Skills and MCP Servers are represented in native Codex formats.
8. Startup Bundle instructions arrive as first-turn action context.
9. Claude-native memory is not falsely presented as Codex memory.
10. JSONL rendering and session resume satisfy the companion contract.
11. Account deletion/revocation blocks the next provider spawn honestly.
12. The integrated two-turn Docker smoke passes with the exact pinned CLI.

---

## 18. Explicitly deferred

- Codex app-server or SDK adoption;
- a persistent Codex controller or mid-turn steering;
- a provider-native Stash UI for Codex memories/Chronicle;
- translation of Claude hooks into Codex hooks;
- changing the global Docker network/security policy;
- migrating existing Claude container credential volumes;
- automatic cross-account thread migration;
- unifying every provider's login UI in the first Codex PR;
- changing Kimi, Gemini, Qwen, Copilot, OpenClaw, Pi, or MuxCode behavior.

These are not prerequisites for honest Codex account, configuration, stream, and
resume integration.

---

## 19. Open questions requiring implementation evidence

1. Does the currently pinned `0.116.0` CLI support every documented profile,
   `developer_instructions`, skill-discovery, and MCP field used here, or must the
   pin advance before Slice E?
2. What legacy Codex provider aliases exist in real shared stores, if any? The
   migration list must be evidence-based rather than guessed.
3. Which Codex auth files must be retained during account-directory cleanup, and
   which non-auth caches can be rebuilt?
4. Are all existing Armory MCP JSON shapes losslessly representable in Codex TOML?
5. Does concurrent token refresh against one shared account home require an
   AgentMux-level per-account lock, or is upstream file handling sufficient?
6. What exact image should become the supported Codex container image, and how is
   its pin verified without running the host binary?
7. Should provider materialization run only at launch or before every turn when a
   Stash binding changes while a pane stays open? The implementation must choose one
   authoritative invalidation rule and test it.

These questions block relying on undocumented behavior. They do not block the
provider architecture or the first evidence/adapter slices.
