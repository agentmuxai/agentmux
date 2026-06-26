# Cron & Loop Robustness — Research + Design

**Date:** 2026-06-25  
**Status:** Draft — pending implementation decision  
**Context:** User-reported: cron jobs not firing reliably; uncertain whether Loop is implemented.

---

## 1. Current State

### Loop — session-scoped recurring injection

| Item | Status |
|---|---|
| `Loop` / `LoopStop` MCP tools | ✅ Implemented — `agentmux-mcp/src/main.rs` |
| `LoopList` | ❌ Missing (noted in SPEC_MCP_LOOP_TOOL_2026_06_16.md §6) |
| `max_iterations` | ❌ Missing |
| Idle-aware firing | ❌ `InjectionRequest.wait_for_idle` field exists but is never read by srv |
| Stuck-loop detection | ❌ Missing |
| Persistence across MCP restart | ❌ By design — loops are session-scoped |

**How it works today:**  
`Loop(prompt, interval, to)` spawns a Tokio task in the MCP process that sleeps `interval`, then POSTs to `/agentmux/reactive/inject` and repeats. The loop lives exactly as long as the agent pane (MCP process lifetime). On MCP restart, all loops are gone — by design, matching Claude Code's `/loop` model.

**Why it feels broken:**  
- Fixed wall-clock interval fires regardless of whether the target agent is mid-turn. Inject hits a busy agent; the message may be queued behind in-progress output or dropped.
- No `LoopList` → no visibility. You can't tell if a loop is running, how many are running, or how many times it has fired.
- No `max_iterations` → a loop that fires every 10s on a noisy agent is impossible to contain without manually calling `LoopStop`.

### Cron — persistent scheduled jobs

**Does not exist.** There is no `CronCreate`, `CronDelete`, or `CronList` in AgentMux's MCP layer, no database schema for scheduled jobs, and no server-side scheduler.

The confusion: Claude Code CLI exposes its own `CronCreate`/`CronList`/`CronDelete` tools (available to agents running in the CLI), but these are ephemeral — they live only while the CLI session is alive and are not persisted by AgentMux's server.

---

## 2. Industry Research

### 2.1 Why Cron Jobs Miss Fires

1. **Controller downtime** — when the scheduler process is down, firing times are silently missed. Jobs don't queue; they're gone.
2. **Clock drift** — system clock skew causes wall-clock mismatches. GitHub Actions workflows averaged 4.5-hour drift by mid-2026 due to platform scheduling backlog.
3. **No built-in failure alerting** — cron failures are silent until data is missing or a dead-man's switch trips.
4. **Desktop-specific: app not running** — on a desktop app, if the process isn't alive there's nothing to fire.

### 2.2 Industry Patterns for Reliable Scheduling

**Quartz misfire model (the gold standard):**  
Each trigger has a configurable "misfire instruction" defining catch-up behavior:
- `DO_NOTHING` — skip missed occurrences, wait for next scheduled time (best for refresh/monitoring jobs)
- `FIRE_ONCE_NOW` — execute one recovery run immediately, don't replay all misses (best after crash recovery)

**Kubernetes CronJob model:**  
- `startingDeadlineSeconds` defines how late a missed job can be created (e.g. 28800 = allow up to 8h late)
- If >100 missed schedules detected, controller skips and logs an error — no cron storm
- Concurrency policy: `Forbid` for long-running jobs, `Replace` for fire-and-forget

**Desktop app patterns:**  
- Persist scheduled task metadata in local DB; on startup, check for missed schedules, execute ONE recovery run
- Use power-save-blocker only while agent is actively working; release after idle threshold
- User notification when jobs skipped due to app downtime

### 2.3 Agent Loop Best Practices

From Claude Agent SDK and LiteLLM production patterns:

| Control | Purpose | Recommended values |
|---|---|---|
| `max_turns` | Hard cap on tool-use round trips | 15–30 |
| `max_iterations` | Cap total loop fires before auto-stop | 1–1000, caller-specified |
| Early-stopping message | When hitting max, append "synthesize and stop" prompt | Always implement |
| Stuck-loop detection | Hash last N tool calls; circuit-break if same hash appears 3× | N = 3–5 |
| Per-tool timeout | Prevent hanging on slow tools | < scheduling interval |

