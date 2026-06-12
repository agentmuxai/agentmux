<!--
Copyright 2026, AgentMux Corp.
SPDX-License-Identifier: Apache-2.0
-->

# SPEC: Agent Failure Diagnostics — Surfacing the "Why" Behind a Non-Zero Exit

- **Date:** 2026-06-11
- **Status:** Draft / proposed
- **Area:** `agentmux-srv` agent runner + translator; `frontend/app` agent-pane state; drone run records
- **Related:** `SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md`, `docs/specs/agent-health-design.md`, `docs/specs/backend-status-tests.md`

---

## 0. One-line

When a Claude agent subprocess dies (rate limit, overload, auth, OOM, crash), AgentMux must **capture the real error, classify it, and show a human-readable explanation** — instead of an opaque "exit 1" / silent status flip.

---

## 1. Motivation

### 1.1 The incident that prompted this

A multi-agent run launched three research subagents in parallel. All three died within ~60–80 s with:

```
API Error: Server is temporarily limiting requests (not your usage limit) · Rate limited
```

What reached the operator was effectively a bare failure — "exit 1," no cause. The actual explanation (server-side rate-limit, transient, **not** a quota problem, retryable) existed but never surfaced. The operator's reasonable question was *"why did it exit 1?"* — and the system couldn't answer.

### 1.2 Why this generalizes to AgentMux agents

AgentMux agents are `claude` CLI subprocesses (`agentmux-srv/src/agents/runner.rs`). They fail for the same family of reasons — rate-limit (429), overloaded (529), auth (401), usage-limit, OOM/SIGKILL, network, max-turns, or a plain crash. Today every one of those collapses to the same opaque terminal string. Agents are long-lived and often unattended; an unexplained death erodes trust and is undebuggable after the fact. This is adjacent to the known "dead but shows running" staleness problem (stale `db_agent_instances.status='running'`).

### 1.3 The hard requirement

> **An explanation of the exit must appear** — in the UI, in plain language, with whether it's retryable and what to do next.

---

## 2. Current behavior — why the reason is lost (code-grounded)

There are two agent execution paths. The **headless/drone runner** (`agents/runner.rs`, stream-json) is where the loss is cleanest and most fixable; the **interactive PTY pane** (`blockcontroller/`) has the exit code but no classified cause.

### Gap G1 — stderr is captured, then thrown on the floor
`agentmux-srv/src/agents/runner.rs:147-181` drains the child's stderr into a 64 KB capped buffer (`STDERR_CAP`, line 156) **and then discards it**:

```rust
// runner.rs:179-181
// buf currently dropped; Phase 2 will plumb it to the broker.
let _ = buf;
```

That buffer is exactly where `API Error: … Rate limited`, `Invalid API key`, `overloaded_error`, etc. live. The code already admits the gap ("Phase 2 surfaces stderr …", lines 152-154). **This spec is that Phase 2.**

### Gap G2 — failure messages carry only the exit status
`agentmux-srv/src/agents/runner.rs:212-227`, the failure branches build strings with *no* stderr, *no* classification, *no* structured exit code:

```rust
// runner.rs:218
return Err("claude exited 0 but stream produced no Done event".to_string());
// runner.rs:223-225
Ok(_) => Err(format!(
    "claude exited with status {exit} but stream emitted no error"
)),
```

`"claude exited with status 1 but stream emitted no error"` **is** the opaque "exit 1."

### Gap G3 — error `result` frames are swallowed by the translator
The Claude CLI can report a failure on **stdout** as a terminal `result` frame with `subtype: "error_*"` / `is_error: true` / an `error` object (so the process may even exit 0). `handle_result` in `agentmux-srv/src/agents/translator/claude.rs:248-266` only reads `cost_usd`, `usage`, and `result` text, then emits `Cost` + `Done`. It inspects `is_error` **only** at the tool_result level (line 198), never at the result level. An error result frame therefore becomes a hollow *successful* `Done` and the reason vanishes.

