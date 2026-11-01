# Spec: container agents get AWS credentials per use, brokered by srv (#13654; build #13655)

**Date:** 2026-10-31 · **Owner:** AgentA (srv + shared-infra), for AgentY's decision of 2026-10-31; first user to check it against: JCont (a container agent on narko)
**Status:** active — approved 2026-10-31 (the operator: "yes, aws decisions are go, proceed"; §17's decisions stand as recommended). Built in PR #6042 (Kanban #13655: the broker: policy, mint, cache, route, audit, `agentmux-bashwrap aws-credentials`, the container's config and variables); §9's pane row and §16's docs page are follow-ups.
**Related:** `SPEC_CONTAINER_AGENTS_AT_PARITY_2026_10_27.md`, `SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md`, `SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md`; the GitHub path this copies: `crates/srv/src/server/app_api/github_token.rs`, `crates/bashwrap/src/github.rs`. The fleet's own policy and infrastructure are in a private repository and are not described here (§13).

## 1. The problem

JCont, a container agent, tried to deploy and could not: there is no `aws` on its PATH, no credentials, and nothing it could be given safely. A native agent deploys because the host has an AWS identity. A container agent runs untrusted-ish code in its own filesystem, so the host's identity must not be copied into it, and long-lived keys must never be written there.

GitHub already solves the same problem. A container agent's `git` and `gh` ask srv for a token each time they need one (`POST /api/v1/github/token`); srv knows who asked from the container's own token, mints a short-lived token on the host, and nothing is stored in the container. This spec does the same for AWS.

## 2. Decisions this spec builds on (AgentY, platform lead, 2026-10-31)

- **No AWS keys in the container.** Credentials arrive per use and expire.
- **Not CI-only.** CI deploys (a workflow per repository) may come later for production, but they don't cover reads, logs or ad-hoc work.
- **No host-run deploys.** srv does not run a command on the host on an agent's behalf. It hands over credentials; the agent runs the command in its own container.
- **Use AWS's standard `credential_process`.** The container's AWS config points each profile at `agentmux-bashwrap aws-credentials --profile <name>`, which asks srv over the container listener with the agent's scoped token. Any AWS SDK or CLI in the container, and tools built on them (CDK, Terraform, boto3), then work unchanged.
- **srv gets credentials from the host's own AWS setup**, whatever the person uses (SSO, a named profile, IAM Roles Anywhere). That makes this work for any AgentMux user, not only our fleet.
- **Policy lives on the host:** which profiles a container agent may get, reads and writes apart, a reason with every write grant, every grant in the Audit view under the agent's name.

## 3. The flow

```
 agent runs: AWS_PROFILE=deploy AGENTMUX_AWS_REASON="deploy stack X" cdk deploy
   └─ the AWS SDK reads $AWS_CONFIG_FILE, finds [profile deploy]:
        credential_process = agentmux-bashwrap aws-credentials --profile deploy
   └─ agentmux-bashwrap (in the container) ── POST /api/v1/aws/credentials ──► srv (container listener)
        header: the container's token; body: {profile, reason, duration_seconds}
   └─ srv: who is this agent? (the token, never the body)  → is native? refuse
        policy: may this agent have this grant?            → refuse with a reason
        reason present when the grant needs one?           → refuse with how to give it
        cache: a session for (agent, grant, reason) with ≥ 10 minutes left? → return it
        else mint on the host (§7), record in Audit (§9), put one row in the agent's pane
   ◄─ {AccessKeyId, SecretAccessKey, SessionToken, Expiration}, as credential_process JSON
```

Nothing is written to disk in the container. The SDK holds the credentials in memory and asks again when they are about to expire.

## 4. What the container gets

srv renders one AWS config from the policy (§6), the same for every agent (who may have which grant is decided on each request, §5), and mounts its folder read-only, as it mounts the git hooks (`github_hooks_host_dir` in `ContainerMountSpec`): `/opt/agentmux/aws/config`. It sets, in the container's exec environment (`container_exec_env`):

