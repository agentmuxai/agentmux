# SPEC: AskUserQuestion — interactive agent questions in the agent pane

**Date:** 2026-06-15
**Status:** Validated — implementing Phase 1 (smoke test passed 2026-06-15; see §10)
**Owner:** Agent pane (frontend stream/reducer/render) + sidecar (controller stdin delivery)
**Related:** `SPEC_DECISION_PROMPT_2026_04_24.md` (sibling feature — tool *permission* gating, distinct from this), `ANALYSIS_AGENT_APP_API_OPEN_IN_EDITOR_2026_05_30.md` (agent→app callback precedent)

---

## 1. Summary

When an agent (Claude Code, and any provider that supports it) calls the
**`AskUserQuestion`** tool, it is asking the human a structured multiple-choice
question and **blocking its turn** until it receives a `tool_result`. Today
AgentMux does not recognise this tool: the call flows through the translator as a
generic `tool_call`, renders as an ordinary "running" tool node, and **never
completes** — because nothing in AgentMux ever sends the answer back. The agent
hangs until the turn times out or the user kills it. (This is the exact failure
the user observed: an `AskUserQuestion` call inside an AgentMux agent pane errored
out with no way to answer.)

This spec defines:
1. **Detection** — recognise the `AskUserQuestion` tool_use in the stream and
   model it as a first-class "awaiting answer" state on the tool node.
2. **Rendering** — an interactive question panel (N questions, single- or
   multi-select options, a free-text "Other") in the agent pane.
3. **Delivery** — route the user's answer back into the running agent CLI as a
   `tool_result` content block over the **persistent controller's live stdin**,
   unblocking the agent's turn.

This is **not** the same feature as `SPEC_DECISION_PROMPT` (Allow/Deny gating of a
tool the agent wants to run). AskUserQuestion is a tool the agent *deliberately
invokes* to consult the human; the answer is data the model consumes, not a
permission verdict. The two share UI patterns but are distinct flows.

---

## 2. The load-bearing finding: the delivery channel already exists

The decision-prompt spec (§9.1) flagged answer-delivery as the unsolved blocker —
because it analysed the **one-shot subprocess** controller, which runs the CLI in
`-p` / `--print` mode with no live stdin. That blocker **does not apply to host
agents**, which use the **persistent** controller:

- `frontend/app/view/agent/providers/index.ts:161` — the Claude provider's
  `persistentLaunchArgs` are
  `["--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--dangerously-skip-permissions"]`.
  This is a **bidirectional** NDJSON session: the CLI reads user messages from
  stdin for the life of the process.
- `agentmux-srv/src/backend/blockcontroller/persistent.rs:4-11` — "A single CLI
  process is spawned on first message and kept alive for the entire session. User
  messages are written as NDJSON lines to stdin **without closing it**. This
  enables mid-turn input."
- `persistent.rs:146-168` — `send_message()` already writes
  `{"type":"user","message":{"role":"user","content":<string>}}` to the live
  stdin. An AskUserQuestion answer is the same channel with a different content
  shape: a `tool_result` block instead of a text turn.

So for **host (persistent) agents the delivery path is feasible today**. The
one-shot subprocess path (container agents, providers launched per-turn with `-p`)
is Phase 2 — see §9.

> **Validation gate (must verify before building):** does
> `claude --input-format stream-json --output-format stream-json --dangerously-skip-permissions`
> actually *emit* an `AskUserQuestion` `tool_use` and then *consume* a matching
> `tool_result` to continue, or does Claude Code suppress the tool in
> headless/stream-json mode? The entire Phase-1 design hinges on this being
> "emits + consumes". §10 specifies the smoke test. If Claude suppresses it,
> Phase 1 still applies to any provider that does emit it, and the Claude path
> becomes a CLI-flag/upstream question.

---

## 3. Current state (evidence)

