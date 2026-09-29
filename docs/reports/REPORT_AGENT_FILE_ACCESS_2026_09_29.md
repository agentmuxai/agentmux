# Report: what agents can reach with the instance auth key, and the fix for container agents

**Date:** 2026-09-29
**Author:** Korp (narko)
**Status:** analysis. Phase 1 (container agents get a scoped credential) ships with this report; §8 lists the follow-ups.
**Builds on:** `docs/specs/SPEC_PANE_CREDENTIAL_HANDOFF_2026_09_18.md` (proposed), the §9 follow-up in `docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md`, and the same-user limit in `docs/specs/SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.1.

## 1. Summary

- Every agent process, **including container agents**, receives srv's single instance auth key (`AGENTMUX_AUTH_KEY`). The UI uses the same key, and it authorizes every route srv serves.
- With that key a caller can read any host file by absolute path (`/agentmux/stream-local-file`, `readeditorfile`, `listeditordir`, `attachments.ingest`), write inside `$HOME` (editor routes), and **run any command on the host** (`/api/v1/shell/create`, `/api/v1/ptyshell/create`, `checkcliauth`).
- **Host agents** already run as the user and can do all of that directly, so for them the key adds no file access. Their remaining problem is impersonation between agents, which is the same-user limit already recorded in the identity spec.
- **Container agents** are the real gap. They are presented as an "isolated Docker sandbox" (`SPEC_HOST_VS_CONTAINER_AGENTS_2026_06_18.md` §3.3) that sees only `/workspace`. We confirmed on narko that a container on the `agentmux-agents` network reaches srv through `host.docker.internal` (§2.3), and the code passes it the instance key. So a container agent can read and change the host, and the sandbox promise does not hold.
- **Fixed here (phase 1):** a container agent's `docker exec` env now carries a per-agent **container token** instead of the instance key. srv accepts it only on a default-deny allowlist of agent routes (messaging, discovery, work queue, WhoAmI, the agent's own memory, read-only Global Memory, quitting itself). It refuses host exec, host file, pane and agent opening, UI automation, cron and the RPC surfaces with 403. The token is pinned to its agent's identity and revoked when the pane closes.
- **Also found:** the memory routes fall back to an agent name in the request body when the caller has no identity token, so any holder of the instance key can read or write another agent's personal memory. Container tokens can no longer do this (§7). For host agents it remains, and is part of the identity spec's M5 (§8).

## 2. Findings

### 2.1 How the key reaches agents

| Holder | How | Reference |
|---|---|---|
| Agent CLI process (host) | `build_persistent_spawn_env` sets `AGENTMUX_AUTH_KEY`, inherited by everything the agent runs | `agentmux-srv/src/server/agent_handlers/input.rs:564`, comment at `:341-347` |
| Terminal / PTY pane | set on every branch; on the pane env allowlist | `backend/blockcontroller/shell/lifecycle.rs:806-811`, `backend/pane_env.rs:48-57` |
| agentmux-mcp | inherits it from the agent CLI | `agentmux-mcp/src/main.rs:14-16, 72-73` |
| **Container agent** | same env map as host agents minus `CONTAINER_ENV_DENYLIST`, which removes only path variables | `backend/blockcontroller/subprocess/container_spawn.rs` (before this change), `backend/container.rs:231-245` |
| Files on disk | `<data_dir>/agents/<id>.json`, `~/.agentmux/shared/agents/reactive/...`, `<data_dir>/authkey.dev` | `backend/reactive/registry.rs:191-239, 462-473`; `agentmux-cef/src/lib.rs:937-964` |

Owner-only file modes do not protect these files from agents, because agents run as the same OS user.

### 2.2 What the key authorizes

`auth_middleware` (`server/mod.rs`) is one constant-time key comparison, with no per-route or per-command check. `/ws` and `POST /agentmux/service` reach every RPC handler. Routes that act on host paths:

| Route / RPC | Operation | Containment |
|---|---|---|
| `GET /agentmux/stream-local-file?path=` (`server/files.rs:199`) | read any file | regular file, 500 MB cap |
| `readeditorfile` / `listeditordir` / `watcheditorfile` (`server/editor_handlers.rs:400, 509, 135`) | read, list, watch any path | size cap only |
| `writeeditorfile`, `rename…`, `create…`, `delete…`, `openinshell` (`editor_handlers.rs:438-779`) | write, delete, exec | canonical path must be under `$HOME` (so `~/.ssh`, shell rc files and the Windows Startup folder are in reach) |
| `attachments.ingest` (`app_api/attachments.rs:18`) | copy any files or folders into the attachment store | count and size caps |
| `writeagentconfig`, `agent.open` (`editor_handlers.rs:205`, `app_api/agent_open.rs:1015`) | mkdir and write in any `working_dir` | file names contained; the base folder is not |
| `POST /api/v1/pane/open` (`server/mod.rs:2145`) | opens a pane on any path, which the UI then reads | none |
| `POST /api/v1/shell/create`, `/api/v1/ptyshell/create` (`mod.rs:1071, 1507`), `checkcliauth` (`cli_handlers.rs:222`) | **run any command** | none |
| CEF IPC `copy_file_to_dir` (`agentmux-cef/src/ipc.rs:435`) | copy anything anywhere | guarded by the CEF IPC token, not the srv key |

So path containment on the file routes cannot be the security boundary on its own. The same key also runs commands.

### 2.3 Container reachability, verified

On narko (Docker Desktop 29.1.3, Linux engine), from `ghcr.io/agentmuxai/agent-claude:latest` on the `agentmux-agents` network with `--add-host host.docker.internal:host-gateway`, which is how srv creates container agents (`backend/container.rs:1113-1122`), unauthenticated requests to srv's loopback port returned:

```
/api/v1/attachments/upload -> 401
/agentmux/stream-local-file?path=/nonexistent -> 401
```

srv binds 127.0.0.1 only, but Docker Desktop forwards `host.docker.internal` to host loopback, so the key is the only barrier. The probe deliberately used no key. Reading a host file with it is not needed to establish the finding.

## 3. Threat model

| Principal | Can already, without the key | The key adds |
|---|---|---|
| Host agent | run any command and read any file as the user | impersonating other agents; nothing new for file access |
| **Container agent** | `/workspace` only (the sandbox) | **all of §2.2 on the host**: this is the escalation |
| Other same-user process | read agents' env and memory | nothing we can prevent with tokens (§6) |
| Web page in the user's browser | nothing (CORS and origin checks, key not in cookies) | n/a |

Phase 1 targets the container row, which is fully fixable with credentials. The host row needs OS-level identity or sandboxing (§8).

## 4. Best practice (sources)

1. **Don't pass the server's master credential to delegated principals; issue tokens scoped to what they need.** The MCP security best practices say servers must not accept tokens not issued for them, warn against omnibus `*`/`full-access` scopes, and recommend least-privilege spawned servers. https://modelcontextprotocol.io/specification/2025-11-25/basic/security_best_practices
2. **Excessive Agency:** keep an agent's permissions on other systems to the minimum, and authorize in the downstream system, not in the model. https://genai.owasp.org/llmrisk/llm062025-excessive-agency/ The OWASP AI Agent Security Cheat Sheet asks for per-tool scoping and independent server-side validation: "Low-trust sessions cannot reach privileged tools." https://cheatsheetseries.owasp.org/cheatsheets/AI_Agent_Security_Cheat_Sheet.html. OWASP Agentic Top 10 ASI03 names "inherited credentials". https://genai.owasp.org/2025/12/09/owasp-top-10-for-agentic-applications-the-benchmark-for-agentic-security-in-the-age-of-autonomous-ai/
3. **Bound, per-principal, revocable tokens:** Kubernetes replaced long-lived shared service-account secrets with short-lived tokens bound to one pod's lifetime and audience. https://kubernetes.io/docs/concepts/security/service-accounts/. GitHub fine-grained tokens are scoped to resources with an expiry. https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens
4. **Separate authentication from authorization:** Jupyter Server's token identifies the caller, and a distinct `Authorizer.is_authorized(user, action, resource)` decides per resource. https://jupyter-server.readthedocs.io/en/latest/operators/security.html
5. **Don't give a sandbox the host's credentials, and don't count on the network to hide host services.** Anthropic's dev-container guidance: "Avoid mounting host secrets… prefer … short-lived tokens." https://code.claude.com/docs/en/devcontainer. Docker Desktop documents `host.docker.internal` as the supported path to host services (https://docs.docker.com/desktop/features/networking/networking-how-tos/). Blocking it needs Business-tier "air-gapped containers", which Docker says advanced users can bypass (https://docs.docker.com/enterprise/security/hardened-desktop/air-gapped-containers/). CVE-2025-9074 was a container reaching an unauthenticated host-side Docker API and taking over the host (https://github.com/advisories/GHSA-4xcq-3fjf-xfqw). srv must therefore authorize each principal itself.
6. **Prefer mounted secret files over environment variables where possible.** https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html, https://docs.docker.com/engine/swarm/secrets/
7. **Path containment, where a route must take a path:** allowlisted roots or opaque ids rather than deny lists (https://cwe.mitre.org/data/definitions/22.html), enforced at open time rather than check-then-open. See CVE-2022-21658 (https://blog.rust-lang.org/2022/01/20/cve-2022-21658/), `openat2` `RESOLVE_BENEATH` (https://man7.org/linux/man-pages/man2/openat2.2.html), and `cap-std` in Rust (https://github.com/bytecodealliance/cap-std). On Windows this also means handling `\\?\`, `\\.\`, UNC and reparse points (https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats).
8. **Same-user limit:** `/proc/<pid>/environ` (https://man7.org/linux/man-pages/man5/proc_pid_environ.5.html) and Windows `PROCESS_VM_READ` (https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights) expose a same-user process's secrets. The remedies are peer credentials (`SO_PEERCRED`, https://man7.org/linux/man-pages/man7/unix.7.html; `GetNamedPipeClientProcessId`, https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getnamedpipeclientprocessid) or OS sandboxes such as AppContainer (https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation) and Landlock (https://docs.kernel.org/userspace-api/landlock.html).

## 5. Recommendations

| # | Recommendation | Source | Status |
|---|---|---|---|
| R1 | Container agents never receive the instance key; they get a per-agent token | §4.1, §4.3, §4.5 | **done (phase 1)** |
| R2 | srv authorizes that token per route, default-deny: no host exec, host files, pane/agent opening, UI automation, cron, or RPC surfaces | §4.1, §4.2, §4.4 | **done (phase 1)** |
| R3 | The token is bound to its agent's identity, so a header or request body cannot point it at another agent | §4.3 | **done (phase 1)** |
| R4 | The token is revoked with the pane, and dies with srv (in-memory) | §4.3 | **done (phase 1)** |
| R5 | Deliver the container token as a tmpfs file rather than an env var, and rotate it | §4.6 | follow-up (§8.1) |
| R6 | Refuse to bind-mount a workspace that contains srv's data dir or `~/.agentmux`, where the key files live | §4.5 | follow-up (§8.2) |
| R7 | Host agents: replace the bearer key with peer-credential IPC (`SPEC_PANE_CREDENTIAL_HANDOFF` option D) | §4.8 | follow-up (§8.3) |
| R8 | Replace absolute-path routes with opaque handles or allowlisted roots enforced at open time | §4.7 | follow-up (§8.4) |
| R9 | Offer OS sandboxing for host agents (AppContainer, Landlock, App Sandbox) | §4.8 | follow-up (§8.5) |

## 6. Accepted residual risk

- A process running as the same OS user can read any agent's env or memory, the key files, and srv's own memory. Tokens cannot fix that (§4.8, identity spec §6.5.1).
- A container agent can still *message* host agents (`/agentmux/reactive/inject`). Messages carry trust markers, and the jekt trust rules decide what a recipient does with them. That is the intended channel, not a bypass.
- Read-only Global Memory still shows a container agent the workspace's shared notes. Those are meant for every agent.

## 7. Phase 1, as implemented

- `agentmux-srv/src/backend/container_credential.rs` (new):
  - `token_for_block` mints a 256-bit `amxc_`-prefixed token per block and reuses it across turns. Lookups go by SHA-256 digest.
  - The grant records the block and the agent's own `AGENTMUX_AGENT_TOKEN`.
  - `revoke_block` invalidates the token.
  - `container_route_allowed` holds the allowlist: exact paths, plus `/agentmux/work/:id[/heartbeat|complete|release]` and `/api/v1/agent/shutdown/:id`, all matched on the raw path exactly as the router matches it.
  - `container_exec_env` applies the denylist and swaps `AGENTMUX_AUTH_KEY` for the token.
- `container_spawn.rs`: every container turn's exec env comes from `container_exec_env`, and this is the only container exec path that carries agent env.
- `server/mod.rs`:
  - `auth_middleware` and `lan_or_full_auth_middleware` accept a container token through `admit_container_agent`. That function returns 403 off the allowlist, and on memory routes when the grant has no identity.
  - On allowed routes the token stands in for the key it replaced, so jekt tiers behave exactly as before.
  - A stale comment claiming agents hold no key is corrected.
- `server/caller.rs`: a request carrying a container grant is attributed to the grant's agent token, and any `X-Agent-Token` header is ignored. So `SelfOwner::of` always resolves a container's memory calls to its own agent.
- `sagas/close_pane.rs`: `revoke_block` runs when an agent pane shuts down.
- Tests:
  - unit tests for minting, reuse, revocation, the allowlist and the env swap;
  - router tests showing the token reaches agent routes on both routers;
  - router tests showing 403 on shell, pty, stream-local-file, `/agentmux/service`, `/ws`, pane and agent opening, Global Memory writes and `/agentmux/reactive/agents`, and on memory without identity;
  - revoked and forged tokens get 401, and the full key is unaffected.

**Compatibility:** agentmux-mcp is not in the container image yet (`server/agent_handlers/input.rs`, the #2939 note), so no current container feature depends on the refused routes. When it lands, container MCP keeps its agent tools, and the host-reaching ones (Shell, PtyShell, OpenEditor, OpenMedia, UI tools, Cron, OpenAgent) return 403. That is the intended container behaviour.

## 8. Follow-ups

1. **Token delivery and lifetime (R5).** Mount the token as a file on a tmpfs and have agentmux-mcp read it, then add expiry with rotation per turn.
2. **Workspace mount guard (R6).** `ContainerManager` should refuse, or require confirmation for, a `/workspace` host path that is an ancestor of srv's data dir, `~/.agentmux`, or the user's home. Otherwise the key files are inside the sandbox.
3. **Host agents (R7).** Implement `SPEC_PANE_CREDENTIAL_HANDOFF` option D on Windows with a named pipe and `GetNamedPipeClientProcessId`, and on Unix with `SO_PEERCRED`. Then drop `AGENTMUX_AUTH_KEY` from host agent and pane envs. This also removes the memory routes' body-slug fallback, which the identity spec's M5 tracks.
4. **File routes (R8).**
   - Move the Media and editor panes to server-minted handles or allowlisted roots, opened with `cap-std`-style containment, and supersede the Media pane spec's "no allowlist" decision.
   - Until then these routes stay UI-only by construction, because no scoped token reaches them.
5. **OS sandboxing (R9).** Evaluate AppContainer (Windows), Landlock (Linux) and App Sandbox (macOS) as the real fix for the host row of §3.
6. **Linux LAN listeners.** With LAN discovery on, srv also binds non-loopback interfaces, which likely includes `docker0`. Those listeners serve only the LAN routes and require the LAN key, but they should skip container bridge interfaces anyway.
7. **Docs.** `SPEC_HOST_VS_CONTAINER_AGENTS_2026_06_18.md` §3.3 "isolated" is accurate for srv's API after phase 1, but not for key files in a broad mount (item 2). Note that there when item 2 lands.
