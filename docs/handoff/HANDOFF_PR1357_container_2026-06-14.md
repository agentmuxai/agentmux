<!--
Copyright 2026, AgentMux Corp.
SPDX-License-Identifier: Apache-2.0
-->

# Handoff: PR #1357 — container Phase 2 (ContainerManager + docker exec)

- **Date:** 2026-06-14
- **From:** AgentO  → **To:** AgentY (PR owner)
- **Branch:** `agenty/container-spawn`
- **Last commit by AgentO:** `7dfd3416` — *"fix(container): address reagent review — robust spawn_container_turn + Docker integration test"*
- **Status:** original review findings cleared + plumbing validated; **3 deeper reagent findings remain open** (1 P1, 2 P2). This is a transient doc — **delete before merge.**

---

## 1. What AgentO changed (commit `7dfd3416`)

Resolved the three **reagent P2s** that were live on the prior head (`7579e92c`). The two **Codex** findings were already fixed in earlier commits on the branch (env now travels via `CreateExecOptions.env`, container name now uses the stable `agentId` UUID) — verified, no action needed.

`agentmux-srv/src/backend/blockcontroller/subprocess.rs` — `spawn_container_turn`:
1. **stdin write inlined.** Was a detached `tokio::spawn`; under runtime load it may not be scheduled for seconds, tripping the in-container CLI's *"no stdin data received in 3s"* abort. Now written inline in the exec task (the host `spawn_turn` uses a dedicated OS thread for the same reason). The CLI drains stdin to EOF before emitting output, so it cannot deadlock the read loop.
2. **5s health watchdog added.** `set_active_turn(true)` alone never drives `check()`, so container turns got no Stalled/Dead detection. Added the same `is_active_turn`-gated 5s watchdog `spawn_turn` uses; it self-terminates when completion calls `set_exited()`.
3. **exec-failure path now finalizes + drains.** The early return after `STATUS_DONE` + `run_lock` release skipped the status publish, health cleanup, and queue drain — client never saw the turn end and a queued message was orphaned. Now mirrors the normal-exit completion.

`agentmux-srv/src/backend/container.rs`:
4. Added a **Docker-gated `#[ignore]` integration test** `itest_container_lifecycle_exec_env_and_io` — see §3.

---

## 2. Remaining reagent findings (OPEN — for AgentY)

Line numbers are against `7dfd3416`.

### [P1] `subprocess.rs:~1110` — turn exit code hardcoded to 0
The success/EOF completion path sets `proc_exit_code = 0` unconditionally and never inspects the exec's real exit status. A container turn whose in-container CLI crashes / exits non-zero — or whose output stream ends with `Some(Err(e))` in the read loop — is reported to the client and to `health_monitor.set_exited(0)` as a **successful** turn (exit 0 → `Idle`). `spawn_turn` captures the real `child.wait()` code; the container path must do the equivalent.
- **Suggested fix:** surface the exec id from `ContainerManager::exec` (today `ExecSession` carries only `input`/`output`), then after the read loop call a new `ContainerManager::inspect_exec(exec_id) -> Option<i64>` (bollard `inspect_exec` → `ExitCode`) and feed the real code into `proc_exit_code` + `set_exited`. Treat a `Some(Err(_))` stream termination as a non-zero/failed turn.

### [P2] `subprocess.rs:~1051` — running exec is not interruptible
No `kill_tx`/`current_pid` is installed for the in-flight exec (only `kill_tx = None` on completion), so `stop_subprocess` finds no channel and is a **no-op** while a `docker exec` is running — a container turn can't be stopped/interrupted. `spawn_turn` wires a kill channel.
- **Note:** non-trivial — bollard has no "kill exec" call. Options: hold a cancel token / `oneshot` that aborts the exec task (drops the attach, closing the stream), and/or `docker exec <ctr> kill` the in-container PID. This is feature design AgentO deliberately left to the PR owner.

### [P2] `container.rs:280` — `pull_image` is unconditional + fatal on error
`ensure_running` always calls `pull_image`, and any `create_image` stream error is fatal. On an offline host whose image is already cached, registry contact fails and container creation aborts — even though `docker run` would start from the local cache.
- **Suggested fix:** gate the pull on a local `inspect_image` check (skip pull when present), or tolerate pull errors when the image already exists locally. *(Easy; also unblocks testing with locally-built images — see §4.)*

---

## 3. Validation done

- `cargo check -p agentmux-srv` — clean (pre-existing warnings only).
- **Integration test passes against a real daemon (Colima):**
  ```
  test backend::container::tests::itest_container_lifecycle_exec_env_and_io ... ok
  ```
  Run it with:
  ```bash
  DOCKER_HOST="unix://$HOME/.colima/default/docker.sock" \
    cargo test -p agentmux-srv --bin agentmux-srv backend::container -- --ignored --nocapture
  ```
  Covers: `ensure_running` create→reuse, env delivered via the Docker socket (not argv — the CWE-214 guard), and the stdin→stdout exec round-trip.
- **Not** exercised end-to-end through the app with a real `claude` turn — the agent image is unavailable (see §4).

## 4. Gotchas / observations discovered (worth folding into the PR)

1. **The agent image is not published.** `ghcr.io/agentmuxai/agent-claude:latest` returns `NAME_UNKNOWN` even authenticated as `a5af` — it has never been pushed (the `container-image.yml` workflow hasn't published it). It **is** buildable from `docker/Dockerfile.agent-agentmux` (`node:22-slim` + tini + `@anthropic-ai/claude-code` + `sleep infinity`), but a real turn needs Claude auth inside the container's `~/.claude` named volume.
2. **`ContainerManager::connect()` won't find Colima/Rancher sockets without `DOCKER_HOST`.** It uses `Docker::connect_with_local_defaults()` (reads `DOCKER_HOST`, else `/var/run/docker.sock`). The doc comment at `container.rs:13` claims macOS resolves the socket "via `docker context inspect`" — **the code does not do this.** Either implement context resolution or fix the comment. Docker Desktop creates `/var/run/docker.sock`; Colima does not.
3. **`drop(input)` does not reliably half-close stdin / signal EOF** over bollard's hijacked exec stream. Fine for `claude` (reads newline-delimited JSON, doesn't wait for EOF), but a latent hang for any EOF-dependent consumer. Surfaced while writing the test (a `cat`-based check hung; switched to a `read`-based check).

## 5. Local environment left in place

- **Colima** VM running (`colima status`), docker context `colima`, socket `unix://$HOME/.colima/default/docker.sock`. Stop with `colima stop` when done.
- `nginx:alpine` pulled (used by the integration test as a long-lived substitute image).
- `docker login ghcr.io` performed as `a5af` (credential in `~/.docker/config.json`; `credsStore: "desktop"` was stripped — backup at `~/.docker/config.json.bak`).
- To run the app against this daemon, export `DOCKER_HOST` in the `task dev` environment.
