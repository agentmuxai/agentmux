# Spec: Container status in the frontend: each agent's runtime kind, and a live list of this machine's agent containers

**Status:** draft. For Camper's agreement (the Atlas side), then the build (#47596).

Task #47595 (build: #47596, after this). Owner: AgentA (srv core). Requested by Camper (Atlas, #47554 part b); routed by
AgentY. Written 2026-11-14 from origin/main `a87476e72`. Part a (the `runtime` field on rows) is built (#7304); part b (the container module, events, route and store) is built in the PR that carries this file.

## 1. What is asked

Atlas wants containers as their own nodes, and agents drawn as host or container, and cannot: no container state reaches
the frontend. Two data pieces, names and states only (nothing from inside a container):

1. The **runtime kind** (host or container) on the `AgentRuntime` row and on the rows srv sends for agents of other
   instances.
2. A **list and a change event** for this machine's agent containers, and the **container runtime's own state** pushed
   when it changes (today it is pull-only).

Container agents may read all of it (the operator's rule of 2026-10-28: a new read route is available to container
agents by default).

## 2. What exists today (re-read before building)

- **The kind is known and not sent.** `RuntimeKind` (`crates/srv/src/backend/runtime_adapter.rs`, `Host` | `Container`)
  is decided by the pane block's meta `agentMode` (`RuntimeKind::of_block_meta`) or, for an agent with no pane, by its
  definition's `agent_type` (`storage/agents/mod.rs`, `RuntimeKind::of_agent_type`). The `AgentRuntime` row
  (`crates/srv/src/backend/agent_runtime_state.rs`, built in `crates/srv/src/server/agent_runtime.rs` `list()`, published
  as `agentruntime:changed:<agent_id>` when any field changes, polled every 2 s) has no runtime field, though `list()`
  holds both the definition and the block. The frontend reads the kind only from block meta
  (`frontend/util/runtime-kind.ts`), so only for an agent that has a pane.
- **Remote rows carry none.** `RemoteAgent` (`crates/srv/src/backend/swarm_remote.rs`) has no kind. The LAN fleet feed
  already advertises it (`agent_kinds`, `lan/discovery/agent_names.rs`, from `FleetSnapshot.agent_kinds`), and the reader
  drops it: the peer record (`lan/discovery.rs` `LanInstance`) has no field for it. This machine's other channels answer
  the same feed over loopback, so the same field is there for them too.
- **No container event exists.** Nothing in `backend/mps_events.rs` is about a container. srv drives Docker through the
  bollard API (`backend/container.rs`; runtime detection in `container/runtime.rs`), looks a container up on demand
  (`find_container`, one filtered `list_containers`), and listens to no Docker event stream and polls nothing between
  turns. The only container pushes are per-turn transcript frames (`ContainerStartState`: pulling, starting, ready,
  failed, `cli_notice.rs`), which are not mux events.
- **A stopped container looks like a stopped agent.** `AgentRuntimeState` comes from agent availability; nothing in it
  reads Docker.
- **The runtime's availability is pull-only**: the `containerruntimeavailable` command (`{available, runtime?, detail?}`),
  answered by a live ping on each call.
- **A pattern to copy:** the message-flow and task-flow events (#7173, #7192): a static with a bounded in-memory list, an
  event name in `backend/mps.rs`, a typed payload in `mps_events.rs` exported by ts-rs, and a read route behind the normal
  auth (`/api/v1/message-flow`, `/api/v1/task-flow`; `route_audience.rs` lists each as `Everyone`).
- **Container agents' reach.** They cannot use the websocket (`/ws` is host-only), so the `agentruntime.list` and
  `containerruntimeavailable` commands do not reach them. An HTTP route that is `Everyone` in
  `backend/route_audience.rs` does; a path under `/api/` with no entry there is refused as unclassified.

## 3. Proposal

### 3.1 The runtime kind on rows

- `AgentRuntime` gets `runtime: "host" | "container"`. `list()` fills it from the agent's block meta when a block exists
  (the current truth) and from its definition's `agent_type` when it has none, so a stopped agent says it too. Anything
  unknown is `host`. A change of the field publishes `agentruntime:changed:<agent_id>` like any other.
- `RemoteAgent` gets `runtime?: "host" | "container"`, absent when its instance did not say (an older build). LAN peers: the
  discovery reader keeps `agent_kinds` (only the two known values, `RuntimeKind::parse`, bounded like the other maps).
  This machine's other channels: `ChannelReport` keeps it from the same feed. Cloud rows (the cloud-tier spec, #47599) fill
  it from the presence record's per-agent `kind`, which is already signed and published.
- One name, `runtime`, on every row the frontend reads; `agent_kinds` stays the wire name of the LAN feed. The frontend
  reads `runtime ?? "host"` for a local row and shows nothing for a remote row that has none.

### 3.2 This machine's agent containers

A new module `backend/container_status.rs`, shaped like the flow modules:

- **Which containers.** Only AgentMux's agent containers: the name is `agentmux-<agent id>` (`container_name_for_slug`,
  where the agent id is the definition's id, the one `AgentRuntime.agent_id` carries). A container of that form whose id is
  no definition's is listed as an **orphan** (`agent_id` null), because that is the one a person wants to see. Another
  container is never listed: the name must start with `agentmux-` and then carry the label `ai.agentmux.agent=<agent id>`
  (new containers get it on create), or look like an agent id (a UUID), or be a definition's id, so a container such as
  `agentmux-<something else>` is not taken for an agent's. The lookup stays by name for the containers that predate the
  label. Decision 4.
- **The row.** `ContainerStatus { name, agent_id, agent_name, state, image, since_ms }`, where `agent_id` and `agent_name`
  are **null** (present, not omitted) for an orphan. `state` is one of `starting`,
  `running`, `stopped`, `gone`; `image` is the reference as srv created it (name and tag, no credentials: a reference
  carries none); `since_ms` is when the container entered that state. Nothing from inside it: no log, no env, no mount,
  no stats.
- **The runtime row.** `ContainerRuntimeStatus { available, runtime?, detail?, at_ms }`, the same fields as
  `containerruntimeavailable`, pushed when they change.
- **Events.** `container-status` (one `ContainerStatus` per change, `gone` once when a container is removed) and
  `container-runtime` (one `ContainerRuntimeStatus` per change), registered in `mps.rs` and `mps_events.rs`, payloads
  exported by ts-rs.
- **The list.** `GET /api/v1/containers` → `{ "runtime": ContainerRuntimeStatus, "containers": [ContainerStatus] }`: what is
  there now (a removed container is only ever an event). `Audience::Everyone` in `route_audience.rs`, behind the normal
  auth, so a container agent can read it; read-only, no query that reaches a container. The route looks at the runtime live, as `containerruntimeavailable` does, so a reader finds a runtime that started after srv did. **Where there is no Docker it
  answers 200 with `containers: []` and `runtime.available: false`, never an error.**
- **Typed events.** Both events are typed mux events (registered in `mps_events.rs`, bindings from `gen-mux-events`,
  checked with `--check`), so a pane subscribes with `onMuxEvent` the way it would for message-flow.
- **Where the states come from.** Docker is the source of truth. srv subscribes to Docker's event stream
  (bollard `events()`, filtered to containers whose name starts with `agentmux-`) and keeps a map; a start, die, stop or
  destroy event updates one entry and publishes it. On connect, and every 60 s as a reconcile (a stream can drop), one
  `list_containers` (all, name filter) rebuilds the map and publishes only what differs. srv's own transitions
  (`ensure_running`'s pulling and starting) mark an entry `starting` at once, ahead of Docker's event. The runtime's
  availability is a ping every 30 s while any agent container is known or the runtime was ever available, and on every
  connect and reconnect; nothing runs on a machine with no container runtime and no container agents.
- **Cost.** One event subscription and one list per minute, only where Docker is present. No per-turn cost is added.

### 3.3 The frontend

A store `frontend/app/store/container-status.ts`: it loads the route once, applies the two events, and reconciles by
re-reading every 60 s (the shape `agent-runtime.ts` already has). Atlas reads `runtime` from the rows and the container
nodes from the store; the container runtime's state is a badge. Nothing here is Atlas-specific, so the Swarm can show the
kind the same way.

## 4. Decisions to confirm

1. **Field and name.** `runtime` on `AgentRuntime` and `RemoteAgent`, values `host` | `container`. *Recommend:* as
   written.
2. **Which containers are listed.** This machine's agent containers, by name, orphans included, nothing else.
   *Recommend:* as written.
3. **Docker events plus a 60-second reconcile**, against polling alone every 10 s. *Recommend:* events plus reconcile
   (state changes are shown within a second and an idle machine costs a list per minute).
4. **A label on new containers.** `ai.agentmux.agent=<agent id>`. *Recommend:* yes, in the same PR; the name check stays for
   older containers.
5. **The route.** `GET /api/v1/containers`, `Everyone`. *Recommend:* as written. *Alternative:* none: the websocket is
   closed to container agents.
6. **The build as two PRs.** (a) the `runtime` field on the three kinds of row, small and with no new event; (b) the
   container module, events, route and store. *Recommend:* two, so (a) lands for Atlas first.

## 5. Plan (the build, #47596)

- **PR a.** `AgentRuntime.runtime`, filled in `list()`; `RemoteAgent.runtime` from the LAN reader and the channel
  report; the TS bindings regenerated (`gen-rpc-stubs --check`); the frontend types; tests: an agent with a container
  definition and no pane says `container`, a block with `agentMode` `container` wins, an unknown value reads as host, a
  peer's unknown kind is dropped, and an older peer's missing field leaves the row's field absent.
- **PR b.** `container_status.rs` with the event source behind a small trait (a real bollard stream, and a fake for
  tests); the events and payloads; the route and its `route_audience.rs` entry (and the test that every route is
  classified); the label on create; the store. Tests: each Docker event maps to one state and one publish, a dropped
  stream resubscribes and the reconcile publishes only the difference, an orphan is listed without an agent, another
  container is not, a machine with no runtime publishes `available: false` once and nothing after, and a container agent's
  request is accepted by the route.
- Each PR: a changeset, light checks, one push; Linux for PR b (the Docker socket) and a look at Atlas by Camper.

## 6. Risks and notes

- **Podman** answers the Docker API through a compatibility socket; its event stream differs in detail (event names for a
  pod's containers). The mapping reads only start, die, stop and destroy, and the reconcile list corrects a miss; the
  build checks both runtimes where one is available.
- **Several channels on one machine** share one Docker daemon and the same `agentmux-<agent id>` names, so each lists the same
  containers; that is true to the machine and costs one event stream per instance.
- **A container started outside srv** under an `agentmux-` name appears as an orphan, which is the point.
- **Older builds** send no `runtime` on remote rows: the frontend shows no kind for them, never a guess.
- **Public repository:** nothing here names a private file or an unfixed weakness.

## 7. Where each fact lives (for the build)

- `crates/srv/src/server/agent_runtime.rs` (`list`, `changed_rows`, the 2 s watch), `backend/agent_runtime_state.rs`.
- `crates/srv/src/backend/runtime_adapter.rs` (`RuntimeKind`), `backend/storage/agents/mod.rs` (`agent_type`).
- `crates/srv/src/backend/container.rs` (`container_name_for_slug`, `find_container`, `create_and_start`),
  `container/runtime.rs` (`RuntimeSlot`, `is_available`), `server/agent_handlers/core.rs` (`containerruntimeavailable`).
- `crates/srv/src/backend/lan/discovery/agent_names.rs` and `lan/discovery.rs` (`LanInstance`), `backend/fleet_feed.rs`
  (`FleetSnapshot.agent_kinds`), `backend/swarm_remote.rs`.
- The pattern: `backend/message_flow.rs`, `backend/task_flow.rs`, `backend/mps.rs`, `backend/mps_events.rs`,
  `server/message_flow_handler.rs`, `server/routes.rs`, `backend/route_audience.rs`.
- Frontend: `frontend/app/store/agent-runtime.ts`, `agent-runtime-live.ts`, `frontend/app/view/swarm/swarm-remote.ts`,
  `frontend/util/runtime-kind.ts`.

Agent: AgentA@Area54
