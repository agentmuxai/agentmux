# AgentMux Process Architecture — Current State & Rethink

**Date:** 2026-07-22
**Scope:** Every mechanism in AgentMux that tracks "is this long-running process/agent alive, and what is
it doing" — across `agentmux-srv/src/backend/blockcontroller/`, `agentmux-srv/src/backend/process_tracker/`,
`agentmux-srv/src/backend/reactive/`, and the frontend consumers in the Agent pane and Swarm overview —
plus the proven "single broker" precedent from the just-shipped Credential Broker, plus external prior art
from mature process-supervision systems, plus a concrete target architecture.

**Motivation:** reported symptom — "agent data" appears inconsistently in two different places (the Agent
pane and the Swarm overview), suggesting the underlying process-tracking architecture is fragmented rather
than backed by one shared source of truth. This report traces that symptom to its actual mechanisms and
proposes a **Process Broker**, modeled explicitly on the Credential Broker that just landed for auth.

**Ground truth basis:** `agentmuxai/agentmux` `main` at commit `ce44c885` (`v0.54.3`), pulled fresh for this
report.

---

## 0. Executive summary

AgentMux has **at least six independent, only-partially-overlapping mechanisms** for answering "is this
agent/process alive and what's it doing," spread across two backend subsystems that don't know about each
other, plus per-pane frontend wiring that reimplements aggregation logic ad hoc:

1. `blockcontroller::CONTROLLER_REGISTRY` — a `HashMap<block_id, Arc<dyn Controller>>`, the coarse
   running/idle/error status every controller type publishes. **Genuinely shared** — both the Agent pane
   and Swarm pane already read this same source via the same RPC/event. Not the problem.
2. `process_tracker::AgentProcessRegistry` — a separate `HashMap<block_id, TrackerHandle>` wrapping real OS
   process trees (Windows Job Objects / Linux cgroups / macOS process groups), polled every 2s. Populated by
   only **2 of 4** controller types (`subprocess`, `persistent` — not `shell`, not `acp`).
3. `backend::reactive`'s registration map — a structurally different, app-level "this block registered
   itself as active" list, **unioned by naive string-ID merge** with #2 to answer "which blocks exist" for
   the Swarm pane, with no reconciliation between the two.
4. `blockcontroller::pidregistry` — a fourth, tiny `HashMap<block_id, u32>` (one PID each), written only by
   the Terminal/shell controller, read only by the Sysinfo widget. Fully disconnected from #2 and #3.
5. `blockcontroller::health.rs`'s `HealthMonitor` — infers liveness from *output-silence timers*, not PID
   existence. Feeds #1's `turn_active` field, so not fully independent, but it's a fifth distinct inference
   mechanism underneath a signal that looks unified from the outside.
6. `blockcontroller::watchdog.rs` — a sixth signal: PTY-idle timers, but *only* for shell-type agent panes.

On top of that, **two independently-tuned pipelines** generate the same conceptual "what's this agent doing
right now" text (a periodic push-based Haiku summary for Swarm vs. an on-demand pull RPC for the Agent
pane), and the **Swarm pane's most granular status chip is client-side-only**, populated by whichever Agent
pane component happens to be mounted in the same renderer — if it isn't, Swarm silently falls back to a
coarser signal. None of this is `//TODO`-flagged as broken; each piece was added correctly for its own
immediate need. That's exactly how six mechanisms accumulate without anyone deciding to build six.

The team just solved a structurally identical problem in the auth domain: three independent OAuth/credential
systems, five duplicated login-trigger code paths, "sharing nothing but a SQLite connection handle" (per
`docs/specs/REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md`), fixed by designing **one Credential
Broker** and shipping its Phase A skeleton the same night (`agentmux-srv/src/broker/`, commit `fba79dcb`).
This report recommends applying the same pattern — one **Process Broker**, one public read/write contract,
existing mechanisms demoted to internal implementation details behind it — while flagging explicitly where
credentials and processes are *not* analogous and the pattern needs to bend (§4.1).

---

## 1. Current state — the six tracking mechanisms

