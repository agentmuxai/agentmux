# SPEC: Jobs and templates: one ledger for everything that runs by itself

**Date:** 2026-10-24
**Status:** active — approved by the operator 2026-10-23 (answers in §10). P1 PR 1 (run ledger and cron fixes) in #5390; P1 PR 2 (the Warden Jobs tab, `jobs.*` RPCs, `JobsChanged`, Pause all) in #5414; P1 PR 3 (failure incidents: one Kanban task per incident for the owning leader, the note to that leader, `NotifyKind::JobFailed`, "recovered" on the next success) in #5416; P3 PRs 6–10 (job definitions, run policy, caps, signed template packages, the ten starters, New job and Edit) in #5439, #5448, #5467, #5434, #5457 and #5488; P2 PR 4, first part (the event schema, signed events into the ledger, pinned reporters, the MuxBus frame) in #6055; cloud manifests in #6127, the watchdog and cloud incidents in #6130, the reporters RPCs in #6159, and cloud jobs in `jobs.list` with the dropped-event counts in #6164; the Jobs tab's cloud rows (read-only, Run now as a link or a note), the dropped-event line and the Cloud reporters panel are in this PR.
**Trigger:** The operator, 2026-10-23, asked how scheduled and automated agent work (cron jobs, and jobs started by an event) should live in AgentMux, whether Drone is the place for it, and for "a good set of templates that are easy to build off of … for common needs, cron and automated". An internal research report answered (an inventory of what runs today, what Drone is, current practice elsewhere, a phased plan, ten starter templates). The operator approved it the same day ("yes proceed with all") with six decisions, listed in §0. This spec covers the report's phases P1–P3 and leaves hooks for P4 and P5.
**Related:** `SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md` (how cron and loops were built); `SPEC_WARDEN_KANBAN_2026_10_20.md` (the task board failed runs file into); `SPEC_WIDGET_SHARING_2026_10_10.md` (the signing and catalog scheme templates reuse); `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` (the notification router); `SPEC_AGENT_WORKERS_2026_10_15.md` (spawned workers); `SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md` (how agents use secrets); `SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md` (the turn ledger runs read their cost from); `SPEC_SWARM_SUPERVISOR_2026_10_16.md`; `SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md`; `SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md` (draft: who an agent's lead is).

## 0. Summary

- **A job is something that runs by itself:** a trigger, an action, a policy and caps. A **run** is one time it was meant to run, including the times it didn't (skipped, missed), each with a reason.
- **One run ledger under everything** (P1). Schedule's cron fires, Drone runs, and later the cloud jobs that report in all write rows to one table, `db_job_runs`, in the global identity store, next to the Kanban's work queue (§3.1).
- **Warden gets a Jobs tab** (P1, operator decision 1). Every job, local and cloud: next run, last outcome, cost, history; **Run now**, **Pause**, and **Pause all jobs** (§3.3).
- **A failed run raises one alert per incident** (operator decision 5): a Kanban task for the job's owning leader, an `[AgentMux]` note to that leader, and a notification to the operator. Further failures of the same job add to the open task instead of raising new alerts (§3.4).
- **Cloud jobs keep running in the cloud and report in over MuxBus** (P2, decisions 3 and 4). The run-event schema is public (§4.1); the events themselves stay private to the account. A **watchdog** turns an expected run that never reported into a `missed` row and an alert (§4.4). No cloud runner yet; revisit after P2.
- **Job definitions and templates are signed data** (P3), like widgets: JSON validated by a published schema, signed with the widget scheme's Ed25519 publisher keys, installed only after the operator approves (§5).
- **Caps are on by default and low** (decision 2): **$5 per run, $20 per job per day, $50 per account per day**, adjustable. A run starts only if the caps leave room for its whole per-run cap; one that reaches its cap mid-run is stopped (§5.3).
- **Ten generic starter templates** ship in agentmux, under `templates/jobs/`; templates for our own jobs go in a private catalog (§5.5).
- **Infrastructure deploys run one at a time** (decision 6): a job that deploys belongs to the `deploy` concurrency group, and the scheduler starts at most one run per group at a time (§5.2).

## 1. Today

Measured on `main` @ `14afc1646`.

### 1.1 Inside AgentMux

| Mechanism | Trigger | What it keeps | Where it fails silently |
|---|---|---|---|
| **Schedule, `cron`**. MCP `crates/mcp/src/tools/schedule_tool.rs` routes to `cron.rs` (`POST/GET /agentmux/cron`, `DELETE`/`PATCH /agentmux/cron/:id`). srv: `server/cron.rs`, `backend/cron/mod.rs`, `backend/storage/cron.rs` | 5-field cron, **UTC only** (`cron` 0.12), one-minute granularity | `CronJob`: `last_fired`, `fire_count`, `max_fires`, `max_age_secs`, `created_by(_uid)`, `target(_uid)`. Table `db_cron_jobs` in the shared store | A fire to a stopped agent isn't held: `hold_applies` excludes `source_agent == "cron"` (`server/reactive.rs:1205`). `fire()` records the fire even when delivery failed (`cron/mod.rs:304-310`). No per-fire history. Nothing prevents a double fire when two instances share the store |
| **Catch-up** | Startup | — | One fire if the slot after `last_fired` has passed (`should_catchup`, `cron/mod.rs:349-363`); a job that never fired gets none |
| **Schedule, `every`** (loops) | Interval, from the agent's own `agentmux-mcp` process | Nothing | Ends with the session; can't be paused |
| **Swarm's Cron bucket** | — | Read-only rows (`CronBucket`, `frontend/app/view/swarm/swarm-view.tsx:940`) from `cron.ListActive` | No create, edit, pause or delete in the UI |
| **Drone** | **Manual Run only** (`rundrone`, `server/drone_handlers.rs:192`) | `db_drone_runs` in the per-channel `objects.db`; `block_states` JSON. An Agent block's `cost_usd` is inside its output only (`drone/executor/blocks/agent.rs:120-130`) | A run interrupted by a restart stays `running` (`drone_handlers.rs:222-226`). No pruning, no cancel |
| **Kanban / work queue** | Pull, gated by `not_before` (`backend/storage/work_queue.rs`, whose header calls it `db_cron_jobs`' sibling with a readiness trigger) | Task rows, cards, events | — |
| **Swarm Supervisor**, worker notes, the runtime watchdog | A finished turn; worker events; a 60 s timer | Block meta; logs | Continues existing work only; the watchdog stops agents on runtime or idle time, never on cost (`blockcontroller/watchdog.rs`) |

**Cost is not stored anywhere.** srv reads the provider CLI's own figures: the turn ledger (`TurnLedger`, `backend/blockcontroller/health.rs:208`) sums each pass's `total_cost_usd`, output tokens and steps, and publishes them as the `agentturn` event; `history_usage.rs` measures disk, not tokens. Only Claude's `result` frame carries a cost today. The account a pane ran on is in its meta, `agent:run_account` (`server/app_api/agent_spawn.rs:459`). There is no spending limit in the code.

### 1.2 Outside AgentMux

Some jobs run outside AgentMux, in a project's cloud and CI: scheduled reports, services triggered by pull requests, nightly builds and checks. They must run while the operator's machine is off. AgentMux doesn't show them: their history lives in each service's own console, the CI tab, email and PR comments.

### 1.3 What this adds up to

Nothing lists every job in one place. A run that didn't happen leaves no trace. A failure reaches whoever happens to look. And an unattended agent run has no spending limit at all.

## 2. Concepts

| Term | Meaning | In the code |
|---|---|---|
| **Job** | Something that runs by itself: a trigger, an action, a policy, an owner. Local (AgentMux runs it) or cloud (it runs elsewhere and reports in) | P1: a cron job, a Drone definition, a cloud job manifest. P3: a `db_jobs` row (§5.1) |
| **Run** | One intended run of a job, whether it ran or not. It has a state (scheduled, skipped, pending, running, finished, missed, lost), an infrastructure status, an outcome, a cost | A `db_job_runs` row (§3.1) |
| **Trigger** | When a job runs: `cron` (with a time zone in P3), `every`, `once`, `manual`; `event` is reserved for P5 | P1: the cron expression; P3: `trigger` in the definition |
| **Action** | What a run does: **message** an existing agent (today's cron), file a **task**, **spawn** a worker with a brief, run a **Drone** graph (P4) | `fire()` today; `action` in P3 |
| **Policy** | How it runs: overlap, catch-up, timeout, retries for infrastructure failures, concurrency group, where results land | `policy` in P3 |
| **Cap** | A spending limit: per run, per job per day, per account per day (§5.3) | Settings plus `policy.caps` |
| **Outcome** | What the run concluded, separate from whether it ran: `ok`, `findings`, `failed`, `unknown`. A green run with no verdict is `unknown`, not `ok` | Reported by the run (§3.2) |
| **Incident** | Consecutive failures of one job, from the first to the next success. One incident, one alert | `incident_task` on the job (§3.4) |
| **Template** | A signed, versioned, parameterized job definition to create jobs from | A package with `template.json` (§5.4) |
| **Owner / owning leader** | The agent responsible for a job, and the leader alerts go to (§3.4) | `owner` on the job; resolved through seats or `agent:worker_of` |

## 3. P1: the run ledger and the Warden Jobs tab

### 3.1 Storage

**Decision: one table, `db_job_runs`, in the global identity store** (`~/.agentmux/shared/identity-store.db`, `state.identity_store`).
- That is where the Kanban's work queue, cards and claims live, so a run and the task it filed are in one database. It is global, so every channel and version on the machine sees one ledger, and one lease (below) stops two instances from both firing a job.
- It is added as a module-level `SCHEMA` run on every open, as `work_claims` and `work_board` are (`backend/storage/store.rs:251-254`), so older builds aren't locked out by a schema version bump (`work_claims.rs:44-48`).
- `db_cron_jobs` stays in the shared store. In P3 cron jobs become `db_jobs` rows (§5.1), which live beside it in the shared store (not the identity store): a run's job is found in the store the scheduler reads, and a dev build that opens its own work store gets an empty `db_jobs` rather than a copy.

**Fields:**

| Field | Meaning |
|---|---|
| `id` | UUID. Also the delivery's `request_id`, so a delivery is never repeated |
| `job_id`, `job_source` | The job, and its kind: `cron`, `drone`, `cloud`, later `job` |
| `job_name` | The name at the time of the run, so history reads after a rename or delete |
| `template_id`, `template_version` | P3; empty before |
| `trigger` | `schedule`, `catch_up`, `manual`, `retry`; later `event` |
| `scheduled_for_ms` | The slot it was meant for (empty for manual) |
| `triggered_by` | `scheduler`, the operator, or an agent uid (Run now) |
| `attempt` | 1, then 2… for infrastructure retries (P3) |
| `state` | `scheduled`, `skipped`, `pending` (catch-up `latest`'s one coalesced fire, waiting for its agent), `running`, `finished`, `missed`, `lost` |
| `reason` | Why it was skipped, missed or stopped (and, for `pending`, `superseded` or `expired` once a newer fire replaced it or the next fire time passed): `paused`, `paused_all`, `overlap`, `group_busy`, `job_cap`, `account_cap`, `run_cap`, `app_not_running`, `target_missing`, `target_stopped`, `timeout`, `no_report` |
| `infra` | `ok`, `error`, `timeout`, `over_budget`, `interrupted` |
| `outcome` | `ok`, `findings`, `failed`, `unknown` |
| `summary` | At most 300 characters, from the run's verdict or the cloud event. Shown as data, never as an instruction |
| `target_agent_uid`, `block_id`, `turn_id` | Who ran it, and the turn it started or joined |
| `account_id`, `provider` | From the pane's `agent:run_account` when the run starts |
| `started_at_ms`, `ended_at_ms` | |
| `cost_usd`, `cost_known`, `cost_partial`, `output_tokens`, `steps` | From the turn ledger. `cost_known = false` when the provider reports no cost; `cost_partial` when the fire joined a turn already running, so the cost includes other work |
| `links` | JSON: the transcript (agent and session), a Drone run (`drone_run_id` and channel), the Kanban task number, a cloud job's links |
| `skip_count`, `last_skip_at_ms` | Consecutive skips for the same reason are one row with a count, so a paused one-minute job doesn't write 1,440 rows a day |
| `reporter` | For cloud rows: the fingerprint of the key that signed the event (§4.2) |
| `recorded_by` | The channel and version that wrote the row |

Index on `(job_id, started_at_ms)`, and a **unique index on `(job_id, scheduled_for_ms)` for scheduled runs**: before firing, the scheduler inserts the row; when two instances share the job, the second insert fails and that instance doesn't fire. That is the lease that fixes the possible double fire.

**Retention: none.** Run history is kept forever (operator decision, 2026-10-23: AgentMux keeps history; a row is small). There is no sweep, by age or by count. `db_drone_runs` keeps its detail too; the ledger row links to it.

### 3.2 What writes rows

- **Cron fires** (`backend/cron/mod.rs`). The scheduler inserts a `scheduled` row before it fires, then:
  - delivers with the row's id as `request_id`. **Cron fires are never held** (`SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` §2.1 item 4: an every-5-minutes job whose agent is down for a day would otherwise replay 288 turns), so `hold_applies` keeps excluding `source_agent == "cron"`. A fire to a stopped agent becomes `missed` (`target_stopped`), and what happens next is the job's catch-up policy (§5.2): `skip` leaves it missed; `latest` keeps one coalesced pending fire;
  - calls `cron_record_fire` only for a run that was delivered, so `fire_count` stops counting fires nobody received. An undelivered fire raises the job's `missed_count` and sets `last_missed_at` and `last_miss_reason` instead; Swarm shows "missed N". Those counters and the delivered-or-missed decision are AgentA's cron fix (platform lane, agreed with AgentY on 2026-10-24); the ledger writes its row from that decision, not from a second copy of it;
  - when the target can't be resolved at all, writes `skipped` (`target_missing`);
  - on startup, writes one collapsed `skipped` row (`app_not_running`, with `skip_count`) for the slots missed while AgentMux was off, beside the one catch-up fire `should_catchup` already makes.
- **The run's turn.** A delivered fire starts or joins a turn whose trigger is `TriggerKind::Schedule` (`health.rs:68-86`). srv follows that block's `agentturn` events: the row becomes `running` when the turn starts and `finished` when it ends (`TurnEnd::Completed`), or `infra: interrupted` when the agent's process exits (`TurnEnd::Exited`). Cost, output tokens and steps come from the ledger's sums.
- **A run delivered to an agent in another channel** (#1988). Each instance publishes turn events to its own broker, so the instance that fired can't follow a pane another channel holds. The receiving instance does: when a delivery forwarded to it carries a `request_id` that names another channel's open cron run, it marks the run `running` with its own pane's account, writes `links.followed_by_channel` (and its process id), and follows the turn as above, before its answer goes back (`backend/jobs/remote.rs`). The firing instance leaves the run `running` and only watches the row: a run nobody followed (a receiver on an older build) is closed after the no-turn timeout, `infra: ok`, cost unknown; a run whose following process is gone is `interrupted`, as is one open when the following instance restarts. Every close is conditional on the row being open, so only the first side to write the end changes it.
- **The verdict.** The brief srv sends with a fire ends with one line: the run id, and how to report. The agent reports with `Schedule` action `report {run_id, outcome: ok|findings|failed, summary, links}`. Without a report, the outcome stays `unknown`. A green turn is not a success: the run didn't say.
- **Drone runs** (`server/drone_handlers.rs:192`). `rundrone` writes a `manual` row linked to the Drone run; its end sets the state, and the cost is the sum of the Agent blocks' `cost_usd`. At startup, Drone rows still `running` from before the restart become `infra: interrupted`, in both tables: the cleanup `drone_handlers.rs:222-226` asks for.
- **Run now** writes a `manual` row and fires at once, through the same path.
- **Loops** don't write rows. They belong to one session and end with it (§10 Q1).

### 3.3 The Warden Jobs tab

A new Warden section, `jobs`, after Kanban (`frontend/app/view/warden/warden-sections.ts`: the id union, the label and an item with icon `calendar-check`; a `<Lazy id="jobs">` in `warden-view.tsx`; the pinned label list in `warden-view.test.tsx`). The title reads "Jobs · N failing" when any job has an open incident, the way Kanban's reads "N need you" (`warden-model.ts:71-77`). `openWarden("jobs")` opens it (`open-warden.ts`).

**The list.** One row per job, local jobs first, then cloud jobs (P2), each group sorted by next run.

| Column | Shows |
|---|---|
| Job | Name; owner agent; a `cloud` badge for cloud jobs |
| Trigger | In words ("Mondays 06:00", "every 15 min"), and the next run ("in 3 h") |
| Last run | Outcome badge (ok, findings, failed, unknown, missed) and when |
| Recent | The last 14 runs as a strip of small marks, skipped ones hollow |
| Cost | Today and 7 days, against the job's daily cap; "—" when the provider reports none |
| State | Active, Paused, Over cap (today), Paused by Pause all |

**A job's history** (click a row): its runs, newest first, with the slot, start, duration, state and reason, outcome and summary, cost and account, and links (the agent's transcript in Warden's History section, the Drone run, the Kanban task, a cloud link).

**Actions:**

| Action | Who | Does |
|---|---|---|
| **Run now** | Operator; an agent for jobs it created or owns (§7) | A `manual` run, subject to the caps and the overlap policy |
| **Pause / Resume** | Same | Sets the job's `enabled`; fires while paused are collapsed `skipped (paused)` rows |
| **Delete** | Same | Removes the job; its runs stay in the ledger until the sweep |
| **Open owner** | Anyone | Opens the owner's pane |
| **Pause all jobs** | **Operator only**, in the tab's header | A global switch, `jobs:paused_all`, stored in the identity store so every instance honours it. While it is on, a banner says so, every fire is a collapsed `skipped (paused_all)` row, Run now is refused, and the Schedule tool says the jobs are paused by the operator. Only the operator turns it off. Cloud jobs keep running (AgentMux doesn't control them) and keep reporting |

**RPCs** follow the Kanban's shape (`server/app_api/kanban.rs`): `jobs.list`, `jobs.runs {job_id, limit, before}`, and `jobs.act {action: run_now | pause | resume | delete | pause_all | resume_all, job_id?}`. `pause_all` and `resume_all` are accepted only from the operator's own client, never from a `Caller::Agent`. A `JobsChanged` event (beside `CronChanged`, `backend/mps_events.rs:255`) refreshes the tab. Swarm's Cron bucket gets an "Open in Jobs" item and otherwise stays as it is.

**MuxAPI.** `Schedule` keeps its name and gains: `list` returns each job's last run and next fire; `history {id, limit}`; `run_now {id}`; `report {run_id, …}` (§3.2). The tool's schema in `crates/mcp/src/tool_schemas.rs:201` and `schedule_tool.rs`'s `ACTIONS` change with it.

### 3.4 When a run fails

**A run fails** when its infrastructure status is `error`, `timeout`, `over_budget` or `interrupted`, its state is `missed` or `lost`, or its outcome is `failed`. `findings` is not a failure, and neither are skips the operator caused (`paused`, `paused_all`). A skip because a cap was reached raises one alert per job per day (§5.3).

**The first failure after a success opens an incident** and alerts three ways at once:

| To | How | In the code |
|---|---|---|
| **A Kanban task** for the owning leader | One task, titled "Job failed: <name>", with the run's state, reason, summary and a link to its history. `kind: "job-failure"`, `target_agent_uid` the owning leader, `requested_by_kind: other`, `filed_by: "jobs"`, `review: false` (the operator reviews by using the app, not in the Kanban's Review column; operator decision, 2026-10-23) | The same store calls as `kanban.act` `create` (`app_api/kanban.rs:254-308`): `work_queue_enqueue` and `work_card_create` |
| **The owning leader** | An `[AgentMux]` note: "Job "<name>" failed at 06:02 (timeout). Kanban #123." It starts a turn | `deliver_system_note` (`backend/reactive/handler.rs:1453`); the summary goes through `quote_in_system_note` (`reactive/sanitize.rs:736`) |
| **The operator** | A notification: "Job failed: <name>", fixed text plus the reason. Clicking it opens Warden → Jobs at the job. Also the tab's "N failing" count | A new `NotifyKind::JobFailed` (`backend/notify/policy.rs:23`), Attention priority, group key `job:<job_id>`, setting `notify:os:jobfailed`, fixed text, sent from srv the way `CloudSignedOut` is (`router.rs:850`), which is about no agent either |

**While the incident is open** (its task isn't done or cancelled), further failures add an event and a line to that task's card ("failed again at …, 3 in a row") and send nothing else. The work queue has no dedupe key (`work_queue.rs:217-224`; both create paths make a fresh id), so the job keeps its incident's task id itself. **The next success closes the incident**: the card gets "recovered at …"; the task stays for its owner to finish, since only a person accepts a reviewed task.

**The owning leader** is resolved when the alert is sent:
1. the job's `owner` (P1: the cron job's `created_by_uid`; P3: the definition's `owner`);
2. if the owner has a seat with a lead (the draft seats spec), that lead; else, if the owner is a spawned worker, its leader (`agent:worker_of`, `agent_spawn.rs:41-58`);
3. otherwise the owner itself.

If none resolves (the agent was deleted), the operator alone is alerted and the task goes to the backlog unassigned.

## 4. P2: cloud jobs report in

**What it is.** A cloud job (a scheduled container task, a serverless function, a CI workflow) posts a small signed event when a run starts and when it finishes. AgentMux writes the events into the same ledger, shows the job in the Jobs tab read-only, and alerts on its failures the same way (§3.4). AgentMux doesn't host or schedule cloud jobs (operator decision 4).

### 4.1 The run event (public schema)

Published as `schema/agentmux-job-event.v1.schema.json`. One JSON object, at most 8 KB:

```
{
  "schema": "agentmux.job-event/1",
  "event_id": "uuid",                 // unique; the ledger ignores a repeat
  "job_id": "publisher.name",         // must match a cloud job manifest (§4.3)
  "run_id": "string",                 // the same for a run's start and finish
  "phase": "started" | "finished" | "heartbeat",
  "at": 1792800000000,                // ms epoch, when it happened
  "scheduled_for": 1792800000000,     // optional: the slot, for the watchdog
  "trigger": "schedule" | "event" | "manual" | "retry",
  "attempt": 1,
  // finished only:
  "infra": "ok" | "error" | "timeout",
  "outcome": "ok" | "findings" | "failed" | "unknown",
  "summary": "string, at most 300 characters",
  "duration_ms": 61000,
  "cost_usd": 0.05,                   // optional
  "tokens": { "input": 0, "output": 0 },  // optional
  "findings": 3,                      // optional count
  "links": [ { "label": "Report", "url": "https://…" } ]   // https only, at most 5
}
```

A `started` event makes a `running` row. `finished` closes it, and creates a closed row on its own if the `started` event never arrived. `heartbeat` only pushes the run's lost deadline back (§4.4). A late `finished` event closes a row the watchdog marked `missed`, and the row reads "late".

### 4.2 Transport, signature and trust

- **Transport:** MuxBus, the account's existing channel for cloud notices (`muxbus/cloud_subscriber.rs`). Job events are a kind of their own: srv's subscriber hands them to the ledger, and they are **never injected into an agent**. Events sent while the machine is off arrive when it is back; the watchdog's grace and "late" handle that.
- **One ledger, many channels.** Each channel's subscriber may pull the same event; `event_id` is unique in the global ledger, so it is written once.
- **Signature:** each event is signed with Ed25519 by its **reporter**, over `agentmux-job-event-v1\0<job_id>\0<run_id>\0<event_id>\0<SHA-256 of the JSON body>`, the same construction as widget signatures (`backend/widget_signature.rs:23-43`). Events older than 7 days are refused.
- **Reporters are pinned, not built in.** The operator decides which jobs trust a reporter (approving a cloud job manifest that names it pins it, §4.3); pinning or forgetting one by hand is open to agents over RPC, recorded as the caller, because a pin alone lets no event in: an event is accepted only when its key is pinned **and** equals the key in an approved template's manifest, and approval stays the operator's. The worst a stray forget does is silence a job, which the watchdog reports as misses. The Jobs tab's Cloud reporters panel lists each reporter's name, key fingerprint and approved jobs (the widget scheme's format: SHA-256, first 10 bytes, base32, groups of four). A reporter is added by pasting its public key, or by approving a cloud job manifest that names it (§4.3). No key for anyone's private jobs is compiled into the public app.
- **What fails is dropped:** an unsigned event, a bad signature, an unknown reporter, or a `job_id` that no approved manifest lists. Each is counted in a diagnostic line in the Jobs tab ("3 events dropped: unknown job"), never shown as a run.
- **Event text is data.** `summary` and link labels are shown as text, quoted (`quote_in_system_note`) when they go into a note or task, and never become an agent's instruction. Links must be `https`.

**What stays private.** The schema is public. The events (job names, summaries, links, costs) travel only between the account's own reporters and its own AgentMux instances over its MuxBus account, and are kept only in the local ledger. Nothing about a particular project's cloud jobs goes into this repo: their manifests live in a private catalog (§5.5).

### 4.3 Cloud job manifests

A cloud job appears in the Jobs tab because a **manifest** says it exists. A manifest is a template package whose `kind` is `cloud` (§5.4), so it is signed, catalogued and approved the same way:

| Field | Meaning |
|---|---|
| `id`, `name`, `owner` | As for any job; `owner` is an agent name or uid on this machine |
| `reporter` | `{name, key}`: the public key (base64) whose events count for this job, and the name it is pinned under in Settings. Approving the manifest pins the pair; a name already pinned to another key stops the approval until the operator forgets that key |
| `expect` | The expected schedule: `{cron, tz, graceMinutes}`; or `"none"` for jobs started by events |
| `maxDurationMinutes` | After a `started` event, how long before the run is `lost` (1 to 10080) |
| `runNow` | Optional: `{link}` (an https page that starts the job, opened in the browser) or `{askOwner: "<instruction>"}` (sent to the owner agent as a message). AgentMux holds no cloud or CI credentials, so it never starts a cloud job itself |
| `aggregate` | `true` for a service that reports a daily summary instead of each run (runs per day, failures) |

### 4.4 The watchdog

A scheduler can't report its own outage, so a separate check compares what should have run with what the ledger has. srv runs it every 5 minutes, and once at startup.

| Case | Becomes | Alert |
|---|---|---|
| A cloud job's expected slot plus its grace has passed with no event for it | A `missed` row (`no_report`) | Yes (§3.4) |
| A `started` run with no `finished` or heartbeat within `maxDurationMinutes` | `lost` | Yes |
| A local job's slot passed while AgentMux was running, and no row exists (the scheduler's task died) | `missed` | Yes |
| Local slots missed while AgentMux was off | One collapsed `skipped (app_not_running)` row | No: the operator turned it off, and the catch-up already ran |

A late event turns a `missed` row into a late run. Several misses of one job are one incident, so one task and one alert. The watchdog lives in srv, next to the cron scheduler (`backend/cron/`), not in it: if the scheduler's loop stalls, the watchdog still sees the missing rows.

## 5. P3: job definitions, caps and templates

### 5.1 The job definition

A `db_jobs` table replaces `db_cron_jobs`, in the same store: the shared store (`store.db`), with its DDL and additive columns in both schema runners next to `db_cron_jobs`, and no schema version bump. `db_cron_jobs` itself is left as it was for older builds. A one-time data migration (`crates/srv/src/migrations/`, `MigrationScope::Global`, run against the shared store only) copies every cron job into it as a job with a `cron` trigger in UTC and a `message` action, keeping its id, so Swarm, Schedule and the ledger don't notice. `POST/GET/DELETE/PATCH /agentmux/cron` stay as aliases of the new `/agentmux/jobs` routes for one release.

Published as `schema/agentmux-job.v1.schema.json`:

```
{
  "jobVersion": 1,
  "id": "uuid",
  "name": "Weekly docs drift",
  "owner": "<agent uid>",
  "created_by": "<agent uid> | operator",
  "template": { "id": "agentmux.docs-drift", "version": "1.0.0", "hash": "…" },  // optional
  "params": { "repo": "…", "doc_globs": ["docs/**"] },
  "trigger": { "kind": "cron", "cron": "0 6 * * 1", "tz": "America/New_York" }
           | { "kind": "every", "seconds": 3600 }
           | { "kind": "once", "at": 1792800000000 }
           | { "kind": "manual" }
           | { "kind": "event", … },            // reserved, P5 (§6)
  "action": { "kind": "message", "to": "<agent uid>", "brief": "…" }
          | { "kind": "task", "title": "…", "payload": "…", "target": "<agent uid>" }
          | { "kind": "spawn", "leader": "<agent uid>", "brief": "…", "model": "…", "account": "…" }
          | { "kind": "drone", "drone_id": "…" },  // reserved, P4 (§6)
  "policy": {
    "overlap": "skip" | "buffer_one" | "cancel_other",   // default skip
    "catch_up": "skip" | "latest",                       // default skip (§5.2)
    "timeout_minutes": 60,
    "retries": 0,                 // infrastructure failures only, at most 3
    "max_turns": 200,             // passed to the CLI where it has the flag (spawn)
    "group": "deploy",            // optional concurrency group
    "caps": { "run_usd": 5, "day_usd": 20, "messages_per_day": 24 },  // messages_per_day: `message` actions to another agent
    "approval": "none",           // reserved, P4
    "results": ["ledger", "task_on_failure"]
  },
  "outputs": ["report_file", "kanban_task", "message:owner", "pull_request"],
  "secrets": [ { "name": "SCANNER_TOKEN", "vault": "<vault entry id>" } ],
  "enabled": true
}
```

**Triggers.** Cron gains a time zone (IANA; a time-zone crate is added, since `cron` 0.12 is UTC only); daylight-saving gaps fire once, at the next valid time. Each job gets a fixed offset of up to 2 minutes, derived from its id, so jobs due at the same minute don't all start at once. Minimum interval for an unattended agent action: 15 minutes.

**Actions** (all four reuse existing paths):

| Action | Runs through | Run ends when |
|---|---|---|
| `message` | Today's cron delivery (`install_cron_delivery`, `bootstrap/delivery.rs`) | The turn it started ends (§3.2) |
| `task` | `work_queue_enqueue` and `work_card_create`, as `kanban.act` does. The task carries `job_run` in its card | The task is enqueued (outcome `ok`); the work itself is the task's |
| `spawn` | `POST /api/v1/agent/spawn` (`agent_spawn.rs`) with the job's leader as the leader: `MAX_LIVE_WORKERS` (3) and "workers can't spawn" apply. The brief is sent as today (`agents_tool.rs:114-119`) plus the run line | The worker's first turn ends; the worker then exits, unless `keep_worker` is set |
| `drone` | P4 | — |

**Policy:**
- **Overlap `skip`:** if this job's previous run is still running, the new slot is `skipped (overlap)`. `buffer_one` keeps one waiting slot; `cancel_other` interrupts the old run (as caps do, §5.3).
- **Catch-up, per job: `skip` (the default) or `latest`.** It covers every slot that wasn't delivered: AgentMux was off, or the target agent was stopped.
  - `skip`: the slot stays `missed` (or `skipped (app_not_running)`), and nothing is made up.
  - `latest`: one coalesced pending fire, for the newest missed slot. Each newer missed slot replaces it, and it expires at the job's next fire time, so it can never pile up. It is delivered when the agent is back (or at startup), and the brief gets `scheduled_for` and `started_at`, so the agent can tell its work is late. A job that never fired gets it too, if it was created before the missed slot.
  - Cron jobs that exist before P1 are migrated with `latest`, so they keep today's one startup catch-up fire (`should_catchup`). New jobs get `skip` unless they say otherwise.
- **Timeout:** the run is interrupted at `timeout_minutes` (`infra: timeout`).
- **Retries:** only for infrastructure failures (the agent never started, the process exited, a provider API error the failure classifier marks transient, `agents/failure.rs`), with backoff of 1, 5 and 15 minutes. A run whose agent concluded `failed` is never retried; it alerts.
- **Concurrency groups:** at most one running run per group across all jobs. **`deploy` is a built-in group**: any job whose template declares a deploy output is placed in it, and its runs wait (`state: scheduled`, `reason: group_busy`) until the running one finishes (operator decision 6).
- A global cap of 3 unattended agent runs at once protects the providers' rate limits; runs above it wait.

**Dry run** (in the Jobs tab and `Schedule` `dry_run`): it shows the resolved brief, the next five fire times in the job's time zone, the caps that would apply, the outputs and secrets the job declares, and which Vault grants are still missing. For an existing agent, AgentMux can't make a turn read-only, so a dry run doesn't start an agent before P4 (§6).

### 5.2 Who creates jobs

- **The operator**, in the Jobs tab: New job (blank, or from a template), Edit, Clone.
- **An agent**, with `Schedule` `create`, as today, now with `template`, `params` and `policy`. The rules are in §7.

### 5.3 Caps

**Defaults** (operator decision 2), in Settings → Jobs, on by default:

| Cap | Default | Counts | Setting |
|---|---|---|---|
| Per run | **$5** | The run's `cost_usd` | `jobs:cap:runusd` |
| Per job per day | **$20** | The job's runs that day | `jobs:cap:jobdayusd` |
| Per account per day | **$50** | Every job run on that account (`account_id`) that day | `jobs:cap:accountdayusd` |

A day is the operator's local calendar day. A job's `policy.caps` may set lower figures. Only the operator raises a cap, in Settings or on a job in the Jobs tab; an agent's `create` with higher figures is clamped to the defaults, and the reply says so.

**Before a run starts:** it starts only if its job's spend today plus its per-run cap fits the job's daily cap, and the same for its account's daily cap. Otherwise it is `skipped (job_cap)` or `skipped (account_cap)`. This reservation means caps hold even if every run spends its whole per-run cap. For a `message` action, the account is the target pane's `agent:run_account`; for `spawn`, the account `choose_account` (`agent_spawn.rs:349`) picks.

**During a run:** srv follows the run's turn ledger. When its `cost_usd` reaches the per-run cap, srv interrupts the turn through the controller's existing interrupt (the Claude control request in `blockcontroller/persistent/mod.rs:104-108`; `interrupt_turn` for app-server providers, `app_server_protocol.rs:664`). The run ends with `infra: over_budget`, which is a failure (§3.4). A spawned worker is also stopped (`sagas::agent_teardown::stop_now`).

**At a daily cap:** later runs that day are collapsed `skipped` rows. The job isn't paused, and runs resume the next day. The first such skip each day raises one alert (a note to the owning leader, a notification to the operator; no Kanban task unless the job also failed). The Jobs tab shows "Over cap (today)".

**Messages to another agent.** A `message` job that targets an agent other than its owner needs no approval (operator decision, §10 Q5), but it has a per-job rate cap: at most `messages_per_day` deliveries a day, default **24** (Settings → Jobs, `jobs:cap:messagesday`). Beyond it, fires are `skipped (message_cap)` and the first one each day raises the daily-cap alert. Only the operator raises it. A job whose every fire messages another agent can't flood that agent's conversation however its trigger is set.

**Providers that report no cost.** Only Claude's `result` frame carries a cost today (`PassStats::from_result_frame`, `health.rs:318-326`). A run on another provider has `cost_known = false`, counts $0 toward the dollar caps, and is bounded by its `timeout_minutes` and, where the CLI supports it, `max_turns`. The Jobs tab marks such jobs "cost not reported: only the timeout limits it", and creating one asks the operator to confirm (§10 Q3).

**Scope.** Caps cover runs AgentMux starts. Cloud jobs report their cost (shown in the tab and the day's totals), but AgentMux can't stop them, so their cost doesn't count toward the account cap. Interactive work isn't counted: AgentMux doesn't store per-account spend today (§10 Q2).

### 5.4 Templates: signed data

**A template package** is a folder named like its id, `<publisher>.<name>/` (the widget id rules: lowercase letters, digits and `-`, one dot), holding:
- `template.json`: `templateVersion: 1`, `id`, `name`, `version` (semver), `kind` (`job` or `cloud`), `description`, the `params` schema (JSON Schema subset: string, number, boolean, enum, string list, and `credential`, a Vault entry reference), the trigger defaults, the action (an agent brief, with `{{param}}` placeholders, in `brief.md`), the policy defaults, and the declared `outputs` and `secrets`;
- `brief.md`;
- `graph.json` for a Drone template (P4);
- `template.sig`, optional: the widget signature file (`widget_signature.rs:25-32`) over `agentmux-job-template-sig-v1\0<id>\0<version>\0<content hash>`. The content hash is the widgets' (`package_hash`, `widget_packages.rs:379-389`) over every file except the signature.

**Code reuse.** `widget_signature.rs` takes the domain prefix as a parameter, so widgets and templates share one verifier with different prefixes (a widget signature can't pass as a template's). The `agentmux-widget` CLI (`sdk/widget-sdk/cli/agentmux-widget.mjs`) signs a folder with `template.json` using the template prefix: `agentmux-widget sign <folder> --key <file>` works for both.

**Approval.** A template is installed as `needs_approval`, as widgets are (`WidgetState`, `widget_packages.rs:432-438`). The operator's prompt names the publisher (with its fingerprint and whether the key is pinned), the trigger, the action, every declared output and secret, and the default caps. Publisher keys are pinned on first approval, in `job-template-publishers.json` beside the widgets' pins, and a different key for a known publisher shows as changed. Agents can't approve. A template that isn't approved can't be used to create a job.

**Versions are pinned.** A job records the template's id, version and hash. A newer version of the template doesn't change the job: the Jobs tab shows "update available" with a diff of the brief, outputs and policy, and the operator accepts it.

**Clone.** Copy a template into a job, or into a user template stored in the global store beside Drone definitions, keeping a link to the source so updates can still be offered.

**Templates are not widgets.** A widget runs only while its pane is open and has no timer API (`sdk/widget-sdk/v1.d.ts`); a job needs a schedule and a runtime. They share signing and approval, not a catalog.

### 5.5 Catalogs

| Catalog | Where | Trust |
|---|---|---|
| **Built-in** | `templates/jobs/` in this repo, compiled into srv | Part of the app; shown as published by AgentMux. Still approved once per template before first use, so the operator sees its outputs |
| **Private catalogs** | A signed index at a URL the operator adds in Settings → Jobs → Catalogs, with its public key | The `widget_catalog.rs` scheme: `index.json` plus `index.json.sig`, Ed25519 against the key the operator pinned, each entry with its zip's SHA-256, content hash and publisher key; installed as `needs_approval` |
| **User templates** | The global store | The operator's own; no signature needed |

Generic templates live in agentmux; a project's own job templates and cloud job manifests go in a private catalog, never in this repo.

### 5.6 The ten starter templates

Shared defaults: overlap `skip`; catch-up `latest`; retries only for infrastructure failures; the run ends by reporting a verdict (`ok`, `findings`, `failed`); writes only through the declared outputs. Caps are per run and within the defaults. Templates 2 and 9 need event triggers: they ship in P3 for Run now and cron, and gain their event triggers in P5.

| # | Template | Trigger | Parameters | Outputs | Approval | Retries / cap |
|---|---|---|---|---|---|---|
| 1 | **`agentmux.weekly-report`**: read repositories and logs since the last successful run, rank findings by severity with evidence, and compare with the previous report (new, unchanged, resolved) | Cron, weekly | `repos[]`, `focus` (security, architecture, docs, website, disclosure, or free text), `since`, `report_dir`, `notify` | A report (Markdown and JSON); a Kanban task per high finding, deduplicated by finding hash; optional fix PRs into allowlisted repos | Required before any PR to a public repo | 2 / $3, 60 min, 200 turns |
| 2 | **`agentmux.pr-review`**: review a pull request's diff against the repo's standards and post one review; PR text is untrusted data | Run now with a PR number; event in P5 | `repo`, `pr`, `rules`, `reviewer_provider`, `max_rounds` | One review per head commit (updated, not repeated) | None (comments only); no secrets beyond the comment scope | 0 / $1, 15 min |
| 3 | **`agentmux.nightly-build`**: build and test; on failure, find the first real error and the likely commit | Cron, nightly | `repo`, `branch`, `command`, `platforms` | Status; a message to the owner on the first red; one Kanban task after N reds in a row | None | 1 / $0.50 (the agent part runs only on failure) |
| 4 | **`agentmux.daily-digest`**: summarise cost or usage since yesterday and month to date, and flag anything over budget or jumping | Cron, daily | `sources`, `thresholds`, `recipients` | A message or report; JSON history; a Kanban task on a budget breach | None | 2 / $0.25 |
| 5 | **`agentmux.dependency-scan`**: run scanners, remove duplicates, rank by reachability, propose upgrades | Cron, weekly | `repos[]`, `scanners`, `severity_floor` | A private report; a Kanban task per actionable finding; optional upgrade PRs | Required for any PR, and for anything that touches a public repo (an unfixed weakness is never published) | 2 / $2, 55 min |
| 6 | **`agentmux.docs-drift`**: find docs whose claims no longer match the code, and dead links | Cron, weekly | `repo`, `doc_globs`, `code_globs`, `since` | One standing Kanban task (updated); optional small fix PRs | Required for PRs to public repos | 1 / $1, 30 min |
| 7 | **`agentmux.stale-sweep`**: list stale branches and PRs, skip protected ones and ones claimed in Presence, message each owner | Cron, weekly | `repos[]`, `stale_days`, `protected_patterns` | Messages to owners; a summary; **proposed** deletions only | Required for any deletion or close | 1 / $0.50 |
| 8 | **`agentmux.backup-verify`**: back up chosen AgentMux data, restore into a temporary folder and check counts and hashes; the agent only reports anomalies | Cron, daily or weekly | `what`, `destination`, `keep`, `verify` | A backup; the verify result; an alert on failure | Required before pruning beyond `keep` | 2 / $0.10; the watchdog flags a missed run |
| 9 | **`agentmux.on-event`**: a skeleton: on an event, message an agent, spawn a worker or run a Drone graph, with the event's fields wrapped as untrusted data | Manual in P3; event in P5 | `event_filter`, `target`, `brief_template`, `allowed_outputs` | Only `allowed_outputs` | Required when the run can both read untrusted input and write outside (enforced, §6) | 0 / cap required, hourly fire cap |
| 10 | **`agentmux.weekly-status`**: collect each agent's merged PRs, open tasks and blocked items from the Kanban and GitHub, and write one page for the operator | Cron, Friday | `agents` or a lead, `since` | A report; one message to the owner | None | 0 / $0.50 |

## 6. Hooks for P4 and P5

Built into P1–P3 so the later phases add, not change:

| Hook | Where | Used by |
|---|---|---|
| `action.kind: "drone"`, validated and refused with "available in a later version" | §5.1 | **P4: Drone as a job action.** The job owns the trigger; the graph shows it. Drone gains cancel, interrupted-run cleanup (started in P1), cost rollup into the ledger (P1), Agent blocks for any provider, "message this agent" and "spawn a worker" blocks, an approval block, and Vault references instead of `AGENTMUX_DR_` variables |
| `policy.approval` (`none` only, until P4) and `outputs` | §5.1 | **P4:** an approval block or point waits (durable, with a timeout that rejects) and files a Needs-you Kanban card showing the proposed action. Outputs become enforced safe-output blocks: open PR, comment, file task, send message, write report |
| `trigger.kind: "event"`, refused until P5; `trigger: "event"` and a `payload_hash` column in the ledger | §5.1, §3.1 | **P5: event triggers.** GitHub events relayed over the existing MuxBus path (no new public endpoint); a file watch in a workspace; inbound messenger messages; a loopback webhook with a per-job secret |
| Rule of Two check at validation | §5.4 | **P5:** a job that reads untrusted input (PR or issue text, a webhook, an inbound message) must not also hold both secrets and outside writes; such a definition is refused unless it requires approval. P3 already runs the check on templates, so templates 2 and 9 are shaped for it |
| `runNow` on cloud manifests as a link or a message | §4.3 | **P6 (if wanted after P2):** a signed run request back over MuxBus, or a cloud runner |

## 7. Security

**Who may do what:**

| | Operator (the app's own client) | An agent (MuxAPI) |
|---|---|---|
| Create a job | Any action | `message` and `task` actions; `spawn` only with itself as the leader, and only after the operator approves the job once (the spawn consent rule, `agent_spawn.rs:632`, made at creation since the runs are unattended); `drone` never |
| Use a template | Approve and use it | Use one the operator approved |
| Edit, pause, resume, delete, Run now | Any job | Jobs it created or owns (`Caller` in `handle_cron_delete` and `handle_cron_patch`, with the missed-fires fix, #5392) |
| Raise a cap | Yes | No: clamped to the defaults |
| Pause all / resume all | Yes | No |
| Approve a template, pin a reporter or catalog key | Yes | No |
| Read jobs and history | Yes | Yes (`Schedule` `list`, `history`) |

Container agents get `list` and `history` only, as they can't spawn either (`agent_spawn.rs:266`).

**Signed templates.** Templates and cloud manifests are data: approving one runs nothing. Their signatures are checked against pinned publisher keys, catalogs against keys the operator pinned, and a job keeps the version it was created from until the operator accepts an update.

**Secrets through Vault only.** A job definition, its parameters, brief and ledger rows never hold a secret value. A `credential` parameter names a Vault entry. At run time the agent uses it through `Terminal run`'s `secrets` (`server/shell_secrets.rs`), which asks the operator or uses an existing "Always" grant, keeps values out of the output and refuses `AGENTMUX_` names. An unattended run with a missing grant waits for the operator's answer like any `Terminal run`, so its timeout covers it; a dry run lists the grants to make first.

**Fires are not the operator.** A job's fire reaches its agent as a scheduled delivery (`source_agent: "cron"`, the creator's uid, `TriggerKind::Schedule`), never as the operator's words, and can't carry an `[AgentMux]` or `[BROADCAST:` marker (srv rewrites them in forwarded text, as it does for jekts).

**Cloud events are data** (§4.2): signed by a pinned reporter, matched to an approved manifest, shown as text, quoted when forwarded, never injected as an instruction.

**Spending:** caps on by default, the reservation rule, Pause all, and the global limit of 3 unattended runs (§5.1, §5.3).

## 8. Tests

**srv:**
- Ledger: a cron fire writes `scheduled` then `running` then `finished`, with the turn's cost; a fire to a stopped agent is never held: it is `missed` (`target_stopped`) and raises `missed_count`, not `fire_count`; with catch-up `latest`, missed fires coalesce into one pending fire that a newer one replaces, which expires at the next fire time and is delivered once when the agent comes back; two schedulers sharing the identity store fire a slot once (unique index); consecutive skips collapse with a count; nothing removes runs (no retention sweep).
- Drone: `rundrone` writes a row with summed cost; rows left `running` become `interrupted` at startup.
- Caller checks: an agent can't delete or pause another agent's job (#5392); `pause_all` refuses an agent caller.
- Failure alerts: the first failure files one task (`job-failure`, the owning leader resolved through `worker_of` and, once seats exist, a seat), sends one note and one `JobFailed`; the second failure only adds to the card; a success adds "recovered".
- Cloud events: a valid signed event writes a row; a repeated `event_id` is ignored; a bad signature, unknown reporter or unknown `job_id` is dropped and counted; a `finished` without `started` makes a closed row; a late event turns `missed` into late.
- Watchdog: a missing expected slot after grace is `missed`; a started run past `maxDurationMinutes` is `lost`; three misses make one incident.
- P3: the cron migration keeps ids and behaviour; time zones across a daylight-saving change; overlap `skip`, `buffer_one`, `cancel_other`; retries only for infrastructure failures; the `deploy` group runs one at a time; reservation skips (`job_cap`, `account_cap`); a run reaching its per-run cap is interrupted (`over_budget`); an agent's higher caps are clamped; a no-cost provider is marked.
- Templates: the signature verifies with the template prefix and fails with the widget prefix; a changed file changes the hash; an unapproved template can't create a job; a changed publisher key shows as changed; a catalog index with a bad signature is refused; the Rule of Two check refuses an event job with secrets and outside writes and no approval.

**Frontend:** the Jobs tab (rows, the 14-run strip, the history, Run now, Pause, Pause all banner and its refusal of Run now, "N failing" in the title); the cloud group with "Run now" as a link or a message to the owner; the Cloud reporters panel in the Jobs tab; Settings → Jobs (caps, catalogs); New job from a template (parameter form, dry run, approval prompt).

**Live**, in an isolated `task dev` with `AGENTMUX_HOME_OVERRIDE`: create a one-minute cron job to an agent, watch rows appear; stop the agent and see `missed` rows; with catch-up `latest`, one `pending` row that each newer fire replaces, delivered once when the agent restarts; set a $0.01 per-run cap and see the turn interrupted and the alerts; turn on Pause all; post a signed sample event from a test reporter key and see a cloud row; stop posting and see `missed`.

## 9. Rollout

Each PR adds a changeset.

| PR | Phase | Scope |
|---|---|---|
| 1 | P1 | `db_job_runs` (identity store), cron fires write runs, cron fires never held (missed, with catch-up `skip` or `latest`; AgentA's cron fix owns the missed counters), `fire_count` only for delivered fires, the slot lease, the turn follower, Drone runs in the ledger and interrupted-run cleanup; `Schedule` `history`, `run_now`, `report`; `list` with last and next run |
| 2 | P1 | The Warden Jobs tab, `jobs.*` RPCs, `JobsChanged`, Pause all |
| 3 | P1 | Failure incidents: the Kanban task, the note to the owning leader, `NotifyKind::JobFailed` |
| 4 | P2 | The job-event schema file; the Cloud reporters panel in the Jobs tab; ingesting job events into the ledger; cloud job manifests; the cloud group in the tab. |
| 5 | P2 | The watchdog: `missed` and `lost`, local and cloud |
| 6 | P3 | `db_jobs` and the cron migration, the job schema file, time zones, policy (overlap, catch-up, timeout, retries, groups, the global limit), the `task` and `spawn` actions, `/agentmux/jobs` routes with `/agentmux/cron` aliases |
| 7 | P3 | Caps: settings, reservation, the interrupt at the per-run cap, daily-cap skips and alerts, no-cost providers |
| 8 | P3 | Template packages: the shared verifier, `agentmux-widget sign` for templates, approval and publisher pins, private catalogs |
| 9 | P3 | The Jobs tab's New job, Edit, Clone, dry run and update diffs |
| 10 | P3 | The ten starter templates in `templates/jobs/` |
| 11 | — | Docs (agentmux-docs): a Jobs page, the job-event schema for reporter authors, glossary entries for job, run, template and cap |

## 10. Decisions (were open questions)

All decided by the operator on 2026-10-23.

1. **Loops in the Jobs tab:** no. They belong to one session; Swarm already shows the agent.
2. **The account cap** counts job runs only in P3. Per-account spend from every turn is not stored today; revisit later.
3. **Providers that report no cost:** allowed for unattended jobs, bounded by the timeout and turn limits, after the operator confirms each such job.
4. **The tool keeps the name `Schedule`.**
5. **Agent-created `message` jobs that target another agent:** allowed without approval, as a jekt is, with a per-job rate cap on messages to another agent (§5.3, `messages_per_day`, default 24).
6. **The incident task on recovery** stays open for its owner, with a "recovered" note.
7. **Daily caps reset at the operator's local midnight.**
8. **Retention:** none. Run history is kept forever (§3.1).
