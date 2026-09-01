# Report — a universal agent work queue: where it belongs, and what to call it

**Date:** 2026-09-01
**Author:** AgentX
**Status:** Analysis + design proposal. Not a commitment to build.
**Prompted by:** operator ask —

> agentmux needs a universal queue of stuff that any agent can set, and any
> agent can pick up when the time comes. we need to build internal infra for
> that, potentially with cloud sync, but not necessarily. considering all our
> current pane types, where would this be? consider a couple names. would this
> be something managed through the drone pane? swarm? a new one?

---

## 1. Bottom line up front

**Three answers, in order of confidence:**

1. **This is a new backend primitive, not a feature of an existing pane.** The
   queue is a durable, globally-scoped store plus a claim protocol. Nothing that
   exists today is shaped like it (§3).
2. **It should NOT be managed through Drone, and NOT through Swarm.** Drone is a
   DAG *authoring* surface; Swarm is a *live-process monitor*. Putting a durable
   backlog in either would repeat a mistake this codebase has already made twice
   — attaching durable state to an ephemeral, block-scoped surface (§4).
3. **The smallest honest build is a sibling of `db_cron_jobs`, not a new
   subsystem.** Cron already is a work queue with a *time* trigger; this is the
   same row with a *readiness* trigger. Delivery, routing, and cross-channel
   forwarding are all already built and in production use (§5).

**The genuinely hard part is not storage or UI — it is the claim protocol**
(exactly-once handoff between concurrent agents) and **scope** (which store the
rows live in). Both are §6. Everything else is assembly.

---

## 2. What the ask actually requires

Reading the ask literally, four properties:

| Property | Implication |
|---|---|
| "any agent can set" | Write path reachable from an agent's own tools (MCP), not just the UI |
| "any agent can pick up" | Rows are **unassigned by default**; the taker is chosen at claim time, not enqueue time |
| "when the time comes" | Pull-based/deferred, not push-at-enqueue — the opposite of jekt |
| "universal" | One queue across agents, panes, tabs, **and channels** — not per-pane |

The third row is the load-bearing one. AgentMux's existing agent-to-agent
mechanism (jekt) is **push, addressed, and immediate**: it targets a named
recipient and delivers now. This ask is **pull, unaddressed, and deferred**.
Those are different primitives, and the difference is why nothing existing fits.

---

## 3. Survey — what exists today, and why none of it is this

Verified directly against the code, not from memory.

### 3.1 Drone (`frontend/app/view/drone/`, `db_drone_definitions` / `db_drone_runs`)

A **visual DAG builder**: `FlowNode`/`FlowEdge`/`DroneViewport`, a draft graph
with selection state, and per-run status folded through the `drone-run-state`
reducer slice (`drone-model.ts`). Runs are authored, then executed as a graph.

**Why it isn't the queue:** a drone is a *predefined pipeline with edges* —
node B runs because node A finished, decided at authoring time. A queue is a
*flat unordered set with no edges*, where the taker is decided at claim time.
Modelling "any agent picks this up whenever" as a DAG means authoring a node per
possible taker, which is the wrong shape.

**Where Drone genuinely fits:** as a **producer**. A drone node that enqueues
work is a natural feature. That is a much smaller and better-motivated
integration than hosting the queue.

### 3.2 Swarm (`frontend/app/view/swarm/`)

A **live monitor + fleet control surface**: `ActiveSubagent` rows
(`agent_id`/`parent_block_id`/`session_id`/`status`), fleet group actions, live
event subscriptions.

**Why it isn't the queue:** everything Swarm shows is *currently running*.
`REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md` §5a
already established this explicitly — the dispatch system is "live/in-memory
only, keyed by `parent_block_id`/a live request's `agent_id`, not
`definition_id`, and **dies with the block**." A backlog whose whole purpose is
outliving the agent that created it cannot live in a store with that lifetime.

**Where Swarm genuinely fits:** as a **viewer of in-flight claims** — "who is
working on what right now" is exactly Swarm's existing job. Claimed-and-running
queue items belong in Swarm's live view; the durable backlog does not.

