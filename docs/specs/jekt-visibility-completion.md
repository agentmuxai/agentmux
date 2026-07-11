# Spec: Jekt Visibility Completion — persistent-agent visibility + outgoing echo

**Date:** 2026-07-10
**Author:** Agent2
**Status:** Implemented (this PR)
**Parent spec:** `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` (completes §3.1 for
persistent agents, implements §3.2)
**Builds on:** PR #2031 (Phase 3 — `JektBubble` renders `[JEKT:...]` markers)

---

## 1. Findings — the two gaps left after PR #2031

PR #2031 shipped the frontend: `tryParseJekt` (stream-parser.ts) turns a `[JEKT:...]`
marker inside a user message into a `JektMessageNode`, rendered by `JektBubble` with
direction support ("incoming"/"outgoing") already built in. But two producer-side gaps
mean the bubble rarely fires where it matters most:

### 1.1 Incoming jekts never reach the pane on persistent agents

For **persistent stream-json agents** (all Claude Code panes), the injection path is
`Handler::inject_message` → `deliver_agent_message` →
`PersistentSubprocessController::send_user_message`, which wrote the wrapped jekt
**only to the process stdin**. Compare `send_message` (the user-typed path), which also
persists the line to the blockfile so the frontend can build a node. `send_user_message`
did neither — so the agent received and acted on the jekt while the human operator's
conversation view showed **nothing**. A silent injection, which parent-spec G1 forbids.
`tryParseJekt` never ran because the marker never landed in the `output` stream.

(PTY-based agents were unaffected — keystroke echo makes the raw marker visible in
xterm. The gap is specific to structured-channel delivery.)

### 1.2 No outgoing echo producer (§3.2)

`stream-parser.ts` (`tryParseJekt`) explicitly notes direction detection is "not yet
emitted by any producer today". Nothing wrote an outgoing record to the sender's pane,
and `setAgentId` — which direction detection depends on — was never called in
production code, only tests.

## 2. Changes

### 2.1 Incoming visibility (backend)

`PersistentSubprocessController::send_user_message` now persists the injected
`{"type":"user",...}` line to the blockfile **with a live WPS append event**
(`handle_append_block_file`), mirroring to the global transcript zone. Non-silent —
unlike `send_message` there is no `agent-message-accepted` pending-echo to pair with,
so there is no duplicate-node risk. The open pane renders the jekt live (Phase 3 bubble
now fires); `parseHistoryLines` rebuilds it on reopen.

### 2.2 Outgoing echo (backend, §3.2)

New `echo_jekt_to_sender` (`server/reactive.rs`): on successful injection, append to the
**sender's** `output` blockfile a `{"type":"user",...}` line carrying the same
`[JEKT:...]` marker the receiver got (re-wrapped via `wrap_jekt_message` with identical
FROM/TO/TIER/DELIVERY/MSGID/PRIORITY fields). The frontend's existing `tryParseJekt`
sees FROM == this pane's agent → renders an **outgoing** `JektBubble`. No new wire
format; the dormant direction support in #2031 becomes live.

Wired into every send path:
- `handle_reactive_inject` — local success, cross-instance forward success (tier 2),
  LAN peer forward success (tier 3). The first hop is always the sender's own instance,
  which is where the sender's pane lives.
- WS `bus:inject` — direct success and messagebus-fallback success.

Skipped when the sender isn't a registered agent on this instance (cron, external
callers) or is messaging itself. `InjectionResponse` gains an `effective_tier` field so
the echo reuses the handler's escalation result instead of re-deriving it.

### 2.3 Direction activation (frontend)

`parser.setAgentId` is now actually called in production:
- `useAgentStream` accepts `agentName` and sets it on every parser (re)construction.
- `parseHistoryLines` accepts an optional `agentName` param; `useHistoryPagination`
  threads it via a new `agentName` accessor option.
- `agent-view.tsx` passes `block.meta.agentName` to both hooks.

Without a name, direction falls back to "incoming" — the only pre-echo behavior, so
older callers are unaffected.

## 3. Files Changed

| File | Change |
|------|--------|
| `agentmux-srv/src/backend/blockcontroller/persistent.rs` | `send_user_message`: persist + live event after stdin send |
| `agentmux-srv/src/backend/reactive/types.rs` | `InjectionResponse.effective_tier` |
| `agentmux-srv/src/backend/reactive/handler.rs` | populate `effective_tier` |
| `agentmux-srv/src/backend/reactive/tests.rs` | fixture update |
| `agentmux-srv/src/server/reactive.rs` | `echo_jekt_to_sender` + wire into 3 success paths |
| `agentmux-srv/src/server/websocket.rs` | echo in `bus:inject` (direct + messagebus fallback) |
| `frontend/app/view/agent/useAgentStream.ts` | `agentName` opt → `setAgentId` |
| `frontend/app/view/agent/parseHistoryLines.ts` | `agentName` param → `setAgentId` |
| `frontend/app/view/agent/hooks/useHistoryPagination.ts` | thread `agentName` accessor |
| `frontend/app/view/agent/agent-view.tsx` | pass `block.meta.agentName` to both hooks |

## 4. Testing

1. Persistent agent A → `SendMessage` → persistent agent B: B's pane shows an incoming
   JektBubble live (no reload); A's pane shows an outgoing bubble. Reopen both panes →
   bubbles rebuilt from history with correct directions.
2. Sensitive-keyword message → incoming bubble renders with sensitive styling (existing
   #2031 behavior, now reachable on persistent agents).
3. Cron-fired inject (unregistered sender) → target sees bubble; no sender echo, no
   error.
4. Agent messaging itself → single incoming marker, no duplicate echo.
5. PTY-agent target → keystroke delivery unchanged; sender still gets the outgoing echo.
