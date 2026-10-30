# Container agents get CPU and memory limits

**Status:** active — built in PR #PRNUM (Kanban #11090); §10 lists where the build differs from the plan above. Operator-approved 2026-10-29 (§9 answered: on for every container agent, the defaults of §2, no machine-wide cap, a container agent cannot raise its own limit).
**Date:** 2026-10-29.
**Requested by:** the operator, 2026-10-29 ("ok, lets get that done", after AgentY recommended per-container CPU and memory limits). It also answers their earlier request for a memory cap on build jobs, for work that runs in containers.
**Author:** Loap-3@narko (Kanban #11089).
**Related:** `SPEC_CONTAINER_AGENTS_AT_PARITY_2026_10_27.md`, `SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md`; Maricon-2's developer-image spec (#11091) needs the limits to leave room for a Rust build; Manoz's failure-row build (#10593) is where an out-of-memory failure shows.

Line references are against `origin/main` at `be2d8f8fe`. "Daemon" is the Docker engine AgentMux talks to.

## 1. Today

`create_and_start` (`crates/srv/src/backend/container.rs:1238`) builds the container with a `HostConfig` of mounts, `network_mode: bridge` and `extra_hosts` only (`:1304-1318`). No `memory`, `memory_swap`, `nano_cpus` or `pids_limit`. So a `cargo build` or test run inside a container agent can take every core and all the memory the daemon has. On native Linux that is the user's machine; on Docker Desktop it is the Docker VM, whose own cap protects the host only as far as that cap is set (§6).

A container agent has one container for its life: the name comes from the agent id (`container_name_for_slug`, `container_turn.rs`), every pane of that agent shares it, and each turn runs as a `docker exec` in it. So the limit is per agent, and one container's limit covers everything its panes do.

Docker applies a memory limit with swap on by default as twice the memory, so a runaway build would first page the host's swap, which is the freeze the limit exists to prevent. The limit has to set swap too.

## 2. Defaults

**Computed from the daemon, not from the host.** `docker info` (bollard `info()`: `MemTotal`, `NCPU`, `CgroupVersion`) is what the daemon can actually give: on Docker Desktop it is the VM, not the machine. A default from the host's RAM would be wrong there.

**On for every container agent**, including the ones that exist today (§4 applies it without recreating them). "No limit" is an explicit per-agent choice, never the default.

| Limit | Default | Why |
|---|---|---|
| Memory | `clamp(MemTotal / 2, 2 GiB, MemTotal − 2 GiB)` | half of what the daemon has, so one agent can't starve the host app or the daemon; the floor keeps a small VM usable and the ceiling leaves 2 GiB |
| Swap | equal to memory (no swap beyond the cap) | §1 |
| CPUs | `max(1, NCPU − max(1, NCPU / 4))` | all but a quarter of the cores, at least one left for the host: 16 gives 12, 8 gives 6, 4 gives 3, 2 gives 1 |
| Processes | `pids_limit` 8192 | a runaway fork loop is the third way a build freezes a machine; invisible to users, no setting |

The stored value is only what a user set. Unset means "auto", recomputed from `docker info` each time the container starts, so a larger Docker VM is picked up with no action. If `docker info` can't be read the start does not fail: no limit is applied and srv logs why.

Per-container limits do not add up to a machine limit: three agents at half the daemon's memory each can still exceed it together. A shared parent cgroup that caps all agent containers at once is possible on native Linux (`cgroup_parent`) and is not in this spec (§9). On Docker Desktop the VM is that parent.

**Parallelism inside the container.** A CPU limit is a quota, so `nproc` still shows every core and `cargo` starts one `rustc` per core, each wanting a gigabyte or more: the limit holds the CPU but the memory limit is then hit by the fan-out. The container's environment gets `CARGO_BUILD_JOBS` (and `MAKEFLAGS=-j`) set to the CPU limit rounded up, unless the agent's own env sets them. This is from my knowledge of how those tools read the core count, not measured here; the measurement in §7 checks it.

## 3. Where the user sets it, and what an agent can do

- **Per agent:** two optional fields on the agent definition next to `container_image` and `container_volumes` (`rpc_types/agent.rs:136-146`): `container_memory_mb` and `container_cpus`; empty means auto, and `0` means no limit. At launch they go into the block's meta as `agent:container_memory_mb` and `agent:container_cpus`, read by `ContainerSpec::of` (`container_turn.rs`) like `agent:container_image`, and carried in the spec `ensure_running` receives (a new field on `ContainerMountSpec`, which is already the per-agent container spec; the name stays).
- **App-wide default:** two settings keys in `schema/settings.json`, `agent:container_memory` and `agent:container_cpus`, each `"auto"` or a number. Precedence: the agent's own field, then the app-wide setting, then auto.
- **Where it is edited:** the container section of the new-agent form (`AgentCreateForm.tsx`, next to the runtime row), and a "Container" part of an existing agent's settings. I found no per-agent container settings screen today besides the create form, so the second is new UI the build adds; both show the effective value ("8 GB of the 16 GB Docker has") and a "No limit" choice.
- **MuxAPI:** `Agents` gets `set_container_limits` (`agent`, `memory_mb`, `cpus`), beside the existing `update_container`. Per the operator's 10-25 rule it carries no operator-only guard. The change is shown in the agent's pane as a notice row ("Memory limit raised to 16 GB by Agent1"), so a container agent raising its own cap is visible, not silent. Flagged for AgentY's security lane: whether a container-token caller may raise its own limit or only lower it is the one place I would want a decision (§9).

## 4. Applying a change

**Update the running container; do not recreate it.** Docker can change `memory`, `memory_swap` and `nano_cpus` on a running container (bollard `update_container`, `UpdateContainerOptions` has all three), so an agent's turn, its processes and its installed tools survive. Recreating would lose all of that, and `container_drift` today recreates on any mismatch (`container.rs:1116`, reason strings for mounts and extra_hosts), so limits must not go through that path.

- At creation, `create_and_start` sets the limits in `HostConfig`.
- On every `ensure_running` for an existing container (so also containers created before this feature, which have no limits), a new `limits_apply` step compares `HostConfig.Memory`, `MemorySwap` and `NanoCpus` with the wanted values and calls `update_container` when they differ. No drift, no call.
- Docker rejects a memory change when the new swap limit is lower than memory, so the update sends both in an order that is valid at every step (raise swap first when raising, lower memory first when lowering).
- If the update fails (an old daemon, a refused value), srv logs it and leaves the container as it is, with a pane notice naming the reason; it does not fall back to recreating a live container.
- A change made while a turn is running applies at once to the running processes; lowering below current use makes the kernel reclaim or kill, which §5 reports.

## 5. What the user sees when a limit is hit

An out-of-memory kill must never reach the user as a bare exit 137. Today a turn's exit code comes from `inspect_exec` (`container_spawn.rs:636-650`) and any non-zero code goes to `failure::classify(Some(exit_code), None, tail, …)` with no way to say why. Worse, the usual victim is not the turn but a child of the CLI (the `cargo` inside a Bash tool): the CLI survives, the tool returns 137, and neither the user nor the agent is told the cause.

**Detecting it.** Exit 137 alone is not proof (a stop is also SIGKILL). The positive evidence is the cgroup's own counter: the container's `memory.events` `oom_kill` value (cgroup v2; `memory.oom_control` on v1, chosen from `CgroupVersion`), read through one `exec` when a turn starts and again when it ends or a tool exits non-zero. A rise means the kernel killed something in this container during that window. The Docker `oom` container event gives the same signal live and is the cheaper path if the build wires an events stream for the container; `State.OOMKilled` from `inspect_container` stays set until the next start, so it is a fallback only.

**Saying it.**
- A pane notice row (`cli_notice`, persisted, one per event, updated in place like the image rows): "A process in this container was stopped: it ran out of its 8 GB memory limit." with a "Raise the limit" action that opens the agent's Container settings.
- If the turn itself died of it, the failure classifies as a new `OutOfMemory` kind carrying the limit, shown through the failure row with the same action (Manoz's #10593 is changing that row; the build coordinates with them, and the row's wording is theirs).
- For the agent: `agentmux-bashwrap exec` (`crates/bashwrap/src/bash_wrap.rs`) already wraps the command of the Bash tool inside the container. When its child ends by SIGKILL it can read the container's own `memory.events` (no daemon call needed, it is a file in the container) and, if `oom_kill` rose, print one line on stderr: "[AgentMux] killed: the container's memory limit (8 GB) was exceeded. Reduce parallelism (`CARGO_BUILD_JOBS`) or ask the user to raise the limit." so the agent does not retry the same build blindly. I did not read `bash_wrap.rs` closely enough to say how much of this it can already do; the build checks. A missing line only loses this hint.
- CPU is never an error: a throttled build is slower, not failed. No UI for it beyond the effective value in settings.

## 6. Docker Desktop on Windows and macOS

My statements here come from how Docker Desktop is documented to work and were not tested on a live Docker Desktop for this spec; the build checks them on a Windows and a macOS machine before it claims them in the UI.

- Every container runs inside one VM (WSL2 or Hyper-V on Windows, a Linux VM on macOS). The VM has its own memory and CPU cap, set outside AgentMux (`.wslconfig` on the WSL2 backend; Settings, Resources on macOS), and a container's limit is enforced inside it and can never give a container more than the VM has.
- `docker info` reports the VM's `MemTotal` and `NCPU`, so the §2 defaults already follow the VM and never exceed it.
- A per-container limit above the VM's memory is accepted by Docker and means nothing. The settings UI clamps what it offers to the daemon's `MemTotal` and says why.
- AgentMux cannot change the VM's cap. When it is small (under 4 GB, or under the default memory floor), the settings show a note: "Docker has N GB in total, so builds that need more will fail whatever this limit says. Raise it in Docker Desktop, Settings, Resources." The same note appears in the OOM notice when the limit equals the whole VM.
- On the WSL2 backend the VM's memory is not given back to Windows quickly after a build, so the machine can stay tight after the container is idle. That is outside this spec; the note above is the most AgentMux can say.

## 7. Tests and measurement

**Unit.** The default function over a table of daemons (2, 4, 8, 16, 64 GiB; 2, 4, 8, 16 cores) including the floor and ceiling; precedence of agent, setting and auto; meta and settings to `HostConfig`; swap equals memory; the update ordering rule; `limits_apply` does nothing when the container already matches and calls once when it doesn't, and never recreates.

**Integration (needs Docker; run on a Linux machine with Docker, ignored locally, like the existing container tests).**
1. A container created with 256 MB runs an exec that allocates 1 GB: the process is killed, the `oom_kill` counter rises, and the pane notice is written with "256 MB".
2. A container created without limits (an existing one) gets limits on the next `ensure_running` without its id changing, and `docker inspect` shows them.
3. `Agents set_container_limits` while a long exec runs: the exec survives a raise and the new value is in `HostConfig`.
4. `docker info` unreadable (a fake client): the container starts without limits and srv logs why.

**Measurement (a Linux machine with room for a build).** A build of this repo's srv inside a container, at limits of 8 GB and 8 CPUs, with `CARGO_BUILD_JOBS` set and unset, and again unlimited: peak container memory (`docker stats`) stays at or under the cap; whether the build finishes, how much slower, and whether the unset case OOMs at the fan-out; and, while it runs, the host stays responsive: a probe process on the host measures its own wake-up latency and a simple CPU-bound timing, compared with the unlimited baseline. The result decides whether §2's jobs setting is needed and whether the CPU default is right.

## 8. Plan

One PR for the build, in this order inside it: the default function and settings (srv), `create_and_start` and `limits_apply`, the pane notice and OOM detection, the `Agents` action, the UI (create form, agent settings), docs (a page and the MuxAPI reference). Changeset `minor`. The check list: the unit tests above, `cargo test -p agentmux-srv --bin agentmux-srv container`, tsc and vitest for the form. The integration tests and the measurement run on a Linux machine with Docker before the PR is opened, and the numbers go in the PR body as ratios.

## 9. Decisions for the operator

1. **On by default for every container agent**, existing ones included (recommended), or opt-in. Opt-in leaves the freeze risk with everyone who doesn't find the setting.
2. **The default values:** half the daemon's memory (2 GiB floor, 2 GiB left over), all but a quarter of the cores, swap equal to memory (recommended).
3. **A machine-wide cap across all agent containers** (a shared parent cgroup, native Linux only): not in this build; wanted later or not?
4. **Whether a container agent may raise its own limit through MuxAPI.** Recommended: yes, shown in the pane, per the "no operator-only gates" rule; AgentY's security lane may prefer lower-only for a container-token caller.
5. **Maricon-2's developer image** needs more than half of a small VM for a Rust build. Proposed with them: the image carries a label with its recommended minimum memory, and auto becomes `max(auto, label)` bounded by the daemon's total.

## 10. As built

Where the build differs from §1-§9, and what was and was not checked.

- **Where an agent's own limits live.** Not two fields of the agent definition: a table of their own, `db_agent_container_limits` (`storage/container_limits.rs`), keyed by the agent's id, with `memory_mb` and `cpus` as text (empty auto, `0` no limit, else MB or CPUs). A column of `db_agents` is read and written in some forty places (every struct literal, three SQL statements, the registry mirror) for a setting only container agents have, and an older build ignores an unknown table, so it needed no schema bump. It is read when a turn starts the container (`ContainerLimits::of_agent`), so a change applies on the next `ensure_running` with no pane meta to refresh. The block meta keys of §3 are not used.
- **Applying a change.** `limits_apply` (`backend/container/limits.rs`) runs on every `ensure_running` of an existing container: one inspect, and a `docker update` only when `Memory`, `MemorySwap`, `NanoCpus` or `PidsLimit` differ. Memory and swap travel in one request, which is how the `docker update` CLI sends them and is valid in both directions, so the ordering rule of §4 is not needed. If Docker refuses, srv logs it and keeps the container. A container is never recreated for a limit.
- **The memory default.** `max(MemTotal / 2, 2 GiB)`, capped at `MemTotal`. §2's ceiling (`MemTotal - 2 GiB`) is never below half from 4 GiB up, and under 4 GiB the floor rules, so it needs no code.
- **Image label.** `ai.agentmux.recommended-min-memory-mb` (MB, on the image), read from the image when a container is made and from the container when one is updated; auto becomes `max(auto, label)` bounded by the daemon's total. The name is a constant, `RECOMMENDED_MIN_MEMORY_LABEL`, for Maricon-2's developer image (#11091) to set or to rename.
- **Settings.** `agent:container_memory` (MB) and `agent:container_cpus`, each `"auto"`, `0` or a number, in Settings, Advanced, read live (`install_config`, like `agent:compactat`).
- **Where a person edits an agent's limits.** A "Container limits…" panel (`AgentContainerLimitsPanel`) in the agent pane's right-click menu for a container agent, and behind the "Raise the limit" button of an out-of-memory row. The create form and My Agents' row menu do not have it yet: a new agent starts with the defaults and its limits are set afterwards.
- **MuxAPI.** `Agents set_container_limits` (`POST /api/v1/agent/container/limits`): `agent`, and `memory_mb` and/or `cpus` to change, neither to read; it answers with what is set, what is in force (`effective_*`) and what Docker has (`daemon_*`). A container agent can read but is refused a change, its own included (§9, answer 4), with a message saying who to ask. The window's `getcontainerlimits` / `setcontainerlimits` RPCs share the same code; the set one is the operator's client only.
- **Out of memory.** Detected by `oom_kill` in the container's `memory.events` (cgroup v2) or `memory.oom_control` (v1), read through one `exec` when a turn starts and again when it ends (`backend/container/oom.rs`); a rise writes one `agentmux_container_limit` pane row naming the limit, with "Raise the limit". A turn that died of it (exit 137) keeps the `Killed` failure class with the title "Ran out of memory" and the limit in its detail: no new class, so Manoz's failure-row work (#10593) is not touched. The hint on the Bash tool's stderr (§5, `agentmux-bashwrap`) is not built: the pane row and the failure text carry the cause, and the wrapper change is its own piece.
- **`CARGO_BUILD_JOBS`.** Not set. §2 left it to the measurement of §7 to decide, and the measurement has not run.
- **Not built:** the machine-wide cap of §9.3 (decided: not in this build).

**What was checked.** The unit tests of §7 (defaults over a table of daemons, precedence, the `HostConfig` and `docker update` computation, the counter parsing, the stored text, the pane frames and rows). **Not tested on a live Docker daemon, and not on Docker Desktop:** the integration tests and the measurement of §7 have not run; they need a machine with Docker and room for a build, and none was available for this PR. Nothing here claims how a build behaves at a limit.

Agent: Loap-3@narko