| Variable | Value | Why |
|---|---|---|
| `AWS_CONFIG_FILE` | `/opt/agentmux/aws/config` | the profiles below; not the agent's `~/.aws/config` |
| `AWS_SHARED_CREDENTIALS_FILE` | `/dev/null` | a key in `~/.aws/credentials` is never read |
| `AWS_EC2_METADATA_DISABLED` | `true` | no 1-second wait for an instance-metadata server that isn't there |
| `AWS_PROFILE` | the policy's `default_grant`, if it names one | a bare `aws s3 ls` works for the free reads |

The config, for a policy with three grants (names are the policy's, not fixed):

```ini
# Written by AgentMux; read-only. Change the grants in ~/.agentmux/aws-grants.json on the host.
[default]
credential_process = agentmux-bashwrap aws-credentials --profile read
region = us-east-1

[profile read]
credential_process = agentmux-bashwrap aws-credentials --profile read
region = us-east-1

[profile deploy]
credential_process = agentmux-bashwrap aws-credentials --profile deploy
region = us-east-1
```

- Every grant in the policy is listed, even one this agent will be refused: the refusal then says why (§10), where a missing profile would only say "profile not found".
- The file holds names and the command, never a secret, so it is safe to mount and to show.
- Nothing about this is a security boundary: an agent can unset the variables or write its own config. It gains nothing, because srv decides on every request (§5).
- **The AWS CLI** and **the AWS CDK CLI** (`cdk`, which the deploy CLI runs for a diff or a deploy) are in the developer image (pinned versions in `docker/Dockerfile.agent-developer`).
- **The host's deploy and secrets CLIs** are private packages, so they can't be in the public image, and no registry token ever enters a container. Instead, for an agent that at least one grant is open to, srv copies the host's installed packages (their `package.json`, `bin`, `dist` and `node_modules`; not their docs) into `container-tools/` under the AgentMux root and mounts it read-only at `/opt/agentmux/tools`. Two `sh` launchers in `bin/` run them with the image's own `node`, and the image's `PATH` ends with `/opt/agentmux/tools/bin`. A new version of a package gets its own folder (a running command keeps its files; an old one is removed a day after the launcher stops pointing at it). The package is found from where the host's `PATH` puts the command (a link into `lib/node_modules` on Unix, a `.cmd` beside `node_modules` on Windows), and only if its `package.json` names the package and its version is a plain word; no program is run to look. A host without a CLI gets a launcher that says, in one sentence, that this container has the AWS CLI only and what a person installs on the host; the container picks the CLI up the next time its mounts are checked.

## 5. srv: `POST /api/v1/aws/credentials`

Request (JSON): `{ "profile": "<grant name>", "reason": "<text, optional>", "duration_seconds": <int, optional> }`. The caller is the agent its container token was minted for, as in `github_token.rs`; a request names no agent. Added to `http_action_routes.rs` (a write route) and to the routes that need an identity (`route_needs_identity`, which is the default for a new route).

Answer, `200`: the `credential_process` JSON, as the SDKs read it: `{"Version":1,"AccessKeyId":…,"SecretAccessKey":…,"SessionToken":…,"Expiration":"<RFC 3339 UTC>"}`. Anything else, `4xx`/`5xx` with `{"error": "<code>: <sentence>"}`.

Checks, in this order (the first that fails is the answer):

1. **A verified container agent.** No agent token: `401`. A native agent: `403 NATIVE_AGENT` (§11).
2. **AWS is shared on this machine.** No policy file, or no grants: `403 AWS_NOT_SHARED`.
3. **The grant exists** and is not disabled: else `404 UNKNOWN_PROFILE`, naming the grants.
4. **This agent may have it** (`agents` allow list, §6): else `403 NOT_ALLOWED_FOR_YOU`.
5. **The reason**, when the grant needs one: else `400 REASON_REQUIRED`.
6. **Duration** within the grant's bounds: clamped to the maximum; below the minimum, raised to it.
7. **Rate:** at most `max_mints_per_hour` mints per agent per grant (default 12; cache hits are not mints): else `429 TOO_MANY_MINTS`.
8. **Cache** (§8), else **mint** (§7). A mint that fails is `503` with the host's own words (`AWS_HOST_REFUSED`, `AWS_HOST_LOGIN_NEEDED`, `AWS_CLI_MISSING`, `AWS_TOOK_TOO_LONG`).

## 6. The policy

`~/.agentmux/aws-grants.json` on the host (`AGENTMUX_AWS_GRANTS_FILE` points an instance at another file, for tests and for an isolated channel that should not share the person's), read on each request and cached by modification time, so an edit takes effect without a restart and a container's config file is re-rendered the next time one starts. A malformed file is an error that names the line, and the previous good version stays in force.

```json
{
  "version": 1,
  "region": "us-east-1",
  "default_grant": "read",
  "grants": {
    "read": {
      "tier": "read",
      "mint": { "kind": "host_profile", "profile": "work" },
      "default_seconds": 3600, "max_seconds": 3600,
      "needs_reason": false,
      "agents": ["*"]
    },
    "deploy": {
      "tier": "write",
      "mint": { "kind": "command", "argv": ["my-escalate", "--reason", "{reason}", "--duration", "{seconds}", "--session-name", "{session}"] },
      "default_seconds": 1800, "max_seconds": 3600,
      "needs_reason": true,
      "agents": ["jcont"],
      "max_mints_per_hour": 6
    }
  }
}
```

| Field | Meaning | Default |
|---|---|---|
| `tier` | `read` or `write`. Decides defaults only (a write grant needs a reason unless it says otherwise) and how the Audit row and the pane row read. AgentMux cannot tell what a credential allows; the cloud-side role does. | — (required) |
| `mint` | how srv gets the credentials (§7) | — (required) |
| `default_seconds`, `max_seconds` | session lifetime asked for and its cap; 900 to 3600 | 3600 (read), 1800 (write); max 3600 |
| `needs_reason` | the request must carry a reason | `false` for read, `true` for write |
| `agents` | agent slugs (lower case) or `"*"` | `[]`: nobody |
| `max_mints_per_hour` | per agent, per grant | 12 |
| `disabled` | keep the grant listed and refuse it with its `note` | `false` |

- **Defaults are closed:** no policy file, no grants. A fresh install shares nothing, and a container agent's `aws` says so (§10).
- Grant names are `^[a-z][a-z0-9-]{0,40}$`. They are what the container's profiles are called.
- **Who writes the policy:** the person, or an agent acting for them. Nothing here is gated on the operator's words; the file is what the person decided to share, and every use is audited.

## 7. Minting on the host

srv never runs a shell. A recipe is an argument vector; placeholders `{reason}`, `{seconds}`, `{session}` (`<agent slug>-<grant>-<unix ms>`), `{agent}` are substituted whole into one argument each and never split. The reason is cleaned to letters, digits, spaces and `_.:/=+-@`, at most 256 characters, because AWS accepts nothing else in a session tag.

- **`host_profile`:** `aws configure export-credentials --profile <profile> --format process`. It works for SSO, assumed-role profiles, `credential_process` profiles and IAM Roles Anywhere alike, and it returns the profile's own credentials with their own expiry (the AWS CLI v2 command; a CLI too old to have it is `AWS_CLI_MISSING` with the version needed). A profile whose SSO login has lapsed is `AWS_HOST_LOGIN_NEEDED`: "run `aws sso login --profile <p>` on this computer".
- **`command`:** any program that prints `credential_process` JSON on stdout. This is how a fleet puts its own escalation (assume a role with a reason, in another account) behind a grant: the program does the assuming, srv does the policy, the cache and the audit. The fleet's recipe is in §13.
- Both run with a 25-second limit (the GitHub path's), a clean environment (`PATH`, `HOME`, `AWS_*` of the host profile only, not the agent's), the container's agent id in `AGENTMUX_AGENT_ID`, and no stdin. Their stderr is kept (first 400 characters) for the refusal text; their stdout is parsed and dropped.
- A recipe's stdout is read as its **last non-empty line**, which must be the credential JSON: a tool that prints a banner first still works, and nothing else on that line does. Credentials with no session token or no expiry (a long-lived key) are refused, whatever the recipe says: nothing that does not expire is handed to a container.
- The result is held in a `Zeroizing` buffer; the secret and the token are never logged, never put in the Audit row or the pane row, never in an error. A recipe that prints something that is not credential JSON is `AWS_HOST_REFUSED` with only the program's stderr.

## 8. Expiry, caching and refresh

- `credential_process` is called by the SDK whenever it needs credentials, and the **AWS CLI calls it on every command** (it does not cache a process provider). Without a cache every `aws` call would be a new mint, a new audit row and, for a fleet, a new notice.
- srv keeps a session per **(agent, grant, reason, account label)** and returns the same credentials, with the same `Expiration`, while at least **10 minutes** remain. After that it mints a new one.
- **Why 10 minutes:** botocore (the CLI, boto3) treats credentials with less than 10 minutes left as expired and refuses ones that are "refreshed but still expiring". A cached session returned any later would fail in those tools. The consequence is that a session's usable life is its duration minus 10 minutes: a 30-minute write session serves 20 minutes of work. That is why the defaults are 30 and 60 minutes, not the 15-minute minimum.
- A **new reason** is a new session, so a CloudTrail-style record of why matches what the agent said it was doing. Repeating the same reason reuses the session.
- A long job (a deploy that outlives its session) asks again in-process; the answer is a new session (a new audit row, "renewed"). The hourly cap in §5 bounds a loop that asks too often.
- The cache is in srv's memory only. A restart drops it; the next request mints. A container stopping or being removed drops that agent's entries (`revoke_block` already runs on that path).
- Concurrent requests for the same key share one mint (the first does it, the others wait for the same result).

## 9. Audit and what the person sees

- **Audit view:** every grant and every refusal is one `action` row (`log_fleet_action_audit`), actor = the container agent, action `aws.credentials`, with the grant name, tier, the account label the recipe reports (if any), the reason as the agent wrote it, the seconds granted, whether it was a fresh mint or a cache hit (cache hits are one row per session, not per call), and for a refusal the code. No credential, no token, no key id.
- **The agent's pane (follow-up):** one row when a fresh write-tier session starts, in the style of the container-limit row (`note_in_pane`): "AWS: deploy credentials for 30 min. Reason: deploy stack X". It needs a new row type in the agent pane's renderer, so it is not in the first build; the Audit row is the record until it lands. A read-tier session makes no row.
- **The person is told, not asked:** the grant exists because the person wrote it into the policy. There is no per-use approval prompt, which would make an agent wait for a human at 3 a.m. and would count as an operator-only gate. Visibility is the Audit view, the pane row, and, for a fleet, the cloud account's own record (§13).
- `GET /api/v1/aws/credentials/status` is not added. `audit.query` already reads the rows.

## 10. How a refusal reads

The text goes to the SDK's stderr through `credential_process`; the AWS CLI prints it after "Error when retrieving credentials from custom-process:". Each says what happened and what to do, in one or two sentences:

| Code | Text |
|---|---|
| `AWS_NOT_SHARED` | "AWS isn't shared with container agents on this machine. A person adds grants in ~/.agentmux/aws-grants.json (the AgentMux docs have a page on it); until then there is no AWS from here." |
| `UNKNOWN_PROFILE` | "No AWS grant named 'x'. This machine shares: read, deploy." |
| `NOT_ALLOWED_FOR_YOU` | "The grant 'deploy' isn't open to you. It is open to: jcont. Ask the person, or an agent that can edit the policy, to add you." |
| `REASON_REQUIRED` | "The grant 'deploy' needs a reason. Run the command with AGENTMUX_AWS_REASON='what and why' in front of it." |
| `TOO_MANY_MINTS` | "You asked for 'deploy' credentials 12 times in the last hour. The job may be looping; wait or ask for a higher cap." |
| `AWS_HOST_LOGIN_NEEDED` | "The host's AWS login for 'work' has lapsed. A person runs `aws sso login --profile work` on this computer." |
| `AWS_CLI_MISSING` | "This computer has no AWS CLI (v2.9 or newer), which the 'read' grant uses. Install it, or use a grant with a command recipe." |
| `AWS_HOST_REFUSED` | "The host's AWS refused: <its message, first 400 characters>" |
| `AWS_TOOK_TOO_LONG` | "AWS credentials took too long (25 s). Try again in a minute." |
| `NATIVE_AGENT` | "This is for container agents. A native agent has the host's own AWS setup." |

A person with no AWS at all gets `AWS_NOT_SHARED` first, and, with a policy but no AWS CLI on the host, `AWS_CLI_MISSING`. Neither falls back to another credential, the way the GitHub path never does.

## 11. What a native agent does differently

A native agent runs on the host and already has the host's AWS setup (its own `~/.aws/config`), so it calls AWS directly and escalates with whatever the person's setup provides. The broker is for containers only: `/api/v1/aws/credentials` refuses a native caller with `NATIVE_AGENT`, so there is one path with one purpose and no policy a native agent could be confused by. The *rule* is the same for both (reads are free, writes need a reason and are recorded), enforced for a native agent by the cloud account and for a container agent by srv's policy and audit on top of it.

## 12. In the container: `agentmux-bashwrap aws-credentials`

`aws-credentials --profile <name>` (synchronous, like `git-credential`, no async runtime to start):

1. Reads `AGENTMUX_AWS_REASON` (the reason) and `AGENTMUX_AWS_DURATION` (seconds, optional) from its environment, which it inherits from the SDK process, which inherits the agent's command line.
2. `POST /api/v1/aws/credentials` through `WpsClient::from_env()` with `AGENTMUX_AGENT_TOKEN`, 30-second limit.
3. On `200`, prints the JSON on stdout and exits 0. On anything else prints the error sentence on stderr, nothing on stdout, and exits 1.
4. Prints nothing but the JSON on stdout, so a broken answer cannot be mistaken for credentials.

## 13. The fleet's policy and infrastructure

The grants for our own fleet (which cloud roles, accounts, escalation tiers and agents), the cloud-side roles a read grant needs, and the changes to our deploy and secrets CLIs so the commands agents already run work the same in a container are in the fleet's private spec of the same date ("container agent AWS broker, fleet half"), kept in its private repository. Nothing in this document depends on them being any particular shape: any `command` recipe that prints `credential_process` JSON works.

## 14. Security notes

- **A compromised container can use credentials while they are valid.** That is why sessions are short (30 to 60 minutes, minus the 10-minute margin), scoped to a grant, tied to a reason and an agent, rate-limited, and every one is in the Audit view. A grant that gives write access should be given to few agents and for narrow roles; AgentMux cannot narrow a cloud role from here.
- **The container token** (`amxc_…`, per block) is what proves who is asking; the listener admits only container tokens from the agents subnet. A different container's token is a different agent's identity: one container cannot ask as another.
- **srv's own AWS identity is never given out.** A `host_profile` recipe returns the profile's credentials, so a policy must not name a profile that can do more than the agents should (a person's administrator profile). The policy docs say so, and the example uses a read profile.
- **No shell, no argument injection:** placeholders substitute whole arguments (§7); the grant name is a closed pattern; the reason is cleaned; a `command` recipe's argv is the person's own text in a file only they edit.
- **No credential in logs, errors, Audit, pane rows or the debug log.** A test greps all of them for a planted secret (§15).
- **Residual risk:** an agent that can write the policy file can grant itself access. The policy file is on the host, outside the container's mounts; a container agent cannot reach it. A *native* agent can edit it, as it can edit any file of the person's; that is the existing trust between a person and the native agents on their host.

## 15. Tests (fail first)

1. **Refusals, one test each, before the code exists:** native caller; no agent token; no policy; unknown grant; an agent not on `agents`; a write grant without a reason; the hourly cap; a recipe that exits non-zero; a recipe that prints non-JSON; a recipe that hangs (timeout).
2. **Mint:** a `command` recipe that prints fixed JSON returns it as given; the placeholders arrive as whole arguments (a reason containing `; rm -rf /` and spaces is one argument, cleaned); the recipe's environment is the clean one.
3. **Cache:** two requests for one key mint once; a different reason mints again; a session with 9 minutes left is not reused; concurrent requests mint once; the cache drops on `revoke_block`.
4. **Config render:** the file lists every grant, marks `default_grant`, contains no secret, and is re-rendered when the policy changes; the mount and the four variables are in the container's exec environment (`container_exec_env`), and a host agent's environment gets none of them.
5. **bashwrap:** stdout is exactly the JSON on success and empty on every failure; the reason and duration reach srv; a refused answer's sentence reaches stderr unchanged.
6. **Audit:** a grant and a refusal each make one `action` row with the fields in §9; a cache hit makes none; no row, log line, error or pane row holds a planted secret, session token or key id (a test seeds the recipe's output with recognisable values and scans everything written).
7. **Isolation:** container agent A's token never receives agent B's cached session.
8. **Live (build task, by hand, with JCont):** in a developer-image container on narko, `AWS_PROFILE=<read grant> aws sts get-caller-identity` works with no `~/.aws`; a write grant without a reason is refused with the §10 text; with a reason, a harmless write succeeds and the Audit view shows the row; `cdk`/the deploy CLI runs a diff. Record the result in the PR.

## 16. Build plan (task #13655)

Small PRs, in this order. Each is mergeable alone and does nothing visible until the last.

1. **Policy + mint + cache + endpoint** (built): `crates/srv/src/identity/aws_broker/` (`policy.rs`, `mint.rs`, `cache.rs`), `server/app_api/aws_credentials.rs`, the route, `http_action_routes.rs`, the audit call; tests 1–3, 6, 7.
2. **bashwrap `aws-credentials`** (`crates/bashwrap/src/aws.rs`, `main.rs`) and its tests (5) (built).
3. **The container side** (built): the rendered config (`identity/aws_broker/container.rs`) and its mount (`container/mounts.rs`), the variables (`container_credential.rs`); test 4. The AWS CLI in the image and the host's CLIs (`identity/aws_broker/tools.rs`, mounted by `container/mounts.rs`) are built; the pane row (§9) is a follow-up.
4. **Docs** (agentmux-docs: a "container agents and AWS" page: the policy file, the refusals, how to share a profile) and the live check (test 8).

Every PR carries a changeset. The public docs page uses the §10 sentences verbatim.

## 17. Decisions (answered 2026-10-31, as recommended)

1. **A fresh install shares nothing.** A first-run prompt that offers a host profile as a read grant was the alternative. Nothing-by-default is safe and matches how the container's git works today (no token until a source exists).
2. **The reason travels in the environment only.** A standing reason (`aws-reason set "..."`, stored per agent for an hour) for a tool launched by something that drops the environment was the alternative, and is deferred: the environment reaches every child of the command line, which covers the deploy and CDK cases.
3. **No Settings page for the policy in this build.** The file and the Audit view are the interface. A page (the grants, who may use them, the last uses) is worth a follow-up if more than the fleet uses it.

## 18. Not doing

- Running `aws`, `cdk` or a deploy on the host for an agent.
- A per-use approval prompt (§9).
- Giving native agents a second path to the same credentials (§11).
- Caching credentials on disk in the container, or in the policy file.
- Narrowing what a cloud role may do: that is the cloud account's job, and the fleet's roles are in the private document.
- CI-only deploys; they can come later, per repository, beside this.