| # | Mechanism | File | Keyed by | Populated by | Consumed by |
|---|---|---|---|---|---|
| 1 | `CONTROLLER_REGISTRY` (turn-active status) | `blockcontroller/mod.rs:214` | `block_id` | Every controller's `start()` | `block.GetControllerStatus` RPC + `controllerstatus` WS event → **both** Agent pane and Swarm pane |
| 2 | `AgentProcessRegistry` (OS process trees) | `process_tracker/registry.rs:40` | `block_id` | Only `subprocess.rs:432`, `persistent.rs:717` | `agent.process-list` RPC (Agent pane's "⚙ N" badge) + half of `agent.tracked-blocks` (Swarm discovery) |
| 3 | `reactive` handler's registration map | `backend/reactive/handler.rs:188,590` | `block_id` | App-level self-registration, separate from #2 | Other half of `agent.tracked-blocks` (Swarm discovery) |
| 4 | `pidregistry` | `blockcontroller/pidregistry.rs` | `block_id` | Only `shell/lifecycle.rs` (Terminal blocks) | Sysinfo widget's CPU/mem loop only |
| 5 | `HealthMonitor` (output-silence heuristic) | `blockcontroller/health.rs` | per-controller-instance | Output/error stream observation | Feeds #1's `turn_active` |
| 6 | PTY-idle watchdog | `blockcontroller/watchdog.rs` | shell-type agent panes only | `last_output_secs_ago()` | Internal restart/kill decisions |

**The concrete mechanism behind the reported symptom** is #2 vs. #3: `agent.tracked-blocks`
(`agentmux-srv/src/server/app_api/agent_io.rs:49-68`) answers "which agent blocks currently exist" by
literally concatenating and de-duping two structurally different lists:

```rust
let process_ids = process_tracker.list_all_blocks();
let reactive_ids = crate::backend::reactive::get_global_handler().list_active_blocks();
let block_ids: Vec<String> = process_ids.into_iter().chain(reactive_ids)
    .filter(|id| seen.insert(id.clone()))
    .collect();
```

A block registered in one but not (yet, or ever) the other appears with different confidence/timing in the
two panes. `swarm-model.ts` itself has a comment distinguishing "phantom OS processes" from "reactive
registrations" — the frontend already knows these are two different kinds of thing, it just merges them
anyway because that's the only RPC that exists.

**Compounding this**, mechanism #2 (`process_tracker`) — which powers *both* the Agent pane's own process
badge *and* half of Swarm's discovery list — only actually registers 2 of the 4 real controller types.
`acp.rs` spawns real child processes (`acp.rs:185,190`) but never calls into `process_tracker` or
`pidregistry` at all; an ACP-driven agent pane (e.g. `gemini --acp`) is structurally invisible to the OS
process registry, visible only through the coarse #1 status. Shell/Terminal controllers are in the same
position relative to #2 (they only reach #4, the sysinfo-only registry). So depending on which provider/
controller type an agent uses, the Agent pane and Swarm pane can be looking at genuinely different subsets
of "the truth" for the same class of question — not a hypothetical, a direct consequence of who currently
calls into which registry.

---

## 2. Current state — two more duplication patterns, found tracing the actual UI paths

### 2.1 Two independently-tuned "activity summary" pipelines

- **Push, Swarm-motivated**: `backend/reactive/activity_watcher.rs` — a periodic sweep loop that generates a
  Haiku-summarized one-liner per running agent and pushes it via the `agent:summary` WS event into block
  meta (`term:ambient_summary`). Its own doc comment states the reason directly: "so panes (the swarm feed,
  in particular) can show a live one-liner without polling."
- **Pull, Agent-pane-motivated**: `useAgentActivitySummary` → RPC `AgentActivitySummaryCommand`, fired on
  turn-end edges, backed by its own generation logic in `app_api/session.rs`.

Same conceptual output (a short "what's this agent doing" string), two separately-tuned generators with
different word budgets, different cadences, different cost accounting, and no shared cache — a change to
one's prompt/budget doesn't propagate to the other, and they can disagree about the same agent at the same
moment.

### 2.2 The Swarm pane's finest-grained signal is client-side-only

The activity chip (working / using tools / stopping / error / disconnected) that Swarm shows per agent is
populated by `getBlockTurnPhase(blockId)` (`frontend/app/store/agentActivity.ts`) — a purely
in-renderer registry that an Agent pane component populates via `registerActivity()` *if and only if that
agent's own pane happens to be mounted in the same renderer*. If it isn't, Swarm silently falls back to the
coarser running/idle signal from mechanism #1. `swarm-view.tsx`'s own comments flag this as a past bug
source. This is architecturally the opposite of what "Swarm shows every agent regardless of what's open"
should mean — the richest signal is only available for agents whose own pane you happen to also have open.

