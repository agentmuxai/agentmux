# SPEC: Drone for real-world jobs: triggers across tiers, cloud nodes, and Drone as a job action

**Date:** 2026-11-11
**Status:** active (shipped: phase 1a server half, a job runs a drone with the run input, in PR #7018; remaining: the `Drone` tool, the Trigger node, "Open graph", and phases 2 to 8 as listed in section 10). Approved 2026-11-11 (about 17:30Z). The operator's words, relayed by AgentY: "drone spec looks good." No exceptions were named, so decisions D1 to D11 stand as recommended (v1 approvals are `person`). Reviewed by AgentX, and for security by AgentY (public side). **Amended by `SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md`**, which reverses D5 and the omissions in §4.2 (loops, sub-drones, mid-graph waits, a run-command step) and moves cancel, timeouts and durable waits into a new engine phase. The cloud phases (7 and 8) do not start until the private companion exists and its security section has been read. The cloud half has a private companion, written or reviewed by AgentA (§7).
**Trigger:** The operator, 2026-11-11, after the nightly report failed: "curious if we can start designing real-world drone tooling. For example, the github nightly report. that could also be something that ties into real time activity in running agentmux instance. drone could work also across tiers. I also realize that drone would need significant cloud resources. for example, timers and an abstraction over [a cloud event bus] hosted on agentmux cloud. cloud users can then use that as a node in the drone and it would work via a cloud trigger. can you also think of more ideas like that? for users without cloud accounts, they are still able to trigger jobs via local/lan level operations."
**Related:** `SPEC_JOBS_AND_TEMPLATES_2026_10_24.md` (the ledger, Jobs tab, job definitions and caps this builds on; its P4 to P6 are what this spec details); `SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md` (the canvas); `SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md` (how a person approves a request); `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md`; `SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md`.

## 0. Summary

- **One graph, every tier.** A drone never depends on where it was started. Only its *trigger* does, and a trigger says which tier it needs: **local** (this AgentMux, no account), **LAN** (a peer host), **cloud** (AgentMux Cloud, works while this machine is off). Every cloud trigger has a **local form** that does the same job with less reach, so a user without a cloud account is never shut out of a graph (§3).
- **A trigger node, owned by the job.** The canvas gets a Trigger node, but it is a view of the job's `trigger` (Jobs spec §5.1), not a second scheduler. The job owns the trigger, policy, caps and ledger row; Drone is the action (`action.kind: "drone"`, the Jobs spec's P4). No second scheduler, no second ledger (§5).
- **Six new things a real job needs** that Drone lacks today: triggers; a way to carry an event's data in; real actions (message an agent, file a task, spawn a worker, call an API with a credential); a durable approval; a cancel, a timeout and a cost cap; and a check that stops untrusted text from reaching an outside write unseen (§4, §6).
- **Events are data, never instructions.** Everything a trigger carries is tainted; only typed fields (ids, numbers, enums) and values matched against a closed set (a known agent, a known repository) are clean. A graph in which tainted data can reach an outside write with no Approval in between is refused when it is turned on (§6).
- **Cloud nodes** are a hosted timer, a generic event source (the operator's event-bus idea, kept behind AgentMux's own API), GitHub events, a per-drone public webhook, shared polling of release feeds, a durable approval answerable from the phone, a once-across-machines lock, and a dead man's switch. The cloud **runner** (a graph that runs with no machine on) is the one expensive node; this spec recommends deciding it last (§7).
- **Smallest useful first:** Drone as a job action with cancel and a timeout, then local event triggers and real actions, then secrets and polling (which already solve most of the five acceptance jobs without any cloud), then approvals, LAN, and finally the cloud nodes (§10).

## 1. Today

Measured on `main` @ `63848b092`.

**Drone** (`crates/srv/src/drone/`, `frontend/app/view/drone/`):

| Piece | What it does today |
|---|---|
| Blocks | Five kinds (`BlockKind`, `drone/types.rs:16`): Variables, Agent, API, Condition, Response |
| Agent block | Starts `claude --print --output-format=stream-json` (`blocks/agent.rs:96`), so **Claude only**; `task` templated, optional `agent_ref` (identity, memory, instance, directory) and `max_turns`; **no timeout**, waits for the final result; output `{response, tokens, cost_usd}` |
| API block | `method`, `url`, `headers`, `body`, `timeout_ms` (30 s); refuses non-http(s) and private, loopback and link-local addresses, also across redirects (`blocks/api.rs:73-147`); **no secret store**: auth comes from `{{env.AGENTMUX_DR_*}}` variables or literal headers |
| Condition | One binary operator (`==`, `!=`, `<`, `<=`, `>`, `>=`); two branches |
| Run | RPC `rundrone` (`server/drone_handlers.rs`), asynchronous; sequential topological layers; **no cancel, no timeout, no retry, no resume**; rows left `running` by a restart become `interrupted` (`backend/jobs/mod.rs:593-616`) |
| Data | `{{block_id.path}}`, `{{var.name}}`, `{{env.AGENTMUX_DR_*}}` (`drone/data_flow.rs`); **a run takes no input** beyond the drone id |
| Jobs | A manual run writes a ledger row (`drone_started`, `drone_finished`, `backend/jobs/mod.rs:530-562`) with the Agent blocks' summed `cost_usd` |
| Absent | A trigger, any approval, "message an agent", "spawn a worker", a task node, sub-drones, loops |

**What exists around it that a trigger or node can reuse:**

