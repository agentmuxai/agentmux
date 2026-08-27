<!--
Copyright 2026, AgentMux Corp.
SPDX-License-Identifier: Apache-2.0
-->

# SPEC: Agent Error Framework — Durable Error State + Global Error Surface

- **Date:** 2026-06-20
- **Status:** Proposed
- **2026-08-27 note:** the `FailureClass::Unresponsive` taxonomy entry this
  framework carried was removed in
  `docs/specs/SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md` — every
  other failure class and this framework's general durable-error-state design
  are unaffected.
- **Author:** AgentA
- **Area:** `agentmux-srv` blockcontroller, `frontend/app/view/agent`, `frontend/app/view/accounts`
- **Related:**
  - `SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md` — classification (Phase 1/2, shipped #1353+#1464)
  - `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` — recovery banner UI (in progress)
  - `docs/specs/archive/SPEC_TRUST_CENTER_CLI_AUTH_BINDING_2026_06_17.md` — re-auth flow

---

## 0. One-line

Agent failure classification exists and is correct, but the failure event is ephemeral
(`persist: 0`) — missed if the tab isn't watching at the exact moment the process exits.
This spec makes error state **durable** (persisted into block meta), visible on any load,
and surfaces auth failures with an unambiguous "you need to log in" prompt.

---

## 1. Retro — How This Was Missed

### 1.1 Incident

Poal (a Claude Code agent) consistently returned no visible output to the user after
every message ("are you there", "u there?", "Code Harry"). The frontend showed "Worked"
(the `agent-message-accepted` toast) with blank pane content. The user correctly
identified the agent as silently unresponsive.

Investigation (2026-06-20) revealed:
- Every single message triggered a **401 auth failure** in the CLI subprocess
- The CLI emits `{"type":"system","subtype":"api_retry","error_status":401,...}` then a
  synthetic assistant frame `{"type":"assistant","error":"authentication_failed",...}`
- `subprocess.rs` captures the assistant frame into `last_inband_error` correctly
- `failure.rs:classify()` correctly returns `FailureClass::Auth`
- A `wps::EVENT_AGENT_FAILURE` event IS published — but with **`persist: 0`**

Because `persist: 0`, the event is fire-and-forget. The lifecycle for each message was:
```
t+0ms   AgentInput received
t+0ms   busyCount=1
t+2s    6× stdout lines written (the full 401 error + CLI init frames)
t+2s    busyCount=0  (process already exited)
t+2s    AgentFailure event fired (persist:0) — GONE if no active subscriber
t+35s   health: Healthy → Stalled
t+90s   health: Stalled → Dead
```

The user's pane had a 2-second window to catch the failure event. They saw "Worked"
(the accepted toast fires instantly) and then silence. The failure event fired and
evaporated before any reaction could occur.

### 1.2 Why Two Prior Specs Missed It

**SPEC_AGENT_FAILURE_DIAGNOSTICS (Jun 11)** focused on _capturing_ the error cause from
subprocess stdout/stderr and classifying it correctly. That problem is solved. The spec
never addressed delivery guarantees — it assumed the frontend would be watching.

**SPEC_AGENT_FAILURE_RECOVERY_UI (Jun 16)** focused on what to _show_ once the event
arrives, and how to wire recovery actions (Login Again, Retry, etc.). Again, assumed
delivery. The note "currently only `opts.log(...)` red text lines" treated the
surfacing gap as a rendering problem, not a delivery problem.

**The shared blind spot:** both specs were written from the perspective of watching a
fresh agent that fails during an interactive session. If you send a message and watch the
pane, you'd see the red lines or the banner fire within 2–5 seconds. The case that fell
through is:

> **Agent is already dead/stale when the user opens the pane, OR the failure fires faster
> than the user can register it.**

Poal's 401 fails in ~2 seconds. By the time the user reads "Worked" and looks at the
pane, the failure event is gone. This is structural — no amount of better banner styling
fixes a dropped event.

### 1.3 What PR #1283 Did and Didn't Cover

PR #1283 fixed the **credential location** problem: when an agent's identity bundle has
an isolated `CLAUDE_CONFIG_DIR`, the seeder now copies tokens from `~/.claude` into it,
so the two-phase CLI auth check finds valid creds in both phases.

What it doesn't fix: tokens that are **genuinely expired or revoked**. Poal had no valid
credentials at all (`apiKeySource: "none"` in every init frame). The seeder copies
whatever is in `~/.claude` — if those tokens are expired, the agent still gets 401.
The fix for that is the re-auth flow (Armory → Accounts → "Login Again"), but
that flow only kicks in if the failure surface is durable enough to be acted on.

### 1.4 Root Causes (ranked)

| # | Root cause | Severity |
|---|---|---|
| R1 | `agentfailure` event emitted with `persist: 0` — evaporates immediately | **P0** |
| R2 | No error state persisted in block WaveObject — pane load can't recover prior failure | **P0** |
| R3 | Auth error text in blockfile is a `type: "result"` line — silently dropped by `claude-translator.ts` | P1 |
| R4 | `Stalled → Dead` transition not rendered inline in pane (user sees silence) | P1 |
| R5 | "Code Harry" replay sends 2024-byte identical response — CLI replays context on 401 but never signals live | P2 |

---

## 2. Do We Need a Global Error Framework?

**Short answer: yes for durability, no for a giant abstraction.**

The classification layer (`failure.rs`) is already good. The recovery UI spec gives us
the right per-class actions. What's missing is a thin **persistence + routing layer**
that makes failure state a first-class durable WaveObject property rather than a
fire-and-forget event.

Specifically:

| Need | Complexity | Value |
|---|---|---|
| Persist latest AgentFailure in block meta | Low (2 lines in subprocess.rs) | High — fixes the core gap |
| Load persisted failure on pane mount | Low (1 selector) | High — survives reload |
| Clear failure on successful agent turn | Low (1 dispatch) | High — auto-heals |
| Cross-agent error summary (notification center) | Medium | Medium — nice for multi-agent; not urgent |
| Global error toast for spontaneous failures | Low | High — "Poal encountered an auth error" system notification |

**Do now:** R1+R2 (durable block meta), R3 (translate auth errors inline), R4 (health
transitions show in pane). Cross-agent notification center is Phase 2.

---

## 3. Architecture

### 3.1 Layers

```
┌─────────────────────────────────────────────┐
│  L3  Global Error Surface                    │  (Phase 2)
│       notification center; cross-agent        │
│       badge on window titlebar                │
├─────────────────────────────────────────────┤
│  L2  Per-Pane Error Surface                  │  ← THIS SPEC (Phase 1)
│       banner (recovery actions) + inline      │
│       transcript node on auth error           │
├─────────────────────────────────────────────┤
│  L1  Error State (durable)                   │  ← THIS SPEC (Phase 1)
│       AgentFailure persisted in block meta    │
│       cleared on successful turn              │
├─────────────────────────────────────────────┤
│  L0  Classification (EXISTS — failure.rs)    │
│       classify() → FailureClass              │
└─────────────────────────────────────────────┘
```

### 3.2 Data model

```rust
// Existing, in agentmux-srv/src/agents/failure.rs
pub struct AgentFailure {
    pub code: FailureClass,   // "auth" | "rate_limited" | "overloaded" | ...
    pub title: String,
    pub detail: String,
    pub stderr_tail: Option<String>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub retryable: bool,
}
```

**Add to block metadata** (`agentmux-srv/src/backend/blockcontroller/core.rs` or
`subprocess.rs` publish step):

```rust
// After classify() succeeds, write to block meta (in addition to the WPS event):
wstore.set_meta_json(block_id, "agent:last_failure", &serde_json::to_value(&failure)?)?;
// Clear on successful turn (exit 0, no error frame):
wstore.delete_meta(block_id, "agent:last_failure")?;
```

Frontend reads this on mount:
```typescript
// useAgentFailure.ts — on mount, read persisted state
const persisted = useBlockMeta<AgentFailure>(blockId, "agent:last_failure");
createEffect(() => {
    const f = persisted();
    if (f && !failure()) setFailure(f);
});
```

### 3.3 Event flow (revised)

```
subprocess exits
    │
    ├─► classify() → AgentFailure
    │       │
    │       ├─► wstore.set_meta(block_id, "agent:last_failure", failure)   ← NEW (durable)
    │       │
    │       └─► broker.publish(AgentFailure event, persist: 1)              ← CHANGE persist to 1
    │
    └─► frontend pane (any load time)
            ├─► useBlockMeta("agent:last_failure") → shows banner on mount  ← NEW
            └─► waveEventSubscribe(AgentFailure) → updates live              ← EXISTS
```

---

## 4. Phase 1 Implementation Plan

### P1.1 — Persist failure in block meta (backend)
**File:** `agentmux-srv/src/backend/blockcontroller/subprocess.rs`  
**Change:** After `broker.publish(agentfailure, ...)` at line 847–854:
```rust
// Also write to durable block meta so the pane can recover on any load.
if let Some(ref failure) = run_failure {
    let _ = wstore_wait.set_meta_json(
        &block_id_wait,
        "agent:last_failure",
        &serde_json::to_value(failure).unwrap_or_default(),
    );
}
```
And bump `persist: 0` → `persist: 1` on the event.

**Clear on success** (same process_waiter, else-branch of `if exit_code != 0 || ...`):
```rust
} else {
    // Successful turn — clear any prior failure state.
    let _ = wstore_wait.delete_meta(&block_id_wait, "agent:last_failure");
}
```

### P1.2 — Load persisted failure on pane mount (frontend)
**File:** `frontend/app/view/agent/hooks/useAgentFailure.ts`

Read block meta `"agent:last_failure"` via `useBlockMeta(blockId, "agent:last_failure")`
on mount, hydrate `setFailure()` if present and if no live failure is already set.

### P1.3 — Inline auth error node in transcript (frontend)
**File:** `frontend/app/view/agent/providers/claude-translator.ts`

When `type: "result"` arrives with `is_error: true` and `api_error_status` present,
emit an additional `{ type: "error_result", code: api_error_status, message: result }`
stream event rather than silently emitting only `session_end`.

**File:** `frontend/app/view/agent/stream-parser.ts`

Add `case "error_result"` → produce a `DocumentNode` of type `"agent_error"` rendered
as an inline red message block in the transcript. This gives the user a visible in-pane
signal even if they miss the banner.

### P1.4 — Health transition nodes in transcript (frontend)
**File:** `frontend/app/view/agent/hooks/useControllerStatusEvents.ts`

When block health transitions to `Stalled` or `Dead`, emit a lightweight status node
into the transcript (a grey italicized system line: "Agent became unresponsive" /
"Agent process exited"). This fills the silence gap even before the failure event fires.

### P1.5 — System notification for spontaneous auth failure (frontend)
When `AgentFailure` with `code: "auth"` fires on a block that the user is NOT actively
viewing, push a system-level notification: `"[AgentName] needs to log in"` with a click
that opens the agent pane. This is the zero-watching-required safety net.

---

## 5. Phase 2 — Global Error Surface (notification center)

Not urgent — schedule after Phase 1 is stable.

| Component | Description |
|---|---|
| Error badge on agent tab | Red dot when `agent:last_failure` is set |
| FleetView error column | Per-agent status at a glance |
| Notification center pane | Aggregated timeline of all agent failures across all panes |
| Failure TTL | Auto-expire old failures after N hours of no activity |

---

## 6. Auth-Specific UX (tying back to Poal)

When `code: "auth"`:

1. **Inline transcript node** (P1.3): `"Authentication failed — the agent's API credentials are no longer valid."`
2. **Recovery banner** (SPEC_AGENT_FAILURE_RECOVERY_UI): primary = "Login Again" inline re-auth; secondary = "Armory → Accounts"
3. **System notification** (P1.5): if pane not focused — `"Poal needs to log in"`
4. **Stale-agent guard**: if block is Dead with `code: "auth"` and user sends a message, show the auth error inline rather than accepting the message silently ("Worked") and failing invisibly

Point 4 is the direct fix for the incident: the `AgentInput` RPC today accepts the message even on a Dead block (the process relaunches). When the block has a persisted auth failure, the input handler should either:
a. Show the banner first and require the user to acknowledge/fix before sending, OR
b. Accept the message, attempt the turn, catch the 401, and immediately surface the error with the recovery action

Option (b) is less disruptive and already partially works — it just needs the delivery fix (P1.1+P1.2) to make the failure visible.

---

## 7. Best Practices Reference

These patterns informed the design above:

| Practice | Applied here |
|---|---|
| **Error states are data, not events.** Persist failures in the object model; use events only for real-time delivery. | Block meta `agent:last_failure` is the source-of-truth; WPS event is a push optimization |
| **Error surfaces should survive reconnect.** Any client that loads the block should see its error state without needing to have been present when the error fired. | Pane mount reads block meta |
| **Errors should have recovery actions.** A user who sees an error should know what to do without leaving the context. | Recovery banner per FailureClass (SPEC_AGENT_FAILURE_RECOVERY_UI) |
| **Auth errors are never silent.** Auth failures are operator-actionable (not retryable automatically) and must surface prominently. | Inline node + banner + system notification |
| **Error state clears on recovery.** When the agent successfully completes a turn, the error state is gone. No stale red banners. | `delete_meta` on successful exit |
| **Don't require watching.** A user who glances away for 2 seconds should not miss a permanent failure. | persist:1 + block meta |
| **Ephemeral events for latency, durable state for correctness.** WPS events deliver low-latency updates; block meta provides the ground truth. | Both used together |

---

## 8. Acceptance Criteria

- [ ] P1.1: After a 401 auth failure, `agent:last_failure` is present in block meta (readable via `muxlog host grep "set_meta.*last_failure"`)
- [ ] P1.2: Opening Poal's pane (from any cold load) shows the auth recovery banner without needing a new message
- [ ] P1.3: A 401-failing message shows an inline red text node in the transcript: "Authentication failed — …"
- [ ] P1.4: Health transitions show as grey system lines in the transcript
- [ ] P1.5: A background-pane auth failure triggers a system notification
- [ ] Regression: A successfully-completing agent turn clears `agent:last_failure` and shows no banner
- [ ] Regression: Transient failures (rate_limit, overloaded) still auto-retry and do not persist error state past a successful retry
