# Spec: MuxAPI agent live facts (model and the rest), on this host and across the LAN

**Status:** active — approved 2026-11-20 with every recommended decision; phase 1 (this host: the facts, `Agents list` fields and filters, `Agents status`, the Swarm row) is built in the first build PR; phase 2 (LAN) and phase 3 (docs page) remain. Task #48221 (spec), build #48222. Author: Agent2-6.
Builds on #48192 (account usage windows, merged as #7872 and #7909) and feeds the Swarm model tags (#48219, #48220).

## 1. Requirement

An agent, and a person through an agent, must be able to ask AgentMux "what is each agent actually running on, and in what state" and get an answer from AgentMux's own runtime state, for every agent: native and container, on this host and on LAN peers. Today `Agents list` says seat, availability, runtime state and provider, but not the model. A question such as "is any agent not on the model we agreed on?" can only be answered by reading each agent's session transcript on disk, which works for native agents on the same machine and nothing else.

Principles:

- **Runtime truth, not the definition and not the agent's say-so.** Every field says where it came from (`source`) and when (`observed_at_ms`), so a requested value is never shown as an effective one.
- **One definition of the facts.** A single struct (`LiveFacts`, srv-core) is built once per agent and used by `Agents list`, `Agents status`, the Swarm's `agentruntime.list` rows and the LAN call. No second source.
- **Small by default.** The default row grows by two short fields; everything else is opt-in by `fields`.
- **No credentials, no content.** See 4.

## 2. What exists today (read from the code on 2026-11-20)