- *Events inside AgentMux:* the broker publishes `EVENT_AGENT_TURN`, `EVENT_AGENT_RUNTIME`, `AgentFailed`, `AgentShutdown`, `JobsChanged`, `CronChanged`, `DroneRun` (`backend/mps_events.rs`). There is no published "task claimed / completed" or "app upgraded" event; those are new publishers on the same broker.
- *Files:* `FsWatchPool` (`backend/fs_watch.rs`).
- *Timers:* the cron scheduler with catch-up (`backend/cron/`), the jobs watchdog (`backend/jobs/watchdog.rs`).
- *Loopback:* srv serves HTTP on loopback with its own auth (`server/routes.rs`); `RegisterDevServer` proxies a project's dev server (`backend/dev_proxy.rs`).
- *LAN:* peers are discovered and key-pinned (`backend/lan/discovery/`, events `laninstances`, `laninstances:health`); `POST /agentmux/remote/op` and `/agentmux/reactive/inject` cross hosts; `Tasks` and `Message` take a `host`.
- *Cloud:* MuxBus delivers frames to a local srv (`muxbus/cloud_subscriber.rs`: `InjectAvailable`, `JobEvent`). A `JobEvent` is signed by a pinned reporter and goes to the ledger, never to an agent.
- *Agent actions:* send a message (`Message` send; the reactive handler), spawn a worker (`POST /api/v1/agent/spawn`, at most `MAX_LIVE_WORKERS` = 12 live), file a Kanban task (`work_queue_enqueue`, the `kanban.act` path). **srv has no GitHub client**: every GitHub action today is an agent running `gh-agent`.
- *People:* a request waits for a person as a Needs-you card or a Vault request (`server/vault_ask.rs`); OS notifications (`backend/notify/`, `NotifyKind`). **Nothing in the repo's specs gives a durable approval that a phone can answer**; that is a dependency (§7.5), not an existing piece.
- *Secrets:* Vault, used by `Terminal run` `secrets` (`server/shell_secrets.rs`), which asks a person and keeps an "Always" grant per entry and agent. A non-agent caller cannot use it today.
- *Cost:* the turn ledger (`TurnLedger`, `backend/blockcontroller/health.rs`); only Claude reports a cost.

**Not built in the Jobs spec:** P4 (Drone as a job action), P5 (event triggers), P6 (a signed run request back, or a cloud runner). This spec is those three, made concrete.

## 2. Principles

1. **The graph never depends on a tier; the trigger does.** Any drone can be run by hand with a sample payload, on any machine, with no account. A trigger is the only thing that needs a tier, and the Jobs tab says which.
2. **Every cloud trigger has a local form,** or says plainly that it has none (§3.5). The local form usually costs the user something (polling instead of a push, a machine that must be on); the table says what.
3. **Cloud is for what a machine cannot do:** fire while it is off, receive from the internet, share one watcher across many users, reach a phone. Anything else stays local.
4. **No second scheduler, no second ledger.** The job's trigger fires; the ledger records; Drone executes.
5. **An event is a fact, not a command.** What it carries is data. It never becomes an agent's instruction (§6).
6. **Deterministic first, agent second.** A check that a node can do without an agent (an API call, a state query, a comparison) is done by a node. The agent block is for judgement, and its result is structured so the next node can branch on it. This also keeps cost down: the nightly triage in §9 spends agent tokens only on a failure.

## 3. Triggers and tiers

### 3.1 The Trigger node

A drone gets one **Trigger node**, at the left of the graph, like the Response node at the right. Its configuration *is* the job's `trigger` (Jobs spec §5.1), extended with the kinds below. Saving the node edits the job; a drone with no job has only the implicit `manual` trigger. A drone may have several jobs (a nightly one and a hand-fired one); the node lists them.

What the node gives the rest of the graph:

| Output | Meaning |
|---|---|
| `trigger.kind`, `trigger.at`, `trigger.scheduled_for`, `trigger.late` | Which trigger, when it happened, the slot it was for, and whether it fired late (a catch-up, or a cloud event held while the machine was off) |
| `trigger.event_id` | Unique; a repeat is ignored (§3.7) |
| `trigger.<field>` | The event's fields, as the trigger kind declares them. **Typed fields** (a number, an enum, a validated id) are clean; **free-text fields** (a title, a commit message, a log) are tainted (§6.2) |

The node shows a **tier badge** (Local, LAN, Cloud) and, for a cloud kind, what the local form is. **Test payload**: any trigger can be run with a sample payload from the node, so a graph built for a cloud event can be tried with no account.

### 3.2 Local triggers (no account)

| Kind | Fires when | Payload | Notes |
|---|---|---|---|
| `manual` | Run is pressed, or an agent calls `Drone` `fire` | Optional input | Exists. `fire` is new: an agent runs a drone it may run, with a payload (§6.5) |
| `timer` | A cron, `every` or `once` time, in a time zone | `scheduled_for`, `late` | The Jobs spec's timers, catch-up included. Cannot fire while this machine is off: the slots missed are one collapsed row (`app_not_running`) and, with catch-up `latest`, one late fire |
| `app_event` | Something inside AgentMux happened | By event, §3.2.1 | Filter by agent, seat, kind |
| `file_watch` | A path in a workspace changes (debounced; globs) | The changed paths, **names only** | Reuses `FsWatchPool`. File contents are never in the payload |
| `webhook_local` | A POST arrives on `http://127.0.0.1:<port>/agentmux/hook/<drone>` with the drone's secret in a header | The JSON body, size-capped | Loopback only, and **hardened against a web page**: any page in a browser pane can POST to loopback, so the secret travels only in a request header (`X-AgentMux-Hook-Secret`, never in the URL, query or body, so a cross-site form cannot send it); the body must be `Content-Type: application/json` (which a cross-site form cannot send without a preflight srv does not answer); a request carrying an `Origin` header or any `Sec-Fetch-*` header is refused (a script or `curl` sends neither); the secret is compared in constant time. For local scripts, git hooks, a CI agent on this machine. Reaching it from another machine is the LAN tier's job (§3.3) or the user's own tunnel |
| `poll` | A timer fires, a read-only check runs, and **something new** appears | One event per new item, keyed by the item's id | The local form of most cloud sources (§3.5). The check is an API node (with a Vault credential) or a state query. **Fan-out is in the trigger:** a poll that finds three new failed runs starts three runs, each with its own item key, and an item is never reported twice. No agent tokens are spent on polling |
| `expect` | An expected event did **not** happen by a time | The expectation and what was last seen | A dead man's switch ("a nightly report by 09:00, else fire"). Locally it checks the ledger and the broker; the cloud form (§7.4) keeps working while the machine is off |

Any trigger may carry `delay: <minutes>`: the fire is scheduled at `event time + delay` as an ordinary `scheduled` ledger row (Jobs spec §3.1), so a restart does not lose it and no run has to wait inside the executor.

