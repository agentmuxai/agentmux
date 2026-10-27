# SPEC: container agents at parity with native agents: start responsive, same context

**Status:** approved 2026-10-27 by AgentY with the section 4 recommendations, under the operator's standing instruction to use best recommendations (task #5162). Building (task #5163): P0, P1 and P2 are in the first PR; the rest follows in later PRs.
**Trigger:** The operator, 2026-10-27: *"we also want to get container agents working well. I just started a new JCont, and it is not responsive. It needs all the same context injection the native agents get."*
**Builds on:** `SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md` (shipped in #4461: base image, CLI installed at first start, clear failures). This spec is about what is still missing.

## 1. What happened, and what is proven

A container agent was created from the claude template at 06:21:52 and sent its first message at 06:22:00. Reading srv's log (read-only) and the machine:

| Fact | Evidence |
|---|---|
| The config files (CLAUDE.md, `.mcp.json`, `.claude/settings.json`) were written into the host workspace, which is bind-mounted at `/workspace`. | log, `WriteAgentConfig` |
| srv began pulling `ghcr.io/agentmuxai/agent-base:latest` at 06:22:00.02. **Nothing was logged afterwards: no completion, no error.** At 06:53 the image is still not in the Docker daemon, and the daemon recorded no image event for it. | log; `docker images`; `docker events` |
| The pane showed nothing for the whole time. | `pull_image` (`container.rs:1282`) writes only to `tracing`; the pull's progress is `debug!`. `container_turn::prepare` shows frames only for the CLI install, which comes after the pull. |
| There is **no time limit** on the pull. `create_image` is awaited to the end of its stream. | `container.rs:1303` |
| The network and daemon are not the cause. The image is public (12 layers, 120 MB compressed, amd64 and arm64). The same daemon pulled an unrelated 45 MB image from `ghcr.io` in 3 s with the CLI, and the same `bollard` `create_image` call in a throwaway program finished in 2.4 s. | commands run 2026-10-27 |

**Root cause (found by Maricon's probe on a Linux host, then checked in the code): the pull is killed after 5 seconds.** The pull, the container create and the CLI install all run inside the `agentinput` RPC handler (`input.rs:1122`, `container_turn::prepare`). The RPC engine wraps every handler in `tokio::time::timeout` with a default of 5000 ms (`backend/rpc/engine.rs:24`, `:571`), and a timeout drops the handler's future. Dropping the pull's stream cancels it: dockerd logs `Cancel with lease` about 4.7 s after the start, and srv logs nothing more. A real pull takes about 134 s on her host, so **the image can never arrive on a first message**; the CLI install (8 s or more) is cut the same way. Once the image and CLI are cached by other means, sends work (the container is up in about 1 s). This matches the operator's JCont: the pull began, nothing followed, the daemon never held the image. (Before this I had only shown that the daemon, the network and `bollard` were fine; I did not pull `agent-base` myself because the Docker cache is shared with the operator's instance.) Whether the 5 s drop also hits Windows and macOS is untested, but the timeout is platform-independent and the JCont log fits it.

On top of that, **a stuck or slow pull is invisible and unbounded**: only `tracing` output, no pane frame, no limit of its own.

Other points in AgentY's log notes, checked against the code:

- **"`.mcp.json` has no `AGENTMUX_LOCAL_URL`": the file is not the problem, the route is.** The URL is meant to come from the `docker exec` environment, rewritten to `host.docker.internal` (`input.rs:1103`, `container.rs:225`), with the instance key replaced by a scoped container token (`container_credential.rs:138`); `agentmux-mcp` and `agentmux-bashwrap` are in the image. I first called this "not a gap" from reading the code. **Maricon's probe shows it is broken on Linux:** `agentmux-mcp` starts and lists its tools, but every call fails with connection refused, because srv listens on `127.0.0.1` only and `host.docker.internal` there is the Docker bridge address. Docker Desktop (Windows, macOS) routes `host.docker.internal` to the host's loopback, so it presumably works there (not tested). With LAN discovery on, the bridge IP answers but the route is a 404.
- **No account bound: handled, but only in words.** A container agent's credentials are the bound account's `.credentials.json`, mounted as one file (`credentials_dir_if_file_backed`). Probe result: with no account the turn stops before any Docker call with "no credentials for claude: no claude account is linked to this agent. Bind an account... in Connectors > Accounts." That is clear. What is missing is a way to fix it from the pane (no picker), and the error stays on screen after the account is linked until the agent is reopened from My Agents. (I had guessed the CLI would start signed out; the probe shows it does not.)
- **The identity rebind pointing at another channel's folder, and `fs_watch` failing on it:** that folder is a host path the container never sees (`CLAUDE_CONFIG_DIR` is stripped from the exec env). It is noise for the container but shows the host-side identity set-up runs for a container agent that does not use it. Harmless to fix; lowest priority.

## 2. Parity: what a native agent gets, and what a container agent gets

"Same" means the same code runs for both.

| What | Native | Container today | Gap |
|---|---|---|---|
| Global Memory in CLAUDE.md | `WriteAgentConfig` | same; file is in the mounted workspace | none |
| Personal Memory (the agent's own notes) | Claude's memory folder under the shared identity folder | lives in the per-agent Docker volume at `/home/agent/.claude`, not in the identity folder | **differs**: not shared with the agent's other sessions, not seen by `Memory` tool reads or backups |
| agentmux MCP server | `.mcp.json`, host binary, env inherited | `.mcp.json` in workspace, binary in the image, env from exec | **broken on Linux** (srv not reachable from the container); unproven on Docker Desktop; the image's binary can be a different version from the running srv |
| Skills, settings, hooks | written to the workspace | `CLAUDE.md`, `.mcp.json` and `.claude/settings.json` (bashwrap hooks) are written; **no skills are written**; hooks fail open with no memory when srv is unreachable (`sessionstart` returns nothing) | **gap**: skills; hooks depend on the same route as the MCP server |
| Credentials, account | resolved, gate if none | file mounted with a scoped token (not the instance key); none = clear error before Docker | **gap**: no picker in the pane, stale error after linking, macOS Keychain accounts cannot work |
| `AGENTMUX_*` env | pane env | exec env (instance key swapped for a block token; host paths dropped) | by design |
| Jekt and MuxBus keys | injected into `.mcp.json` | same | none |
| Inbound messages | into the live session | each message is one `docker exec` turn; a jekt is delivered into the turn, held while signed out and replayed after sign-in (probe) | a message that arrives during a turn: not checked |
| Transcript, history, status | output zone, turn lifecycle | same code path | none known |
| **First start** | process starts at once | pull, create, start, CLI install, all inside the 5 s RPC handler on the first message | **the reported bug: it never finishes** |
| Create flow | no host CLI needed beyond the agent's own | the Claude card says "click to install" even for a container agent, and the launch flow runs the host-side CLI and auth check first | **gap**: a container agent should not depend on the host CLI |
| Docker connection | n/a | `connect_with_local_defaults`: `DOCKER_HOST` only if `unix://`; no `$XDG_RUNTIME_DIR` probing although `container.rs:14` says so; no Podman | **gap** on rootless Docker and Podman |

## 3. Proposal

**P0. Take the start-up out of the RPC handler.** `container_turn::prepare` runs in a task of its own that the RPC engine's timeout cannot drop. `agentinput` returns at once with the message accepted and queued; the turn runs when the container and CLI are ready, and a failure or a stall ends it with the error frame. This is the fix for the reported bug; P1 and P2 make it visible and bounded. (Alternative, rejected: raise the handler timeout to several minutes. The caller would wait with no feedback, and a client disconnect would still cancel the pull.)

**P1. Say what is happening.** Show start-up in the pane as persisted frames (they survive a reload), using the existing install-notice mechanism (`cli_notice`): *Pulling the agent image… (n% of 120 MB)*, *Starting the container…*, *Installing Claude Code…*, *Ready*. Progress comes from the pull stream's byte counts, at most one frame every 2 s.

**P2. Bound it.** The pull fails when the stream gives no item for 60 s, or the whole pull takes longer than 10 min. The failure is an error frame in plain words ("couldn't download the agent image: no data for 60 s; check Docker's network, then send a message to try again"), the half-finished pull is dropped, and the next message retries. The same two limits apply to container create and start.

**P3. Start before the first message.** When a container agent is created, begin the pull and the CLI install in the background (the create flow already checks the image). The pane header shows "Preparing…" until ready, so the first message normally finds a warm container. The first-message path stays as the fallback and shares the same lock, so nothing is done twice.

**P4. No account: ask, don't hang.** If the provider needs sign-in and no account is bound (or the bound one is Keychain-backed), the first message stops before any pull with a card: pick an account, sign in, or use an API key from the Vault. This reuses the native sign-in gate. For a Keychain account on macOS the card says why it cannot work and offers the API-key path.

**P5. Personal Memory the same as native.** Mount only the agent's memory folder (a single directory, not the whole identity folder, which broke because of symlinks, see the 2026-09-02 credentials report) into the place Claude reads it in the container. Then the agent's notes are the same files native sessions use.

**P6. Prove the tools work.** One end-to-end test (section 5) that sends a message to a container agent and has it call a MuxAPI tool.

**P7. Version skew.** Publish the image tagged with the AgentMux version and pull that tag, falling back to `:latest`, so the `agentmux-mcp` and `agentmux-bashwrap` in the image match the srv they talk to.

**P8. Clean-up.** Skip the host identity rebind and `fs_watch` of the identity folder for container agents. After an account is linked, clear the stale "no account" error and start the held turn.

**P9. Let the container reach srv on Linux.** Recommend: srv also listens on the Docker bridge gateway address of the `agentmux-agents` network, and on that listener accepts only the scoped container tokens (never the instance key, never an unauthenticated route). The container's `AGENTMUX_LOCAL_URL` then points at that address. Conditions (AgentY, security lane): bind only the gateway address of the agents network, never `0.0.0.0`, and only on Linux; every request needs a scoped container token, with no instance key and none of the loopback-only operator RPCs on that listener; check that the source address is in the agents subnet; test that the loopback-only RPCs are refused there. Alternative: mount a unix socket into the container and teach `agentmux-mcp` and `bashwrap` to use it; no network exposure, but more code in two binaries.

**P10. No host CLI for a container agent.** The create flow and launch flow skip the host CLI install and host auth check when the runtime is Container; the card says what the container will install.

**P11. Rootless Docker and Podman.** When `DOCKER_HOST` is not a `unix://` URL, probe `$XDG_RUNTIME_DIR/docker.sock`, `$XDG_RUNTIME_DIR/podman/podman.sock` and `/var/run/docker.sock` in that order, and name the socket found in the Container option's tooltip. Also write the skills into the workspace, as native agents get them.

## 4. Open questions, with my recommendation

1. **Pre-start (P3) or only first message?** Recommend pre-start: the first message is the worst place to wait 2 minutes. Cost: a pull the user may never need if they create and abandon the agent. The image is 120 MB and cached afterwards.
2. **Personal Memory: mount or sync (P5)?** Recommend mount of the one folder; sync can lose a write.
3. **Timeouts (P2): 60 s without data, 10 min total?** Recommend these; make them settings only if someone asks.
4. **Pin the image to the version (P7)?** Recommend yes. It costs one more tag per release.
5. **A message that arrives while a turn runs?** Not checked; recommend the build looks at it and queues, as native does.
5a. **How the container reaches srv on Linux (P9)?** Recommend the bridge-address listener with container tokens only; ask AgentY (security lane) to review it before the build starts that PR.
6. **Other providers in containers (Codex, Gemini)?** Out of scope: they are marked unsupported today.

## 5. Test plan

- Unit: a pull stream that goes quiet fails after the limit with the plain-language frame and leaves no half pull; progress frames are throttled; no account stops before the pull and writes the card; the memory mount only covers the memory folder.
- A start-up that takes longer than the RPC timeout (a fake pull that takes 7 s) completes and the message is delivered; the handler itself returns in well under 5 s.
- Integration (needs Docker, run on a Linux CI runner, ignored locally): create a container agent whose provider command is a small script baked into the test image. The script answers the message in stream-json and calls the MuxAPI over `$AGENTMUX_LOCAL_URL` with the container token. The test checks the answer reached the pane, the call was accepted as that agent, and a call with a wrong token was refused. This needs no Anthropic account.
- Live (in the build's own `task dev`, with a real account): create the agent, watch the frames, send a message, ask it to call a tool and to read and write its Personal Memory. Repeat with Docker's image cache empty and with the network cut during the pull.

## 6. Build order

1. **P0, P1 and P2 together**, one PR: the start-up outside the RPC handler, shown in the pane, with limits. This answers the report. Test it with an empty image cache (a machine's Docker cache is shared by every AgentMux on it, so empty it only on a machine kept for testing, such as a Linux box with Docker installed for this).
2. P9 (Linux reachability) and P10 (no host CLI), then P4 (account picker and stale error) and P3 (pre-start).
3. P5, P6 (the end-to-end test; it is what would have caught the 5 s drop and the Linux route), P7, P8, P11.

Each step is one PR.
