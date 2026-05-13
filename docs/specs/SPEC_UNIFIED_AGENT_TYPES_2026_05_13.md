# SPEC: Unified agent type system (Workflows Phase 1.5)

**Date:** 2026-05-13
**Status:** Draft — lands after PR #755 (Workflows Phase 1) merges
**Author:** AgentA
**Issue:** RFC #753 (Workflows pane), follow-up to PR #755
**Related memory:** [`project_workflows_phase_1_5.md`](../../../../.claude/projects/C--Systems/memory/project_workflows_phase_1_5.md)

---

## 1. Why this PR exists

PR #755 ships the Workflows Phase 1 MVP with a **stub Agent block** that returns `{ response: "[stub agent=...]", status: "stub" }`. RFC #753 specced this block as "a reference to a Forge agent definition," but two things have happened since 2026-05-08 when the RFC was written:

1. **"Forge" is dead terminology.** v7 schema migrated `db_forge_agents` into `db_memories` ([#746](https://github.com/agentmuxai/agentmux/pull/746)). Identity + Memory landed as **in-pane tabs** ([#749](https://github.com/agentmuxai/agentmux/pull/749), [#750](https://github.com/agentmuxai/agentmux/pull/750)). v8 added named-agent continuation ([#816](https://github.com/agentmuxai/agentmux/pull/816)) with `instance_name` / `working_directory` / `display_hidden` columns. The Agent block needs to reference `(identity_bundle_id, memory_bundle_id, instance_name?)`, not a single `forge_agent_id`.

2. **The agent pane already has the full runner pipeline.** Identity injection, memory binding, named-agent continuation, tool-chunk streaming via the new `agentmux-bashwrap` crate → WPS event channel ([#804](https://github.com/agentmuxai/agentmux/pull/804), [#809](https://github.com/agentmuxai/agentmux/pull/809)), cost capture from `cost_usd` result events. A workflow Agent block should be a **headless invocation of the same controller**, not a parallel implementation.

This PR introduces shared types so the agent pane and the workflow Agent block consume one event stream, one cost record, one tool surface — DRY in the right place, single audit trail, ready for future workflow→agent→workflow composition.

---

## 2. Out of scope (deferred)

- **Phase 2 trigger surface** (cron / webhook / dependency / schedule) — separate PR series after Phase 1.5.
- **Sub-workflow invocation** (Workflow block calling another Workflow) — Phase 2.
- **Function block sandbox** (`quickjs-rs`) — Phase 2.
- **Cancellation / abort handle** — a deferred P2 from PR #755 ([`engine.rs:64`](../../agentmux-srv/src/workflows/executor/engine.rs)). Land separately.
- **DNS-resolution-time SSRF validation** in the API block — a deferred P2 from PR #755, sibling concern but not gating Phase 1.5.
- **Workflow Agent block streaming into the canvas** — Phase 1.5 delivers `Done` + `AgentRunResult` to downstream blocks; live per-token streaming into a hover-expanded canvas inspector is Phase 2.

---

## 3. Shared type definitions

### 3.1 Rust (`agentmux-srv/src/agents/types.rs` — new module)

```rust
/// Identifies "which agent." Same shape used by the launch modal
/// (interactive agent pane spawn) and the workflow Agent block
/// (headless run). All fields optional — empty-string sentinel
/// matches the existing wstore conventions on AgentInstance.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    /// FK to db_identities.id. Empty = blank singleton (ambient
    /// creds, no env-var injection).
    #[serde(default)]
    pub identity_id: String,
    /// FK to db_memories.id. Empty = blank singleton (vanilla CLI).
    #[serde(default)]
    pub memory_id: String,
    /// User-chosen instance name. Empty for one-shot launches.
    /// Non-empty triggers the named-agent continuation path
    /// (look up existing AgentInstance by name, reuse its
    /// working_directory + session_id if present).
    #[serde(default)]
    pub instance_name: String,
    /// Optional explicit working directory override. Empty falls
    /// back to allocate_agent_workdir() at run time.
    #[serde(default)]
    pub working_directory: String,
}

/// What the agent should do, plus the variables for {{ }} resolution
/// inside the prompt. The agent pane uses prompt=user-typed-text and
/// an empty context; the workflow Agent block uses prompt=block.data.task
/// resolved against scope.outputs + scope.vars.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTask {
    pub prompt: String,
    #[serde(default)]
    pub context: serde_json::Map<String, serde_json::Value>,
    /// Hard cap on turns. None = use the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

/// Discriminated streaming event. Same union for both the agent
/// pane (renders into the UI) and the workflow Agent block
/// (accumulates until Done, returns AgentRunResult).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Streaming text chunk from the assistant — agent pane appends
    /// to its visible transcript; workflow Agent block buffers.
    AssistantText { delta: String },
    /// Tool invocation about to run. `input` is the provider's raw
    /// tool input JSON; renderers may dispatch on tool name.
    ToolUse {
        tool_use_id: String,
        tool: String,
        input: serde_json::Value,
    },
    /// Tool execution result.
    ToolResult {
        tool_use_id: String,
        output: serde_json::Value,
        is_error: bool,
    },
    /// Final cost + token accounting. Emitted once per run.
    Cost {
        cost_usd: f64,
        tokens: TokenCounts,
    },
    /// Run completed successfully. `response` is the final
    /// assistant message text (the workflow Agent block's primary
    /// output). `transcript` is the full ordered turn list for
    /// audit / replay.
    Done {
        response: String,
        transcript: Vec<AgentTurn>,
    },
    /// Run failed. `message` is the user-facing error.
    Error { message: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    #[serde(default)] pub input: u64,
    #[serde(default)] pub output: u64,
    #[serde(default)] pub cache_creation: u64,
    #[serde(default)] pub cache_read: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurn {
    pub role: String,  // "user" | "assistant" | "tool_result"
    pub content: serde_json::Value,
    pub timestamp_ms: i64,
}

/// Final structured result of a complete agent run — the value the
/// workflow Agent block returns to downstream blocks. The agent
/// pane discards this (it's already rendered the stream), but
/// constructs the same struct for the in-progress audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunResult {
    pub response: String,
    pub tokens: TokenCounts,
    pub cost_usd: f64,
    pub transcript: Vec<AgentTurn>,
}
```

### 3.2 TypeScript (`frontend/types/gotypes.d.ts` — re-exported via the existing serde→ts script)

The Rust types are camelCase via `rename_all`, so the TS shape is automatic:

```typescript
type AgentRef = {
    identityId?: string;
    memoryId?: string;
    instanceName?: string;
    workingDirectory?: string;
};

type AgentTask = {
    prompt: string;
    context?: Record<string, unknown>;
    maxTurns?: number;
};

type AgentEvent =
    | { type: "assistant_text"; delta: string }
    | { type: "tool_use"; toolUseId: string; tool: string; input: unknown }
    | { type: "tool_result"; toolUseId: string; output: unknown; isError: boolean }
    | { type: "cost"; costUsd: number; tokens: TokenCounts }
    | { type: "done"; response: string; transcript: AgentTurn[] }
    | { type: "error"; message: string };

type TokenCounts = {
    input: number;
    output: number;
    cacheCreation: number;
    cacheRead: number;
};

type AgentTurn = {
    role: "user" | "assistant" | "tool_result";
    content: unknown;
    timestampMs: number;
};

type AgentRunResult = {
    response: string;
    tokens: TokenCounts;
    costUsd: number;
    transcript: AgentTurn[];
};
```

---

## 4. Backend architecture

### 4.1 New module: `agentmux-srv/src/agents/`

```
agentmux-srv/src/agents/
├── mod.rs              — pub re-exports
├── types.rs            — AgentRef, AgentTask, AgentEvent, etc. (§3.1)
├── runner.rs           — run_agent() unified entry point
└── translator/
    ├── mod.rs          — Translator trait
    ├── claude.rs       — Claude Code stream-json → AgentEvent
    └── acp.rs          — ACP frame → AgentEvent (already exists in part as frontend providers/acp-translator.ts; backend-side mirror)
```

### 4.2 The unified runner

```rust
/// Spawns the agent subprocess (claude / aider / ACP-compatible)
/// per the AgentRef, drains its stdout, translates frames into
/// AgentEvent, broadcasts them on `tx`. Returns a handle whose
/// `final_result` future resolves to AgentRunResult when Done fires.
pub async fn run_agent(
    agent_ref: AgentRef,
    task: AgentTask,
    tx: mpsc::UnboundedSender<AgentEvent>,
) -> Result<AgentRunHandle, AgentError> {
    // 1. Resolve identity_id → Identity bundle (creds, env-var injection)
    // 2. Resolve memory_id → Memory bundle (system instructions, working files)
    // 3. Resolve instance_name → existing AgentInstance row or allocate new
    // 4. Build spawn env (AGENTMUX_AGENT_ID, identity vars, bashwrap WPS endpoint)
    // 5. Spawn subprocess via the existing shell controller
    // 6. Pipe stdout through Translator::translate() → AgentEvent
    // 7. Forward each AgentEvent on `tx`
    // 8. On Done: build AgentRunResult, fulfill the handle's final_result
}

pub struct AgentRunHandle {
    pub instance_id: String,
    pub final_result: oneshot::Receiver<Result<AgentRunResult, String>>,
}
```

### 4.3 Agent pane integration

The current agent pane code in `agentmux-srv/src/backend/blockcontroller/shell.rs` is the spawn path. The refactor:

- **Today**: shell controller spawns claude, parses stream-json via `ClaudeHistoryAdapter` in `agentmux-srv/src/backend/history/claude_adapter.rs`, persists turns into `db_messages`.
- **After Phase 1.5**: shell controller calls `run_agent(agent_ref, task, tx)` where the tx is a fan-out: one sink writes turns to `db_messages` (preserved), another forwards `AgentEvent`s to the frontend via the existing `agentmux-bashwrap` WPS endpoint (so the agent pane's tool-chunk reducer keeps working unchanged).

This is a *refactor*, not a rewrite: the agent pane's user-visible behavior is identical, the underlying spawn just goes through the unified runner.

### 4.4 Workflow Agent block integration

`agentmux-srv/src/workflows/executor/blocks/agent.rs` today returns the stub. Replacement:

```rust
pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let agent_ref: AgentRef = serde_json::from_value(
        node.data.get("agent_ref").cloned().unwrap_or_default()
    ).map_err(|e| format!("agent block: invalid agent_ref: {e}"))?;

    let task_template = node.data.get("task").and_then(|v| v.as_str())
        .ok_or_else(|| "agent block missing `task`".to_string())?;
    let task = AgentTask {
        prompt: scope.resolve(task_template),
        context: scope_to_context_map(scope),
        max_turns: node.data.get("max_turns").and_then(|v| v.as_u64()).map(|n| n as u32),
    };

    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = agents::runner::run_agent(agent_ref, task, tx).await
        .map_err(|e| e.to_string())?;

    // Drain events, forwarding to the run broker so the canvas can
    // surface per-turn progress in the workflow Agent block's
    // hover-expanded inspector (Phase 2 UI). Phase 1.5 just collects.
    while let Some(_ev) = rx.recv().await {
        // Phase 2: re-emit as workflowrun:<id> events
    }

    let result = handle.final_result.await
        .map_err(|e| format!("agent runner cancelled: {e}"))?
        .map_err(|e| format!("agent run failed: {e}"))?;

    Ok(json!({
        "response": result.response,
        "tokens": result.tokens,
        "cost_usd": result.cost_usd,
    }))
}
```

The downstream block reads `{{<agent_block_id>.response}}` for the text and `{{<agent_block_id>.cost_usd}}` for cost.

---

## 5. Frontend architecture

### 5.1 Existing translator pattern stays

`frontend/app/view/agent/providers/claude-translator.ts` and `acp-translator.ts` already translate provider frames → internal events. Phase 1.5 keeps this pattern, with one change: the internal event type becomes the shared `AgentEvent` (from §3.2) instead of an agent-pane-private union. Both providers' translators output `AgentEvent` now.

### 5.2 New consumer: workflow Agent block inspector

When a user selects an Agent block in the workflows canvas and the run is in progress, an inspector panel subscribes to `workflowrun:<id>` events and renders the live `AgentEvent` stream (hover-expand tool blocks, accumulated `AssistantText`, final `Cost`). This is a copy-and-evolve of the agent pane's renderer, *not* a fresh component — the goal is to reuse `agent-view.tsx` block components where possible.

Phase 1.5 ships the type plumbing + a minimal "final result" inspector. Phase 2 polish PR brings the full hover-expand parity.

### 5.3 Reducer integration

The agent pane uses a tool-chunk reducer (`frontend/app/store/agent-pane-state/reducer.ts`). Per the master reducer-stack status, slice #9 (browser pane) just shipped the reducer-routed dispatch pattern. Phase 1.5 introduces a `workflow-run-state/` reducer slice (slice #10) over the per-run `AgentEvent` stream. Commands: `RunStarted | BlockStarted | AgentEvent({ runId, blockId, event }) | BlockDone | RunDone`. The current `workflows-model.ts` view model gets demoted to a thin wrapper that dispatches into the reducer.

This closes the third drift item from the memory note: workflows graduates onto the reducer-stack convention.

---

## 6. Migration plan

| Step | Branch / PR | Description |
|------|------------|-------------|
| 0 | `feat/workflows-phase-1-5/types` | Land §3 types + `agents::runner::run_agent` skeleton. Wire stub claude-code spawn so the runner is callable but blocking. **Zero behavior change** for the agent pane. |
| 1 | `feat/workflows-phase-1-5/refactor-shell-controller` | Refactor `agentmux-srv/src/backend/blockcontroller/shell.rs` to call `run_agent` for claude/aider spawns. Stream output stays identical at the WPS endpoint. Tests verify byte-identical event sequences before/after. |
| 2 | `feat/workflows-phase-1-5/wire-workflow-block` | Replace `workflows/executor/blocks/agent.rs` stub with the real runner call (§4.4). Drop the `"[stub]"` test. |
| 3 | `feat/workflows-phase-1-5/inspector` | Frontend: workflow Agent block inspector subscribed to `workflowrun:<id>` showing final `AgentRunResult` post-completion. Hover-expand parity deferred to Phase 2 polish. |
| 4 | `feat/workflows-phase-1-5/reducer-slice` | Slice #10 reducer for workflow run state. Closes the "workflows-model.ts is not a reducer" drift item. |

PRs 0–4 each independently shippable; Phase 1.5 closes when all four land. Estimated 2 weeks if one engineer dedicates focus.

---

## 7. Acceptance criteria

- [ ] `AgentRef`, `AgentTask`, `AgentEvent`, `AgentRunResult` live in `agentmux-srv/src/agents/types.rs` and mirror exactly in `frontend/types/gotypes.d.ts` (camelCase, `rename_all` confirmed).
- [ ] `run_agent()` spawns Claude Code, drains stream-json, and emits the unified `AgentEvent` sequence. Backend test verifies a recorded transcript replays into a known event sequence.
- [ ] Agent pane spawn path goes through `run_agent()` — verified by a parity test that captures the WPS event stream before and after the refactor and asserts byte-equality.
- [ ] Workflow Agent block runs a real claude task, returns `{response, tokens, cost_usd}` to the next block. End-to-end test: `Variables → Agent → Response` chain with `{{<agent_block_id>.response}}` template.
- [ ] Workflow inspector renders `AgentRunResult` post-completion (markdown response + final cost).
- [ ] No regression on the agent pane's existing tool-chunk reducer tests.
- [ ] `cargo check` clean; `tsc --noEmit` clean (frontend untouched by Rust changes).

---

## 8. Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Shell controller refactor breaks agent pane behavior in subtle ways | High | Parity test captures the WPS event byte stream before refactor and asserts byte-equality after. Land PR 1 behind a feature flag if the parity test surfaces ANY diff. |
| `AgentEvent` design forces a future provider to fork the enum | Medium | Reserve a `Custom { kind: String, data: Value }` variant for provider-specific extensions. Don't ship it Phase 1.5 — but the enum is designed not to preclude it. |
| Workflow Agent block run blocks the executor thread for long agents | Medium | Phase 1 executor is per-block sequential; long agents *will* block. The "live SSE wiring" Phase 2 polish moves this off the request thread. Phase 1.5 documents the limitation. |
| Named-agent continuation collides with workflow runs (same `instance_name`, two callers) | Medium | Workflow Agent block requires a `workflow_run_id` suffix on `instance_name` when set, or the AgentInstance row is locked by the workflow run. Phase 1.5 spec: workflow runs allocate fresh `instance_name` unless the user explicitly opts into a named instance. |
| The agent pane's existing claude_adapter.rs duplicates translator logic | Low | PR 1 deprecates `claude_adapter.rs` in favor of `agents::translator::claude`. Migration is mechanical; deprecation comment + removal in PR 2. |

---

## 9. References

- RFC #753 (Workflows pane)
- PR #755 (Workflows Phase 1 — supersedes this in code, supersedes by this in design)
- PR #816 (named-agent continuation + v8 schema)
- PR #746 (v7 schema: Forge → Identity + Memory)
- PR #749 (Memory pane), #750 (Identity pane)
- PR #804 (`agentmux-bashwrap` β.A), #809 (β.B wiring)
- Existing memory: [`project_workflows_phase_1_5.md`](../../../../.claude/projects/C--Systems/memory/project_workflows_phase_1_5.md)
- Master reducer status: `docs/specs/MASTER_REDUCER_STACK_STATUS_2026-05-05.md`
