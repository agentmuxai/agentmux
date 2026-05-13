# SPEC: Unified agent type system (Workflows Phase 1.5)

**Date:** 2026-05-13
**Status:** Draft — lands after PR #755 (Workflows Phase 1) merges
**Author:** AgentA
**Issue:** RFC #753 (Workflows pane), follow-up to PR #755

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
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
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

### 4.2 What's shared, and what isn't

An audit of the existing agent-pane code surfaced an important wrinkle: **the spawn function is NOT naturally shareable** between the two consumers.

- The **agent pane** spawns `claude` as a long-lived PTY subprocess. User input flows into stdin, output streams forever, the session is multi-turn and interactive. There's no per-task lifecycle at the spawn level — `claude` itself owns the conversation.
- The **workflow Agent block** is one-shot: send a task, drain events until `Done`, return `AgentRunResult` to the next block. No PTY, no stdin, no multi-turn.

What IS naturally shareable is the **translator + event shape**. Both consumers receive Claude's stream-json on stdout; both want it converted to `AgentEvent`s. The pane's read loop and the workflow runner are *different spawn drivers* feeding the *same translator* and emitting the *same event union*.

```
                            ┌─→ AssistantText / ToolUse / ToolResult / Cost / Done
ClaudeTranslator (shared) ──┤
                            ↑
        stream-json lines from claude stdout
                            ↑
       ┌────────────────────┴────────────────────┐
   Pane PTY read loop                       Workflow one-shot runner
   (multi-turn, interactive,                (`run_agent` — single
   user stdin via PTY,                       task, no PTY, headless)
   long-lived session)
```

### 4.3 The one-shot runner (workflows only)

```rust
/// Spawn `claude` as a non-interactive one-shot for the workflow
/// Agent block: prompt → drain stream-json → emit AgentEvents →
/// resolve `final_result` with AgentRunResult on Done. Headless;
/// no PTY, no stdin pipe back to the user.
///
/// The agent pane does NOT use this — it has its own multi-turn
/// PTY-driven spawn in `blockcontroller/shell.rs` which shares only
/// the translator.
pub async fn run_agent(
    agent_ref: AgentRef,
    task: AgentTask,
    tx: mpsc::UnboundedSender<AgentEvent>,
) -> Result<AgentRunHandle, AgentError> {
    // 1. Resolve identity_id → Identity bundle (creds, env-var injection)
    // 2. Resolve memory_id → Memory bundle (system instructions, working files)
    // 3. Allocate working directory (no continuation — workflows always
    //    spawn fresh AgentInstance rows for audit clarity)
    // 4. Spawn `claude --print --output-format=stream-json` with prompt
    //    on argv, capturing stdout
    // 5. Pipe stdout lines through ClaudeTranslator → AgentEvent
    // 6. Forward each AgentEvent on `tx`
    // 7. On Done: build AgentRunResult, fulfill the handle's final_result
}

pub struct AgentRunHandle {
    pub instance_id: String,
    pub final_result: oneshot::Receiver<Result<AgentRunResult, String>>,
}
```

### 4.4 Agent pane integration

The agent pane code in `agentmux-srv/src/backend/blockcontroller/shell.rs` keeps its PTY model — that's what enables interactive multi-turn use. What changes is **how the read loop interprets stdout**:

- **Today**: read loop drains PTY stdout in 4 KB chunks, publishes each chunk to WPS as `EVENT_BLOCK_FILE` (raw terminal data, the pane renders it as a terminal).
- **After Phase 1.5**: read loop additionally line-buffers stdout, feeds each parsed JSON line through `ClaudeTranslator`, and emits the resulting `AgentEvent`s on a second channel alongside the existing raw-chunk publish. The raw-chunk path stays byte-equal so the pane's tool-chunk reducer keeps working unchanged.

This is an **additive** change — zero risk of breaking the pane's existing behavior. The new `AgentEvent` stream becomes available for the in-pane Identity/Memory cog tabs, future per-turn audit views, etc.

### 4.5 Workflow Agent block integration

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

    // NOTE: This output object is *manually constructed* to match the
    // existing workflow block convention (snake_case keys, like API's
    // `status`/`body`, Condition's `result`, Response's `value`).
    // Do NOT use `serde_json::to_value(&result)` here — that would
    // emit camelCase (`costUsd`) because AgentRunResult carries
    // `#[serde(rename_all = "camelCase")]` for IPC consistency with
    // the frontend, breaking `{{<block_id>.cost_usd}}` templates.
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

The plan was revised after the §4.2 audit: PR 1 no longer refactors the pane's spawn function (it doesn't fit). Instead, the pane keeps its PTY model and gains parallel `AgentEvent` emission via the shared translator.