| Concern | State | Reference |
|---|---|---|
| `AskUserQuestion` recognised anywhere | ❌ literal string absent in repo | grep `docs/ specs/ frontend/ agentmux-srv/` → none |
| Tool_use → StreamEvent | ✅ generic | `claude-translator.ts:174-196`, `:301-322`, `:363-389` emit `{type:"tool_call", tool, id, params}` |
| StreamEvent → DocumentNode (ToolNode) | ✅ generic | `stream-parser.ts` `eventToNode()` switch; unknown tools → `tool: "Other"` |
| ToolNode model + status enum | ✅ | `frontend/app/view/agent/types.ts:207-243` — `status: "running" \| "pending_approval" \| "success" \| "failed" \| "denied" \| "canceled"`, plus `pendingPermission?` |
| Interactive panel pattern | ✅ (for permissions) | `components/AgentDecisionPanel.tsx`; collected via `pendingDecisions()` in `agent-view.tsx:432-489`, submitted via `RpcApi.ToolDecisionCommand` |
| Decision RPC backend | ⚠️ audit-log only | `websocket.rs:717-774` — validates + logs; delivery deferred |
| **Live stdin to running agent** | ✅ **persistent only** | `persistent.rs:146-168` `send_message()` |
| Document reducer | ✅ | `agent-document-store.ts:84-158` dispatch; state `nodes/sessionPhase/nodeIdSet` in `agent-document/types.ts:17-37`; commands in `:53-131` (`StreamFlush`, `ToolChunkAppend`, …) |
| Pane-state reducer (turn phase) | ✅ | `agent-pane-state/reducer.ts:65-200`; `TurnPhase` union in `types.ts:118-152` |

**Net:** an `AskUserQuestion` call today becomes a `ToolNode{ tool:"Other",
status:"running" }` that never resolves. Everything needed to fix it — translator,
reducer, node model, panel pattern, live stdin — exists; it must be connected and
given a question-specific shape.

---

## 4. The `AskUserQuestion` tool contract

The tool input (Anthropic/Claude Code schema) is an object with a `questions`
array (1–4 questions). Each question:

```jsonc
{
  "questions": [
    {
      "question": "Which auth method should we use?",
      "header": "Auth method",          // short chip label (≤12 chars)
      "multiSelect": false,              // true → checkbox semantics
      "options": [
        { "label": "OAuth (Recommended)", "description": "Browser-based device flow" },
        { "label": "API key",             "description": "Static key in settings" }
      ]
    }
    // … up to 4 questions
  ]
}
```

Semantics AgentMux must honour:
- **1–4 questions**, rendered together in one panel; all must be answered before
  submit (unless the panel allows partial → see §8 open questions).
- **`multiSelect`** — single-select (radio) vs multi-select (checkbox) per question.
- **Free-text "Other"** — the harness always allows the user to supply custom text
  instead of a listed option. The panel MUST offer an "Other" free-text entry per
  question.
- The agent expects a **`tool_result`** keyed on the tool_use `id`, whose content
  conveys the user's selection(s) per question.

---

## 5. Architecture overview

```
 Claude CLI (persistent, stream-json)
        │  tool_use: AskUserQuestion {questions:[…]}   (stdout NDJSON)
        ▼
 useAgentStream → ClaudeTranslator.translate()           [unchanged: emits tool_call]
        ▼
 ClaudeCodeStreamParser.eventToNode()                    [NEW: special-case AskUserQuestion]
        │  → ToolNode{ tool:"AskUserQuestion", status:"awaiting_answer", question:{…} }
        ▼
 dispatchDoc(StreamFlush)  → agent-document reducer       [node carries `question`]
        ▼
 AgentDocumentVirtualList renders the node
        │
 agent-view.tsx pendingQuestions()  ─────────────────────[NEW: mirror of pendingDecisions()]
        ▼
 <AgentQuestionPanel question=… onAnswer=…>               [NEW component]
        │  user picks options / types "Other" / submits
        ▼
 RpcApi.AgentAnswerCommand({ blockid, tool_use_id, answers })   [NEW RPC]
        ▼
 websocket.rs COMMAND_AGENT_ANSWER handler                [NEW handler]
        ▼
 PersistentSubprocessController.send_tool_result(id, content)   [NEW method]
        │  writes {"type":"user","message":{"role":"user",
        │           "content":[{"type":"tool_result","tool_use_id":id,"content":…}]}}
        ▼
 Claude CLI consumes tool_result → turn continues → emits next assistant message
        ▼
 ToolNode transitions running→success (answer echoed as its result)
```

---

## 6. Data model changes

### 6.1 `frontend/app/view/agent/types.ts`

Add the request shape (parsed from the tool input) and an answer shape:

```typescript
export interface AskUserQuestionOption {
    label: string;
    description?: string;
}

export interface AskUserQuestionItem {
    question: string;
    header: string;
    multiSelect: boolean;
    options: AskUserQuestionOption[];
}

export interface AskUserQuestionRequest {
    type: "ask_user_question";
    tool_use_id: string;            // the AskUserQuestion tool_use id; echoed in the tool_result
    questions: AskUserQuestionItem[];
}

/** One question's answer. `selected` are chosen option labels; `other`
 *  is free-text the user typed instead of (or in addition to) options. */
export interface AskUserQuestionAnswer {
    header: string;
    selected: string[];             // chosen option labels (length 1 for single-select)
    other?: string;                 // free-text "Other"
}
```

