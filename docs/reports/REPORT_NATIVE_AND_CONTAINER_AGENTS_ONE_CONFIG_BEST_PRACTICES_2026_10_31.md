# REPORT: Native and container agents from one configuration: best practice and a DRY target

**Status:** analysis — approved by the operator on 2026-10-31; step 1 of section 5 (the parity harness) is in PR #6018, which also corrects section 3 (the files are built in three places, not one) and records what the harness found (section 8). Task #13666.

**Date:** 2026-10-31. **Code read:** `origin/main` at `0aa23ed8d`. **Web sources:** read 2026-10-31.
**Question (operator):** "ideally we want a clean and DRY architecture. the one path isnt the highest ideal, its just one, the highest is DRY, but also best practice for this use case, search online for latest practices, record findings to file"

## 1. Summary

- **Describe the agent environment once, as data, and run it through thin per-runtime adapters.** The products checked do this in some form: Dev Containers, OpenHands V1 (one agent plus a `Workspace` that is local, Docker or remote), Cursor `environment.json`, Copilot `copilot-setup-steps.yml`.
- **Put the native/container difference in tagged data and one adapter each, not in `if container` branches.** There are at least 15 such branches outside the container modules (section 3).
- **The content pipeline is built in three places, not one** (corrected by step 1; section 3): Rust `build_config_files` for `agent.open`, a TypeScript re-implementation (`agent-config-builder.ts`) for the window's Launch, and the part srv adds to the window's files. Make one Rust builder's output an explicit, snapshot-testable value (`AgentEnvSpec`) and delete the TypeScript one.
- **Replace four separate "who gets what" mechanisms with one audience/capability tag:** the `in_containers` flag, Operator Config `agent_types`, `CONTAINER_ENV_DENYLIST` and the `REFUSED` route table.
- **Binaries: one committed tool lockfile** read by both the image build and the native resolver. Image version skew is already handled by signed, digest-pinned images.
- **Reachability and credentials: keep the current design** (gateway-bound listener, per-block scoped tokens, no secrets baked into images). The weak spot is the Claude account file mounted into the container, which vendor guidance treats as exfiltratable.
- **Parity is a test result, not a claim.** Render one fixture agent for both runtimes and compare the difference against an explicit allowlist.

## 2. What best practice says

### 2.1 Define once, run anywhere