| Fact | Where it lives today | Notes |
|---|---|---|
| Rows of `Agents list` | `GET /agentmux/discovery` (`http_health.rs`), MCP `run_discover_agents` returns the JSON unchanged | `list` ignores all arguments; no filters, no fields. Row: name, id, definition_id, working_directory, addressable, block_id, seat, spend, availability, runtime. |
| Effective model and effort | `get_settings` control request, answered into `PersistentInner.effective_runtime`; published on the `agentruntime` event as `effective_model`, `effective_effort` | Asked at spawn and after every turn, only for control-protocol panes (argv has `--permission-prompt-tool`). Updates at the next turn boundary. |
| Model of the last call | `ContextReading.model` (in memory), from `assistant` and `message_start` frames | Not persisted. |
| Requested model, effort, permission mode | block meta `agent:runtime`; the definition's `provider_flags`; `db_agents.last_runtime` | What was asked, not what runs. |
| Codex model | block meta `agent:model` (app-server controller) | |
| Context tokens, window, auto-compact point, last compaction, calls since | `ContextReading` in `BY_BLOCK` (in memory) | No compaction counter. |
| Turn active, turn start | `TurnActivityTracker` (`health.rs`); controller status `turn_active` | |
| Last activity, line count | block meta `session:last_activity_ms`, `session:line_count` | `session:start_ts_ms` is the first output line, not the process start; process spawn is an atomic in the health tracker. |
| Claimed task | `WorkItem` in the work queue (`claimed_by_uid`, state `claimed`) | No per-agent index. |
| Spend today, cap, stopped | `spend_watch::view` (route agents only); already in the list row | Keyed by agent slug. |
| Account | block meta `agent:run_account` (a local id); `stable_id_of_block` gives the stable id; label as in Identity accounts | |
| Usage windows | `account_usage::of_account` (#48192); already in `availability.usage` | |
| Runtime kind and image | `container_of_meta`; default image `agent-base` | |
| Host, version, build | discovery `host.*`; `AGENTMUX_VERSION`, `AGENTMUX_BUILD` | Per host, not per agent. |
| Other machines | discovery `lan` section: name, `AgentState`, provider, kind, stable account id, reset time, seat | From the peer's `agent-names` poll. No model, no context, no turn, no spend. |

One cost note: discovery computes availability twice per agent (once inside `agentruntime.list`, once for the row) and does a full block scan per call. The new builder shares one pass.

## 3. Fields

Ranked by cost to produce (cheapest first). "Free" means already computed on the path that builds the row. Every group carries `source` and `observed_at_ms`.

**Tier A, free (already computed per agent)**

1. `model` (id) and `model_label` (short display name, for example "Sonnet 5.5"), `model_source`: `effective` (the CLI's own `get_settings` answer) > `observed` (last call's model) > `requested` (`agent:runtime`, `last_runtime`) > `unknown`. Codex reads `agent:model`; a route agent reads the route's model; a harness that reports nothing says `unknown` with a `why`. The label comes from the provider model catalog (`providers.models`), falling back to the id.
2. `effort` (effective, else requested), same `source` rule.
3. `harness` (provider and harness name) and `runtime` (`native` or `container`, plus `image` for a container).
4. `route`: the route name if the agent runs through one, and `differs_from_definition` (true when the effective model or route is not what the definition's flags and last choice say; both values shown when true).
5. `permission_mode` (from `agent:runtime`).
6. `turn`: `active`, `since_ms`, `duration_ms`.
7. `pane`: `shown` (on screen or not), `block_id`.
8. `account`: `stable_id` and `label` only (the label is the one Identity accounts shows).
9. `usage`: the account's windows exactly as #48192 returns them (percent, reset, age), not recomputed.
10. `spend`: today, cap, stopped (route agents; "not measured" with a reason for a harness that cannot say).

**Tier B, cheap (one in-memory or meta read per agent)**

11. `context`: `tokens`, `window`, `percent`, `auto_compact_at`, `compacting`. In memory only: null after an srv restart until the agent's next frame.
12. `last_activity_ms` (and `idle_ms`).
13. `session`: `started_ms` (process spawn, not first output) and `uptime_ms`.

**Tier C, new work**

14. `compactions`: a counter. `ContextReading` has only the last time and calls since; add `compactions` (incremented on `compact_boundary`), in memory, reset with the pane.
15. `task`: the current claimed task number. Built by one pass over claimed work items for the whole list (not one query per agent) and joined by uid. Cost grows with claimed items, not with agents.
16. `host`: name, AgentMux version, build, channel. Per host, repeated on each row only in `detail=full` or in the `hosts` block.

**Not exposed** (also stated in the tool description, so an agent knows not to look for them): credentials of any kind (tokens, keys, OAuth data, the run-account's secret), vault entries and what an agent used from the vault, prompts and system prompts, message and jekt text, the transcript, tool arguments and results, environment values, file contents. The account email is not in a row (stable id and label only). Nothing here lets an agent read what another agent said or did, only what state it is in.

## 4. Who may read

Every registered agent, container agents included. The discovery route is already open to every audience and to a container's minted token; the build adds the new routes and the LAN call to the same audience and tests it with a container token. A container agent's facts are read from the host's own state (the image from the container manager, the model from the host's readback), never from inside the container, so a container agent cannot set what is reported about it.

What an agent may not read about another: nothing in section 3 "not exposed". `Agents context` (the hint flags and the compaction schedule) stays own-pane; only its three numbers (tokens, window, percent) are in the shared facts, because they say how close an agent is to compacting and carry no content. This follows the container rule: a refusal needs a host-exec or host-file reason, and there is none here.

## 5. Where: `Agents list`, `Agents status`, filters, the Swarm

### 5.1 `Agents list`

- **Default rows keep every field they have today and gain `model_label` and `model_source`.** Nothing is removed or renamed, so existing callers and prompts keep working.
- `fields`: an array of field names or groups: `model`, `runtime`, `turn`, `context`, `account`, `usage`, `spend`, `task`, `session`, `host`, or `all`. Only those are added to each row.
- `detail`: `basic` (the default, as above), `facts` (every Tier A and B field), `full` (all, including `host` on each row).
- Filters, all optional and combined with AND: `host` (this host's name, a LAN peer's name, or `all`), `state`, `model` (substring of the id or the label; a leading `!` negates it, so `!sonnet` is "not on Sonnet"), `provider`, `runtime`, `account` (stable id or label), `seat`.
- `limit` and `offset`, and the answer says `total` and `returned`, so a caller never mistakes a page for the fleet.
- Size: today the full list for a few dozen agents is tens of kilobytes. With `fields=all` a row is about 0.7 to 1 KB (to be measured in the build and asserted in a test); the default row grows by about 40 bytes. The answer reports `bytes_estimate` and, past a threshold (default 60 KB), truncates with `truncated: true` and a pointer to `limit`/`offset` or `Agents status`.

### 5.2 `Agents status` (new, compact)

One line per agent, cheap to read at 100 agents (about 100 bytes a line):

```
name | host | state (turn 4m) | model_label effort | ctx 62% | account label | 5h 60% 7d 44% | #task
```

plus a footer of counts: `by_state`, `by_model`, `by_host`, and `group_by` (`model`, `state`, `host`, `account`) to return groups instead of lines. It takes the same filters as `list`. "Which agents are not on Sonnet, and what are they doing" is one call: `Agents status` with `model: "!sonnet"`.

### 5.3 The Swarm and other windows

- srv adds the same `LiveFacts` fields to the `AgentRuntime` row of `agentruntime.list` (the Swarm's source), so the Swarm needs no second call. The model tag (#48219/#48220) reads `model_label` and `model_source`; a `requested` or `unknown` source is shown muted.
- Live updates ride the existing `agentruntime:changed:<agent id>` event. It fires when model, effort, turn state or account change. Context and usage do not fire it (they change too often); the window reads them with the periodic pull it already does.
- Bindings: the row type is generated (`AgentRuntime.ts`); `LiveFacts` is a generated type shared by the row, the status line and the LAN answer.

## 6. Across hosts

### 6.1 Today

A LAN peer's agents show as liveness only (`AgentState`, provider, kind, stable account id, reset time, seat), from the peer's `agent-names` poll. `host` is refused on `Agents list`.

### 6.2 The call

A new remote op `agents.status` in the existing remote-ops table (`remote_ops/mod.rs`), implemented next to the identity ops: it calls the same local builder and returns its answer. It is `read_personal`: an approved install only, using the trust a peer already pins (the same as `identity.accounts`). It adds no key, no port and no widening of trust, because it carries nothing the pinned peers cannot already read from `identity.accounts` plus liveness; the account email is not in it.

- `Agents list` and `Agents status` accept `host: "<peer>"` (one peer) and `host: "all"` (this host plus every pinned LAN peer, in parallel, 3 s timeout each). Without `host` the answer is this host only, as today. The answer has a `hosts` block: `{host, reachable, version, as_of_ms, age_ms, error?}`, and each row says which host it came from.
- A peer that does not answer, or answers `unknown op` (an older build): its agents appear with liveness only from the LAN poll, `facts: "liveness_only"`, and the `hosts` entry says `unreachable` or `needs_upgrade` with the version if known. Nothing is guessed; a missing field is `null`, never zero or the definition's value.
- The cost to the answering host is one extra op per peer per call; nothing is polled in the background for this.

### 6.3 WAN-only and cloud-only agents

An agent reachable only through the relay (no LAN trust) is not readable at the field level: the relay carries presence, not facts. It is listed (name, host if known, presence state) with `facts: "not_available"` and a `why` ("reachable only through the relay"). It is never omitted silently, so the count is honest, and the build does not add facts to the relay (a possible later step, out of scope here).

## 7. Staleness and switching

| Event | What the facts do |
|---|---|
| Model switch or `Agents update` with a model | The row shows the **requested** model with `model_source: "requested"` and `pending_switch: true` until the CLI's next `get_settings` answer (at the next turn boundary) confirms it, then `effective`. A switch that restarts the pane shows state `starting`. |
| A pane that never answers `get_settings` (no control protocol) | `model_source: "observed"` from the last call, else `requested`, else `unknown` with a `why`. |
| Route change | `route` and `differs_from_definition` follow the launch choice at the next start; a running pane keeps reporting what it started with. |
| Account move | `account` is the pane's run-account; while a queued move waits for the end of the turn the row shows `account.pending_move` with the target label. |
| Agent stopped or pane closed | Runtime-only groups (turn, context, session, pane) are `null`; `model` falls back to the remembered request with `model_source: "requested"` and state `stopped`. |
| srv restart | Context and compaction counters are null until the agent's next frame; usage returns with the next rate-limit frame (#48192 rules). |
| Reading too old | Every group has `observed_at_ms`; `age_ms` is computed at answer time. A group older than its threshold (context: 5 min in a turn; usage: 15 min, as on screen) is returned with `stale: true`, not dropped. |

While a value is unknown the row says so in words (`unknown`, with `why`), and the Swarm tag shows nothing rather than a guess.

## 8. Docs and tests

- Tool descriptions: `Agents` (`list` fields, `detail`, filters, `status`, `host`), with the "not exposed" list.
- MuxAPI reference page in the docs repo (its own PR, since it is a different repository), showing the fields, a field's `source` meanings, filters, the `!model` form, and what a LAN peer or relay-only agent returns.
- Tests (srv): a fake runtime with two models (one effective, one only requested) and the filter `!sonnet` returns only the other; `fields` and `detail` add exactly the named groups and the default row keeps every old key; a container agent's token reads the list and status and sees the image; a fake LAN peer answers `agents.status`; a peer that times out, a peer on an older build, and a relay-only agent each come back as liveness-only with the right `hosts` entry; `pending_switch` clears at the readback; no field or serialized row contains a token, a key or an email; a row-size bound for `fields=all` and the truncation answer; `Agents status` footer counts match the rows.
- Tests (frontend): the Swarm tag shows `model_label`, muted for `requested` and `unknown`; the generated bindings are current (`gen-rpc-stubs --check`).

## 9. Build plan (#48222)

1. **P1, this host:** `LiveFacts` in srv-core (one builder, one pass; availability no longer computed twice), the `compactions` counter, `Agents list` `fields`/`detail`/filters/paging, `Agents status`, `AgentRuntime` fields for the Swarm, tool descriptions, tests.
2. **P2, LAN:** the `agents.status` op, the `host` argument and fan-out, the `hosts` block and the liveness-only fallbacks, tests with a fake peer.
3. **P3, docs:** the MuxAPI reference page.

Each is one PR. P1 unblocks the Swarm model tags.

## 10. Decisions for the approver

1. The default `Agents list` row gains `model_label` and `model_source` (about 40 bytes a row). Recommended yes; the alternative is no change to the default and `fields=model` always.
2. `host: "all"` fans out only when asked (recommended), versus by default.
3. The LAN op is `read_personal` (approved installs only), as `identity.accounts` is. Recommended; a plain `read` would also expose spend and task numbers to any caller.
4. Read the init frame's `model` too, so the model is known before the first `get_settings` answer? The spawn-time readback already asks early, so this is optional; recommended only if the first answer is observed to lag.
5. Compaction count in memory only (recommended) or persisted per session.