#### 3.2.1 App events

Normalised names, with the payload's typed fields in brackets. The first four exist on the broker today; the rest are small new publishers on it.

| Event | Typed payload |
|---|---|
| `agent.turn_ended` (exists: `EVENT_AGENT_TURN`) | agent id, outcome (`completed`, `errored`, `interrupted`), cost |
| `agent.waiting` (from `EVENT_AGENT_RUNTIME`) | agent id, since |
| `agent.stopped`, `agent.crashed` (`AgentShutdown`, `AgentFailed`) | agent id, reason class |
| `job.finished` (`JobsChanged`, the ledger) | job id, run id, outcome. Chains one job after another without a second scheduler |
| `task.created`, `task.claimed`, `task.completed`, `task.failed` | task number, holder agent id, `links.pr` if any |
| `app.started` | version, `upgraded_from` if it changed |
| `cap.reached` | job or account, which cap |

### 3.3 LAN triggers

LAN peers are already discovered and pinned (`backend/lan/discovery/`, `/agentmux/remote/op`), so a LAN trigger needs no account and no new trust.

| Kind | Fires when | Notes |
|---|---|---|
| `peer_event` | A pinned peer asks this host to fire a drone (`Drone` `fire` with a `host`) | Signed with the peer's pinned key; the payload is untrusted like any other. **The receiving host applies its own turn-on check (§6.3) and its own caps:** a peer's signature proves who asked, not that the payload is clean, and a drone that is not turned on there does not run. This is how "the Linux host finished its nightly build" starts work on the Windows one |
| `peer_presence` | A peer comes online or goes away | From `laninstances`; payload is the peer's name and the change |
| `run_on` | Not a trigger: a property of the job. Run the graph on another host: by name, or by label (an operating system, a tag) | Reuses the remote-op path and the fleet host tags. The run appears in the Jobs tab on **both** hosts, linked (`links.followed_by_channel`, as for forwarded cron runs). A host with no match makes the run `skipped (target_missing)` |

Without LAN peers these kinds simply have nothing to fire; the graph still runs locally.

### 3.4 Cloud triggers (AgentMux Cloud account)

The cloud half is detailed in the private companion (§7). What a user sees:

| Kind | Fires when | What is cloud about it |
|---|---|---|
| `hosted_timer` | A schedule held by AgentMux Cloud fires | **Fires while this machine is off.** The fire is held and delivered when AgentMux reconnects, marked `late`; it is not lost, and the Jobs tab shows the miss. It does not run the graph on its own (that is the runner, §7.7) |
| `cloud_event` | An event arrives on a named **event channel** the account owns | A generic event source: anything that can send a signed HTTPS request, or that AgentMux Cloud receives on the user's behalf (below), puts an event on a channel; drones subscribe with a filter. The operator's event-bus idea; the public contract is AgentMux's own, so what runs underneath can change |
| `github_event` | A GitHub event the user subscribed to (a workflow run completed, a pull request merged, a release published, an issue labelled) | A typed source on `cloud_event`, using the GitHub App the user installs. Payloads are verified in the cloud before they are delivered |
| `public_webhook` | An outside service POSTs to the drone's own https address | A stable public address with a per-drone secret (HMAC-checked in the cloud). Without cloud, the user can use `webhook_local` behind their own tunnel |
| `feed` | A public feed changed (a package registry, a runtime's release feed, a status page) | **Shared watching:** one cloud poller serves everyone who subscribed to the same feed, so each user pays for a lookup, not for a poller. The local form is `poll` |
| `expect` (cloud form) | Same as §3.2, but the check runs in the cloud | Fires while the machine is off |

Further sources are added as typed sources on `cloud_event` (a store submission's status, a cloud provider's alarm or cost anomaly, an inbound email address per drone), each with its own review. They are listed in §8 and are not in the first build.

### 3.5 Every cloud trigger, and what a user without an account gets

| Cloud kind | Local form | What the local form cannot do |
|---|---|---|
| `hosted_timer` | `timer` (catch-up `latest`) | Fire while this machine is off; the slot is a late fire when it is back |
| `cloud_event` | `webhook_local`, or `poll` of the source's own API | Receive from the internet; push (it polls, so it is minutes late); share one watcher |
| `github_event` | `poll` of the GitHub API with a read-only token in Vault, one item per new run, PR or release | Instant delivery; costs the user's API rate limit instead of ours |
| `public_webhook` | `webhook_local` plus the user's own tunnel | A stable public address that is ours; while the machine is off, nothing is held |
| `feed` | `poll` of the feed | Sharing; each machine polls |
| `expect` (cloud) | `expect` (local) | Run while the machine is off |
| Approval by phone (§4.2) | A Needs-you card and an OS notification on this machine | Reach the user away from the machine |
| `once` across machines (§4.2) | `once` on this machine's store | Stop two machines doing the same work |
| Cloud runner (§7.7) | `run_on` another host that stays on, else the next time this machine is on | Run with every machine off |

**No graph is cloud-only by accident.** A cloud node that has no local form is called out in the node's own tooltip, and a drone that contains one cannot be turned on without an account, but can still be tested by hand.

### 3.6 Payload rules

- A payload is at most 64 KB, stored with the run (the Drone run's `trigger` field), and hashed into the ledger row (`payload_hash`, the column the Jobs spec reserves).
- Every trigger kind publishes a **payload schema** that marks each field typed-clean or tainted. The canvas uses it to autocomplete `{{trigger.…}}` and to colour tainted references.
- Free text is truncated at the trigger (a log to its last 200 lines, a body to 2,000 characters) before it reaches the graph. The full thing stays at its source, behind a link.

### 3.7 Delivery

- **At least once, deduplicated by `event_id`.** The ledger's unique index (Jobs spec §3.1) turns a repeat into nothing, including when two machines subscribe to the same cloud channel.
- **Held, not replayed without end.** A cloud event not delivered within 7 days is dropped and counted ("3 events dropped: too old"), like a bad job event is today. Local app events are never held: they happened while AgentMux was running.
- **Late is data.** `trigger.late` and `trigger.scheduled_for` let a graph say "this ran 4 hours late".
- **Catch-up is the job's policy** (`skip` or `latest`), as for timers. An event trigger's default is `latest` for a `poll` (the next poll sees what was missed anyway) and `skip` for everything else.
- **Per-job rate limit.** A trigger fires a job at most N times in an hour (the Jobs spec's `messages_per_day`-style cap, default 30 an hour), so a stuck webhook or a loop of `job.finished` cannot run away. Beyond it, fires are collapsed `skipped (rate)` rows and the first one raises the usual alert.

## 4. Nodes

### 4.1 Changes to the five blocks

| Block | Change |
|---|---|
| **Agent** | A **provider and model** (any provider the agent panes support, not only Claude) and an **account**; a **timeout** (default 15 minutes) and a per-node **cost cap** counted into the job's caps; **structured output**: the node declares a small schema (`{verdict: ok\|findings\|failed, cause: string, confidence: low\|medium\|high, …}`) and the result is validated against it, so the next node branches on a field instead of on prose; a **read-only profile**, enforced differently by where the node runs: *in a container where Docker answers*, no host writes (the workspace is mounted read-only) and network only to the declared hosts, which is a real sandbox; *natively*, only the provider CLI's own tool permissions (deny shell and edits, allow reads), which is **weaker**: it cannot restrict the network, and a steered model can use whatever the permissions allow. The turn-on prompt says which of the two the node gets, and the check in §6.3 treats a native read-only node as *reads outside*, not as safe. Anything the node may run is a named allowance, below; `allow_commands` lists the exact command lines it may run (for example one release-check script with a typed argument); each is **host execution**, shown in the turn-on prompt, counted as *writes outside* in §6.3, and given only typed-clean arguments; `agent_ref` may name an existing agent (its identity and memory) or leave it fresh. A cost is recorded only where the provider reports one (Jobs spec §5.3) |
| **API** | A **credential**: `auth: {vault: <entry>, as: bearer \| header:<Name> \| basic}`. The value is added by srv at call time, is never in the graph, a block's output, a log or the ledger, and needs the same "Always" grant a `Terminal run` secret needs. Response size cap; `timeout_ms`; retries on 5xx (at most 2). The loopback and private-address refusal stays; it is what keeps an event from steering a call at the user's own network |
| **Condition** | `&&`, `\|\|`, `!`, `contains`, `matches` (a regular expression, length-capped; the engine is linear-time, the Rust `regex` crate, so a hostile pattern or input cannot hang a run), `exists`, `in_set` (a value is one of a closed set: a known agent, a known repository, a seat). `in_set` is how a tainted string becomes clean (§6.2) |
| **Variables** | Template **filters** on any reference: `tail:N`, `head:N`, `json:path`, `lower`, `default:x`, `trunc:N`. Enough to cut a log to its last lines or pull a field out of an API body, without a code node |
| **Response** | Unchanged. Its output becomes the run's `summary` (300 characters) and `links` in the ledger |

### 4.2 New nodes

| Node | Does | Tier | Reads tainted | Uses a secret | Writes outside |
|---|---|---|---|---|---|
| **Trigger** (§3.1) | Starts the graph with the event | any | yes | no | no |
| **App query** | Reads AgentMux's own state, read-only: `agents` (running, waiting, stopped), `muxbus` (connected, signed in), `lan_peers` (proven, last seen), `tasks` (by holder, state, `links.pr`), `jobs` (recent runs). Deterministic and free | local | no | no | no |
| **Message** | Sends a message to an agent, through the normal path. The body is a template; every tainted value is inserted as quoted data (§6.2). Counts toward `messages_per_day` | local, LAN (`host`) | yes (quoted) | no | no (internal) |
| **Task** | Files a Kanban task for an agent or a seat. Resolves the **owning leader** the way a job failure does (Jobs spec §3.4). A `dedupe` key makes it "one task per incident": a second fire with the same key adds a line to the open task | local | yes (quoted) | no | no (internal) |
| **Spawn** | Starts a worker with a brief, through `POST /api/v1/agent/spawn`, under the worker limit and the leader's consent rules | local | yes (quoted) | no | no (internal) |
| **Notify** | A notification to the operator: fixed text plus quoted fields, a link that opens the run | local; phone with cloud | yes (quoted) | no | no |
| **Approval** | Stops the graph and asks a person (§6.4). Shows the proposed action and its inputs; has a timeout, and a timeout **rejects** | local card; cloud adds the phone | yes (shown) | no | no |
| **Once** | "Do this once for this key": the first caller gets through, later ones stop. Local form: this machine's store. Cloud form: across machines | local; cloud | no | no | no |
| **HTTP preset: GitHub** | Not a new kind: an API node with the URL, headers and response shaping filled for the common GitHub reads and writes (a run and its jobs, a job's log, re-run failed jobs, a PR, a comment, a merge) | local | yes (responses) | yes | the write presets |

Three deliberate omissions. **No shell or "run command" node:** host execution is what agents already do, and a node that runs text on the host is the first thing an injected event would aim at. **No loop and no sub-drone** in the first build: fan-out is in the trigger (§3.2), so each run handles one item; the canvas spec's Phase 2 kinds stay deferred. **No mid-graph wait** in the first build: `delay` on the trigger covers "30 minutes after", and a durable wait comes with the durable approval (§10, phase 5).

### 4.3 Run lifecycle

The executor gains what the Jobs spec's P4 already lists: **cancel** (Run now's Stop, the Jobs tab, and the job's timeout), a **run timeout** (the job's `timeout_minutes`; Agent nodes have their own), **retries** only for infrastructure failures (Jobs spec §5.1), **cost rollup** from every node that reports one (not only Agent blocks), and **resume of the ledger row** after a restart (a run that was running is `interrupted`, as now; a durable resume is the phase-5 work). `rundrone` keeps its name and takes an optional input, so `Drone` `fire`, the Run button and a trigger all start a run the same way.

## 5. How it relates to Jobs

| Question | Answer |
|---|---|
| Who owns the trigger? | The **job**. The Trigger node edits the job's `trigger`. Turning a drone on creates (or enables) a job with `action: {kind: "drone", drone_id, input}` |
| Who owns policy, caps, approval, owner, alerts? | The job (Jobs spec §5). A drone-level default is only a suggestion the job copies |
| Where is a run recorded? | One `db_job_runs` row (`job_source: "drone"`, `trigger: "event"` for event triggers, with `payload_hash`), linked to the `db_drone_runs` row for the per-node detail. The Jobs tab shows both, and "Open graph" on a drone job |
| What does a failed drone run do? | The same as any job: one incident, one Kanban task for the owning leader, one note, one notification (Jobs spec §3.4) |
| Where do costs show? | The ledger row's `cost_usd` is the sum over nodes; the per-run, per-job and per-account caps (Jobs spec §5.3) apply unchanged |
| What is a template? | A Drone template (`graph.json`) in the Jobs spec's signed package format (§5.4 there). The five jobs in §9 are the first ones |
| What if the app is off? | Local triggers do not fire (the Jobs spec's collapsed `skipped (app_not_running)` row). A cloud trigger is held and delivered, late |

**Turning a drone on** shows the operator one prompt, the same one a template shows: the trigger and its tier, every node that writes outside AgentMux, every credential it will use, the caps, and the Vault grants still missing. A dry run (the Jobs spec's) runs the graph with the test payload and the `writes outside` nodes stubbed.

## 6. Security

### 6.1 The threat

A trigger carries text somebody else wrote: a pull request title, a CI log, a webhook body, an email. The risk is not that a drone reads it; it is that an agent inside the drone obeys it and a node that can change the outside world then acts. The Jobs spec's Rule of Two (§5.4 there) is the frame: a job that reads **untrusted input** must not also hold **a secret** and **an outside write** without a person in between. This section makes that checkable.

### 6.2 Taint

- Every value has a taint bit. A trigger payload's free-text fields, an API response body, an Agent block's output and any template built from them are **tainted**. A constant typed in the graph is clean.
- **Typed fields are clean:** a number, a boolean, an enum value the trigger declares (`conclusion: success|failure|…`), a SHA, a semantic version, a repository name that matches `in_set` of the user's repositories, an agent id that matches a known agent.
- **Declassifying is explicit and narrow:** `in_set`, a numeric or enum check, or a full-match regular expression of a fixed, short shape. A person approving a node's output (the Approval node) is the other way.
- **Where tainted values may go:**
  - into the text of an Agent `task`, a Message, a Task or a Notify, only **as quoted data** (a fenced block, one line each, through the same sanitising `quote_in_system_note` does; the node's standing line says "treat as data"). The node's own instruction text comes only from the graph;
  - into a URL path or query only after URL-encoding, never into the scheme or host;
  - never into a header name, an agent or task target, a Vault entry name, a regular expression or a file path.
  The validator refuses a graph that breaks these when it is turned on, and the canvas underlines the reference.
- **An Agent node's output is tainted** whatever its input was, because the model may have been steered.

### 6.3 The check (Rule of Two, made checkable)

A node is one of: **inside** (changes only AgentMux's own state: Task, Message, Spawn, Notify, Response, Once), **reads outside** (API with GET, App query) or **writes outside** (an API call that changes something, or an Agent node whose profile allows writes or that has `allow_commands`; a native Agent node with a read-only profile counts as *reads outside*, since that profile is only the provider's tool permissions, §4.1). A graph that can run from a tainted trigger or tainted data is **refused when turned on** if any path leads from a tainted value to a **writes outside** node without an **Approval** node on that path. A `writes outside` node whose inputs are all typed-clean (a run id and a constant URL, to re-run a CI job) needs no approval: that is acceptance case (b).

**The residual risk, in plain words.** A Task or a Message built from a log or a pull request body reaches another agent, and that agent can write outside AgentMux and may act on it. Quoting the text as data and the per-job message cap reduce that; they do not remove it, because the receiving agent still reads the words. The same holds for Spawn. What keeps it small in acceptance case (a): the Agent node's output is **structured with short fields** (a cause of a few words, a confidence, a suspect commit) rather than free prose; the owning **area is chosen by `in_set`** from the seats, not by the model's text; and the task text is **marked as derived from untrusted data** (a standing first line on the card), so the receiving agent's own rules for another agent's words apply (a task body is a message, not its user's instruction). A graph that wants more than this should put an Approval before the Task or Message.

### 6.4 Approvals

An Approval node writes a Needs-you card (and, with cloud, a phone push). The card shows the proposed action in full, the inputs that came from tainted data marked as such, a timeout, and the run it belongs to.

Two separate things, kept apart:

**(a) An answer recorded as the person can only come from the person.** An answer given *as the person* is the person's voice, and an agent giving it would be putting words in the person's mouth, the one thing agents may not do. So an answer is recorded as the person only when it arrives from the app's own client over the person's session (the desktop card, or the phone app signed in as that person), never through MuxAPI. Every answer is recorded as who gave it, and the card shows it. This is the forging rule and it stays hard.

**(b) Who the approver is is a property of the node.** `approver: person` (the default) or `approver: <agent or seat>`. A named approver agent can answer through MuxAPI; its answer is recorded **as that agent**, never as the person. When it answers, the person gets a notification at once and a **veto window** (default 60 seconds, 15 seconds to 10 minutes) during which the write does not run; a veto cancels the run. An approver agent cannot be an Agent node of the same drone.

**What (b) costs in security terms.** Approval exists to put a reader who was not steered between untrusted text and a write. A person reading the card is that reader. An approver agent reads the same tainted inputs, as quoted data, and **can be steered by them**; the veto window leaves a person able to stop a bad approval only if they see the notice in time. So on a tainted-to-write path an agent approver turns "a human read this" into "an agent read this and a human could object". The check is still satisfied, more weakly, and the weakness is written into the turn-on prompt.

**Recommended default per case.** *Person*, always, for a write to a public repository, a merge, a deletion, anything that spends money, touches a credential or leaves AgentMux (an email, a post), and for the first run of any drone. An *approver agent with notice and veto* is reasonable for writes that are reversible and small (a comment on our own pull request, a re-run, a label, a draft) on a drone that has already run cleanly with a person approving. v1 ships `person`; (b) is designed here so it can be added without changing the node, and the operator decides when.

A timeout rejects. A rejected approval ends the run `outcome: failed, reason: approval_rejected` for a person's "no", and `approval_timeout` for silence; neither raises a new incident if the job already has one open.

### 6.5 Who may do what

| | Person (the app's own client) | An agent (MuxAPI) |
|---|---|---|
| Create or edit a drone | Yes | Yes |
| Turn a drone on (create its job) | Yes | Yes, subject to the validator (§6.3) and the prompt in §5 being shown to the person for a graph that has any `writes outside` node, any credential or any trigger from outside this machine. A graph of inside nodes and a local timer needs no prompt |
| Fire a drone with a payload (`Drone` `fire`) | Yes | Yes, for a drone it created, owns or has been given; the payload is tainted. The run is recorded as the agent |
| Answer an Approval *as the person* | Yes, from the app's own client | **No**: never through MuxAPI (§6.4 a) |
| Answer an Approval as a named approver | n/a | Yes, only if the node names it as `approver`; recorded as that agent, with a notice and a veto window to the person (§6.4 b) |
| Raise a cap, add a Vault grant | Yes | Per the Jobs spec §7 |

Container agents get every node and trigger, with `fire` and `turn on` through the same RPCs: no node executes on the host except the Agent node, which the host's srv starts exactly as it does today. Their browser, Vault and account isolation is unchanged.

### 6.6 Secrets

A credential is a Vault reference on an API node. It never appears in the graph JSON, a template, a payload, a node output, the ledger, a note or a log; srv adds it to the one outgoing request, and a response that echoes it is redacted before it is stored. The Agent node never receives a Vault value (an Agent that must call an API with a credential is replaced by an API node followed by an Agent node that reads the response). A missing grant is listed by the dry run, and an unattended run that reaches it waits for the person like any `Terminal run`.

## 7. Cloud nodes: what runs where, and who pays

The cloud half (the hosted timers, the event source, the runner, the relay's part, the cost shape, security at the edge) is the private companion's subject. The companion is written or reviewed by AgentA, and AgentY reviews its security section. This public spec states only the product contract.

### 7.1 What needs AgentMux Cloud

Only these, and nothing else in Drone does: `hosted_timer`, `cloud_event` (and its typed sources `github_event`, later others), `public_webhook`, `feed`, the cloud form of `expect`, the phone side of Approval, the cross-machine form of Once, and, if ever built, the cloud runner. Everything in §3.2, §3.3 and the nodes in §4 works with no account.

### 7.2 Cost shape (percentages and orders of magnitude; figures are in the companion)

- **Per fire and per event, never per idle second.** There is no always-on compute per user. A hosted timer costs a scheduled invocation each time it fires; an event costs one delivery and a short hold. A typical user's set of timers and events is a rounding error beside the agent tokens their jobs spend.
- **Fan-out is the lever.** A shared `feed` is one lookup for any number of subscribers; a per-user poller would multiply it. That is why `feed` is a cloud node and a per-user timer is not.
- **The runner is the only node that adds compute time per run** and the only one that needs a dollar cap of its own. It is also the only one that would hold a provider credential for the user, which is why it is last (§7.7).
- Abuse is the other cost: a public webhook is open to the internet. A per-drone secret, a body-size cap, a rate limit and an auto-disable on a flood are part of the node, not an extra.

### 7.3 Limits and what is free (proposed; AgentA to confirm against the cost shape)

| | Proposed |
|---|---|
| Everything local and LAN | Free, no limit but the job caps |
| Hosted timers per account | 10; minimum interval 5 minutes |
| Event channels per account | 5; events per account per day 1,000; payload 64 KB; held 7 days |
| Public webhook | One address per drone; 60 requests an hour per drone |
| Shared feeds | A curated list first (our own releases, the runtimes we depend on); a user-added feed counts as an event channel |
| Approval pushes | 50 a day per account |
| Runner | Not offered until it has its own spec |

These are product limits, not prices. Whether any of it is paid is the operator's call; the spec's recommendation is that they are part of an AgentMux Cloud account at no extra charge until the cost data says otherwise.

### 7.4 Which cloud nodes to keep (the candidate list, with reasons)

| Candidate | Verdict | Why |
|---|---|---|
| GitHub events | **Keep, first** | The operator's own example, and four of the five acceptance jobs start from one. Local form exists, so it is an upgrade, not a gate |
| Hosted timer | **Keep, first** | The operator asked for it; the only way a schedule survives a closed laptop |
| Event channel (the event-bus idea) | **Keep, first** | The base the others sit on; one contract, many sources |
| Public webhook per drone | **Keep** | Cheap once the event channel exists; the only way to receive from a service that cannot reach a laptop |
| Durable approval from the phone | **Keep** | Turns every unattended write into something that can be allowed from a bus stop; needs the mobile app's push path (§7.5) |
| Shared state or lock (`once`) | **Keep, small** | Two machines on one account will otherwise do a job twice; local form is already in the ledger |
| Dead man's switch (`expect`) | **Keep** | This is the failure that started the spec: a nightly report that failed without anyone seeing it. Silence is the event |
| Release feeds | **Keep, shared** | Highest fan-out saving; drives the "a dependency released" task |
| Email-in | **Defer** | Needs address handling, spam control and a mail provider; no local form; revisit with a user ask |
| Store submission status | **Defer** | Narrow; a typed source on the event channel when someone needs it |
| A cloud provider's alarms and cost anomalies | **Defer** | A typed source; needs onboarding the user's account, which is a security review of its own |
| Cloud runner | **Decide last** | §7.7 |

### 7.5 Dependency: the phone

The mobile app today lets a person see agents and send to one (`agentmux-mobile`: `SPEC_SEND_FROM_THE_PHONE_2026_10_13.md`). This spec found no push-notification path in the repository's specs. A phone-answerable Approval needs one: a push that opens the card, and an answer that goes back over MuxBus signed by the phone. That is the mobile lane (QTest, with AgentX's input) and the relay's (AgentY); until it exists, Approval is a desktop card and an OS notification, and the phone form is marked "when push lands".

### 7.6 Without an account

Nothing in the first four phases needs one. A user with no account gets: timers, app events, file watches, a local webhook, polling (with their own token in Vault), LAN triggers and `run_on`, every node, approvals on the desktop, and the whole Jobs tab. What they do not get is in the table in §3.5, one row each.

### 7.7 The cloud runner (recommendation: decide last)

A graph that runs with every machine off needs AgentMux's headless server running in the cloud, with the user's provider credentials and a sandbox. It is the largest cost, the largest security surface (unattended agents in our infrastructure, holding the user's secrets) and the one with the least independent demand: a user who wants "always on" can run `run_on` against a machine that is always on. Recommendation: ship the hosted timer, the event channel and `late` delivery first, measure how often "my machine was off" is the real reason a job did not run, and write the runner's own spec only if that number is high.

## 8. More ideas like that

Each is a trigger or a node on what this spec already defines; none needs new machinery.

| Idea | Built from | Tier |
|---|---|---|
| A dependency released (a runtime security update, a crate or package major) → file one task for the owning seat | `feed` / `poll` + Task with a dedupe key | local or cloud |
| An agent has waited on a person for more than N minutes → nudge the person on their phone | `agent.waiting` + `delay` + Notify | local; phone with cloud |
| A pull request approved but not merged for an hour → message its author | `github_event` / `poll` + `delay` + Message | local or cloud |
| Daily digest: what merged, what is red, what each agent cost | `timer` + API + App query + Agent (one structured call) + Notify | local |
| A job cap reached → tell the owner and offer to raise it | `cap.reached` + Approval | local |
| The nightly did not run at all | `expect` | local or cloud |
| An agent's account hit its limit → say which jobs it will starve | `agent.stopped` (reason class) + App query (`jobs`) + Message | local |
| New mail or a calendar event starts in ten minutes → brief the right agent | The Gmail and Calendar connectors as polled sources (`poll`) | local |
| A file in a workspace changed → run the project's checks and tell the owner | `file_watch` + Agent + Message | local |
| Backup verify failed or is late | `expect` + Notify | local or cloud |

## 9. The five acceptance drones

Each was a real job done by hand on 2026-11-11. Each is shown with its trigger, nodes and tier, the local form (what a user with no account would get), and the check from §6.3. "Typed" means every value that reaches a write is clean.

### (a) Triage a failed nightly

- **Trigger:** `github_event` `workflow_run.completed`, filter `event = schedule`, `conclusion = failure`, workflow in the repository set (cloud). **Local form:** `poll` every 15 minutes on the repository's workflow-runs list with a read-only token in Vault; one run per new failed scheduled run, keyed by run id. Cost: up to 15 minutes later, and the user's API quota.
- **Nodes:** Trigger → API (run and its failed job) → API (that job's log, `tail:200`) → Agent (read-only profile, structured: `cause`, `suspect_commit`, `area`, `confidence`) → Condition (`confidence != low` and `area in_set seats`) → Task (to the seat that owns `area`, dedupe key `workflow + first_failing_step`, so repeated red nights are one task with a line per night) → Message (the owning leader, one line).
- **Check:** the log is tainted and goes to an Agent, whose output is tainted; the only things written are a Task and a Message (both inside). `area` is declassified by `in_set`. No approval needed. No secret reaches the Agent (the token is on the two API nodes).
- **Without cloud:** works (the poll). **Cost shape:** polling spends no tokens; the Agent runs only on a failure.

### (b) Re-run a flaky job once, and tell the PR's author

- **Trigger:** `github_event` `workflow_run.completed`, `conclusion = failure`, `run_attempt = 1`, job name in a flaky list (cloud). **Local form:** `poll` the open pull requests' latest runs.
- **Nodes:** Trigger → Once (key `run_id`) → API preset *re-run failed jobs* (typed: `run_id` is a number, the URL is constant) → API (the PR's body) → Condition (`agent_id` extracted by a full-match regular expression, then `in_set` known agents) → Message (that agent: "your run #N was re-run once").
- **Check:** the only write outside is the re-run, and its inputs are typed-clean, so no approval. The PR body is tainted; the agent id is declassified by `in_set`. `Once` stops a second fire, from this machine or another.
- **Without cloud:** works. Two machines on one account may both re-run until the cloud `Once` exists; the local `Once` covers one machine.

### (c) Verify a release at tag + 2 h 30 min

- **Trigger:** `github_event` `release.published` or a tag push (cloud), with `delay: 150`. **Local form:** `poll` the releases list, `delay: 150`.
- **Nodes:** Trigger → API (the release and its assets) → Condition (`published`) → Agent *verify* (its only input is the tag, a typed semantic version; `allow_commands`: the repository's one release-check script, argument `tag`; structured: `manifest_ok`, `notes`) → Condition (`manifest_ok`) → Message (the release leader) → Approval ("merge these held PRs: #…, #…", the list typed from an API call) → API (merge each, one node per PR from a fixed list).
- **Check:** running the script is host execution, so it is a named allowance (§4.1) and the Agent node gets **no API response text**, only the tag; tainted data never reaches a node that can execute. The merge is a write outside; the PR numbers come from an API response and are numbers (typed). The Approval puts a person on the write anyway, since "merge" is the kind of action the operator wants to see.
- **Without cloud:** works, with the approval as a desktop card. With cloud, the same approval can be answered from the phone.

### (d) A merged PR whose Kanban task is still claimed after 30 minutes

- **Trigger:** `github_event` `pull_request.closed` with `merged` (cloud), `delay: 30`. **Local form:** `poll` the repository's recently closed pull requests, `delay: 30`, one run per merged PR.
- **Nodes:** Trigger → App query (`tasks`, `links.pr = trigger.url`, state `claimed`) → Condition (found) → Message (the holder: "PR merged 30 minutes ago; complete #N or tell me why not").
- **Check:** no outside write, no secret reaches an agent. A holder agent id comes from the app's own store, not the event.
- **Without cloud:** works. This is the drone that catches an agent left idle on work that is already done.

### (e) After an upgrade: the steward's health check, reported to the leader

- **Trigger:** `app_event` `app.started` with `upgraded_from` set (local); nothing to fall back to, because nothing cloud is involved.
- **Nodes:** Trigger → App query (`agents`) → App query (`muxbus`) → App query (`lan_peers`) → Condition (all running, connected, proven) → Message (the leader: "0.60.24 is up: N agents resumed, MuxBus connected, N peers proven") ; on the false branch → Agent (the steward, with the failing facts quoted) → Task (to the steward) + Notify.
- **Check:** every value is the app's own state; nothing is tainted. The Agent branch runs only when something is wrong.
- **LAN:** each host has its own copy; `run_on` is not needed. A fleet-wide version is the same graph with `run_on` set to each host's label.

## 10. Phasing

Each phase is shippable and useful alone, smallest first. Numbers in brackets are the Jobs spec's phases.

| # | Phase | Delivers | Needs |
|---|---|---|---|
| 1 | **Drone as a job action** [P4] | `action.kind: "drone"` accepted; the Trigger node (manual, timer); run input; cancel; run and Agent timeouts; retries; cost rollup from every node; the Jobs tab's "Open graph"; `Drone` `fire` for agents | Jobs P1–P3 (built) |
| 2 | **Local events and real actions** [P5, local] | `app_event` (and the new publishers), `file_watch`, `webhook_local`, `delay`, the rate limit; App query, Message, Task, Spawn, Notify; structured Agent output, provider and account choice; richer Condition and filters. **Acceptance (e).** | 1 |
| 3 | **Secrets and polling** | API credentials from Vault; the GitHub preset; the `poll` trigger with fan-out; `expect` (local); the payload schemas and the tainted-reference underline. **Acceptance (a), (b) and (d)** in their local forms, with no cloud | 2 |
| 4 | **Approval and the check** | The Approval node (a desktop card), the taint rules and the §6.3 validator, the turn-on prompt, Once (local). **Acceptance (c)** | 3 |
| 5 | **Durable waits** | A run can wait (an approval for hours, a `Wait` node): the run's state is persisted and resumed after a restart | 4 |
| 6 | **LAN** [P5, LAN] | `run_on` (by name, by label), `peer_event`, `peer_presence`, the linked run on both hosts | 2 |
| 7 | **Cloud, first** | `hosted_timer`, `cloud_event` and the channel contract, `github_event`, the delivery of held events as `late`, the per-account limits | 3; the companion; AgentA and AgentY |
| 8 | **Cloud, rest** | `public_webhook`, `feed`, `expect` (cloud), `Once` (cloud), the phone side of Approval | 7; the mobile push path (§7.5) |
| 9 | **Runner** [P6] | Not specified. Decide after phase 8 on measured demand (§7.7) | its own spec |

Phases 1 to 4 stand on what is already built. Phase 6 does not depend on 3 to 5. Phase 7 is the first that touches AgentMux Cloud.

## 11. Decisions, with a recommendation each

| # | Decision | Recommendation |
|---|---|---|
| D1 | Where the trigger lives | On the job; the Trigger node edits it. Not in the graph, which would be a second scheduler (§3.1, §5) |
| D2 | Tainted data policy | Taint bit with typed-clean fields and `in_set` declassification; quoted data only into Agent, Message, Task, Notify; never into hosts, targets or paths (§6.2) |
| D3 | When an approval is required | Only for a path from tainted data to a write outside; typed-clean writes (re-run a CI job) need none (§6.3) |
| D4 | Polling as the local form of cloud sources | Yes, with fan-out in the trigger and no loop node (§3.2) |
| D5 | A shell node | No (§4.2) |
| D6 | Who may answer an Approval | Two rules (§6.4): an answer recorded *as the person* only from the person's own client (the forging rule, hard); and `approver` as a node property, `person` by default or a named agent whose answer is recorded as that agent, with a notice and a veto window. v1 ships `person`; the agent approver is designed, with its security cost stated, for the operator to turn on per case. Recommended: person for public-repo writes, merges, deletions, money, credentials and anything that leaves AgentMux; an agent approver only for small reversible writes on a drone that has run cleanly |
| D7 | Agent-created drone jobs | Allowed. Turning one on shows a prompt (the trigger, the nodes that write outside, the credentials, the caps) for a graph that writes outside, holds a credential or triggers from outside this machine. Same split as D6: the prompt is the person's to answer *as the person*; a leader agent may be named to answer it as itself, with a notice and a veto window. Recommended: person for the first turn-on of any drone with a credential or an outside write; later edits by the same owner need none unless they add one. This replaces the Jobs spec §7 line "`drone` never" |
| D8 | Cloud runner | Decide last, on measured demand (§7.7) |
| D9 | Cloud limits and price | The table in §7.3 as a starting proposal, free with an account until the cost data says otherwise; AgentA's figures decide |
| D10 | The phone | Wait for the mobile push path; Approval is a desktop card until then (§7.5) |
| D11 | Event retention | Held cloud events 7 days; payload in a run 64 KB; the ledger keeps a hash (§3.6, §3.7) |

## 12. Tests (for the build specs)

- Trigger: a timer, an app event, a file change and a local webhook each start one run with the payload; a repeat `event_id` starts none; two machines on one store start one; a `delay` survives a restart; the rate limit collapses a flood.
- Poll: three new items start three runs; the same item is never started twice; an empty poll costs no tokens.
- Taint: a tainted value in a URL host, a header name or an agent target is refused when the drone is turned on; the same value as quoted data in a Message is accepted; `in_set` clears it; an Agent's output is tainted.
- Rule of Two: a path from a log to a PR comment with no Approval is refused; with one it is accepted; a re-run with a typed id needs none.
- Approval: an agent's answer is refused; a timeout rejects; the card shows the tainted inputs marked.
- API credential: the value is not in the node output, the stored run, the ledger or a log; a response that echoes it is redacted; a missing grant is listed in the dry run.
- Ledger: an event-triggered drone run writes one row with `trigger: event`, `payload_hash` and the summed cost; a failure opens one incident.
- Without an account: each cloud trigger shows its local form; a graph with a cloud-only node refuses to turn on and still runs with a test payload.
- Container agents can create, fire and turn on a drone; none of the new nodes executes on the host.
- The five acceptance drones are the live tests: each is built, then run against a recorded payload (a, b, c, d) or a real upgrade (e) in an isolated `task dev` with `AGENTMUX_HOME_OVERRIDE`.