- The Dev Container spec separates the definition from the tool that runs it. Properties can sit in `devcontainer.json` or in an image label (`devcontainer.metadata`) and are merged: lifecycle hooks are collected into an ordered list, `containerEnv` and `remoteEnv` merge per variable, `mounts` are collected. `remoteEnv` is for values that change without a rebuild. Features (OCI artifacts) add tools and can contribute env, mounts and hooks. Sources: [JSON reference](https://containers.dev/implementors/json_reference/), [Features](https://containers.dev/implementors/features/).
- 12-factor config: keep config out of code; vary what differs per deploy through granular env vars; config that does not vary between deploys stays in code. Source: [12factor.net/config](https://12factor.net/config).
- Nix/devbox lockfiles pin transitive packages by hash; a Dev Container lockfile pins only the Feature artifact, not what its script installs. Sources: [DEV comparison, 2026](https://dev.to/guang_diyuzhi_74b0fec2c09/dev-containers-vs-nix-vs-devbox-standardizing-local-dev-in-2026-gp4), [lockfile spec](https://github.com/devcontainers/spec/blob/main/docs/specs/devcontainer-lockfile.md).
- Inference: Nix is a heavy requirement for end users; a lockfile of AgentMux's own tools is the proportionate version.

### 2.2 How agent products split local and sandboxed

- **Claude Code:** settings merge by scope (managed, CLI, local, project, user); project `.mcp.json` is the shared MCP config. The documented container route is a Dev Container Feature plus a named volume at `~/.claude` with `CLAUDE_CONFIG_DIR` set; the same settings and `.mcp.json` are used inside. Binaries that local MCP servers need go in the Dockerfile. The docs warn that with permission prompts skipped, anything in the container (including `~/.claude` credentials) can be exfiltrated, and advise short-lived tokens over mounted host secrets. Sources: [dev containers](https://code.claude.com/docs/en/devcontainer), [settings](https://code.claude.com/docs/en/settings).
- **Codex CLI:** the same CLI and `~/.codex/config.toml` everywhere. Sandbox mode (`read-only`, `workspace-write`, `danger-full-access`) and approval policy are separate settings. In Docker the OS sandbox (`bwrap` + `seccomp`) may not work; guidance is to let Docker isolate. Source: [Codex approvals and security](https://learn.chatgpt.com/docs/agent-approvals-security).
- **Cursor cloud agents:** the environment is a Dockerfile plus an `install` command in a committed `.cursor/environment.json`; secrets are separate. Source: [Cursor setup](https://cursor.com/docs/cloud-agent/setup).
- **Copilot cloud agent:** one workflow job, `copilot-setup-steps`, defines the environment. The firewall covers the agent's shell, not MCP servers or setup steps. Source: [GitHub docs](https://docs.github.com/en/copilot/tutorials/cloud-agent/give-access-to-resources).
- **OpenHands SDK V1:** the agent code is identical; a `Workspace` (local, Docker, remote) decides where tools run. The closest published match to "data plus adapters". Sources: [workspace architecture](https://docs.openhands.dev/sdk/arch/workspace), [paper](https://www.alphaxiv.org/abs/2511.03690).
- **Docker MCP Gateway:** each MCP server runs in its own container; secrets are mounted only into the target container. The gateway needs the Docker socket, so it is privileged. Sources: [Docker blog](https://www.docker.com/blog/mcp-toolkit-gateway-explained/), [repo](https://github.com/docker/mcp-gateway).
- **Common thread:** the environment definition and agent configuration are shared; the isolation and credential layer is what differs. E2B and Daytona differ mainly in isolation (microVM vs container), per [ZenML](https://www.zenml.io/blog/e2b-vs-daytona). Devin was not researched.

### 2.3 Tools into containers without listing them twice

- Options: bake into the image, Dev Container Features, init containers or shared volumes, Nix. (Init containers are from background knowledge, not searched.)
- All of them leave the tool list inside an install script rather than in data. The remedy used by Features and Devbox is a lockfile with versions and digests that tooling verifies. Source: [Features distribution](https://containers.dev/implementors/features-distribution/).

### 2.4 Host services from containers

- `host.docker.internal` is defined on Docker Desktop; on Linux it needs `--add-host=...:host-gateway` and resolves to the bridge gateway, not loopback, so a service bound only to `127.0.0.1` is unreachable that way. Hardening: bind to the bridge IP rather than `0.0.0.0` and firewall to the Docker subnet; user-defined networks have their own gateway. Sources: [Dash0](https://www.dash0.com/faq/how-to-access-a-host-port-from-inside-a-docker-container), [Baeldung](https://www.baeldung.com/ops/docker-compose-add-host).
- Alternative: a Unix socket. Anthropic's sandbox routes network through a Unix socket to a proxy outside it ([Anthropic engineering](https://anthropic.com/engineering/claude-code-sandboxing)); the cost is that both client binaries must speak the socket. Inference: socket mounts behave differently under Docker Desktop's VM.

### 2.5 Credentials into sandboxes

- Real secrets stay outside; the sandbox gets a scoped, short-lived token or a placeholder. Docker Sandboxes inject credentials from a host-side proxy and the sandbox sees a sentinel ([docs](https://docs.docker.com/ai/sandboxes/security/credentials/)). Anthropic keeps git credentials and signing keys out of the sandbox and proxies git ([Anthropic engineering](https://anthropic.com/engineering/claude-code-sandboxing)).
- Other documented patterns: socket-forwarded helpers (git credential helper, SSH agent) and brokers minting 60-300 s tokens (secondary sources: [AgentPatterns](https://agentpatterns.ai/security/scoped-credentials-proxy/), [DEV](https://dev.to/gabrielanhaia/computer-use-agents-3-sandboxing-patterns-that-dont-leak-credentials-4hci)).

### 2.6 Testing parity

- One contract run against every implementation; fixtures as language-neutral data; pin the contract with a checksum; fail when zero cases ran; include negative cases that prove the suite can fail. Sources: [.NET contract tests](https://ilovedotnet.org/blogs/contract-tests-the-acceptance-gate-that-proves-your-nuget-replacement-behaves-identically-in-dotnet/), [throttlekit](https://github.com/AmeyaBorkar/throttlekit-py/wiki/Conformance-and-Development).

## 3. What AgentMux does today

Paths are under `crates/srv/` at `0aa23ed8d`. V = verified by reading the code; L = from commit subjects only.

**Corrections to the 2026-10-27 parity spec (partly stale):**
- **Skills:** `build_config_files` writes `.claude/skills/<slug>/SKILL.md` and `.claude/commands/*.md` for every agent (`src/backend/agent_config.rs:146-170`), and the workspace is bind-mounted, so "no skills are written" no longer holds. Not shown: that Claude inside the container loads them (V for the code, unverified live).
- **Linux reachability: fixed.** `src/backend/container_listener.rs` binds the `agentmux-agents` gateway, admits only container tokens from the agents subnet and refuses the denylisted routes (`:1-24`, layer at `:84`); `container_exec_env` points `AGENTMUX_LOCAL_URL` at it (`src/backend/container_credential.rs:180`). The SessionStart memory fetch for containers landed in #5593 (L).
- **Personal Memory:** now the same folder native sessions use, mounted into the container (`src/backend/container/mounts.rs:19,28,47-58`; #5592). The per-agent volume at `/home/agent/.claude` remains for the rest of Claude's state.

| Piece | Status | Evidence |
|---|---|---|
| Instructions, settings, hooks, `.mcp.json`, skills | BUILT THREE TIMES | `agent.open`: Rust `build_config_files` `agent_config.rs:81`, `build_settings_with_hooks` `:428`, `build_mcp_config` `:218`. The window's Launch: TypeScript `buildConfigFiles` in `frontend/app/view/agent/agent-config-builder.ts`, called from `agent-model.ts` (a re-implementation; its comments say "the same rule as Rust's ..."), then `writeagentconfig` adds the catalog servers, signing keys, Global Memory and picked bundles (`server/picker_config.rs`). Hooks name `agentmux-bashwrap` by bare name. V |
| Entry points into that code | DUPLICATED (and different inputs) | `agent.open` (`server/app_api/agent_open.rs:1022-1079`) and `writeagentconfig` (`server/editor_handlers.rs:271`, via `mcp_launch::add_to_files`) each decide "container" and handle signing keys separately. V |
| MCP servers | Shared code, different rule | `in_containers` flag at `storage/mcp_servers.rs:168-180`; a second derivation of "is container" at `backend/mcp_launch.rs:68`. V |
| Operator Config audience | DUPLICATED mechanism | `agent_types` in `operator-config-seed.json:20-27` (workspace), `:33-37` (widgets); filter at `backend/operator_config_seed.rs:131-138`. V |
| Env | DUPLICATED mechanism | `CONTAINER_ENV_DENYLIST` at `backend/container.rs:176`, applied in `container_credential.rs:145-180`. A denylist by name: a new host-path variable leaks until someone lists it. V |
| Tool binaries | DUPLICATED | `agentmux-mcp` and `agentmux-bashwrap` copied in `docker/Dockerfile.agent-agentmux:144-145`; other tools added per image (#5746, #5940, L). No list shared with the native side. Images pinned by signed-manifest digest, version tag and idle auto-update (L). |
| Skills | SHARED code | `agent_skill_files.rs` refreshes skill files on controller start; container coverage not confirmed live. |
| Personal Memory | Shared data, different mount | `container/mounts.rs:47-58`. V |
| Credentials | ADAPTER (good) | Account file mounted when file-backed (`mounts.rs:47-52`); instance key swapped for a per-block container token (`container_credential.rs:152-160`); git through tokens and in-container hooks. V |
| Host reachability | ADAPTER (good) | `container_listener.rs`; the `REFUSED` table in `container_routes.rs` is open by default for any new route (header comment). V |
| "Is container?" branches | SCATTERED | At least 15 sites outside the container modules, including `server/agent_handlers/input.rs:910,994,1063`, `server/app_api/agent_io.rs:491,551`, `agent_open.rs:493,498,1039`, `blockcontroller/mod.rs:965`, `server/cron.rs:267`, `server/app_api/github_token.rs:58`, `agent_context.rs:49`, `mcp_json_identity.rs:194,235`. V |
| In-binary special case | SPECIAL CASE | `AGENTMUX_IN_CONTAINER=1` read by `crates/bashwrap/src/mps_client.rs:41` and `crates/mcp/src/self_keys.rs:64`, so the binaries branch too. V |

**Reading:** the content is not DRY either: the same files are built in Rust and in TypeScript, and the two already differ (section 8). The audience (who gets what) is expressed four ways plus call-site branches. Both are where the refactor belongs.

## 4. Target architecture

**One description, `AgentEnvSpec`** (serializable, snapshot-testable), produced by a single `resolve_agent_env(agent_def, stores)` that replaces the two inline assemblies in `agent_open.rs` and `editor_handlers.rs`:
- `files`: startup file, settings, `.mcp.json`, skills. Content refers to tools by logical id (`ToolId::Bashwrap`, `ToolId::Mcp`), not runtime facts.
- `mcp_servers`: each with an audience.
- `env`: `Vec<EnvSpec { name, value, scope }>` with `scope` = Everywhere | HostOnly | Rewrite, set where the variable is created (`pane_env.rs`, the gh and publish guards). Replaces `CONTAINER_ENV_DENYLIST` and its allow-by-default hole.
- `memory_sections`: each with an audience, replacing the seed's `agent_types`.
- `tools`: `{ id, version, sha256 }` from a committed `tools.lock.json`.
- `mounts`: roles (Workspace, PersonalMemory, Credentials, Home), not paths.
- `capabilities`: `host_exec`, `host_write`, `host_read`, encoding the operator's rule that a container can do everything except write or run on the host.

**One audience/capability enum and one `filter(spec, runtime_caps)`.** Route registration carries the tag; `REFUSED` is derived from it.

**Thin adapters behind one `RuntimeAdapter` trait** (`materialize`, `launch`, `tool_path`, `srv_endpoint`, `map_mount`, `credentials`): `NativeAdapter` wraps the current subprocess controller; `ContainerAdapter` wraps `container*.rs`, the gateway listener and token minting. A later WSL or remote adapter implements only those methods.

**Why this beats `if container` branches:**
1. The differences are listed in one place and show up as a diff of two renderings.
2. A new runtime adds one adapter and passes the same conformance test; nobody audits 15 call sites.
3. The default flips from "open unless listed" to "every item declares its audience", the safer failure mode for a security boundary (inference; the current route rule is deliberately open by default, see risk 1).
4. It matches the OpenHands and Dev Container shapes, familiar to outside contributors.

`devcontainer.json` itself is not recommended as the source: it is container-only, with no native equivalent.

## 5. Migration plan

One PR each; each test fails on `origin/main` today.

1. **Parity harness, no behaviour change.** Render a fixture agent through `agent.open` and `writeagentconfig`, native and container. Assert the entry points agree, and that the native-vs-container difference equals an explicit allowlist file.
2. **Tag env at the source.** Every `pane_env` and guard variable gets a scope; delete `CONTAINER_ENV_DENYLIST`. Test: every emitted variable has a scope.
3. **Extract `resolve_agent_env`.** Both RPCs call it; branches that pick files or servers read the spec. Test: byte-identical output from both entry points, one "is container" derivation (retire the split at `mcp_launch.rs:68`).
4. **Tool lockfile.** `tools.lock.json` consumed by the Dockerfile stage and the native resolver. Test: the Dockerfile `COPY` list equals the lock entries, and every `ToolId` used in hooks or MCP config is in the lock.
5. **Unify audience.** Replace `in_containers`, `agent_types` and the separate route list with the shared tag; the Operator Config `workspace` and `widgets` entries become tagged data. Test: every registered route carries a tag, and the generated denylist equals today's `REFUSED` table. Highest risk, so after 1-4.
6. **Adapter trait.** Move `container_exec_env`, mounts, endpoint and token minting behind `RuntimeAdapter`; the existing live tests (`container_listener_itest.rs`, `container_turn_itest.rs`) run unchanged against it.
7. **Prove it with a third runtime** (WSL or remote): adapter only, same conformance test.

## 6. Risks and open questions for the operator

1. **Route default.** Today a new route is open to containers unless listed. A tag per route turns that into "declare it or fail the build". Acceptable, or is open-by-default intentional?
2. **Claude credentials in the container.** The account file is mounted (`mounts.rs:47-52`). Vendor guidance (Claude Code docs, Docker Sandboxes) prefers a proxy that injects the credential so it never enters the sandbox. Accept the mount for now, or plan an injecting proxy?
3. **TCP gateway vs Unix socket.** The gateway listener is tested on Linux; Docker Desktop goes through `host.docker.internal`, not tested here. A socket means changing both client binaries. Keep TCP?
4. **Skills in containers.** Shared code, not confirmed live; needs one end-to-end check before step 1 claims parity.
5. **Native tool resolution.** `.mcp.json` uses the bare command `agentmux-mcp` for both runtimes; how the native pane's `PATH` finds it was not traced.
6. **Review.** Step 5 touches security routing; the platform (security) lead should review it before the build starts.
7. **Dev Container Features for the developer image?** Reuses ecosystem tools but adds a toolchain; not before step 4.

## 8. Step 1 result: what the parity harness found (2026-10-31)

The harness (`crates/srv/src/server/app_api/agent_config_parity_tests.rs`, fixture and allowlist in `crates/srv/tests/fixtures/agent_config_parity/`, window side in `frontend/app/view/agent/agent-config-parity.test.ts`) renders one fixture agent through `agent.open` and through the window's Launch, as a native and as a container agent, and compares a container turn's environment with a native one's. Every difference must be in the allowlist with its reason and what differs inside the file; an entry that stops being a difference fails too.

- **Native vs container (both entry points):** `.mcp.json` (host-only catalog servers left out; no signing keys, no `AGENTMUX_CHANNEL`) and the startup file (Global Memory entries for host agents only). Wanted.
- **Environment:** eleven names differ (host paths dropped, the instance key swapped for a container token, build info and the in-container hint added, file-watcher polling on Windows and macOS). Wanted; step 2 replaces the list with tags at the source.
- **Between the two entry points (a native or a container agent): one real divergence.** `agent.open` writes `Bash(agentmux-bashwrap *)` into `permissions.allow` of `.claude/settings.json`; the window's TypeScript builder does not. Everything else the harness compares (the startup file after Global Memory, `.mcp.json`, skills) agrees, JSON key order aside.
- **Not covered yet:** the window's side is its *output* pinned as a golden, not its code run against the Rust builder; the Rust half of the picker path (`picker_files`) is the code `writeagentconfig` runs, extracted unchanged so the test and the handler share it. Skills inside a container are still unconfirmed live (section 6, question 4).
- **Next:** step 3 builds the files in Rust for the window's Launch too (by agent id) and deletes `agent-config-builder.ts`'s `buildConfigFiles`; this harness is the proof it changed nothing it should not.