**Session persistence pattern:**  
Session object is ephemeral; conversation log is source of truth. For loops that survive restarts, store loop definition in DB; re-create the Tokio task on server startup.

### 2.4 Rust Scheduling Libraries

| Crate | Notes |
|---|---|
| `tokio-cron-scheduler` | Async cron via Tokio; timezone-aware; no persistence layer; time drift possible over uptime |
| `cronexpr` | Low-level cron expression parsing; used by higher-level frameworks |

**Critical pitfall:** `tokio-cron-scheduler` has no durability. Jobs must be re-registered from DB on every startup. Tests must use `#[tokio::test(flavor = "multi_threaded")]` or `scheduler.add()` hangs.

---

## 3. Proposed Design

### 3.1 Loop Improvements (agentmux-mcp — no DB changes)

**Priority: High. Low risk. Fixes the immediate "loop feels broken" problem.**

#### 3.1.1 `LoopList` tool

```
LoopList() → [{ id, prompt, interval_secs, target, fires_remaining, fire_count }]
```

Implementation: scan `LoopRegistry` (existing `HashMap`) and return entries. Each entry needs to be enriched at creation time with metadata (prompt, interval, target, optional max_iterations, fire_count atomic).

#### 3.1.2 `max_iterations` on Loop

```
Loop(prompt, interval, to, immediate, max_iterations?)
```

- `max_iterations = None` → run forever (current behavior)
- `max_iterations = Some(n)` → auto-`LoopStop` after n fires

Implementation: pass an `Arc<AtomicU64>` counter into the task; decrement each fire; abort when it hits zero.

#### 3.1.3 Idle-aware firing

`InjectionRequest.wait_for_idle` currently exists as dead scaffolding. Wire it:

- `agentmux-srv` reactive handler: before injecting, check if target agent's `last_turn_completed_at` is recent. If agent is mid-turn, queue the message or skip this fire.
- Define "idle" as: no active stream in the last N seconds (configurable, default 5s).

The handler already has `AgentRegistration` with `last_seen`; extend with `last_turn_start` / `last_turn_end` timestamps updated by the controller on turn boundaries.

#### 3.1.4 Stuck-loop detection

Track last 5 injection request hashes per loop. If the same hash appears 3× consecutively (meaning the agent is not making progress), auto-stop the loop and log a warning.

---

### 3.2 Cron System (agentmux-srv — new DB schema + scheduler)

**Priority: Medium. Larger lift. Addresses "cron doesn't fire" for persistent jobs.**

#### 3.2.1 Data model (SQLite, agentmux-srv)

```sql
CREATE TABLE cron_jobs (
    id          TEXT PRIMARY KEY,        -- uuid
    name        TEXT NOT NULL,
    expression  TEXT NOT NULL,           -- cron expression "0 9 * * 1-5"
    prompt      TEXT NOT NULL,
    target      TEXT NOT NULL,           -- agent id
    created_by  TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    last_fired  INTEGER,                 -- unix timestamp
    fire_count  INTEGER NOT NULL DEFAULT 0,
    max_fires   INTEGER,                 -- null = unlimited
    created_at  INTEGER NOT NULL
);
```

#### 3.2.2 Server-side scheduler

- On `agentmux-srv` startup: load all `enabled` cron jobs from DB, register with `tokio-cron-scheduler`
- On each fire: POST to `/agentmux/reactive/inject` (existing endpoint), update `last_fired` + `fire_count`, auto-disable if `fire_count >= max_fires`
- **Missed-fire recovery**: on startup, for each job where `last_fired < now - expected_interval`, execute ONE catch-up fire immediately (DO_NOTHING for jobs missed by < 1 interval; FIRE_ONCE_NOW for longer gaps)

#### 3.2.3 MCP tools

```
CronCreate(name, expression, prompt, to?, max_fires?) → { id, next_fire }
CronDelete(id)                                         → { ok }
CronList()                                             → [{ id, name, expression, next_fire, fire_count, enabled }]
CronPause(id)                                          → { ok }
CronResume(id)                                         → { ok }
```