### 3.3 Cron (`db_cron_jobs`, `backend/cron/mod.rs`) — the closest existing thing

```rust
pub struct CronJob {
    id, name, expression,        // 5-field cron, UTC
    prompt,                      // ← the work
    target,                      // ← target agent id for injection
    created_by, enabled,
    last_fired, fire_count, max_fires, created_at,
}
```

Strip `expression` and make `target` optional-until-claimed and this **is** the
queue row. Cron fires by POSTing to `/agentmux/reactive/inject`
(`backend/cron/mod.rs:225`) — the same tiered delivery path muxbus uses.

**This is the single most important finding in this report.** The queue is not a
new subsystem; it is cron with the trigger swapped from *time* to *readiness*.

### 3.4 Background tasks (`db_background_tasks`)

```rust
pub struct BackgroundTask { id, block_id, label, pid, started_at_ms,
                            status, last_seen_ms, ended_at_ms }
```

**Why it isn't the queue:** `block_id` + `pid` — it tracks an OS process already
running in a specific pane. It is an *execution record*, not a *work request*.
Its liveness semantics (`last_seen_ms` heartbeat) are, however, a good model to
copy for claim leases (§6.1).

### 3.5 Jekt / muxbus reactive (`server/reactive.rs`)

The delivery substrate, already built and hardened:

- **Tier 2a** — same channel, same host
- **Tier 2b** — same host, *different channel* (host-global shared registry,
  `~/.agentmux/shared/agents/reactive/`, issue #1916)
- **Tier 3** — LAN peer via mDNS → HTTP

Plus a full trust layer (signing, tier rules, `ESCALATE`).

**Why it isn't the queue:** jekt is addressed push delivery — it needs a
recipient *now*, and fails if that agent isn't reachable. But **it is exactly
what the queue should use for notification**, and Tier 2b already solves
cross-channel reach, which is otherwise the hardest part of "universal."

### 3.6 Agent groups (`db_agent_groups`)

`{ id, name, member_ids, created_at }` — a ready-made routing primitive. A queue
item targeted at a *group* rather than an agent gives "any agent of this kind
can pick it up" for free, with no new concepts.

### 3.7 Summary table

| Surface | Durable? | Cross-agent? | Cross-channel? | Unassigned work? | Fit |
|---|---|---|---|---|---|
| Drone | ✅ | ❌ (graph-scoped) | ❌ | ❌ (edges decide) | producer only |
| Swarm | ❌ (dies with block) | ✅ | ❌ | ❌ (live only) | viewer only |
| Cron | ✅ | ✅ | ⚠️ per-channel today | ❌ (time-triggered) | **template** |
| Background tasks | ✅ | ❌ (block-scoped) | ❌ | ❌ | lease model |
| Jekt/reactive | ❌ (transport) | ✅ | ✅ **2a/2b/3** | ❌ (addressed) | **delivery** |
| Agent groups | ✅ | ✅ | ⚠️ | — | **routing** |

Nothing has ✅ in the "unassigned work" column. That column is the feature.

---

## 4. Where it should live: a new pane

**Recommendation: a new pane, with Swarm and Drone integrating into it rather
than hosting it.**

Reasons, strongest first:

1. **Lifetime mismatch is a known, repeated bug class here.** Swarm's dispatch
   state dies with its block. Attaching a durable backlog to a block-scoped
   surface would reproduce that, and this codebase has already paid for it twice
   (the §5a dispatch finding; the per-channel identity split that produced five
   auth bypasses, `ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`).
2. **The audiences differ.** Swarm answers *"what is happening now?"*. A queue
   answers *"what is outstanding, and who should take it?"* — a planning surface,
   not a monitoring one. Merging them makes both worse.
3. **The widget bar has room and precedent.** Eleven widgets exist; four are
   pinned by default. A twelfth is not a structural cost.
4. **Drone would have to grow a second, contradictory execution model** (edgeless
   pull-based work) alongside its DAG model. That is a strictly worse outcome
   than a separate surface both can talk to.

**Counter-argument, stated honestly:** a new pane is real surface area — view
registration, block type, layout persistence, docs, and a widget entry. If the
first version needs to be cheap, the queue can ship **backend-first with no pane
at all** — MCP tools for enqueue/claim/complete plus a `muxspect`-style CLI view.
That is genuinely useful to agents on day one and defers the UI decision until
the semantics are proven. **This is the recommended sequencing** (§7).

---

## 5. Proposed shape

### 5.1 Storage — `db_work_queue`, global scope

```
id             TEXT PRIMARY KEY
title          TEXT NOT NULL      -- human-scannable
payload        TEXT NOT NULL      -- the prompt/instruction injected on claim
kind           TEXT               -- free-form tag: "review", "repro", "triage"
target_agent   TEXT               -- optional: a specific agent
target_group   TEXT               -- optional: db_agent_groups id
priority       INTEGER NOT NULL DEFAULT 0
state          TEXT NOT NULL      -- open | claimed | done | failed | cancelled
claimed_by     TEXT               -- agent_id holding the lease
claim_expires  INTEGER            -- ms epoch; lease, not a lock (§6.1)
created_by     TEXT NOT NULL
created_at     INTEGER NOT NULL
updated_at     INTEGER NOT NULL
not_before     INTEGER            -- optional "when the time comes"
result         TEXT               -- completion note / error
```

`not_before` gives deferred work without a scheduler, and makes the cron
relationship explicit: **cron is this row with a recurring `not_before`.**

### 5.2 Claim protocol

```
enqueue()                → INSERT state=open
claim(agent, filter)     → UPDATE ... SET state='claimed', claimed_by=?,
                           claim_expires=now+lease
                           WHERE state='open' AND <filter> AND
                                 (not_before IS NULL OR not_before<=now)
                           ORDER BY priority DESC, created_at ASC LIMIT 1
                           RETURNING *
heartbeat(id, agent)     → extend claim_expires while working
complete(id, result)     → state=done
release(id)              → state=open (explicit give-back)
reap()                   → claimed rows past claim_expires → open (+ attempt++)
```

The claim is a **single conditional UPDATE...RETURNING** — atomic in SQLite
under the existing single-writer connection. That is the whole concurrency
story, and it is why this doesn't need a real message broker.

### 5.3 Delivery

Do **not** invent a new push path. On claim (or on enqueue for a targeted item),
POST `/agentmux/reactive/inject` exactly as cron does. That inherits Tier
2a/2b/3 routing, including same-host cross-channel — which is most of "universal"
already solved.

### 5.4 Cloud sync — explicitly out of v1

The ask says "potentially with cloud sync, but not necessarily." Recommend
**not** in v1:

- The hard problems (claim races, lease expiry, ordering) must be correct
  locally first; distributed claiming is strictly harder and would be built on
  unproven semantics.
- muxbus already provides cross-host *delivery*; cross-host *claiming* is a
  different and much harder guarantee.
- Local-first keeps the whole feature inside one SQLite store with one writer.

Design the row so sync is possible later (stable ids, `updated_at`, no
host-local paths in the schema) and stop there.

---

## 6. The two genuinely hard decisions

### 6.1 Leases, not locks

A claimed item whose agent dies must return to the pool. Use a **lease with
heartbeat**, copying `db_background_tasks`'s `last_seen_ms` pattern, plus a
reaper. Do **not** use a bare `claimed` boolean — this codebase has an existing
incident class of exactly this shape (`db_background_tasks` rows stuck
`running` forever, agentmux issue #2518).

Add `attempts` and a cap so a poison item that crashes every taker doesn't
cycle forever.

### 6.2 Which store — and this is where it can go wrong

**The queue must be GLOBAL, not per-channel.** If it lands in the per-channel
store, "any agent can pick it up" silently becomes "any agent *in this channel*"
— and given that every local/dev/portable build is its own channel, that is
close to useless.

This is not hypothetical. `SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` split
storage into a per-channel store and an always-global one; **five of its six
steps were never built** (tracking issue #2627 was closed the day step 1
landed). `db_cron_jobs` is named in that spec's §2.3 audit as having *no
legitimate reason to be per-channel* — and its call sites are still unmigrated
("step 1b — not yet started"). Today's `ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`
traced five real auth bypasses to precisely that unfinished split.

**Recommendation:** put `db_work_queue` in the always-global identity store from
day one, and treat finishing step 1b (cron + friends) as a prerequisite or an
immediate follow-up — not as someone else's problem. Building a second
per-channel-by-accident table into that same seam is the predictable failure
mode here.

---

## 7. Suggested sequencing

1. **Backend + MCP tools only.** `db_work_queue` in the global store, the five
   claim operations, MCP `WorkEnqueue`/`WorkClaim`/`WorkComplete`. No UI. Agents
   can use it immediately; semantics get proven under real load.
2. **`muxspect work`** — list/inspect, matching the existing diagnostic CLI.
   Cheap, and makes the queue observable before any pane exists.
3. **Reaper + lease expiry**, with a test that a killed claimant's item returns
   to the pool (the #2518 lesson).
4. **Pane**, once the shape has settled. Board or list view.
5. **Drone producer node** and **Swarm claimed-items view** — the integrations
   that made those panes look like candidates in the first place, done as
   integrations rather than as ownership.
6. **Cloud sync**, only if a real cross-host need appears.

---

## 8. Names

Ranked, with the reasoning that matters — this codebase already has strong
naming conventions (muxbus, muxspect, muxlog, jekt) and a queue should sit
inside them rather than beside them.

| Name | For | Against |
|---|---|---|
| **Muxqueue** | Fits the `mux*` family exactly (muxbus/muxlog/muxspect); instantly legible; the CLI writes itself (`muxqueue list`) | Least imaginative; "queue" undersells the routing/claim semantics |
| **The Docket** | A docket is precisely "outstanding matters awaiting assignment" — semantically the most accurate word available; distinctive; pane name reads well ("Docket") | Doesn't fit the `mux*` family; slightly formal |
| **Hopper** | Strong physical metaphor for pull-based work (things go in, workers draw from it); short; pairs naturally with Drone/Swarm's machine-and-insect vocabulary | Generic in data engineering (hopper = ingest buffer), could mislead |
| **Backlog** | Zero explanation needed to any engineer | Loaded with sprint/Jira connotations this is not; boring |
| **Relay** | Captures hand-off between agents, which is the actual point | Collides conceptually with muxbus/jekt, which are the real relays — actively confusing |

**Recommendation: `Muxqueue` as the internal/system name** (store, MCP tools,
CLI — consistency with `muxbus`/`muxspect` matters more than flair for
infrastructure), **and `Docket` as the pane label** if and when a pane ships.
Precedent exists for exactly this split: the backend table is `db_bundles` while
the UI says "Armory Bundle Format," and the persisted view key stays `"memory"`.

Avoid **Relay** outright — the confusion cost against muxbus/jekt is real.

---

## 9. Open questions for the operator

1. **Claim policy** — should an idle agent auto-claim matching work, or only
   claim when explicitly asked? Auto-claim is the more useful product and the
   more dangerous one (an agent picking up work unattended).
2. **Does a queue item carry authority?** If an item says "merge PR #123", does
   claiming it authorize the merge? Recommend **no** for v1 — items are prompts,
   and existing per-action gates still apply.
3. **Cross-channel claiming**, given §6.2 — should an agent in channel A be able
   to claim an item enqueued in channel B on the same host? Tier 2b makes it
   *possible*; per-channel isolation just got deliberately tightened, so this
   deserves an explicit decision rather than a default.
4. **Retention** — do completed items persist as an audit trail, or get GC'd?
   `db_agent_native_memory_versions` has an existing retention/GC pattern (#2728)
   to copy.