### 2.3 What's genuinely *not* broken

Worth stating plainly, since it's easy to over-read "spaghetti" as "everything is duplicated": the core
turn-active/running signal (mechanism #1, `CONTROLLER_REGISTRY` + `GetControllerStatus`/`controllerstatus`)
**is** already a single shared backend source that both panes read identically. The Swarm pane's only
structural inefficiency there is calling it once per tracked block in a loop (`swarm-model.ts:1134-1145`)
instead of through a batch/aggregate query that doesn't exist yet — real, worth fixing, but it's redundant
*plumbing*, not redundant *truth*. The fragmentation is concentrated specifically in: OS-process-level
tracking (2 vs 3, and the controller-type coverage gap), the two activity-summary pipelines, and the
client-side-only fine status chip.

---

## 3. Two precedents, not one: the Credential Broker, and the Phase E srv reducer

### 3.0 The precedent: `agentmux-srv/src/broker/` (Credential Broker)

Built the same night as `docs/specs/REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md` was written
(commit `fba79dcb`, "feat(auth): credential broker skeleton — single-flight refresh + keychain-backed MuxBus
tokens"). Its own module doc states the shape of the problem it solves in language that transposes almost
directly onto this report's §0-§2:

> "AgentMux runs three independent OAuth/credential systems today ... with no shared refresh or storage
> model. This module is the consolidation point: a single, generic, single-flight-guarded,
> proactively-scheduled refresher, with each credential system registering its own load/refresh/save
> behavior against its own real backing store."

#### 3.0.1 The actual mechanism

`RefreshScheduler` (`broker/scheduler.rs:47`) is a process-wide singleton (`OnceLock`, `broker::init_global`),
constructed once, spawning exactly one background sweep task. It owns **no credential data itself** — each
registrant (`register(credential_id, is_fresh_closure, refresh_closure)`) supplies its own load/save logic
against its own real store, so MuxBus's SQLite row and (eventually) CLI-provider keychain entries can both
register without the broker needing to know their internals. Concurrency safety comes from a per-credential-
id `AsyncMutex` with double-checked freshness after acquiring it — proven directly by a test named
`concurrent_callers_collapse_onto_one_refresh`.

#### 3.0.2 What transfers cleanly to a Process Broker

- The **generic registration shape** — `register(id, closures)` against callers' own backing stores, not a
  broker-owned data model — fits process tracking well: mechanism #2's Job-Object trees, #4's Terminal PIDs,
  and #5/#6's heuristic inference all stay as-is internally; the broker's job is to be the one place that
  *asks* them and reconciles the answer, not to reimplement them.
- **Single-flight-guard-per-key** directly solves the "Agent pane mounts and Swarm's periodic refresh fire
  near-simultaneously" race — collapse concurrent "what's block X's status" queries onto one computation.
- **Global-singleton-behind-`OnceLock`-with-one-spawned-background-task** is the right lifecycle shape here
  too — a single continuous sweep, not per-caller polling loops.
- The **"credential broker" vs "credential provider chain" distinction** the auth report's external research
  turned up (§5.6 there) applies verbatim: a *broker* is centrally-addressable, consumers explicitly call
  into it; a *provider chain* is an ambient, client-side, ordered fallback search. `agent.tracked-blocks`'s
  literal `.chain()` of two registries **is a provider chain today** — exactly the anti-pattern the auth
  report flags as worse than what it replaces, now found again in the process domain by coincidence of
  implementation rather than by anyone choosing it deliberately.

#### 3.0.3 Where it must NOT be copied literally

The prior research pass that read `RefreshScheduler` in full flagged four real disanalogies, worth
repeating verbatim because they'll matter at implementation time:

1. **Cheapness/blocking contract**: a credential's `is_fresh` closure is a fast, side-effect-free store read
   by contract. A process's "is it alive" check is not always free (an OS process-tree walk, a health-probe
   round-trip) — the broker's equivalent closure contract needs an explicit async/non-blocking guarantee,
   not an assumption inherited from the credential case.
2. **Idempotency**: refreshing a credential is safe to retry and has clean preserve-on-failure semantics
   (never overwrite a good token with a failed refresh's garbage). Respawning a dead process is **not**
   idempotent — it has side effects (new PID, potentially a new listening port, lost in-memory state) that a
   credential refresh never has. The broker must not treat "process looks dead" the same way it treats
   "token looks stale" — restart, if the broker does it at all, needs to be an explicit, opt-in action a
   caller requests, not something automatically triggered by a freshness check the way credential refresh is.
3. **Polling vs. event-driven**: the credential broker's fixed-interval sweep is fine because token expiry is
   itself schedule-driven. Process liveness is fundamentally event-driven (a process exits, crashes, or gets
   killed at an unpredictable moment) — mechanism #2 already gets this right today (Windows Job Object
   `KILL_ON_JOB_CLOSE` + a `Drop` handler catch crash-without-cleanup, *on top of* a 2s poll as a backstop).
   The Process Broker should formalize that hybrid (event-driven primary signal, polling as a backstop for
   what the OS primitive can't tell you promptly), not regress to pure polling because that's what the
   credential broker's sweep loop does.
4. **Fail-closed vs. graceful degradation**: `resolver.rs`'s spawn gate is deliberately strict — refuse to
   spawn rather than risk running with the wrong credentials, because a security mistake is worse than a
   blocked spawn. A Process Broker answering "is this thing alive" should default to graceful degradation
   (report unknown/best-effort rather than block a UI from rendering) — the two domains have opposite risk
   profiles for what "I'm not sure" should do.

Also worth noting directly: the auth domain **still has an unfixed duplicate** of this same class of problem
even after the broker's Phase A shipped — `identity/oauth_client.rs` has its own separate, structurally
near-identical `OAuthSessionManager` (own `Mutex<HashMap>`, own timeout/prune logic, own `OnceLock`
singleton) for Armory service-account OAuth, entirely independent of `identity/auth_session.rs`'s
`AuthSessionManager`. The fix for one domain didn't retroactively catch every instance of the pattern in that
same domain — worth keeping in mind as a caution against declaring victory the moment a Process Broker's
Phase A lands, given this report has already found six-plus instances, not three.

### 3.1 The other precedent: the Phase E srv reducer (`agentmux-srv/src/reducer.rs`)

The Credential Broker answers *where does the authoritative read come from*. It does not, by itself, answer
*what stops two call sites from mutating shared state into a mutually inconsistent shape* — which is the
sharper version of this report's actual root cause: nothing today arbitrates whether a given state
transition is even legal before applying it. This codebase already has a mature, load-bearing answer to
exactly that question, applied to a different domain (window/tab/workspace/block lifecycle), and it's a
stronger, more directly applicable precedent for the Process Broker's *internal* design than the credential
broker is.

`agentmux-srv/src/reducer.rs` (4,044 lines) plus its `reducer/{block,layout,lifecycle,snapshot,tab,window,
workspace}.rs` submodules (another ~2,000 lines) implement a single pure dispatch function:

```rust
// Phase E — srv reducer.
// Pure functional core: `update(&mut State, Command, &Ctx) -> Vec<Event>`.
// Never blocks, never awaits, never does I/O. Same discipline as
// `agentmux-launcher::reducer`. Mutex held only during dispatch
// (sub-millisecond).
```

Every `Command` (`Register`, `CreateBlock`, `SetActiveTab`, `ReorderTab`, ...) funnels through this one
function, which mutates `State` and emits `Vec<Event>` — there is no second code path anywhere in the
codebase that's allowed to mutate window/tab/workspace/block state directly. `reducer/lifecycle.rs`'s
`handle_register` (governing `state.rs`'s `ProcessRecord`/`ProcessState` — the IPC-client process
bookkeeping mentioned in §1's table as "unrelated noise" to the six mechanisms, but directly relevant
*here* as a pattern example) shows the shape concretely: before inserting a new `ProcessRecord`, it computes
`allow_register` from the *current* state (`None` or `Exited{..}` → allowed; anything else → rejected with
an explicit `ErrorCode::AlreadyRegistered` event) — an explicit legal-transition guard, not an unconditional
field write.

**This is precisely what's missing from all six of §1's mechanisms.** `CONTROLLER_REGISTRY`'s status is set
from inside each controller impl's own methods; `process_tracker`'s registry is written from two spawn call
sites and a poll loop; `reactive`'s registration map is written from wherever a block "registers itself
active." None of these five funnel through one arbiter that can say "no, you can't transition from `Done` to
`Running` without going through `Idle` first" (or whatever the real legal-transition rules turn out to be) —
they're each a plain `HashMap` any authorized caller can `.insert()`/`.remove()` into directly.

**Recommendation, refining §5.1:** the Process Broker's core `lifecycle` field should be built the same way
`agentmux-srv/src/reducer.rs` already builds window/tab/block state — a pure `update(&mut ProcessBrokerState,
ProcessEvent, &Ctx) -> Vec<ProcessStatusEvent>` function, single Mutex-guarded dispatch point, explicit
transition table (mirroring supervisord's `STOPPED→STARTING→RUNNING→[BACKOFF|STOPPING|EXITED|FATAL]` from
§4 below), illegal transitions rejected and logged rather than silently applied. The two precedents compose
rather than compete: the credential broker's "register your own backing store" shape governs how
heterogeneous *inputs* reach the broker (mechanism #2's OS poller, #5's health monitor, #6's watchdog each
emit `ProcessEvent`s into the reducer rather than writing a `HashMap` field directly), while the reducer
governs what happens once an event arrives — the same input source, e.g. `process_tracker`'s poll-diff, that
today directly mutates a `HashMap` and fires a raw WS event becomes instead a `ProcessEvent::PidObserved`
dispatched into `update()`, which decides whether that's actually a legal/meaningful transition before
emitting anything downstream. Fields that aren't naturally enum-shaped (the PID set, the activity-summary
cache string) still go through the same single dispatch point — "single writer, explicit transitions" is the
generalizable principle even where "state machine with a small enum" doesn't literally apply.

One naming note worth flagging as an open question (folded into §6): this codebase's `reducer` module
already exists for a different, adjacent concept (`state.rs`'s IPC-client process bookkeeping — the
`ProcessRecord` used by `Register`/`Goodbye`, not agent-spawned subprocess tracking). A Process Broker
reducer would need a name that doesn't collide with or get confused for that existing one, even though
architecturally it's the same pattern intentionally reapplied — see open question §6.6.

---

## 4. External prior art — process-supervision status models

The auth report grounded its recommendation in how `gh`/`docker`/`az`/`aws` handle credentials. The
equivalent grounding here is how mature process-supervision systems model "is this thing running and
healthy" as a single, canonical status — since that's precisely the question AgentMux currently answers six
different, partial ways.

- **systemd** models every managed unit with exactly one canonical `ActiveState`
  (`active`/`reloading`/`inactive`/`failed`/`activating`/`deactivating`), computed from lower-level signals
  (`SubState`, cgroup membership, exit codes) but exposed as one value every consumer queries the same way
  (`systemctl status`, D-Bus, `systemd-notify`). Nothing downstream re-derives liveness from raw process
  data itself — they all ask systemd.
- **Kubernetes** separates three different questions that AgentMux currently blurs together into overlapping
  registries: **Pod phase** (`Pending`/`Running`/`Succeeded`/`Failed`/`Unknown` — coarse lifecycle),
  **container status** (`waiting`/`running`/`terminated`, per-container, with reason/exit-code detail), and
  **probes** (`livenessProbe` — is it still working, restart if not; `readinessProbe` — is it ready to serve
  traffic right now, distinct from "is it running at all"). This maps cleanly onto AgentMux's actual
  distinct questions: mechanism #1 is closest to Pod phase (coarse lifecycle), mechanism #2 is closest to
  container status (OS-process-level detail), and mechanisms #5/#6 (output-silence, PTY-idle) are exactly a
  liveness-probe concept AgentMux has organically reinvented per-controller-type instead of as one general
  primitive.
- **supervisord** uses a single explicit state machine per managed process
  (`STOPPED→STARTING→RUNNING→[BACKOFF|STOPPING|EXITED|FATAL]→...`, with `UNKNOWN` as an explicit escape
  hatch) — every state transition is an event other parts of the system can subscribe to, not a value
  polled from N different places.
- **Docker** separates container **state** (`created`/`running`/`paused`/`restarting`/`exited`/`dead`) from
  an optional **health status** (`starting`/`healthy`/`unhealthy`) computed by a user-defined healthcheck —
  the same separation of concerns as Kubernetes' phase-vs-probe split, and notably: Docker's healthcheck is
  pluggable per-container (exactly like the credential broker's per-credential `is_fresh` closure), not a
  single hardcoded heuristic.

**The convergent lesson**: every one of these systems has exactly **one** component that owns "current
status of this managed unit," computed internally from whatever signals are actually available for that
unit type (a cgroup, a healthcheck script, a probe endpoint), but exposed to every consumer as one queryable
value with one canonical vocabulary of states — never a per-consumer re-derivation from raw signals, and
never two consumers each maintaining their own partial view. That is precisely what AgentMux is missing:
mechanisms #1-#6 are the AgentMux-specific equivalents of "cgroup membership," "probe result," and "exit
code" — real, legitimate signals — but nothing plays systemd/kubelet's role of being the *one* thing that
turns them into a single canonical answer every pane queries identically. Notably, systemd's `ActiveState`
and supervisord's `STOPPED→STARTING→RUNNING→...` are both explicit **state machines with defined legal
transitions**, not just a currently-set value any subsystem can overwrite — which is exactly the shape
`agentmux-srv/src/reducer.rs` already gives window/tab/block state internally (§3.1). The industry precedent
and this codebase's own existing precedent point at the same design independently.

*(Caveat on this section: unlike the auth report's §5, which cited specific, dated GitHub issues found via
live research, this section draws on stable, long-established public documentation for systemd/Kubernetes/
supervisord/Docker's process/status models — not fast-moving specifics that need a live source check. If a
deeper pass with live citations (e.g. how a specific orchestrator's exact API evolved recently) would be
valuable, that's a reasonable follow-up, not done here.)*

---

## 5. Recommended target architecture

### 5.1 One Process Broker, exposing one canonical status per block — reducer-governed, not a plain map

The public read shape:

```
ProcessBroker::status(block_id) -> ProcessStatus {
    lifecycle: Running | Idle | Done | Error | Unknown,   // mechanism #1's role — kept as-is internally
    processes: Vec<TrackedPid>,                           // mechanism #2's role — kept as-is internally
    liveness_confidence: High | BestEffort | None,         // already exists (process_tracker::TrackingConfidence) — promote it to the top-level contract
    activity_summary: Option<String>,                      // one cache, not two generators (§5.3)
    last_output_at: Option<Instant>,                        // mechanisms #5/#6's role, generalized
}
ProcessBroker::list(filter) -> Vec<(block_id, ProcessStatus)>   // the missing batch query (§5.4)
ProcessBroker::subscribe(block_id | All) -> Stream<ProcessStatusEvent>
```

Modeled on Kubernetes' phase/status/probe separation: `lifecycle` is the Pod-phase-equivalent coarse
question (already correctly unified today via mechanism #1 — keep it), `processes`/`liveness_confidence`
is the container-status-equivalent OS-level detail (today mechanisms #2+#3+#4, unify the *read* side behind
one field), and `last_output_at`/health inference is the probe-equivalent (today mechanisms #5+#6,
generalized to every controller type instead of shell-only/subprocess-only).

This does **not** require merging every backing mechanism's internals overnight — Job-Object process-tree
tracking is legitimately different infrastructure from output-silence heuristics, exactly as the auth
report's §6.1 noted MuxBus's single-global-row shape is legitimately different from per-account CLI
credentials. It means **one component owns the read contract**, so today's `agent.tracked-blocks`
`.chain()` provider-chain pattern goes away, replaced by the broker internally reconciling #2 and #3 (or,
better, closing the controller-type coverage gap per §5.2 so there's only one real source to reconcile).

**But the read shape alone repeats the actual bug if the write side stays a plain shared `HashMap`.** Per
§3.1, the internal state backing this struct should be governed the same way `agentmux-srv/src/reducer.rs`
already governs window/tab/block state: a pure `update(&mut ProcessBrokerState, ProcessEvent, &Ctx) ->
Vec<ProcessStatusEvent>`, one Mutex-guarded dispatch point, no second code path allowed to mutate the state
directly. Today's five write-side inputs (the OS-process poller, the `reactive` self-registration calls,
`pidregistry`'s inserts, `HealthMonitor`'s inferences, `watchdog.rs`'s idle detection) each become a
`ProcessEvent` variant (`PidObserved`/`PidExited`/`ControllerStatusChanged`/`SilenceDetected`/...) dispatched
into this one function, instead of each directly writing its own `HashMap`. The reducer decides whether a
given transition is legal (mirroring `reducer/lifecycle.rs`'s `allow_register` guard — e.g. reject/log a
`Running`→`Running` "transition" that isn't actually a transition, or a `PidExited` for a PID the broker
never saw `PidObserved` for) before updating state and emitting events, rather than every caller's write
being trusted unconditionally the way `process_tracker.insert()`/`reactive_handler.register()` are today.

### 5.2 Close the controller-type coverage gap as a structural invariant

Every controller type (`shell`/`subprocess`/`persistent`/`acp`/`tsunami`) must register with the broker at
spawn. Make this part of the `Controller` trait contract itself — e.g. a required call inside `start()`'s
default implementation, or a compile-time assertion that every impl calls it — not a convention each impl
remembers to follow a few lines after `Command::spawn()`, which is exactly how `acp.rs` ended up with zero
registration today despite spawning a real child process.

### 5.3 One activity-summary cache, not two generators

Retire the fully-independent pair (push-sweep `activity_watcher.rs` vs. pull-RPC `session.rs`) in favor of
one cached-with-TTL value the broker owns: a sweep refreshes it proactively for the common case (cheap,
matches the credential broker's own proactive-refresh philosophy — §3.2), and an on-demand force-refresh
path exists for a caller that needs a guaranteed-fresh read, but both write into the **same** cache instead
of maintaining separately-tuned generation logic.

### 5.4 A real batch/list query, replacing N-way client-side fan-out

`swarm-model.ts`'s `subscribeToBlockStatuses` currently loops, issuing one `GetControllerStatus` RPC and one
WS subscription per tracked block. `ProcessBroker::list(filter)` plus one aggregate subscription
("everything matching this filter changed") replaces N round-trips and N subscription objects with one of
each — this is pure plumbing simplification, since the underlying truth (mechanism #1) is already unified.

### 5.5 Single-flight guard, borrowed directly

Per-`block_id` `AsyncMutex` with double-checked freshness, identical in shape to
`RefreshScheduler`'s proven pattern (§3.1) — collapses concurrent "what's block X's status right now"
computations (e.g. Agent-pane-mount and Swarm's periodic refresh landing near-simultaneously) onto one
actual computation instead of racing.

### 5.6 Move the finest-grained signal server-side

The activity chip's richest detail (working/using-tools/stopping) should be computed and owned by the
broker from the same signals `HealthMonitor`/`watchdog.rs` already produce, not populated client-side by
`registerActivity()` from whichever Agent pane happens to be mounted. This removes the "Swarm shows less
detail than an Agent pane happens to reveal" dependency entirely — Swarm sees the same signal whether or not
any given agent's own pane is open anywhere.

### 5.7 A documented, explicit ownership contract

Both `process_tracker`'s and `AgentProcessListCommand`'s doc comments currently describe themselves as
"swarm-owned"/"consumed by the swarm Activity tab," despite the Agent pane's `useProcessCount` hook being an
equally real, independent, direct consumer of the exact same RPC and events. That drift is itself a symptom
of not having one place whose job is to say, in writing, who this data is for. The Process Broker's own
module doc comment should state its consumers and scope explicitly — mirroring `broker/mod.rs`'s own doc
comment, which plainly states Phase A's scope (MuxBus only) and what's deliberately deferred (CLI-provider
credentials, the Armory scaffold) rather than leaving that undocumented and discoverable only by grepping
call sites.

### 5.8 What this replaces, concretely

| Today | Target |
|---|---|
| 6 independent tracking/liveness mechanisms, 2 of them unioned via a literal `.chain()` | 1 broker exposing one `ProcessStatus`, computed internally from whichever of today's mechanisms are the right tool for each field |
| `process_tracker` (and therefore the Agent pane's own badge + half of Swarm's discovery) only covers 2 of 4 controller types | Broker registration required in every `Controller` impl's `start()` |
| 2 independently-tuned activity-summary generators (push sweep vs. pull RPC) | 1 cached value, sweep-refreshed proactively + force-refreshable on demand |
| N sequential single-block RPCs + N WS subscriptions for Swarm's block list | 1 batch `list()` query + 1 aggregate subscription |
| Finest-grained activity chip is client-side-only, dependent on another pane being mounted | Broker-sourced, available regardless of what's mounted where |
| No single-flight guard on status computation — concurrent callers can race | Per-`block_id` single-flight guard, same proven pattern as the credential broker |
| Doc comments claim single-consumer ("swarm-owned") ownership of genuinely shared infra | Broker's own doc states its real, multi-consumer contract explicitly |

---

## 6. Open questions / decisions needed

1. **Sequencing**: does v1 wrap today's mechanisms behind one read API immediately (lower risk, ships
   sooner, matches how the credential broker itself started — Phase A was one consumer, MuxBus, with
   everything else deferred) and migrate write-side registration (§5.2) per controller type incrementally
   afterward, or does the broker require full registration-side migration before anyone can consume it?
   **Recommendation**: wrap-first, same sequencing as the credential broker — hide the fragmentation behind
   one API before touching how each controller type populates it.
2. **Does the broker generate activity summaries itself, or just cache/single-flight-guard whichever
   generator(s) keep producing them?** Recommendation: start as a caching/coordination layer over the
   existing two generators (lower risk), converge on one generator later once the cache contract is proven.
3. **Should `pidregistry`/`health.rs`/`watchdog.rs` be retired outright, or kept as the broker's own internal
   implementation detail?** They serve genuinely different, legitimate purposes (sysinfo needs real OS PID
   trees for CPU/mem; output-silence is a real way to infer liveness for a piped stdout stream with no other
   liveness API). Recommendation: keep them as internal signals the broker consumes, per Kubernetes'
   phase/probe separation (§4) — don't expose them as separate consumer-facing APIs, but don't delete
   working, purpose-built code either.
4. **Module location**: `agentmux-srv/src/broker/` already exists as the Credential Broker. Should the
   Process Broker live as a sibling (`broker::process`), establishing "broker" as this codebase's general
   name for "the one place that owns X and mediates access to it," or as an independent top-level module?
   Recommendation: sibling under `broker/` — the auth report itself frames "credential broker" as an
   instance of a named, precedented industry pattern (§3.3 there), and reusing the same top-level module
   signals to future readers this is the same architectural move made twice on purpose, not a coincidence of
   naming.
5. **Scope of "process" for v1**: this report focused on agent-CLI-driven blocks (the reported symptom).
   Drone block subprocesses, shell/Terminal panes, and cron-triggered runs all have their own
   process-lifecycle concerns not fully traced here. Recommendation: design the broker's interface
   generally enough to cover all of them from day one (same reasoning as the auth report's own §7.1 —
   retrofitting a unified interface onto an agent-CLI-only design later repeats the exact mistake this
   report is trying to avoid), even if migrating drone/cron onto it is phased later.
6. **Naming collision with the existing `reducer` module**: `agentmux-srv/src/reducer.rs` already governs a
   different, adjacent concept (`state.rs`'s IPC-client `ProcessRecord`/`ProcessState` — AgentMux's own
   host/srv/launcher/tool processes registering over the named pipe, not agent-spawned subprocesses). A
   Process Broker built as a reducer (§3.1, §5.1) needs a name distinct enough not to be confused with that
   existing one despite being the same pattern intentionally reapplied — e.g. `broker::process` for the
   module path (per open question 4) with its internal reducer function named something like
   `process_broker::update` or `agent_process_reducer::update`, not a bare `reducer::update` that would read
   as extending the existing Phase E reducer's `Command` enum. Worth an explicit decision before
   implementation starts, not left to whoever writes the first line of code.

---

## Appendix: research method

This report synthesizes six parallel, read-only codebase investigations (auth broker precedent, the
`process_tracker` registry, `blockcontroller`'s `pidregistry`/`health`/`watchdog`, the standalone
`broker/scheduler.rs` module, the Agent pane's full data-flow trace, and the Swarm pane's full data-flow
trace), each independently file:line-citing its claims, cross-checked against each other for consistency
(all six independently converged on the same `agent.tracked-blocks` `.chain()` mechanism as the direct cause
of the reported symptom, without being told about each other's findings), plus direct reads of
`docs/specs/REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md`, `agentmux-srv/src/broker/mod.rs`, and
`agentmux-srv/src/server/app_api/agent_io.rs` to verify the most load-bearing claims firsthand rather than
taking any single research pass's word for it.