### Gap G4 — the structured error channel exists but is never used on failure
`AgentEvent::Error { message }` is defined (`agentmux-srv/src/agents/types.rs:104-108`) and is the intended user-facing error channel. But the runner's failure path returns a bare `Err(String)` on the `final_result` oneshot (`runner.rs:184-186, 223-227`) and **never sends `AgentEvent::Error` on the live event stream (`tx`)**. Stream-watchers (the agent pane) get nothing; only the drone's terminal accumulator sees the string.

### Gap G5 — the interactive path knows the code but not the cause
The PTY pane plumbs an exit code end-to-end — `proc_exit_code` (`agentmux-srv/src/backend/blockcontroller/subprocess.rs:94-95`), set on `child.wait()` (lines 727-749), surfaced to the frontend as `shellprocexitcode` (lines 208, 811). The frontend agent state machine has an `errored` outcome (`frontend/app/store/agent-pane-state/types.ts:76-80`) reached via bounded force-transitions with **generic** reasons like `"stream-stalled"` and timeout (`frontend/app/store/agent-pane-state/reducer.ts:663, 718`). So it can say *"errored"* but not *why* — the underlying error text is never captured or classified on this path either.

### What already works (reuse, don't rebuild)
- **Token/cost capture** is solid: `TokenCounts { input, output, cache_creation, cache_read }` (`types.rs:110-121`) parsed from the result frame's `usage` (`translator/claude.rs:268-280`) and carried by `AgentEvent::Cost`. The failure struct should ride alongside this, not replace it.
- **The drone already persists an `error` column** on a run row (`agentmux-srv/src/drone/storage.rs:141, 151, 167, 175`) — today it stores the opaque G2 string. We upgrade *what* goes in, not the plumbing.
- **The sidecar connection already models cause well**: `backendStatus` distinguishes `crashed`, tracks exit code, and handles "exit code null when signal kill" (`frontend/app/store/backendStatus.test.ts`). That's the precedent to mirror for agents (it is a *different* subject — the srv sidecar, not the agent — but the shape is exactly what we want).

---

## 3. Goals / Non-goals

**Goals**
1. **Capture** the failure evidence: exit code, signal (SIGKILL ⇒ OOM), the tail of stderr, and any error `result` frame.
2. **Classify** it into a small, stable taxonomy (§5).
3. **Surface** a plain-language explanation + actionable hint in the UI, replacing bare "exit 1" everywhere an agent can die.
4. **Persist** it (drone run record; `db_agent_instances` last-failure) so it's inspectable after the fact.
5. **Mark retryability** — transient classes (rate-limit, overloaded, network) say so explicitly and emit a `retryable` flag.

**Non-goals**
- Auto-retry / backoff / auto-resume orchestration. This spec *emits* `retryable`; acting on it is a follow-up.
- Modifying the `claude` CLI.
- Context-window / compaction handling (parked, separate spec) — except the one cross-link in §10.
- Reconciling the stale `status='running'` liveness bug (related, separate).

---

## 4. Design

### 4.1 Capture (`agents/runner.rs`)
- **Retain the stderr buffer.** Replace the discard at `runner.rs:180` by handing the capped buffer back to `drain_and_collect` — give the stderr-drain `tokio::spawn` a `oneshot<Vec<u8>>` (or a shared `Arc<Mutex<Vec<u8>>>`) it sends on EOF, and `await` it after `child.wait()`. Keep the existing cap + keep-draining-and-discarding-past-cap behavior (it prevents pipe-fill stalls).
- **Capture exit precisely.** From `child.wait()` (`runner.rs:210`), record both `status.code()` and, on Unix, `std::os::unix::process::ExitStatusExt::signal()` so a SIGKILL/OOM (code `None`, signal `9`) is distinguishable from a clean `exit 1`.
- **Assemble a structured failure** on every error branch (§6) instead of the bare strings at lines 218/223-224.

### 4.2 Classify (new `agents/failure.rs`)
A **pure, data-driven, unit-testable** function:

```
classify(exit_code: Option<i32>, signal: Option<i32>,
         stderr_tail: &str, result_frame: Option<&Value>) -> AgentFailure
```

A match table maps stderr substrings + exit/signal to a class. The Anthropic error phrasings are stable enough to match on; include the exact incident string `"Server is temporarily limiting requests"` / `"Rate limited"` → `RateLimited { retryable: true }`. Purity keeps it trivially testable with real captured strings.

