# SPEC: container agents that work for everyone, not only for people who can already pull our image

**Date:** 2026-10-07
**Status:** implemented — PR #0000 (this document ships with its code). The one step it cannot do itself is making the new image package public (section 7). Verified 2026-10-07.
**Author:** Agent1@narko
**Related:** `docs/spec-claude-code-versioning.md` (how CLI pins are kept in step), `docs/specs/SPEC_HOST_VS_CONTAINER_AGENTS_2026_06_18.md` (container is the default runtime), `docs/specs/container-agent-runtime.md` (history), `docs/specs/SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md` section 6.3 (the pane's CLI install row this reuses)

---

## 1. Problem

The default agent template is a container agent. When Docker is running, the create-from-template modal preselects it, and its image is `ghcr.io/agentmuxai/agent-claude:latest`. For anyone outside the organization that agent never starts, and the pane says only `container ensure_running failed: Docker API error: ...`.

Verified 2026-10-06:

| Fact | Evidence |
|---|---|
| The GHCR package is private; an anonymous pull is refused with 401. | `curl` of the manifest without credentials. |
| srv pulls anonymously, and the Docker daemon does not use the CLI's `docker login`. | `crates/srv/src/backend/container.rs` `pull_image` calls `create_image(..., None, None)`, so bollard sends no `X-Registry-Auth`. |
| The failure reaches the user as a raw bollard error. | `crates/srv/src/server/agent_handlers/input.rs` (`container ensure_running failed: {e}`); `crates/srv/src/server/app_api/agent_io.rs` returns the same string with no pane frame at all. |
| It is invisible internally because org members have the image cached. | `pull_image` skips the registry when `inspect_image` succeeds. |
| Nothing tells users to log in, and nothing checks visibility. | No doc mentions it; `.github/workflows/container-image.yml` never pulls anonymously. |
| The default is baked into three places. | `crates/srv/agent-seed.json`, `frontend/app/view/agent/defaults/cli-catalog.ts`, and the fallback string in `input.rs` and `agent_io.rs`. |

Making the current image public is not an option. `docker/Dockerfile.agent-agentmux` installs `@anthropic-ai/claude-code` into it, a package under a proprietary license whose redistribution has not been cleared.

Stored agents are a second constraint. `reseed_if_needed` in `crates/srv/src/backend/agent_seed.rs` does not rewrite an existing definition's `container_image`, and a pane copies the value into `agent:container_image` when it is opened. Changing the seed therefore fixes new installs only; every existing install keeps naming the private image, and those must keep working.

## 2. Decision

Container agents work for everyone with Docker and a network connection:

1. **A public base image without Claude Code.** The same image minus the baked-in `claude`, published as a new package, `ghcr.io/agentmuxai/agent-base`.
2. **Claude Code is installed on the user's machine at first container start**, from npm, into a persistent volume, at the version AgentMux already pins for host installs.
3. **Failures say what happened.** A refused or unreachable pull produces a plain message with a next step, never a raw Docker error. The create modal stops preselecting a container it already knows cannot pull.
4. **Old agents keep working.** A cached legacy image runs unchanged. A legacy image that cannot be pulled falls back to the base image.

## 3. Design

### 3.1 Base image (`docker/Dockerfile.agent-agentmux`)

The file keeps its name (several comments and the workflow point at it). Changes:

- Remove `ARG CLAUDE_VERSION` and the `npm install -g @anthropic-ai/claude-code` line. Everything else stays: `node:24-slim`, tini, git, curl, jq, procps, the `agent` user, `agentmux-mcp` and `agentmux-bashwrap`.
- Create `/home/agent/.agentmux/cli`, owned by `agent`. A named volume mounted there is initialised from the image, so it is writable by uid 1000.
- Keep `DISABLE_AUTOUPDATER=1`: the CLI must not self-update inside the container; AgentMux owns the version.

The old `agent-claude` package is no longer built. Machines that already have it keep running it (section 3.5).

### 3.2 Install at first start

Container turns are `docker exec` into one long-lived container per agent (`container_spawn.rs`: `sh -c '"$@" < prompt-file'`). The CLI is resolved by name, `claude`, on the image's `PATH`.

- **Where.** `npm install --prefix /home/agent/.agentmux/cli <package>@<pin>` puts the binary at `/home/agent/.agentmux/cli/node_modules/.bin/claude`. The directory is a per-agent named volume, `agentmux-cli-<container-name>`, added at container creation. It is deliberately not in the set of mounts that trigger container recreation (`owned_mount_targets`): an existing container is never rebuilt for it, and an existing container whose image already has `claude` has no use for it. The directory also exists in the image, so a container without the volume still works, and loses only the install when it is recreated.
- **Version.** The package and version come from `ProviderConfig` in `crates/srv/src/backend/providers.rs` (`npm_install_specs(pinned_version)`), the pin the host installer already uses and `pin-consistency.test.ts` already guards. The workflow input and the Dockerfile `ARG` that duplicated the pin go away, so a bump touches one fewer place.
- **Idempotent.** A marker file, `.installed-<package>-<version>`, is written after the installed binary answers `--version`. A start that finds `claude` already on the image `PATH` (a legacy image) or the marker does nothing. A new pin changes the marker name, so the next start reinstalls. srv remembers a successful check per container in memory, so a running agent pays one extra `docker exec` per srv lifetime, not one per turn; the cache is dropped whenever the container is created or removed.
- **Found at turn time.** The turn wrapper appends `/home/agent/.agentmux/cli/node_modules/.bin` to the end of `PATH`. A baked-in `claude` therefore still wins on a legacy image.
- **Progress and errors in the pane.** The install emits the existing `agentmux_cli_install` frames (`crates/srv/src/backend/cli_notice.rs`): "Installing Claude Code 2.1.288...", then "Installed ... (8 s)" or "Couldn't install ...", with the tail of npm's output as the detail. They are persisted like any other pane frame, so they survive a reload. No frontend change is needed for this part.
- **Isolation.** The install runs as `agent` inside the container, with the npm cache in `/tmp` and removed afterwards. Nothing is installed on the host.

Measured against `node:24-slim` on 2026-10-07: install takes about 8 s and leaves 235 MB on disk; `claude --version` prints `2.1.288 (Claude Code)`.

### 3.3 Clear failures

A new `ContainerError::ImagePull { image, kind, detail }` replaces the pass-through of the bollard error. `kind` is one of:

| kind | Typical Docker text | Message to the user |
|---|---|---|
| `Denied` | `denied`, `unauthorized`, `pull access denied`, HTTP 401/403 | The registry refused access to the image. AgentMux downloads images anonymously, so the image must be public. Run the agent on this computer (host) instead. |
| `NotFound` | `manifest unknown`, `not found`, HTTP 404 | The image does not exist in the registry. Check the image name in the agent's settings, or run it on this computer. |
| `Network` | `no such host`, `timeout`, `connection refused`, `TLS handshake` | Could not reach the registry. Check the connection and any proxy configured in Docker, then try again, or run on this computer. |
| `Other` | anything else | Could not download the image, with the daemon's text. |

`ContainerError::user_message()` renders these, and a daemon that is not answering gets its own sentence ("Docker is not responding; start Docker Desktop"). Both call sites that start a container turn (`input.rs` and `agent_io.rs`) go through one helper that also writes the frame to the pane. The App API path previously wrote none.

The message begins "Couldn't start the container ..." and `muxspect_handlers::classify_last_error_source` learns that prefix, so the diagnostic still reports `container_spawn`.

### 3.4 Create flow: preselect only what can work

A new RPC, `containerimagecheck { image }`, answers whether an image can be had: `local` (the daemon has it), `public` (an anonymous registry check succeeds), `denied`, `not_found`, or `unknown` (no answer: offline, a proxy, a registry that does not speak the token flow). Only `denied` and `not_found` count against the container. `unknown` never does, because a false "no" would take away a working default.

The registry check is a small anonymous request (`backend/image_probe.rs`): the manifest URL, the bearer-token challenge if one is returned, a second request with the anonymous token. It runs in srv, so the renderer needs no network access and the answer matches what the daemon-side pull will do.

`AgentCreateFromTemplateModal` runs the check once Docker is known to be available, and `containerPreselect()` (new, in `frontend/app/view/agent/defaults/container-default.ts`) decides the default: container only if the template is a container template, the CLI supports containers, Docker answers, and the image is not `denied`/`not_found`. When it declines because of the image, the Runtime row says why and the host option is already selected: a one-click fallback is the existing dropdown. The container option stays selectable, since the user may know better.

For a stored legacy image the check also tries the base image, mirroring the runtime fallback below, so a machine that can run the fallback is not told it cannot.

### 3.5 Existing agents

- **Legacy image cached:** `inspect_image` succeeds, no pull, same container, same behaviour. The CLI check finds `claude` on `PATH` and does nothing.
- **Legacy image not cached and refused (`Denied` or `NotFound`):** `ensure_running` falls back once to `ghcr.io/agentmuxai/agent-base:latest`, logs it, and the CLI install runs. The stored `agent:container_image` value is not rewritten.
- **Empty stored image:** resolves to the base image (it used to resolve to the legacy one).
- **A custom image:** untouched. If it has no `claude` and no marker, the install runs; if npm is missing in it, the install fails and the pane says so.

### 3.6 Publishing (`.github/workflows/container-image.yml`)

- `IMAGE_NAME` becomes `agentmuxai/agent-base`; the `claude_version` input and the `build-args` go away.
- A manual dispatch can now publish `:latest` (new boolean input, default off). Tags still publish `:latest` as before. Without this the first publish after merging could only be a release tag.
- After the push, a step checks an anonymous pull in the style of `mirror-reviewer-image.yml`, but inverted: that workflow fails when an anonymous pull works, this one fails when it does not. It runs `scripts/check-anonymous-pull.sh`, which fails with a message naming the exact UI step (section 7).

## 4. Phases

1. **Pull errors and message mapping.** `ImagePull`, classification, `user_message()`, the shared turn-prepare helper, the muxspect prefix. Useful on its own.
2. **Image check and create flow.** `image_probe.rs`, the RPC and its generated types, `container-default.ts`, the modal.
3. **Base image and install.** Dockerfile, CLI provisioning (script builders, install, frames, cache), the `PATH` append, the legacy fallback, defaults in the seed and catalog, workflow and script, pin tests, `docs/spec-claude-code-versioning.md`.

## 5. Test plan

| Layer | What | Where |
|---|---|---|
| Rust unit | Pull-error classification across the real message shapes; message text per kind; no raw `Docker API error` in a pull message | `container.rs` tests |
| Rust unit | Image reference parsing, token-challenge parsing, legacy-image detection, default resolution | `image_probe.rs`, `container.rs` tests |
| Rust unit | Install and check script builders: arguments are positional, never interpolated; marker name; the turn wrapper's `PATH` append | `container.rs`, `container_spawn.rs` tests |
| Rust unit | `classify_last_error_source` recognises the new prefix | `muxspect_handlers.rs` tests |
| Frontend unit | `containerPreselect()` truth table; the Runtime row text per image state | `container-default.test.ts`, modal test |
| Static | The Dockerfile no longer installs Claude Code and creates the CLI directory; the workflow publishes `agent-base` and has no Claude pin; the seed and catalog name the same default image | `pin-consistency.test.ts`, a new image-default test |
| Script | `scripts/check-anonymous-pull.sh` exits non-zero for a private package and zero for a public one | run by hand against the real registry; recorded in the PR |
| Docker (local) | Build the base image; start a container from it; run the install script; one real container-agent turn | recorded in the PR with what could and could not be run |

## 6. Not doing

- **Docker login / registry credentials in srv.** It would make a private image work for one user and does nothing for everyone else.
- **Making `agent-claude` public, or bundling Claude Code in any image we publish.** Redistribution is not cleared.
- **Installing other providers' CLIs in containers.** Only Claude supports containers today (`containerSupported` in `cli-catalog.ts`). The install code takes its package from the provider, so another provider is a catalog change later.
- **Rewriting stored `agent:container_image` values** or recreating existing containers. The runtime fallback covers them without touching user data.
- **A pre-pull progress bar** for the base image. The first start is slow (an image download plus an install); the install row appears, the download does not. Worth doing, but separate.
- **Registry proxy or mirror settings.** `unknown` keeps these users on the default path and the daemon's own proxy configuration applies to the real pull.

## 7. What the operator must do

A new GHCR package is private by default and its visibility can only be changed in the GitHub UI by an organization owner; there is no API for it.

After the first publish of `agent-base` (run **Container Agent Image** with `tag_latest` on, or push a release tag), an organization owner opens `https://github.com/orgs/agentmuxai/packages/container/package/agent-base`, then **Package settings**, **Danger Zone**, **Change visibility**, **Public**. The workflow's anonymous-pull step fails with those words until this is done, and passes on the next run.

Until then the create modal's image check reports `denied` for the base image, so the default falls back to a host agent rather than a container that cannot start.
