# SPEC: Agent Control Protocol — fix AskUserQuestion (+ unblock tool-permission UI) and align muxbus delivery

**Date:** 2026-06-15
**Status:** Draft — evidence captured, ready to implement
**Owner:** agent pane / sidecar (persistent controller)
**Supersedes the delivery design in:** `SPEC_ASK_USER_QUESTION_2026_06_15.md` (its §2/§7 premise — "deliver a `tool_result` on stdin to answer AskUserQuestion" — is empirically **disproven**; see §2 below).
**Related/unblocked:** `SPEC_DECISION_PROMPT_2026_04_24.md` (#551 tool-permission UI), `SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md` (Tier-1 injection coupling, §6).

---

## 1. Summary

The built-in **`AskUserQuestion`** tool never works in an AgentMux Claude pane: the CLI emits the tool_use and then **auto-rejects it with `Error: Answer questions?`** ~3–7 ms later, regardless of controller (subprocess `-p` *or* persistent `--input-format stream-json`) and regardless of model. No question panel ever renders.

**Root cause (proven, §2):** in any headless/stream-json mode the CLI has no interactive surface for AskUserQuestion. The *only* supported way to answer it is the **Claude Agent SDK control protocol**: the driver launches the CLI with `--permission-prompt-tool stdio`, and answers a `can_use_tool` **`control_request`** with a `control_response` carrying `updatedInput.answers`. AgentMux never speaks this protocol — it pipes user messages and reads assistant messages, nothing more. Worse, it launches Claude with **`--dangerously-skip-permissions`, which disables the very routing AskUserQuestion needs.**

This spec replaces the (wrong) tool_result-on-stdin delivery with a real **control-protocol client** in the persistent controller. The same channel also carries **tool-permission** `can_use_tool` requests — so implementing it once unblocks both AskUserQuestion **and** the per-tool decision prompt (#551). Finally, persistent stream-json Claude has **no PTY**, so muxbus Tier-1 injection (currently raw PTY writes) must become controller-aware (§6).

---

## 2. Evidence (captured 2026-06-15, bundled CLI v2.1.178)

All four findings below are from direct measurement, not docs.

### 2.1 The bug reproduces in BOTH controllers, BOTH models

- Real subprocess agent (block `27b6f287`, opus): tool_use `22:17:52.361` → `tool_result {"content":"Answer questions?","is_error":true}` `22:17:52.368` (**+7 ms**), turn continued.
- Real persistent agent "Makp" (block `55a0265a`, opus, launched with `--input-format stream-json`): tool_use `22:57:59.764` → auto-error `22:57:59.767` (**+3 ms**). Persistent made **no** difference.
- Clean standalone harness (no AgentMux), bundled CLI, persistent args, **sonnet AND opus**: `emitted=True, auto_error=True, turn_ended=True` both times.

→ The controller flip (PR #1451) is necessary-but-insufficient; the spec's §10 "validation passed" does **not** reproduce.

### 2.2 The control protocol DOES make it work

Driving the **bundled** CLI v2.1.178 through the official `@anthropic-ai/claude-agent-sdk` with a `canUseTool` callback (executable pointed at the bundled binary via a tee-proxy):

```
ANSWERED_VIA_canUseTool = true | AUTO_ERROR_SEEN = false
RESULT: "You picked Red! 🔴"
```

The model consumed the answer we returned. No "Answer questions?".

### 2.3 Exact wire protocol (captured bytes, request_ids preserved)

1. **Driver → CLI — initialize** (first stdin control message):
   ```json
   {"type":"control_request","request_id":"n5eu8utar8","request":{"subtype":"initialize","systemPrompt":[""]}}
   ```
2. **CLI → driver — initialize response** (echoes `request_id`, returns slash-commands etc.):
   ```json
   {"type":"control_response","response":{"subtype":"success","request_id":"n5eu8utar8","response":{"commands":[…]}}}
   ```
3. **CLI → driver — can_use_tool** (when AskUserQuestion fires):
   ```json
   {"type":"control_request","request_id":"04d963d9-…","request":{
     "subtype":"can_use_tool","tool_name":"AskUserQuestion","display_name":"AskUserQuestion",
     "input":{"questions":[{"question":"…","header":"…","options":[…],"multiSelect":false}]},
     "tool_use_id":"toolu_01Lx…"}}
   ```
4. **Driver → CLI — control_response with the answer** (echoes `request_id`):
   ```json
   {"type":"control_response","response":{"subtype":"success","request_id":"04d963d9-…","response":{
     "behavior":"allow",
     "updatedInput":{"questions":[…original…],"answers":{"<question text>":"<label|[labels]|freetext>"}},
     "toolUseID":"toolu_01Lx…"}}}
   ```

Correlation key is `request_id` (CLI→driver requests use a UUID; driver→CLI requests use a short token). For permission gating of an ordinary tool, the same `can_use_tool` request arrives with that tool's name; the response is `{"behavior":"allow","updatedInput":…}` or `{"behavior":"deny","message":…}`.

### 2.4 The launch-flag diff (the load-bearing change)

| Purpose | SDK (works) | AgentMux persistent today (broken) |
|---|---|---|
| streaming I/O | `--input-format stream-json --output-format stream-json --verbose` | same ✓ |
| **enable control protocol** | **`--permission-prompt-tool stdio`** | **absent** ✗ |
| **permission mode** | **`--permission-mode default`** | **`--dangerously-skip-permissions`** (bypasses the routing) ✗ |
| partial streaming | (absent) | `--include-partial-messages` (orthogonal, keep) |

**`--dangerously-skip-permissions` must be removed for Claude** and replaced with `--permission-prompt-tool stdio --permission-mode <mode>`. Consequence: AgentMux now **owns every permission decision** over the control channel — it must answer `can_use_tool` for *all* non-auto-allowed tools, not just AskUserQuestion (see §4.3, §5 Phase 1 "auto-allow" handler to preserve today's yolo UX).

---

## 3. Why the previous design can't work

`SPEC_ASK_USER_QUESTION` assumed: emit tool_use → turn **blocks** → driver sends a `tool_result` on stdin → turn resumes. But the CLI **resolves the tool_use itself** (auto-error) within the same turn — it never blocks on a tool_result. The answer is delivered as a **`control_response` to a `can_use_tool` control_request**, carrying `updatedInput.answers` — a *different channel and shape*. So the `agent.answer` → `send_tool_result` backend (#1441) targets a channel the CLI is not listening on. **Reusable** from #1441: the frontend `AgentQuestionPanel`, the queue/`pendingQuestions()` UI. **Must replace:** the detection source (control_request, not a stream tool_call) and the delivery (control_response, not tool_result), and the answer encoding (`answers` map, not `"Header: label"` text).

---

## 4. Architecture

### 4.1 Control-channel demultiplexer (persistent controller)

The persistent CLI's stdout NDJSON now carries two interleaved classes:
- **conversation** — `system|stream_event|assistant|user|result` (today's path → blockfile/frontend, unchanged),
- **control** — `control_request` (CLI asks us) and `control_response` (CLI answers our `initialize`).

Add a demux in the stdout reader (`persistent.rs`): lines with `type ∈ {control_request, control_response}` are routed to a new `ControlChannel`; everything else flows as today.

### 4.2 ControlChannel responsibilities

1. On spawn, **send `initialize`** (generate a short `request_id`, keep a pending map) and await its `control_response`.
2. For each inbound `control_request`:
   - `subtype == "can_use_tool"`, `tool_name == "AskUserQuestion"` → surface to the frontend (questions), **await the user's answer**, reply `control_response{behavior:"allow", updatedInput:{questions, answers}, toolUseID}`.
   - `subtype == "can_use_tool"`, any other tool → **auto-allow** (`behavior:"allow", updatedInput:input`) in Phase 1 to preserve today's bypass UX; in Phase 2 route to the decision prompt (#551).
   - other subtypes (`set_permission_mode`, hook callbacks, `control_cancel_request`) → Phase 1: log + safe default (allow / ack); Phase 2: handle.
3. Correlate by `request_id`; tolerate out-of-order; never block the conversation reader (answers arrive asynchronously from the UI).

### 4.3 Answer delivery (replaces `send_tool_result`)

New controller method `respond_control(request_id, response_json)` writes a `control_response` line to the live stdin (same writer task as `send_message`). The frontend `agent.answer` RPC now lands here instead of `send_tool_result`. Encode the answer as `updatedInput.answers` (question text → label / `[labels]` for multiSelect / free-text for "Other"); pass through the original `questions`; include `toolUseID`.

### 4.4 Frontend

- Detection moves from `stream-parser.ts` (a stream tool_call) to a **control event** the sidecar forwards (e.g. a new `agent:question` blockfile/event carrying `{tool_use_id, request_id, questions}`). The `AgentQuestionPanel` + `pendingQuestions()` queue are reused.
- `handleAnswer` builds the `answers` map and calls the (rewired) `agent.answer` RPC with `{blockid, request_id, tool_use_id, answers}`.
- The optimistic dismiss stays; final state lands when the turn resumes.

---

## 5. Phased implementation

### Phase 1 — AskUserQuestion via control protocol (Claude persistent) ← THIS CHANGE
1. `providers.rs` (+ frontend `providers/index.ts`): Claude `persistent_launch_args` → replace `--dangerously-skip-permissions` with `--permission-prompt-tool stdio --permission-mode default`. Keep `--include-partial-messages`. (Controller already `Persistent` from #1451.)
2. `persistent.rs`: control demux in the stdout reader; `ControlChannel` (initialize handshake, pending-request map); `respond_control()` on the stdin writer.
3. `persistent.rs` control handler: auto-allow non-AskUserQuestion `can_use_tool`; for AskUserQuestion emit an `agent:question` event + await answer.
4. `websocket.rs`: rewire `COMMAND_AGENT_ANSWER` → `respond_control()` (control_response), drop the `send_tool_result` path. `rpc_types.rs`: answer payload becomes `{blockid, request_id, tool_use_id, answers}`.
5. Frontend: consume the `agent:question` event; reuse `AgentQuestionPanel`; build `answers`; call rewired `agent.answer`.
6. Remove/▢ the now-dead `send_tool_result` + stream-parser AskUserQuestion special-case (or keep parser detection only as a fallback "blocked" indicator).

**Phase 1 exit criteria (must verify in a real pane):** create a Claude agent, trigger AskUserQuestion, panel renders, answer it, the model continues with the chosen answer; ordinary tools (Bash/Edit) still run without per-call prompts (auto-allow).

### Phase 2 — tool-permission decision prompt (#551) [follow-up]
Route non-auto-allowed `can_use_tool` to `AgentDecisionPanel` instead of auto-allow; persist allow-rules; honor `--permission-mode` from settings. The control channel built in Phase 1 is the missing transport `SPEC_DECISION_PROMPT §1` flagged.

### Phase 3 — muxbus Tier-1 controller-aware injection (§6) [follow-up]
Make `ReactiveHandler` delivery controller-aware so messages reach persistent stream-json agents.

### One-shot/container agents
No live control channel → AskUserQuestion stays unanswerable there; use the SDK's **`defer`** semantics (process exits, resumes from persisted session) or fall back to "ask as a normal follow-up message." Out of scope for Phase 1 (persistent/host agents only).

---

## 6. muxbus coupling (why this spec touches delivery)

muxbus Tier-1 (`ReactiveHandler::inject_message`) delivers by **PTY keystrokes** (`message\r` + 3 delayed `\r`) and is explicitly "terminal" injection; it does **not** branch on controller type. A persistent stream-json Claude has **no PTY** — its inbox is `persistent.rs::send_message` (a `{type:"user",…}` NDJSON line). So once Claude is persistent (Phase 1), **PTY injection silently fails to reach it.** Fix: a controller-aware "deliver to running agent" primitive — `send_message` (stream-json) for persistent agents, PTY write for shell/term agents — that muxbus Tier-1 calls. This is the *only* genuine overlap with the Agent SDK: the SDK is the local-injection arm for stream-json agents, **not** a replacement for muxbus's routing/addressing/relay (which the SDK has no concept of). Tracked as Phase 3.

---

## 7. Risks / open questions

- **Permission chatter:** `--permission-mode default` may route many tools through `can_use_tool`. Mitigation: auto-allow in the handler (Phase 1); consider `--permission-mode acceptEdits` or allow-rules to cut volume. **Verify** which mode still routes AskUserQuestion through the control channel (bypass/skip modes do **not** — that's the current bug).
- **initialize necessity:** the load-bearing flag is `--permission-prompt-tool stdio`; `initialize` may be optional for `can_use_tool`. Send it to match the SDK; confirm during impl whether it's required.
- **`--include-partial-messages` interaction** with the control protocol (SDK omits it) — keep but verify no interference.
- **Hooks/`set_permission_mode`/`control_cancel_request`** subtypes — Phase 1 logs + safe-defaults; enumerate fully in Phase 2.
- **Frontend reload/dedup** of an answered question (the `scrubOrphanedInProgress` logic from #1445) must move to the control-event model.

---

## 8. Validation assets

Reproduction harness lives at `/tmp/askq-evidence/` (this machine): `driver.mjs` (SDK + canUseTool), `claude-proxy.js` (tee-proxy), captured `wire-in.ndjson` / `wire-out.ndjson` / `argv.log`. Re-run to re-confirm the wire shapes on CLI upgrades.