Extend `ToolNode` (mirrors the existing `pendingPermission` pattern):
- add `"awaiting_answer"` to the `status` union,
- add `question?: AskUserQuestionRequest`.

> **Modeling choice:** AskUserQuestion is carried on `ToolNode`, not a new node
> type, because it *is* a tool_use with a tool lifecycle: `awaiting_answer` →
> (answer delivered as tool_result) → `success` with the answer as `result`. This
> reuses the tool node's rendering, dedup index, and the `pendingPermission`
> collection analogue with the least new plumbing.

### 6.2 Backend wire types — `agentmux-srv/src/backend/rpc_types.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandAgentAnswerData {
    pub blockid: String,
    pub tool_use_id: String,
    /// Canonical, human-readable rendering of the user's selections, ready to
    /// hand to the model as the tool_result content (see §7.3 for the format).
    pub answer_text: String,
}
```

Add `COMMAND_AGENT_ANSWER: &str = "agent.answer"` alongside the existing command
constants.

---

## 7. Pipeline changes (implementation-ready)

### 7.1 Detection — `frontend/app/view/agent/stream-parser.ts`

In `eventToNode()` (the `tool_call` case that today builds a `ToolNode`), branch on
`event.tool === "AskUserQuestion"`:
- when the params are fully parsed (the translator emits a final `tool_call` with
  populated `params` from `content_block_stop` / the top-level `assistant`
  message — `claude-translator.ts:363-389`, `:174-196`),
- build the node with `status: "awaiting_answer"` and
  `question: { type:"ask_user_question", tool_use_id: event.id, questions: params.questions }`.

Guard: if `params.questions` is missing/empty (e.g. only the streaming
`content_block_start` placeholder arrived), keep `status:"running"` and let the
later fully-parsed `tool_call` upgrade it — the node-id dedup in `useAgentStream`
already updates an existing tool node in place.

No translator change is required — `AskUserQuestion` already arrives as a
`tool_call`. Detection lives entirely in the parser/reducer layer.

### 7.2 Rendering — new `components/AgentQuestionPanel.tsx` + `agent-view.tsx`

- **Collection:** add `pendingQuestions()` next to `pendingDecisions()`
  (`agent-view.tsx:432-489`): scan document nodes for
  `node.type === "tool" && node.status === "awaiting_answer" && node.question`,
  oldest-first. Render the panel for the head of the queue (same single-active
  pattern the decision panel uses).
- **Panel** (`AgentQuestionPanel.tsx`, mirroring `AgentDecisionPanel.tsx`):
  - one section per question: header chip + prompt + options as radios
    (`multiSelect:false`) or checkboxes (`multiSelect:true`), plus an **"Other"**
    free-text input,
  - keyboard-driven (number keys to toggle options, Enter to submit, Esc to
    minimise/defer), matching the decision panel's interaction language,
  - submit is disabled until every question has at least one selection or an
    "Other" value,
  - on submit, encode answers (§7.3) and call
    `props.onAnswer({ tool_use_id, answers })`.
- **Submit handler** in `agent-view.tsx` (`handleAnswer`, mirror of `handleDecide`)
  builds `answer_text` and dispatches `RpcApi.AgentAnswerCommand(TabRpcClient,
  { blockid, tool_use_id, answer_text })`. Optimistically flip the node to a
  pending/answered state so the panel dismisses immediately; the real `success`
  transition lands when the tool_result is echoed back in the stream.

### 7.3 Answer encoding (the `tool_result` content)

The model needs a clear, unambiguous rendering. Canonical format (one block of
text), per question:

```
<header>: <comma-joined selected labels>[, Other: <free text>]
```

Example for a two-question answer:

```
Auth method: OAuth (Recommended)
Library: date-fns, Other: luxon
```

This string is `CommandAgentAnswerData.answer_text` and becomes the `tool_result`
content. (Rationale: a flat labelled block is robust for the model to parse and
mirrors how the question was posed. A structured JSON tool_result is a possible
future refinement — see §8.)

### 7.4 Delivery — `agentmux-srv`

**RPC handler** (`websocket.rs`, register `COMMAND_AGENT_ANSWER` near
`COMMAND_AGENT_INPUT` at `:844`): parse `CommandAgentAnswerData`, look up the
controller via `blockcontroller::get_controller(&cmd.blockid)`, downcast to
`PersistentSubprocessController`, and call the new `send_tool_result`. If the
controller is not persistent (container/one-shot), return a typed error
(`UNSUPPORTED_CONTROLLER`) so the frontend can show "answering isn't supported for
this agent yet" — Phase 2 (§9).

**Controller method** (`persistent.rs`, beside `send_message` at `:146`):

```rust
/// Deliver an AskUserQuestion answer as a tool_result on the live stdin,
/// unblocking the agent's turn. Mirrors send_message but emits a user turn
/// whose content is a single tool_result block keyed on the tool_use id.
pub fn send_tool_result(&self, tool_use_id: String, content: String) -> Result<(), String> {
    let json_msg = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [
                { "type": "tool_result", "tool_use_id": tool_use_id, "content": content }
            ]
        }
    });
    let inner = self.inner.lock().unwrap();
    let tx = inner.stdin_tx.as_ref()
        .ok_or("persistent process not running")?;
    tx.try_send(json_msg.to_string())
        .map_err(|e| format!("stdin send failed: {e}"))
}
```

No new env/identity injection is needed (the process is already running; we are
only writing to its stdin). The existing stdin-writer task (`persistent.rs:317-335`)
appends the newline and flushes.

### 7.5 Completion

When the CLI consumes the `tool_result`, it continues the turn and the existing
stream path renders the next assistant content. The AskUserQuestion ToolNode
should transition `awaiting_answer → success`; drive this from the optimistic
update in 7.2 (and/or a matching `tool_result` echo if the provider emits one). The
`result` carries `answer_text` for display.

---

## 8. UX details

- **One panel at a time** — queue multiple pending questions oldest-first (reuse
  the decision-panel single-active discipline).
- **Per-question validation** — submit disabled until each question is answered
  (selection or "Other").
- **"Other" is always present** — required by the tool contract; never hide it.
- **Multi-select** renders checkboxes; **single-select** renders radios. "Other"
  coexists with selections for multi-select; for single-select, choosing "Other"
  clears any radio selection.
- **Defer/minimise** (Esc) — collapses the panel but keeps the node in
  `awaiting_answer`; a banner/affordance lets the user reopen it. The agent stays
  blocked (it asked a question), so the panel must remain reachable.
- **Dismiss safety** — there is no "ignore" that silently drops the question: the
  agent is blocked on a tool_result. If the user truly wants to abandon, offer an
  explicit "Skip / answer later" that either (a) keeps it pending, or (b) sends a
  tool_result of "(user declined to answer)" so the agent can proceed. Default:
  keep pending; never auto-send without user action.

---

## 9. Phasing

**Phase 1 — host (persistent) agents, Claude.**
- Detection + node model + `AgentQuestionPanel` + `agent.answer` RPC +
  `send_tool_result`. Single & multi-select & "Other". Local host agents only.
- Gated on the §2 validation result.

**Phase 2 — one-shot / container agents.**
- The subprocess controller has no live stdin. Options: resume the session via the
  provider's `--resume <session_id>` with the tool_result as the new input
  (a fresh subprocess per answer), or buffer the answer for the next turn. Decide
  per-provider. Until then, those agents surface "answering not supported on this
  connection" (the `UNSUPPORTED_CONTROLLER` path from 7.4).

**Phase 3 — refinements.**
- Structured (JSON) tool_result instead of the flat text block, if the model
  benefits. Cross-provider parity (Gemini/Codex/Kimi translators emit the same
  `tool_call` shape — most of Phase 1 is provider-agnostic once detection keys on
  the tool name the provider uses). Persisted answer history in the transcript.

---

## 10. Validation / smoke test — ✅ PASSED 2026-06-15

Run against the user's `claude` (`C:\Users\asafe\.local\bin\claude`) with the
**exact** persistent args from §2 (`--input-format stream-json --output-format
stream-json --verbose --include-partial-messages --dangerously-skip-permissions`,
plus `--model sonnet`). Two-phase Python harness (real OS pipes; an MSYS FIFO won't
reach the native Windows binary). Result:

```
emits_AskUserQuestion = True      # tool_use emitted, turn blocked (no 'result' first)
consumes_tool_result  = True      # after the tool_result, the turn produced a 'result'
```

Confirmed exact wire shape of the tool_use (matches §4/§6 schema):

```jsonc
{ "type":"tool_use", "id":"toolu_…", "name":"AskUserQuestion",
  "input": { "questions": [
    { "question":"Pick a color", "header":"Color", "multiSelect":false,
      "options":[ {"label":"Red","description":"Red"}, {"label":"Blue","description":"Blue"} ] }
  ] },
  "caller": { "type":"direct" } }