`expression` supports standard 5-field cron: `"0 9 * * 1-5"` (9am weekdays). Validate at creation time and return next 3 scheduled fires for confirmation.

#### 3.2.4 HTTP endpoints (agentmux-srv)

```
POST   /agentmux/cron          CronCreate
DELETE /agentmux/cron/:id      CronDelete
GET    /agentmux/cron          CronList
PATCH  /agentmux/cron/:id      CronPause / CronResume
```

All auth-gated with `X-AuthKey` (same pattern as reactive endpoints).

#### 3.2.5 Reliability rules

- **Idempotent delivery**: each fire generates a unique `request_id`; reactive handler deduplicates by `request_id` (add to existing audit log check)
- **No cron storm**: if app was closed and >10 missed fires exist, execute one catch-up and log a warning — never replay all misses
- **Timezone**: store all times as UTC in DB; `expression` evaluated in UTC by default; `timezone` field optional for user-local scheduling
- **Max scheduler drift**: if `tokio-cron-scheduler` fires > 60s late (detectable by comparing `now` vs `expected`), log a warning

---

## 4. Implementation Order

| Phase | Scope | Risk | Time estimate |
|---|---|---|---|
| **P1** | `LoopList` + `max_iterations` | Low — MCP only, no server changes | 1–2h |
| **P2** | Idle-aware firing (`wait_for_idle`) | Medium — requires srv changes to track turn state | 3–4h |
| **P3** | Stuck-loop detection | Low — MCP only | 1h |
| **P4** | Cron DB schema + server scheduler | High — new DB migration, new routes, scheduler lifecycle | 4–6h |
| **P5** | Cron MCP tools + CronList/Pause/Resume | Medium — depends on P4 | 2–3h |
| **P6** | Frontend cron management UI | Medium — depends on P4+P5 | 4–6h |

---

## 5. Files to Touch

### Loop (P1–P3)
- `agentmux-mcp/src/main.rs` — `LoopRegistry`, tool handlers, `max_iterations`, stuck-loop tracking
- `agentmux-srv/src/backend/reactive/handler.rs` — `wait_for_idle` check, turn-boundary timestamps
- `agentmux-srv/src/backend/reactive/types.rs` — `AgentRegistration` turn timestamps
- `agentmux-common/src/api_types.rs` — `InjectionRequest.wait_for_idle` (already exists, unused)

### Cron (P4–P5)
- `agentmux-srv/src/db/migrations/` — new migration for `cron_jobs` table
- `agentmux-srv/src/backend/cron/` — new module: scheduler, job runner, missed-fire recovery
- `agentmux-srv/src/server/cron.rs` — new HTTP handlers
- `agentmux-srv/src/server/mod.rs` — route registration
- `agentmux-mcp/src/main.rs` — `CronCreate`, `CronDelete`, `CronList`, `CronPause`, `CronResume` tools
- `Cargo.toml` — add `tokio-cron-scheduler`, `cronexpr` (or equivalent)

---

## 6. Decisions (closed)

1. **Cron timezone**: UTC-only for now. No `timezone` field in v1. Avoids DST bugs and saves scope; can add later.

2. **Missed fires on startup**: FIRE_ONCE_NOW — execute one catch-up run immediately on `agentmux-srv` startup if a job missed its window. Never replay all misses (no cron storms).

3. **Cron persistence scope**: Global — cron jobs survive the creating agent's pane being closed. That's the defining difference from Loop. A cron job fires as long as `agentmux-srv` is running, regardless of which (if any) agent panes are open.

4. **Frontend UI**: Deferred. MCP-tool-only for v1 (`CronCreate`, `CronDelete`, `CronList`, `CronPause`, `CronResume`). Frontend management UI is a follow-up.

5. **`LoopList` scope**: All loops across all agents (like `ps`). No filtering by caller.

6. **P2 idle-aware firing**: Deferred to follow-up. Implementing turn-boundary timestamps in the reactive handler is significant scope. Ship P1 + P3 + Cron first.