### 4.3 Translator (`translator/claude.rs`)
`handle_result` (lines 248-266) must inspect the result frame's `subtype` / `is_error` / `error`. When it's an error frame, emit `AgentEvent::Error` (classified via §4.2) **instead of** a hollow `Done`. This catches failures the CLI reports on **stdout** (the exit-0 path G3).

### 4.4 Emit on the live stream (`types.rs` + `runner.rs`)
On any failure, the runner **sends `AgentEvent::Error` on `tx`** (the live stream) *before* resolving `final_result`. Extend `AgentEvent::Error` from `{ message }` to carry the structured failure (`code`, `detail`, `retryable`) — wire-format camelCase, mirrored in `frontend/types/gotypes.d.ts`. Both the agent pane (stream-watcher) and the drone accumulator then see the same classified failure.

### 4.5 Persist
- **Drone:** write the structured failure as JSON into the existing `DroneRun.error` column (`drone/storage.rs`), not a bare string.
- **Agent instance:** add a `last_failure` field on `db_agent_instances` (class + detail + ts) so the Swarm/Warden overview and the agent pane can show *"last died: rate-limited, 3 m ago, retryable."* This also gives the liveness reconciler something truthful to display.

### 4.6 Surface in the UI (the hard requirement)
- **Agent pane:** map the runner's `AgentEvent::Error` → the `errored` outcome (`agent-pane-state`) carrying `failureCode` + `failureDetail`, and render an **explanation banner** instead of a generic errored state, e.g.:
  > ⚠️ Claude hit a **rate limit** (server-side, temporary — not your quota). Retry in a moment.
- **Drone run inspector:** show class + hint + stderr tail (collapsible).
- **Swarm / Warden overview:** badge the agent with the failure class + retryable hint.
- **Interactive PTY pane:** feed `shellprocexitcode` + captured stderr tail through the same `classify()` so this path gets the same banner (closes G5).

### 4.7 Retryability hook (future)
`retryable: true` is emitted but acting on it (backoff/auto-resume) is out of scope. Named so a later orchestration spec can consume it.

---

## 5. Taxonomy

| Class | Trigger (stderr / exit / signal) | User-facing message | Retryable | Suggested action |
|---|---|---|---|---|
| `RateLimited` | `"Rate limited"`, `"temporarily limiting requests"`, HTTP 429 | Server-side rate limit (transient, **not** your quota) | ✅ | Retry shortly; reduce concurrency |
| `Overloaded` | `overloaded_error`, HTTP 529 | API temporarily overloaded | ✅ | Retry with backoff |
| `UsageLimit` | `"usage limit"`, quota/billing text | Your plan/usage limit reached | ❌ | Check plan/billing |
| `Auth` | `authentication_error`, `"Invalid API key"`, `/login`, 401 | Not authenticated | ❌ | Re-auth (Identity tab) |
| `Killed` (OOM) | signal 9 / code 137 / `None` | Process killed (likely OOM) | ⚠️ | Reduce load; check memory |
| `Network` | `APIConnectionError`, DNS/connreset | Network error reaching API | ✅ | Check connectivity; retry |
| `MaxTurns` | `--max-turns` reached / `error_max_turns` | Hit the turn cap | ❌ | Raise cap or split task |
| `NoOutput` | exit 0, no `Done` (runner.rs:218) | Exited cleanly but produced nothing | ⚠️ | Inspect transcript/logs |
| `SpawnFailure` | `AgentError::Spawn` (runner.rs:133) | Couldn't launch `claude` | ❌ | Check binary / `AGENTMUX_CLAUDE_BIN` |
| `UnknownNonZero` | any other non-zero exit | Failed (exit N) — see detail | ⚠️ | Show stderr tail |
| `Normal` | exit 0 + `Done` | — (success) | — | — |

Every non-`Normal` class carries the **stderr tail** so the raw text is always one click away.

---

## 6. Data shapes (proposed)

```rust
// agents/failure.rs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentFailure {
    pub code: FailureClass,           // serde-tagged enum, snake_case wire
    pub title: String,                // short, user-facing
    pub detail: String,               // explanation (may embed stderr tail)
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stderr_tail: String,          // capped, already in runner
    pub retryable: bool,
}
```