```

And the answer that unblocked it:
`{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_…","content":"Color: Blue"}]}}`

**Conclusions:** (a) `--dangerously-skip-permissions` does **not** suppress the
tool; (b) `input.questions[]` is the exact schema; (c) a **flat labelled-text**
tool_result content is consumed correctly — so §7.3's encoding is validated. Phase 1
is unblocked. Original procedure retained below for re-validation on CLI upgrades.

---

### Original procedure (for re-validation)

Before writing UI, confirm the contract empirically against the bundled Claude CLI:

1. Spawn `claude` with the persistent args (§2) in a scratch dir.
2. Send a user turn that reliably elicits an `AskUserQuestion` (e.g. "Ask me a
   multiple-choice question about X using the AskUserQuestion tool").
3. Capture stdout NDJSON. Confirm a `tool_use` with `name:"AskUserQuestion"` and a
   `questions` array appears, and that the turn **blocks** (no `result` event).
4. Write to stdin:
   `{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"<id>","content":"Auth method: OAuth"}]}}`
5. Confirm the agent **continues** the turn (emits further assistant content /
   `result`) — i.e. it consumed the tool_result.

If (3) shows the tool is suppressed in this mode, escalate to a CLI-flag/upstream
question and scope Phase 1 to whichever provider does emit it. If (5) fails, the
content shape is wrong — adjust the `tool_result` block accordingly.

---

## 11. Files to touch (summary)

| File | Change |
|---|---|
| `frontend/app/view/agent/types.ts` | `AskUserQuestion{Option,Item,Request,Answer}` types; `ToolNode.status += "awaiting_answer"`; `ToolNode.question?` |
| `frontend/app/view/agent/stream-parser.ts` | special-case `tool === "AskUserQuestion"` in `eventToNode()` |
| `frontend/app/view/agent/components/AgentQuestionPanel.tsx` | **new** — interactive question panel (mirror of `AgentDecisionPanel`) |
| `frontend/app/view/agent/agent-view.tsx` | `pendingQuestions()`, `handleAnswer()`, render the panel |
| `frontend/app/store/rpc-api.ts` | `AgentAnswerCommand` binding |
| `agentmux-srv/src/backend/rpc_types.rs` | `CommandAgentAnswerData`, `COMMAND_AGENT_ANSWER` |
| `agentmux-srv/src/server/websocket.rs` | register `COMMAND_AGENT_ANSWER` handler |
| `agentmux-srv/src/backend/blockcontroller/persistent.rs` | `send_tool_result()` |

---

## 12. Non-goals

- Permission/Allow-Deny gating — that's `SPEC_DECISION_PROMPT_2026_04_24.md`.
- One-shot/container answer delivery (Phase 2).
- Persisting/replaying answered questions across session reloads beyond what the
  normal transcript already captures (Phase 3).
- A generic "agent asks app for arbitrary input" framework — this spec is scoped to
  the concrete `AskUserQuestion` tool contract. Generalisation can follow once the
  one verb is solid (same philosophy as the `pane.open`/`amux` analysis).

---

## 13. Open questions — RESOLVED (2026-06-15)

- **Validation gate (§2/§10):** ✅ **RESOLVED** — Claude Code *both* emits the
  `AskUserQuestion` tool_use *and* consumes a `tool_result` to continue, under the
  exact persistent args. Smoke test passed (§10).
- **`--dangerously-skip-permissions` interaction:** ✅ **RESOLVED** — does not
  suppress the tool; it still fires (confirmed in §10).
- **Answer format:** ✅ **RESOLVED — flat labelled text** (§7.3). The smoke test fed
  `"Color: Blue"` and the model consumed it cleanly. Structured JSON deferred to
  Phase 3 only if a real mis-parse surfaces.
- **Optimistic vs echo-driven completion:** ✅ **RESOLVED — optimistic on submit,
  reconciled by the stream.** Flip the node out of `awaiting_answer` immediately so
  the panel dismisses; the subsequent assistant content / `result` (which the smoke
  test confirmed arrives) drives the final `success`. No artificial timeout needed
  in the happy path; keep a long fallback only to clear a stuck panel if the process
  died.
- **Defer semantics:** ✅ **RESOLVED — keep-pending.** Esc minimises; the node stays
  `awaiting_answer` and reachable. Never auto-decline or auto-send. An explicit
  "answer later" affordance keeps it pending; no silent drop.

No blocking unknowns remain for Phase 1.