| Step | Branch / PR | Description |
|------|------------|-------------|
| 0 | `agenta/workflows-phase-1-5-types` (#831) | Land §3 types + `agents::runner::run_agent` skeleton (`NotImplemented`) + `ClaudeTranslator` skeleton. **Zero behavior change** anywhere. |
| 1 | `feat/workflows-phase-1-5-translator` | Implement `ClaudeTranslator::translate()` for the full stream-json frame set. Wire it into `shell.rs`'s read loop *additively*: existing raw-chunk WPS publish stays byte-equal, new `AgentEvent` stream emits in parallel on a second channel. Golden-file tests over recorded stream-json sessions verify the translation. |
| 2 | `feat/workflows-phase-1-5-runner` | Implement `run_agent()` as a one-shot headless spawn (`claude --print --output-format=stream-json` style). Replace `workflows/executor/blocks/agent.rs` stub with a `run_agent` call. End-to-end test: `Variables → Agent → Response` chain. |
| 3 | `feat/workflows-phase-1-5-inspector` | Frontend: workflow Agent block inspector subscribed to `workflowrun:<id>` showing final `AgentRunResult` post-completion. Hover-expand parity deferred to Phase 2 polish. Closes issue #830. |
| 4 | `feat/workflows-phase-1-5-reducer-slice` | Slice #10 reducer for workflow run state. Closes the "workflows-model.ts is not a reducer" drift item. |

PRs 0–4 each independently shippable. The pane's PTY behavior is untouched throughout; the byte-equal WPS path is preserved across all four PRs.

---

## 7. Acceptance criteria

- [ ] `AgentRef`, `AgentTask`, `AgentEvent`, `AgentRunResult`, `AgentTurn`, `TokenCounts` live in `agentmux-srv/src/agents/types.rs` and mirror exactly in `frontend/types/gotypes.d.ts` (camelCase via `serde(rename_all)` + `rename_all_fields`).
- [ ] `ClaudeTranslator::translate()` converts the full stream-json frame set to `AgentEvent`s. Golden-file tests over recorded sessions verify the translation byte-for-byte.
- [ ] Agent pane's `shell.rs` read loop emits `AgentEvent`s in parallel with its existing raw-chunk WPS publish. Byte-equal parity test on the raw-chunk path confirms zero regression on the pane.
- [ ] `run_agent()` spawns `claude --print --output-format=stream-json`, drains the line stream through `ClaudeTranslator`, emits `AgentEvent`s, resolves `AgentRunResult` on `Done`.
- [ ] Workflow Agent block runs a real claude task, returns `{response, tokens, cost_usd}` to the next block. End-to-end test: `Variables → Agent → Response` chain with `{{<agent_block_id>.response}}` template.
- [ ] Workflow inspector renders `AgentRunResult` post-completion (markdown response + final cost).
- [ ] No regression on the agent pane's existing tool-chunk reducer tests.
- [ ] `cargo check` clean; `tsc --noEmit` clean.

---

## 8. Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Pane's additive `AgentEvent` emission breaks existing behavior in subtle ways | Medium | The translator's output goes to a *new* channel, separate from the existing WPS raw-chunk path. Byte-equal parity test on the raw-chunk path is the gate before merge. |
| `ClaudeTranslator` misinterprets a stream-json frame variant we haven't seen | Medium | Golden-file tests use real recorded sessions (long agent runs with tool use, errors, cancellations). Unknown frames return empty `Vec<AgentEvent>` rather than panic — the pane's raw-chunk path still publishes the underlying text. |
| `AgentEvent` design forces a future provider to fork the enum | Medium | Reserve a `Custom { kind: String, data: Value }` variant for provider-specific extensions. Don't ship it Phase 1.5 — but the enum is designed not to preclude it. |
| Workflow Agent block run blocks the executor thread for long agents | Medium | Phase 1 executor is per-block sequential; long agents *will* block. Phase 2 moves this off the request thread. Phase 1.5 documents the limitation. |
| Workflow `run_agent` spawn collides with named-agent continuation | Low | Workflow runs always allocate fresh `instance_name` (never reuse). The named-agent continuation path stays exclusive to the pane. |
| `claude_adapter.rs` (history file parser) duplicates translator logic | Low | The history parser stays — it operates on JSONL files for past-session browsing, not the live stream. Different lifetime, different inputs, no consolidation needed. |

---

## 9. References

- RFC #753 (Workflows pane)
- PR #755 (Workflows Phase 1 — supersedes this in code, supersedes by this in design)
- PR #816 (named-agent continuation + v8 schema)
- PR #746 (v7 schema: Forge → Identity + Memory)
- PR #749 (Memory pane), #750 (Identity pane)
- PR #804 (`agentmux-bashwrap` β.A), #809 (β.B wiring)
- Master reducer status: `docs/specs/MASTER_REDUCER_STACK_STATUS_2026-05-05.md`