```rust
// types.rs — extend the existing variant (back-compat: message stays)
AgentEvent::Error {
    message: String,                  // existing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<AgentFailure>,    // new structured payload
}
```

TS mirror in `frontend/types/gotypes.d.ts`; the agent-pane reducer reads `failure.code` / `failure.detail`.

---

## 7. Touch points

| Gap | File | Change |
|---|---|---|
| G1 | `agentmux-srv/src/agents/runner.rs:147-181` | Return the stderr buffer instead of `let _ = buf;` |
| G2 | `agentmux-srv/src/agents/runner.rs:212-227` | Build `AgentFailure` via `classify()`; stop returning bare strings |
| — | `agentmux-srv/src/agents/failure.rs` *(new)* | `classify()` + `FailureClass` + table |
| G3 | `agentmux-srv/src/agents/translator/claude.rs:248-266` | Inspect `subtype`/`is_error`/`error`; emit `AgentEvent::Error` |
| G4 | `agentmux-srv/src/agents/types.rs:104-108` | Extend `AgentEvent::Error` with `failure` |
| G4 | `agentmux-srv/src/agents/runner.rs:183-186` | Emit `AgentEvent::Error` on `tx` before resolving `final_result` |
| persist | `agentmux-srv/src/drone/storage.rs:141-175` | Store `AgentFailure` JSON in `DroneRun.error` |
| persist | `db_agent_instances` schema + store | Add `last_failure` column/field |
| G5 | `agentmux-srv/src/backend/blockcontroller/subprocess.rs:727-749` | Capture stderr tail; run `shellprocexitcode` through `classify()` |
| UI | `frontend/app/store/agent-pane-state/reducer.ts:633-720`, `types.ts:76-80` | `errored` outcome carries `failureCode`/`failureDetail` |
| UI | agent pane view + Swarm/Warden overview | Render explanation banner + badge |

---

## 8. Testing
- **Unit (`failure.rs`):** table tests with **real** captured strings, including the incident's `"Server is temporarily limiting requests … Rate limited"` ⇒ `RateLimited { retryable: true }`; SIGKILL ⇒ `Killed`; `Invalid API key` ⇒ `Auth`.
- **Translator:** an error `result` frame ⇒ `AgentEvent::Error` (not a hollow `Done`). Extends the existing `result_*` tests in `translator/claude.rs`.
- **Runner:** extend `run_agent_with_bin_surfaces_spawn_failure` (`runner.rs:460-488`) with a stub binary that writes a rate-limit line to stderr and exits 1 ⇒ assert the structured failure carries the classification **and** the stderr tail.
- **Frontend:** `errored` outcome renders the explanation banner with the right copy + retryable hint.

---

## 9. Rollout / phasing
- **P1 — kill the opaque string at the source.** G1 + G2 + `failure.rs`: capture stderr, classify, put the explanation into the runner's terminal error and the drone `error` column. Smallest change, biggest win — literally finishes the `runner.rs` "Phase 2" TODO.
- **P2 — live surfacing.** G3 + G4: emit `AgentEvent::Error` on the stream; agent-pane banner.
- **P3 — persistence + overview.** `db_agent_instances.last_failure`, Swarm/Warden badge, interactive-path parity (G5), `retryable` hook.

---

## 10. Open questions
1. Should the interactive PTY path and the headless runner converge on one capture/classify helper now, or stay separate per `SPEC_UNIFIED_AGENT_TYPES` (runner.rs:14-19 keeps spawn separate, translator shared)? Recommended: share `classify()` only.
2. Retryable failures — inline banner only, or also a transient toast?
3. `model_context_window_exceeded` is a failure *and* a context-window concern — does its messaging live here or in the parked context-window spec? Recommended: classify it here (`ContextExceeded`), link the remediation there.

---

## 11. Back to the incident
Under this spec, the failure that triggered it — three subagents, `Server is temporarily limiting requests · Rate limited`, surfaced as an opaque exit — classifies as **`RateLimited`, retryable = true**, and the operator sees:

> ⚠️ Rate-limited by the API (server-side, temporary — **not** your quota). The agent can be retried shortly; consider lowering parallelism.

…instead of "exit 1."
