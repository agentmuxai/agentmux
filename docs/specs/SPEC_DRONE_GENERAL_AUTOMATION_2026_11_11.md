# SPEC: Drone as a general automation tool: loops, state, sub-drones, a Run step, durable waits

**Date:** 2026-11-11
**Status:** active (shipped: the real-world spec's phase 1a server half, a job runs a drone, in PR #7018; the interfaces of the engine's modules A, B and C with their acceptance tests, in PR #47450's branch; remaining: the engine itself, task #47445, and the other phases in section 5). Approved 2026-11-11. The operator answered the decisions table (G4, G7, G10, G13, G16, G17, recommendations as presented) with: "great, who's scheduled to build it?", relayed by AgentY, who read it as approval. Reviewed by AgentX (product half) and AgentY (security half). **An amendment:** it changes the approved `SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` (the "real-world jobs spec") where it says so, and cites it everywhere else. It is a second document, not an edit of the first, because the first is approved and ships in the first build PR as written; this one is reviewed on its own and ships in the engine PR that builds it.
**Trigger:** The operator, 2026-11-11, in three messages, relayed by AgentY: "how realistic do you think it would be to organize all our drone-like, tasks. It would normally only be for tasks that involve an agent, although users could use it for anything." / "why cant we make it a general automation tool?" / "we want it as a generalized tool including loops so users have design sophistiacted setups. reach parity with the industry leader now, which I believe are features built by langchain".
**Related:** `SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` (triggers, tiers, nodes, taint and the Rule of Two check, phases 1 to 8); `SPEC_JOBS_AND_TEMPLATES_2026_10_24.md` (the ledger, caps, jobs); `SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md` (the canvas; its Phase 2 kinds, Function, Loop, Parallel, Router and Subdrone, were deferred and are settled here).

## 0. Summary

- **What changes in the approved spec.** Decision D5 ("no shell node") and the three omissions in §4.2 of the real-world jobs spec (no loop or sub-drone, no mid-graph wait, no run-command node) are reversed. Drone is a general automation tool; agent work is what it is best at, not all it does. Everything else in that spec stands: the trigger belongs to the job, the graph never depends on a tier, events are data, `person` approves.
- **The engine is replaced; the blocks are not.** Today's executor walks acyclic layers over an in-memory scope and checkpoints nothing (`drone/executor/engine.rs`). The new one is a **step scheduler over a checkpointed run state**. **A rule that holds for every phase: a block keeps the signature `run(node, scope) -> Result<Value, String>` and the `{{block.path}}`, `{{var.name}}` and `{{trigger.*}}` references,** so nodes written before the engine lands survive it.
- **Eight additions** (§3): **loops** (guarded cycles and a bounded for-each, caps counted across iterations, a visible reason when a cap ends a run), **state with reducers**, **checkpoint, resume and replay**, **sub-drones**, **parallel branches and joins**, a **Run step** (a fixed command line), and an allowlisted **Tool step** (an MCP tool called with no agent turn), and a **Transform** node to shape lists and objects with no code.
- **Security holds for each, not by exception** (§4). Taint is a property of state, so a loop cannot launder it and a sub-drone's output carries it. The turn-on validator either decides or refuses. A Run step takes only typed-clean arguments, runs in a container by default, and is host execution only as a named allowance shown at turn-on.
- **Where we stand against the leader** (§2): the leader for agent-graph runtimes by every measure we could read is LangChain's LangGraph, with its deployment platform and tracing product; for plain automation, n8n. We match LangGraph's runtime features with these seven additions, are ahead on five things it does not have (§2.3), and deliberately do not copy seven (§2.4).
- **Three leader features are NOT planned in this amendment:** long-term memory with semantic search, a node-level result cache, and an evaluation platform. They are named so that the operator accepts or rejects the gap knowingly (decision G13, with a recommendation and a cheap first step for each). Everything else in the parity table is either built, in the approved spec, or in a phase below.
- **Shortest order to loops and durable state:** phase 1a, then the engine, then sub-drones, parallel branches and the Run step (§5). Phases 2 and 3 do not depend on the engine and run beside it.

## 1. What this reverses, and what it keeps

| Approved spec said | Now |
|---|---|
| D5: no shell or run-command node | A **Run step** (§3.6), argv only, typed-clean arguments, container by default, host as a named allowance |
| §4.2: no loop and no sub-drone in the first build | **Loops** (§3.1) and **sub-drones** (§3.5), after the engine |
| §4.2: no mid-graph wait in the first build; durable waits in phase 5 | **Wait and durable approval move into the engine phase** (§3.3), because they are what checkpointing is for. Phase 5 is cancelled and absorbed |
| §4.3: cancel, timeouts and retries in phase 1 | They move to the engine phase (§5); a thin version before the engine would be thrown away |
| §6.5: an agent can turn a drone on, with a prompt for outside writes | Unchanged. The **Run step in host mode is the one thing a container agent cannot create** (§4.4), because it executes on the host, the single exception the standing rule allows |
| D1 to D4, D6 to D11 | Unchanged |

Kept without change: the Trigger node and the three tiers (§3 of the real-world jobs spec); the taint rules (§6.2 there); the Rule of Two check (§6.3); approvals (§6.4: the answer *as the person* only from the person's own client; `approver: person` in v1); the Jobs ledger as the only ledger; no new "operator only" gate. The one hard boundary stays forging the operator's voice.

## 2. Parity with the leader

**Method and honesty note.** Every row was read on **2026-11-11** from the public documentation, the GitHub API and the PyPI and npm download APIs, not from memory. The pages were fetched through a tool that returns a short machine-written summary of each page, and none of the pages showed a "last updated" date, so each row's date is the date it was read. Rows marked (S) rest on a search-result summary only; they are the weakest. LangChain's product names have changed and are used here as the docs use them today: the hosted platform is **LangSmith Deployment** (it was LangGraph Platform), its API is the **Agent Server**, the studio is **LangSmith Studio**, and the no-code agent builder is **LangSmith Fleet** (it was Agent Builder).

### 2.1 Is LangChain the leader? (checked, not assumed)

| Measure (read 2026-11-11) | LangGraph | Next best |
|---|---|---|
| Monthly downloads, PyPI | **44.8 million** | OpenAI Agents SDK 12.4 M; Google ADK 10.1 M; Pydantic AI 5.4 M; CrewAI 2.4 M; Microsoft Agent Framework 0.19 M |
| Monthly downloads, npm | 17.8 million (`@langchain/langgraph`) | Mastra 11.1 M (`@mastra/core`) |
| GitHub stars | 44.8 k | AutoGen 61.8 k and CrewAI 60.8 k are higher |
| Latest release | 1.2.20, 2026-11-05, MIT | |

**Verdict:** by usage LangGraph is the leader of agent-graph runtimes, as the operator believed; by GitHub stars it is third. Temporal (38 M PyPI downloads a month) is the leader of *durable execution*, which is not an agent framework. For plain workflow automation that people self-host, **n8n** leads by a wide margin (210 k stars, release 2.47.4 on 2026-11-11, licence fair-code, see §2.4). Sources: the GitHub REST API for each repository; `pypistats.org/api/packages/<name>/recent`; `api.npmjs.org/downloads/point/last-month/<name>`. Download counts include CI and mirror traffic.

### 2.2 The parity table

"Leader" is LangGraph and its platform unless a row says n8n. **Us** is what Drone has on 2026-11-11 / after the real-world jobs spec ("RW") / after this amendment. **Lands in** uses §5's phase names. Sources are tagged `[lg:…]` (docs.langchain.com/oss/python/langgraph/…), `[ls:…]` (docs.langchain.com/langsmith/…) and `[n8n:…]` (docs.n8n.io/…); the full list is at the end of the section.

| # | Capability | Leader (what the docs say) | Us | Lands in |
|---|---|---|---|---|
| 1 | **Cycles and loops** | A conditional edge routes back to an earlier node; `recursion_limit`, default 10,007 super-steps (the constant `DEFAULT_RECURSION_LIMIT` in `langgraph/_internal/_config.py`, read from the library source on 2026-11-11), ends the run with `GraphRecursionError`; `RemainingSteps` lets a node finish early `[lg:graph-api-control-flow]`. n8n: *Loop Over Items* `[n8n:flow-logic/looping]` | **Missing:** the engine refuses any cycle | **E1**: guarded cycles (default 25, ceiling 1,000), for-each, caps in dollars and minutes as well as steps |
| 2 | **Shared state and reducers** | Typed state; default last-write-wins; a reducer per key (`Annotated[list, operator.add]`); concurrent writes without one are unordered `[lg:graph-api]` | **Partly:** `vars` and node outputs, last-write-wins, no reducers | **E1**: `replace`, `append`, `merge`, `add`; concurrent `replace` writers refused at turn-on |
| 3 | **Conditional edges and routing** | `add_conditional_edges`, `Command(update, goto)` `[lg:graph-api]` | **Have:** Condition with two branches; richer expressions in phase 2 | Phase 2 (expressions); multi-way `switch` on Condition in E1 |
| 4 | **Parallel branches, map-reduce, joins** | Same super-step runs concurrently; `Send` fans out one worker per item; `add_edge([b,c],d)` joins; `defer=True` `[lg:graph-api-control-flow]` | **Missing:** sequential layers | For-each with `concurrency` in **E1**; branches and Join in **E3** |
| 5 | **Sub-graphs** | A compiled graph is a node; own schema with a mapping wrapper `[lg:use-subgraphs]`. n8n: *Execute Sub-workflow* `[n8n:flow-logic/subworkflows]` | **Missing** | **E3**: the Drone node, typed inputs and outputs, inherited caps |
| 6 | **Human-in-the-loop** | `interrupt()` pauses, `Command(resume=…)` continues; approve, reject, edit; needs a checkpointer; **the interrupted node restarts from its start on resume** `[lg:interrupts]`. n8n: *Send and Wait for Response* on Slack, Gmail, Teams and others `[n8n:…/slack/approvals]` (S) | **RW:** the Approval node, a desktop card, `person` approver (phase 4) | Needs E2; Approval in **phase 4** |
| 7 | **Durable execution and checkpoints** | Checkpointers (in-memory, SQLite, Postgres, others); `durability` of `exit`, `async` or `sync`; nodes that finished in a failed step are not re-run; resume with `invoke(None, config)` `[lg:persistence]` `[lg:checkpointers]` | **Missing:** a restart marks the run `interrupted` | **E2**: a checkpoint after every node; resume; `needs_attention` for an effect in flight |
| 8 | **Replay and time travel** | `get_state_history`; replay from a checkpoint **re-executes nodes** (LLM and API calls fire again); `update_state` forks `[lg:use-time-travel]`; Studio: edit state, fork, re-run from here `[ls:studio]` | **Missing** | **E5**: replay forks a new run; **effects stubbed by default** |
| 9 | **Streaming of intermediate steps** | Modes `values`, `updates`, `messages`, `custom`, `checkpoints`, `tasks`, `debug` `[lg:streaming]` | **Have:** the per-run event stream | E2 adds `StepCheckpointed`, `LoopIteration`, `Waiting`, `Resumed`, `CapReached` |
| 10 | **Retries, error handling, timeouts, caching** | `RetryPolicy` (backoff, jitter, `retry_on`), `TimeoutPolicy`, `error_handler`, `CachePolicy`, graph-wide `set_node_defaults` (langgraph 1.2+) `[lg:fault-tolerance]`. n8n: *Retry On Fail*, error workflows, *Continue using error output* `[n8n:flow-logic/error-handling]` | **Missing** (the Jobs spec retries whole runs for infrastructure failures only) | **E2**: node retries (infrastructure only), timeouts, error edges. **Node caching: not planned** (G13) |
| 11 | **Memory across runs** | Checkpointer plus `thread_id` for a thread; a `Store` with namespaces and optional semantic search for long-term memory `[lg:add-memory]` | **Partly:** agents have Memory; a drone has none of its own | The **Tool** step (E4) can read and write the allowlisted `Memory` tool; no new store (G13) |
| 12 | **Tool and model nodes** | `ToolNode` runs tool calls in parallel and accepts direct tool-call dictionaries (no LLM turn); the prebuilt agent moved to LangChain's `create_agent` `[lg:…ToolNode]` | **Partly:** an Agent block, **Claude only**; an API block | Provider choice (phase 2); the **Tool** step (E4) |
| 13 | **Scheduled and webhook runs** | Cron jobs, **UTC only**; webhooks are **outbound only** ("don't trigger runs"); Fleet starts agents from Gmail, Slack and Teams `[ls:cron-jobs]` `[ls:use-webhooks]` `[ls:fleet/channels]`. n8n: schedule, webhook (Basic, Header, JWT auth), polling triggers `[n8n:…/webhook]` | **Ahead after RW:** timers with a time zone and catch-up, app events, file watch, an inbound loopback webhook, polling with fan-out, LAN and (later) cloud triggers | RW phases 1a to 8 |
| 14 | **Versions of a graph** | *Assistants*: a graph plus a configuration, each edit a new version, rollback with a "latest" pointer; **Deployment only, not in the open-source library** `[ls:configuration-cloud]`. n8n: history 24 h to unlimited by plan `[n8n:workflows/history]` | **Missing:** a drone is edited in place | **E5**: numbered versions, jobs pin one, running runs keep theirs |
| 15 | **Run history** | Threads and runs; Studio's history `[ls:agent-server]`. n8n: execution list, copy into editor, retry with current or original workflow `[n8n:workflows/executions]` | **Have:** the Jobs ledger (kept forever) and `db_drone_runs` | E5 adds the replay view |
| 16 | **Traces and cost** | Every step recorded with inputs, outputs, tokens, cost, latency; retention 14 or 180 days by tier `[ls:observability-concepts]` `[ls:billing]` | **Partly:** per-node states and cost for Claude; kept forever, locally | E1 and E2 add per-iteration cost; the run overlay |
| 17 | **Evaluation** | Datasets, offline and online evals, LLM-as-judge, pairwise, regression comparison, alert rules `[ls:evaluation-concepts]` `[ls:rules]` (S) | **Missing** | **Not now.** A cheap form later: "run this drone against a saved set of recorded trigger payloads" (replay plus the test payload), decision G13 |
| 18 | **No-code builder** | LangSmith Fleet: templates, "Build with AI", Gmail, Slack, Calendar and GitHub tools, remote MCP, sub-agents `[ls:agent-builder]` `[ls:fleet/essentials]` | **Have:** the canvas, and templates (Jobs spec) | "Build with AI" (an agent drafts a drone and the validator checks it): later, G14 |
| 19 | **Concurrent inputs to one run** ("double texting") | `multitask_strategy` of `enqueue`, `reject`, `interrupt`, `rollback`; **Agent Server only** `[ls:double-texting]` | **Have after RW:** the Jobs spec's overlap policy `skip`, `buffer_one`, `cancel_other` | Jobs spec P3 (built) |
| 20 | **Budgets** | A step limit; call-count limits as LangChain middleware; **no cost or time budget found** `[lg:graph-api-control-flow]` `[lc:middleware/built-in]` | **Ahead:** caps in dollars per run, per job per day and per account per day, enforced mid-run | Jobs spec P3 (built); E1 across iterations |
| 21 | **Untrusted input** | Thin: an Engine page says trace data can carry adversarial content, so its suggestions go through pull-request review; no general guide for Deployment or Fleet `[ls:engine-security]`. n8n: *Execute Command* is **disabled by default since n8n 2.0** and absent on Cloud `[n8n:hosting/securing/blocking-nodes]` | **Ahead:** taint bits, typed-clean fields, the Rule of Two check at turn-on, approvals as a person's voice | RW §6; §4 here |
| 22 | **A shell step** | n8n *Execute Command*: off by default, no sandboxing documented `[n8n:…/executecommand]` | **Missing** | **E4**: the Run step, argv only, container by default |
| 23 | **MCP and agent protocols** | The Agent Server exposes agents at `/mcp` and at an A2A endpoint (v1.0, without push notifications) `[ls:server-mcp]` `[ls:server-a2a]` | **Have:** MuxAPI is MCP; `Drone` `fire` makes a drone callable by agents | Phase 1a. **A2A: not planned** |
| 24 | **Auth and secrets** | `@auth` handlers; secrets as environment variables or workspace references `[ls:auth]`. n8n: credentials encrypted in the database `[n8n:hosting/…/encryption-key]` | **Ahead:** Vault references and per-entry "Always" grants; values never in a graph, state, output or ledger | RW phase 3 |
| 25 | **Deployment** | Cloud, hybrid, self-hosted (Enterprise), BYOC; priced by seat, trace and compute `[ls:platform-setup]` `[ls:billing]`. n8n: queue mode with workers, fair-code licence | **Different:** local-first, tiers local, LAN and cloud; no metering of runs | RW |

Sources (all read 2026-11-11): LangGraph docs under `https://docs.langchain.com/oss/python/langgraph/` (`graph-api`, `graph-api-control-flow`, `graph-api-nodes`, `use-subgraphs`, `interrupts`, `persistence`, `checkpointers`, `use-time-travel`, `streaming`, `fault-tolerance`, `add-memory`, `functional-api`); LangSmith docs under `https://docs.langchain.com/langsmith/` (`agent-server`, `cron-jobs`, `use-webhooks`, `double-texting`, `configuration-cloud`, `studio`, `observability-concepts`, `billing`, `agent-builder`, `fleet/essentials`, `fleet/channels`, `auth`, `server-mcp`, `server-a2a`, `engine-security`, `platform-setup`); LangChain middleware pages under `https://docs.langchain.com/oss/python/langchain/` (`middleware/built-in`, `guardrails`); n8n docs under `https://docs.n8n.io/` (`flow-logic/looping`, `flow-logic/subworkflows`, `flow-logic/error-handling`, `workflows/executions`, `workflows/history`, `hosting/securing/blocking-nodes`, and the node pages for Wait, Webhook, Schedule Trigger and Execute Command); `https://api.github.com/repos/<owner>/<repo>`; `https://pypistats.org/api/packages/<name>/recent`; `https://api.npmjs.org/downloads/point/last-month/<name>`.

### 2.3 Where we are ahead

1. **A security model the leader does not have.** Taint bits, typed-clean fields, the Rule of Two check at turn-on and approvals that only a person can give as the person (row 21). The leader's documentation on untrusted input is thin, and n8n's answer is to switch the shell node off.
2. **Three tiers.** Triggers that work with no account (local), across machines (LAN) and, with an account, while the machine is off (cloud). The leader's webhooks are outbound only and its cron is UTC only (row 13).
3. **Spending limits that actually stop a run.** Dollars per run, per job per day and per account per day, enforced while the run is in flight (row 20). The leader has step and call limits.
4. **One ledger.** Every cron fire, drone run and cloud job is one run in one history, with incidents and alerts, kept forever and local (rows 15 and 16). The leader's retention is 14 or 180 days by tier.
5. **Replay that does not repeat the world.** A replay stubs effects unless a person asks for them (row 8), where the leader's replay "re-executes nodes".

### 2.4 Where we should NOT copy

1. **A graph written as Python or JavaScript** (LangGraph's graph and functional APIs). A graph that is data can be drawn, versioned, validated and taint-checked; code cannot. We copy the runtime ideas, not the authoring model.
2. **Interrupt-and-restart-the-node.** LangGraph's interrupted node re-runs from its start on resume, so side effects before the interrupt must be idempotent. Our approvals are nodes whose answer is stored, and a checkpoint is taken after the node before it.
3. **A recursion limit of 10,007 steps** as the default guard (confirmed in the library source). Ours is 25 iterations by default and 1,000 at most, plus a cost cap and a time cap.
4. **UTC-only schedules and outbound-only webhooks.**
5. **Features that only exist when you buy the platform** (assistants and versions, double-texting policies, time travel in Studio). They are local features here.
6. **n8n's licence and its "shell node off by default" as the whole answer.** n8n's fair-code licence limits embedding and redistribution, so we do not depend on it or ship any of it; and a shell step that is merely off is not a security design, which is why the Run step has typed-clean arguments and a container by default.
7. **Deterministic replay in the Temporal style.** It requires workflow code to be deterministic and fails a run on a reordered call. Checkpoint-per-step tolerates graph edits and is enough for the jobs we have; the price is that an effect in flight at a crash is not re-run on its own (§3.3, decision G5).

**A fair note on the other side:** the leader's long-term memory store with semantic search, its node-level cache and its evaluation platform are real features we do not have and are not planning in this amendment (rows 11, 10, 17). They are named so the omission is a decision, not an oversight.

## 3. Design

### 3.1 Loops

Two constructs, both bounded. **No unbounded cycle exists in a drone.**

- **Guarded cycle.** The graph may contain a back-edge (a cycle), but every cycle must pass through a **Loop guard**: a Condition node with `max_iterations` (default 25, hard ceiling 1,000). This is retry-until, poll-until and plan-then-revise. The validator rejects a cycle with no guard, as today's engine rejects every cycle ("drone contains a cycle").
- **For-each.** A node that takes a list (a trigger field, an API array, an App query result) and runs a **body** once per item: `concurrency` 1 to 8 (default 1), `max_items` (default 100, ceiling 1,000), `on_item_error: stop | continue`, and a **collector**: results are appended to a list the next node reads (`{{foreach_id.results}}`, a reducer of kind `append`, §3.2). In the first build the body is a **sub-drone** (§3.5): it keeps the typed boundary the taint story needs, and it reuses one UI instead of inventing a nested canvas region. An inline body region on the canvas is a later convenience over the same engine.

**Caps count across iterations.** The job's caps (Jobs spec §5.3: per run, per job per day, per account per day) are charged per node execution, so iteration 20 spends the same budget iteration 1 did. A loop also takes its own `max_iterations`, `max_cost_usd` and `max_minutes`, all lower than or equal to the run's. **When any cap ends a loop, the run says why in plain words**, both in the ledger row's `reason` (`loop_iterations`, `loop_cost`, `loop_time`, `loop_items`, `run_cap`, `run_timeout`) and on the canvas ("stopped after 25 of at most 25 iterations: the check never passed"). `on_cap: end_run` (the default) fails the run; `exit_loop` leaves the loop and continues, with `{{loop_id.capped}}` set so a later node can tell.

### 3.2 State

A run carries three things between nodes:

| Part | Meaning | Today |
|---|---|---|
| `trigger` | The event that started the run (read-only) | The real-world spec's §3.1 |
| `nodes.<id>` | The latest output of each node: `{{node_id.path}}`. In a loop, the latest iteration, with `{{node_id.history}}` for earlier ones (capped at the last 50) | Exists: `scope.outputs` |
| `vars` | Named values the graph writes: `{{var.name}}` | Exists: `scope.vars` (Variables block) |

**Reducers.** A variable declares how writes combine, the idea LangGraph calls a channel's reducer: `replace` (default; last write wins), `append` (a list), `merge` (shallow object merge), `add` (numbers). **Two branches that can run at the same time may both write a variable only if its reducer is not `replace`**; the validator refuses the graph otherwise, so there is no silent race (§3.7).

State is at most 256 KB, JSON only, and **never holds a credential**: a Vault value is added to one outgoing call and is not state (§4.1).

### 3.3 Checkpoint, resume, replay

After every node finishes, the engine writes a **checkpoint**: the run id, a step number, the state (with each value's taint bit), the frontier (nodes ready, running, waiting), loop counters, the cost and time spent, the deadlines, and the id and hash of the graph version the run started on. Checkpoints go to the local store beside the run row (`db_drone_runs`); the ledger row links to them.

- **Resume.** At startup, a run left `running` is no longer marked `interrupted` and forgotten (`backend/jobs/mod.rs`, `recover_after_restart`). The engine reloads the last checkpoint and continues from the frontier. This is what makes a restart or an upgrade survivable.
- **What happens to the step that was in flight when the machine stopped.** A node declares whether it is `read_only`, `idempotent` or `effectful` (an API write without an idempotency key, Message, Task, Spawn, Run, Tool). A read-only or idempotent step is simply run again. **An effectful step is not run again on its own**: the run becomes `needs_attention` with the step shown, and a person or an agent chooses *Run it again*, *Skip it* (supplying its output, which is **tainted** if an agent supplies it through MuxAPI) or *Cancel the run*. This is "at most once for effects", honest about what a checkpoint-per-step model can promise; it is weaker than the deterministic replay of a workflow engine such as Temporal, which we do not copy (§2.4).
- **Waiting.** A run can be `waiting`: for a duration, until a time, until an event key, or for an Approval's answer. The checkpoint holds `resume_at` or `resume_on`; the scheduler that already fires jobs (Jobs spec §5.1) wakes it; if the app was off, it wakes on the next start and the node sees `late`. A waiting run costs nothing and holds no process. **Wait** is a node (`for: 30m`, `until: <time>`, `on: <event key>`); **Approval** (phase 4) is the same state with a card. A wait `on: <event key>` is resumed only by an authenticated source (the local webhook with its secret, or the account's event channel); the key is a random 128-bit token made when the wait starts, not derived from anything an outsider can see, and the resuming payload is **tainted**.
- **Graph versions.** Saving a drone makes a numbered version (`db_drone_versions`: drone id, version, graph, saved at, saved by). A job pins `latest` or a number. A running run keeps the version it started on, however the drone is edited meanwhile. Rolling back is pointing the job at an older number.
- **Replay and time travel.** From a finished or failed run, *Replay from step N* creates a **new run** (`forked_from`, `from_step`) that starts from checkpoint N's state, on the same graph version. **Effectful nodes are stubbed in a replay unless the person ticks "do it for real"**, because replaying a Message or a merge is not what anyone means by looking at a run again. An edited state value is recorded with who edited it; a value typed by a person is clean, one set through MuxAPI by an agent is tainted (§4.2).
- **Streaming.** The existing per-run event stream (`DroneRun`, `dronerun:<id>`) gains `StepCheckpointed`, `LoopIteration`, `Waiting`, `Resumed`, `CapReached`, so the canvas and an agent follow a run live.
- **Retention (decision G17).** The ledger row and the step metadata (step numbers, node ids, outcomes, durations, costs, and hashes of state and payloads) are kept forever (Jobs spec §3.1). The **bodies** (state values, trigger payloads, Run output, API responses) are different: they can hold a CI log with a leaked token or personal data, so they are kept **30 days after the run ends** (proposed), then replaced by their hash and size. A checkpoint stores only the diff from the one before; the ceiling is 1,000 steps. Consequence, stated plainly: *replay from a step needs that step's state, so a run older than 30 days can be read but not replayed.* This amends D11 of the real-world spec, which kept 64 KB of payload in the run.

### 3.4 Cancel, timeouts, retries

The engine owns these (they were phase 1b). **Cancel** stops a run at the next step boundary and kills an Agent or Run step in flight; it is available from Run now's Stop, the Jobs tab, the `Drone` tool, and the job's own timeout. Every node has a `timeout` (Agent default 15 minutes, Run 5 minutes, API 30 seconds), and the run has the job's `timeout_minutes`. A node may `retry` up to 3 times with backoff, only for failures the node classifies as infrastructure (a network error, a 5xx, a provider rate limit), never for a node that finished and said `failed`. An error can also leave a node on an **error edge** to a handler node instead of failing the run.

### 3.5 Sub-drones

A **Drone** node calls another drone as a step.

- **Typed boundary.** A drone declares `inputs` and `outputs` (name, type: string, number, boolean, enum, list, object; and `clean` when it can promise a typed-clean value) in its Trigger and Response nodes. The caller maps values onto the inputs; the callee's Response fills the outputs. A type mismatch is refused when the caller is turned on, not at run time.
- **Version pinned.** The callee's version is pinned when the caller is turned on, so editing the callee does not silently change a running job.
- **Caps inherited.** The callee runs on its caller's remaining budget: cost, time, steps and loop iterations are shared, and a callee's own caps can only be lower. It cannot spend more than the run that called it.
- **No recursion.** The validator rejects a call cycle; the nesting depth is at most 4.
- **One ledger row, nested detail.** The call is a child run (`parent_run_id`) shown nested under the parent in the Jobs tab and the canvas; there is still one job and one ledger row, with the summed cost. An Approval inside a child surfaces on the parent run.
- **Taint crosses the boundary** (§4.2).

### 3.6 The Run step

A **Run** node runs a fixed command line.

- **argv, not a shell string.** `command` is a list (`["scripts/verify-release.sh", "{{tag}}"]`), never interpreted by a shell. No pipes, no expansion.
- **Arguments are typed-clean.** Every `{{…}}` in `command`, `cwd` and `env` must resolve to a clean value (a number, an enum, a validated id, a constant). **A tainted value there is refused when the drone is turned on**, and again at run time. The command line is never built from event text.
- **Container by default.** `where: container` runs it in the user's sandbox container with only the declared mounts (read-only unless declared) and **no network unless the step declares the hosts it needs**, the same shape as the Agent node's read-only profile. `where: host` is host execution and a **named allowance** (decision G16): its exact argv is listed in the turn-on prompt. **A container agent cannot create or edit a host Run step** (it executes on the host, the one exception the standing rule allows); the refusal says what to do instead: use container mode, or ask a native agent or the person.
- **Secrets through Vault grants,** as `Terminal run` does, and **only through `env`, never through argv** (argv is visible to other processes): a Vault entry named in `env`, an "Always" grant per entry and drone. Nothing in the graph, state, output, checkpoint or ledger: **stdout and stderr are scanned and the granted values are redacted before they become state, a checkpoint or a ledger line**, as an API response is (§6.6 of the real-world spec).
- **A fixed command is fixed content, not just a fixed line.** `argv[0]` and every argument the node marks as a script are files an agent can edit after the allowance, so a host Run step **pins the SHA-256 of the executable and of each script argument** when the allowance is given. A mismatch at run time stops the run as `needs_attention` with the reason and asks for the allowance again. A container step pins too when it has a Vault grant (recommended), since a changed script would then read a secret the person never saw it used with. **Editing a host Run step's argv makes a new graph version and needs a new allowance.** **A host Run step never executes through `run_on` unless the allowance was given on the machine that executes it.**
- **Output.** stdout and stderr, capped at 64 KB, are **tainted** (they are data), plus `exit_code` (a clean number) and `duration_ms`. A non-zero exit is a failure unless the node says `ok_exit_codes`.
- **Counts as *writes outside*** in the check (§6.3 of the real-world spec). With only clean arguments it needs no Approval, like a typed re-run of a CI job; a host Run still needs the allowance.
- A required `timeout` (default 5 minutes, ceiling 60).

### 3.7 Parallel branches and joins

An edge fan-out starts every successor at once, up to `parallelism` per run (default 4). Agent nodes in parallel branches still count against the global limit of 3 unattended agent runs (Jobs spec §5.1), so a wide graph waits its turn instead of flooding a provider. A **Join** node waits for its incoming branches: `all`, `any` or `n_of_m`, with `on_branch_error: fail | skip`. Branches write shared state only through reducers (§3.2). Fan-out over a list is the for-each (§3.1); a fixed fan-out to different nodes is a plain split.

### 3.8 The Tool step

A **Tool** node calls an MCP tool agents already have, with no agent turn. **Option, with a recommendation:** adopt it narrowly, and list it **per action, not per tool**, because a tool's actions differ in what they reach. It is the generic form of the Message, Task and Spawn nodes (those keep their own typed nodes for the UI); it is not a way to reach the dangerous tools.

| Tool | Actions offered | Not offered, and why |
|---|---|---|
| `Tasks` | `list`, `enqueue` (a task for a named agent or seat), `update` (a note or links on a task this drone filed) | `cancel`, `reassign`, `move`, `hold`, `unhold`, `accept`, `send_back`, `edit`, `reopen`, `retry`, `take_back`, `answer`, board changes: they change other people's work or answer for a person |
| `Message` | `send` (to one agent; counts toward the message cap) | `broadcast`, `bulletin`, `quota_mode`: fleet-wide effect |
| `Presence` | `who` | `claim`, `status`: the state of the agent that makes the call, not a drone's to set |
| `Memory` | `read` | any write with a tainted value, ever (below) |
| `Agents`, `Pane` | `list` | `spawn` (use the Spawn node), `open`, `set_seat`, and every other action that changes something |

**Memory is a special sink.** Global and Personal Memory are loaded into agents' startup instructions, so a tainted value written there is a stored prompt injection, quoted or not. **No tainted value goes into a Memory write, ever**: the validator refuses it at turn-on, and an Approval does not lift the refusal. A Global Memory write with typed-clean values is allowed, counts as *writes outside* and is listed in the turn-on prompt; Personal Memory writes are not offered.

**Not offered at all:** `Terminal`, `Browser`, `Vault`, `App`, `Stop`, `Desktop` and `Google` (their actions write outside or act as the person), and **`Schedule` and `Widget`**, because they create something that runs later, outliving the review that approved the drone: a stored job with its own trigger, or an installed widget. A drone that needs another timer uses its own Trigger node. Arguments follow the same rules as the typed nodes: tainted values only as quoted data, never in a target, a path or a name.

### 3.9 The Transform node

The data-shaping node. The leader's graphs shape data in arbitrary code and n8n has a Code node; Drone has neither, and a sophisticated setup needs to cut a list, pick fields, group and count. A **Transform** node does that with a restricted expression language and **no code execution**. It also settles the canvas spec's deferred *Function* kind.

- **Operations** over lists and objects: `map`, `filter`, `pick` (fields), `omit`, `sort` (by a key), `group` (by a key), `unique`, `slice` and `first`/`last`, `flatten`, `join` and `split`, `merge` (objects), `count`, `sum`, `min`, `max`, `exists`. An operation chain reads top to bottom, each step's result feeding the next.
- **Expressions** use the Condition node's grammar (§4.1 of the real-world spec: `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||`, `!`, `contains`, `matches`, `exists`, `in_set`), field paths (`item.user.login`), arithmetic (`+ - * / %`), and the template filters (`lower`, `trunc`, `default`). There are **no variables, no loops, no recursion, no function definitions, no calls to the host and no network**. The language cannot express an unbounded computation.
- **Bounds.** Linear time in the size of the input apart from `sort` and `group` (n log n); `matches` uses the Rust `regex` crate (linear-time); an evaluation budget of 100,000 operations per node; an input of at most 10,000 items; an output of at most the state cap (256 KB). Over any bound the node fails with the bound named.
- **Taint.** A Transform's output is **tainted if any input it reads is tainted**. The exceptions are results that cannot carry text: a `count`, `sum`, `min`, `max` or `exists` is a typed-clean number or boolean even over tainted input. These results are clean **as data only**: a number derived inside the run from tainted input is still chosen by an attacker when it is used as control or as a numeric argument (the derived-value rule, §4.5). Use-site declassification works as everywhere else (`in_set` at the point of use); a `filter` by `in_set` yields a list whose *membership* is clean but whose element contents stay tainted unless the elements are themselves the matched closed-set values.
- **Where it lands:** in **E1**, because a for-each over an API array or a trigger list is not useful without `map`, `pick` and `filter` beside it.

### 3.10 Canvas

Back-edges draw as a looping arrow with the guard's iteration count; a for-each and a Drone node draw as a node with an inner-view button; the run overlay shows the iteration (`3 of 25`), the waiting reason and its wake time, and `needs_attention` with the step and its three choices. The validator's reasons ("this cycle has no guard", "two branches write `var.log` with `replace`", "a tainted value reaches the command line") underline the reference, as the taint underline in the real-world spec does.

## 4. Security, for every new construct

The rules of the real-world spec hold unchanged: events are data, never instructions (§6.1); a value has a taint bit and typed fields are clean (§6.2); a path from a tainted value to a *writes outside* node needs an Approval (§6.3); an answer as the person comes only from the person's client and `approver: person` is v1 (§6.4); no credential in a graph, state, output or ledger (§6.6); no new "operator only" gate, the hard boundary is forging the operator's voice. This section shows that each addition keeps them.

### 4.1 Each construct

| Construct | The risk | What holds |
|---|---|---|
| **Loops** | A cycle launders taint (a value tainted in iteration 1 reads clean in iteration 2); a loop spends without end | Taint is computed to a **fixpoint** over the cycle: a variable tainted at any iteration is tainted everywhere in the loop. The only declassification is `in_set` or a typed check **at the point of use**, which clears that use, not the variable. Iteration, cost, time and item caps are hard (§3.1) and count into the run's caps |
| **State and reducers** | State written from a tainted value is read later as if clean; a credential ends up in a checkpoint | A state variable is tainted if any value written to it was; `append` is tainted if any element is. State never holds a Vault value (it is added to one outgoing call); the checkpoint stores no credential. A reducer cannot clear taint |
| **Checkpoint, resume, replay** | A replay re-sends a message or re-merges a PR; a checkpoint keeps hostile text for a long time | Effectful steps are never re-run on their own (`needs_attention`) and are stubbed in a replay unless a person ticks "do it for real". A checkpoint stores tainted payloads as data only, size-capped, in the local store, bodies kept 30 days after the run ends (G17); the tainted bit is saved with the value and survives resume. An edited state value is recorded with who edited it; a person's typed value is clean, an agent's is tainted |
| **Sub-drones** | Taint disappears at the call boundary; a callee spends more than the caller was allowed; a cycle of calls | An **output field is clean only if the callee declares it `clean` and the validator proves every value that can reach it is clean**; otherwise it is tainted, whatever its type says. An input the caller passes keeps its taint inside the callee. Budgets are shared and only narrow. The callee version is pinned; a call cycle is refused; depth ≤ 4. The turn-on prompt for the caller lists the callees' outside writes and credentials too |
| **Run step** | The classic injection: event text becomes a command | argv only, no shell; every argument, `cwd` and `env` value must be clean or the drone is not turned on; container by default; host execution is a named allowance with the exact argv; Vault grants for secrets; output tainted. The rule "tainted data never reaches a node that can execute" is checked as a path property, like §6.3 |
| **Parallel branches** | A race between branches writes state; branches together exceed the budget | Concurrent writers need a non-`replace` reducer or the graph is refused; the budget and the global unattended-run limit are shared |
| **Tool step** | An MCP tool reaches something dangerous without an agent's own limits | Allowlist of tools that change only AgentMux's own state or read it (§3.8); the dangerous tools are not offered; quoted-data and typed-argument rules apply |
| **Wait and approvals** | A run waits for hours holding an old, since-revoked grant | A waiting run keeps no credential and no process. On resume, grants are checked again, as for a new run, and a revoked one stops the run with a reason |

### 4.2 Taint across the new boundaries

- State: tainted if any contributing write is tainted; the bit travels in the checkpoint.
- Loop: fixpoint; use-site declassification only.
- Sub-drone: input taint passes in; an output is clean only by the callee's proven `clean` declaration.
- Edited state: a person's typed value is clean; a value set through MuxAPI is tainted. Both are recorded with who set them. This includes the output an agent supplies on *Skip it* (§3.3): tainted.
- Run step: arguments must be clean; the output is tainted.
- Agent output: tainted whatever the input was (unchanged).

### 4.3 What the validator cannot decide

The turn-on validator decides everything above by static analysis of the graph and its pinned callees. Where it cannot, it **refuses and says what it could not decide** instead of guessing: a callee whose `clean` outputs it cannot prove, a Join whose branches it cannot show do not share a `replace` variable, a `matches` pattern it cannot bound, a Tool name outside the allowlist, a for-each over a list of unknown element taint (treated as tainted). A refusal names the node and the missing fact, so the author can fix the graph or add a typed check.

### 4.4 Who may do what (additions to §6.5 of the real-world spec)

| | Person | An agent (MuxAPI) |
|---|---|---|
| Create or edit a graph with loops, state, sub-drones, parallel branches, Tool nodes | Yes | Yes, subject to the validator |
| Create or edit a **host** Run step | Yes, and gives the allowance | **A native agent** can create it, and give the allowance in form (b) of G16 (recorded as that agent, with a notice and a veto window to the person before the first run, the step visible in the Jobs tab) or only the person can (form a); **a container agent cannot** (host execution); the refusal says to use container mode or ask a native agent or the person |
| Create a container Run step | Yes | Yes (container agents get every node that does not execute on the host) |
| Resume, skip or cancel a `needs_attention` run | Yes | Yes (recorded as the caller) |
| Replay a run "for real" | Yes | Yes, recorded as the caller; the turn-on prompt shows the effectful nodes it would repeat |
| Answer an Approval *as the person* | Yes, from the app's own client | No (§6.4 a of the real-world spec) |

### 4.5 Control flow is an input too

The check in §6.3 of the real-world spec speaks of a path from a tainted *value* to a write. With loops, Conditions and for-each, a tainted value also decides **whether** and **how often** a write with perfectly clean arguments runs: an Agent's verdict can gate a Run step; a tainted list sets a for-each's item count; a Join `any` fires on whichever branch an attacker made finish first. So:

- **The validator treats every branch decision, loop guard, Join `any` and `n_of_m`, multi-way switch, for-each item count and Wait `until` that depends on a tainted value as chosen by an attacker.** A write that is control-dependent on such a value is on a tainted path for the §6.3 check exactly as one that is data-dependent: it needs an Approval between, or the graph is refused.
- **The turn-on prompt lists every outside write reachable under attacker-chosen control, with its fixed arguments and its maximum count** (iterations times items times sub-drone calls, each at its ceiling): "this drone can run `scripts/x.sh v1.2.3` up to 25 × 100 = 2,500 times".
- **Nothing in the design leans on "the agent said ok" as protection.** An Agent node's verdict is tainted, and a Condition over it is attacker-chosen control, whatever the agent's instructions say.
- **The derived-value rule.** A typed-clean value **derived inside the run from tainted input** is clean as *data* and still attacker-chosen as *control* and as a *numeric argument*. That covers a Transform `count`, `sum`, `min`, `max` or `exists` over a tainted list, a Run step's exit code when its input or output was tainted, and an Agent's enum cleared by `in_set`. So "count a tainted list, branch on the count, run a clean-argument write" is refused without an Approval, exactly like branching on the tainted list itself. **Only fields typed by the trigger's own schema are exempt** (an enum, a number or a boolean the event source is validated against before the run starts), and what an external event can cause through them is bounded by the trigger's rate limit and `Once`. **A derived number used as an argument** must have its allowed range declared on the node (a minimum and a maximum, enforced at run time), and the turn-on prompt shows that range.
- **Acceptance (b), re-run a flaky job once:** the re-run is control-dependent only on `conclusion`, `run_attempt` and the job name, all three typed by the trigger's own schema (an enum, a number, and an enum whose members are the flaky list, so the name is a trigger field and not a derived value). They are exempt under the rule above; what an external event can cause is the trigger's business, and it is bounded by the per-job rate limit (30 an hour) and `Once` on the run id, both of which the prompt states ("up to 30 re-runs an hour, once per run"). So (b) needs no Approval. (c), whose merge is control-dependent on an Agent's verdict, keeps its Approval.

## 5. What this does to the build

Measured against the six phases of the real-world spec (§10 there).

| Phase | Starts now, unchanged? | Why |
|---|---|---|
| 1a: Drone as a job action, the Trigger node (manual, timer), run input, `Drone` fire, "Open graph", cost rollup | **Yes** | Everything above the engine. Cost rollup becomes a sum over node executions, which stays true when iterations arrive |
| 1b: cancel, timeouts, retries | **No: moves into the engine phase** | They live inside the scheduler that is being replaced |
| 2: local events and real actions | **Yes** | Trigger-side work plus new blocks on the unchanged block interface |
| 3: secrets and polling | **Yes** | Vault credentials on the API node, `poll`, `expect`, payload schemas. The tainted-reference underline waits for the validator in phase 4 |
| 4: Approval node, validator, turn-on prompt | **No: after the engine** | The Approval needs the durable wait; the validator must reason about loops, state and sub-drones. `Once` (local) moves into phase 3 |
| 5: durable waits | **Cancelled, absorbed** | The engine's checkpointing is durable waits |
| 6: LAN | **Triggers yes; `run_on` after the engine** | `peer_event` and `peer_presence` are trigger-side; a remote run must be a checkpointed run |

**New phase list, shortest order to loops and durable state first** (sizes are estimates: S ≈ one PR, M ≈ two or three, L ≈ four to six):

| # | Phase | Delivers | Size |
|---|---|---|---|
| 1a | Drone as a job action | See above | S |
| E1 | **Engine, part 1** | The step scheduler over a state object; reducers; guarded cycles; for-each with a sub-drone body stubbed to a single-node body until E3; caps across iterations; the visible cap reasons; the Transform node; the validator rules for cycles and concurrent writers | L |
| E2 | **Engine, part 2** | Checkpoints; resume; `needs_attention`; Wait; cancel; timeouts; retries; error edges; the new run events | L |
| E3 | **Sub-drones, parallel branches, joins** | The Drone node and typed boundaries; Join; parallelism; the nested view in the Jobs tab | M |
| E4 | **The Run step and the Tool step** | argv, container and host modes, the allowance in the turn-on prompt, the Tool allowlist | M |
| E5 | **Versions and replay** | `db_drone_versions`; pinned versions; replay and time travel with stubbed effects | M |
| 2, 3 | Local events and actions; secrets and polling | As in the real-world spec; independent of the engine; a second builder may take them | M each |
| 4 | Approval and the validator | After E2 and phase 3 | M |
| 6 | LAN | Triggers any time; `run_on` after E2 | S to M |
| 7, 8 | Cloud | Unchanged; wait for the private companion and the security read | |

E1 and E2 are the bulk of the risk. The block interface rule (§0) is what lets phases 2 and 3 proceed beside them.

## 6. Decisions for the operator, each with the recommendation first

| # | Decision | Recommendation |
|---|---|---|
| G1 | Update the approved spec in place, or amend | **Amend** with this second document; the first ships as approved |
| G2 | Loops: guarded cycles and a bounded for-each, no unbounded cycle | **Yes**, with the hard ceilings in §3.1 |
| G3 | The for-each body in the first build | **A sub-drone** (typed boundary, one UI); an inline region later |
| G4 | The effectful step in flight at a crash | **Stop the run as `needs_attention`** and let a person or an agent choose; never re-run an effect on its own |
| G5 | Deterministic replay (Temporal style) | **No**: checkpoint-per-step is enough for the jobs we have; revisit if a user needs exactly-once effects |
| G6 | The Run step | **Yes**: argv only, typed-clean arguments, container by default, host as a named allowance; a container agent cannot create a host step |
| G7 | The Tool step | **Yes, narrowly:** an allowlist **per action** (§3.8), reads and a few inside writes; no tainted value into a Memory write, ever; no `Terminal`, `Browser`, `Vault`, `App`, `Stop`, `Desktop`, `Google`, `Schedule` or `Widget` |
| G8 | Graph versions | **Yes**: numbered versions, jobs pin one, running runs keep theirs |
| G9 | Replay effects | **Stubbed by default**; "do it for real" is a person's or an agent's explicit tick, recorded |
| G10 | Order | **1a, E1, E2, E3, E4, E5**, with 2 and 3 on a second builder and 4 and 6 after E2 |
| G11 | The no-code builder | **Not now**: the canvas is the builder; LangChain's agent builder is a different product (§2.4) |
| G12 | Copy LangGraph's Python/JS code API | **No**: the graph is data a person can see and the validator can check (§2.4) |
| G13 | The three leader features not planned here: **long-term memory with semantic search**, a **node-level result cache**, an **evaluation platform** (rows 11, 10 and 17) | **Accept the gap for now,** with a cheap first step each, in the order they are likely to matter: (1) *memory:* the Tool step can read and write the allowlisted `Memory` tool and a drone's `vars` persist across a loop, but no vector store; (2) *cache:* a node `cache_for` setting keyed by its resolved inputs, a small addition on the checkpoint store, after E2; (3) *evaluation:* "run this drone against a saved set of recorded trigger payloads" (replay plus the test payload; no new platform). Decide per item whether to promote it. The operator asked for parity now; this is the part of the parity table where we are not at parity |
| G14 | "Build with AI" (the leader's row 18) | **Later:** an agent drafts a drone from a description, with the turn-on validator as its check. Not in this amendment's phases |
| G15 | A Transform node (no code, restricted expressions, linear-time, size-capped) instead of a code node | **Yes, in E1** (§3.9) |
| G16 | Who gives the host Run allowance | Two forms: **(a)** the person only; **(b)** a native agent may give it, recorded as that agent, with a notice and a veto window to the person before the first run, and the step visible in the Jobs tab. **Recommended: (b)** for a drone whose trigger is local to this machine or its own LAN, and **(a)** for a drone whose trigger comes from outside this machine (a webhook, a cloud event, a public feed). The real added risk either way is **persistence**: a steered native agent leaves a recurring host command behind, which a one-off shell call from that agent does not. Pinning (§3.6) and the visible step are what bound it. The container-agent refusal stays |
| G17 | How long state bodies are kept (§3.3) | **30 days after the run ends,** then replaced by their hash and size; ledger rows and step metadata forever. Amends D11 of the real-world spec. Cost: a run older than 30 days cannot be replayed from a step |

## 7. Tests

- Cycles: a cycle with no guard is refused; a guarded cycle stops at `max_iterations` with `reason: loop_iterations` and a visible message; `exit_loop` continues with `capped` set; cost and time caps end a loop with the right reason; a for-each over 150 items with `max_items` 100 stops at 100.
- State: `replace`, `append`, `merge`, `add`; two parallel branches writing a `replace` variable are refused; the same with `append` runs; state over 256 KB fails the step with a reason.
- Checkpoint and resume: kill the app mid-run; on restart the run resumes from the last checkpoint; a read-only step in flight is re-run; an effectful one makes `needs_attention`; Run it again, Skip and Cancel each work; an upgrade between does not break a resumed run (checkpoint schema version).
- Wait: a 2-minute wait survives a restart and wakes late if the app was off; an Approval waits for hours with no process; a revoked grant stops a resumed run.
- Versions and replay: editing a drone does not change a running run; a job pinned to version 3 runs version 3; replay from step 4 starts a new run with the same state, effects stubbed; "do it for real" repeats them and says so.
- Sub-drones: a type mismatch is refused at turn-on; a callee's cost counts against the caller; a call cycle is refused; a `clean` output the validator cannot prove is treated as tainted; an Approval in a child shows on the parent.
- Run step: a tainted value in `command`, `cwd` or `env` is refused at turn-on and at run time; a container run has no write outside its mounts; a host Run needs the allowance; a container agent cannot create one; the output is tainted; a non-zero exit fails the node unless allowed.
- Tool step: an allowlisted tool works; `Terminal` is refused at turn-on.
- Transform: each operation on a small list and object; chained operations; an input over 10,000 items, an evaluation over 100,000 operations and an output over 256 KB each fail naming the bound; a hostile `matches` pattern cannot stall it; `count` over a tainted list is clean, `map` over one is tainted; there is no way to write a loop or call out.
- Taint: a variable tainted in iteration 1 is tainted in iteration 2; `in_set` at the use clears that use only; a sub-drone's tainted output reaching an outside write with no Approval is refused.
- Derived values: a Transform `count` over a tainted list that gates a Run step is refused without an Approval; the same count used as a numeric argument needs a declared range, which the turn-on prompt shows and run time enforces; a field typed by the trigger's own schema gating the same step is accepted.
- Control flow: a Run step behind a Condition over an Agent verdict is refused without an Approval; the turn-on prompt lists each outside write with its maximum count; acceptance (b) is accepted with no Approval and its prompt states the rate limit and `Once`.
- Host Run step: the SHA-256 pin of the executable and of a marked script argument; a changed script stops the run as `needs_attention`; editing argv makes a new version needing a new allowance; a host step does not run through `run_on` on a machine that did not give the allowance; a Vault value is accepted in `env` and refused in argv; a granted value in stdout is redacted before state, checkpoint and ledger.
- Tool step: each offered action works; every action not in the table (`Message` `broadcast`, `Tasks` `cancel`, `Schedule`, a Memory write with a tainted value) is refused at turn-on.
- Retention: state bodies are replaced by hash and size 30 days after a run ends; step metadata remains; replay of an older run is refused with the reason.
- Wait on an event key: a wrong or guessed key does not resume the run; the resuming payload is tainted.
- Ledger: one row per job run with the summed cost across iterations and sub-drones; the child runs nest; a failure opens one incident.
- Parity: the acceptance drones (a) to (e) of the real-world spec each still run on the new engine unchanged.
