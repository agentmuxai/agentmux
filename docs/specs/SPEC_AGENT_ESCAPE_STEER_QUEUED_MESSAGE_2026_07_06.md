# SPEC — Escape delivers a queued message immediately (mimic Claude CLI's interrupt-and-steer)

**Date:** 2026-07-06
**Author:** Agent2
**Status:** Draft
**Scope:** `frontend/app/view/agent/components/AgentFooter.tsx`, `frontend/app/view/agent/hooks/useAgentCommands.ts`, `frontend/app/view/agent/agent-view.tsx`; a Tier 2 follow-on may touch `agentmux-srv/src/backend/blockcontroller/persistent.rs`.
**Related (must-read first):**
`docs/specs/SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16.md` (the empirical proof this spec leans on — mid-turn stdin steering for persistent Claude Code is proven, with a reusable probe-script methodology this spec follows for its own open question),
`docs/analysis/ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md` (Issue 3 — the "Send now" button this spec's design replaces),
`docs/specs/SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md` (the immediately-preceding PR — this spec is a direct continuation of that thread, not a new investigation),
`docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md` (the existing control-protocol demultiplexer in `persistent.rs` this spec's Tier 2 would extend, if it turns out to be needed).

---

## 1. Summary

The user asked to mimic real Claude Code CLI's queued-message behavior: type a message while the agent is busy, it queues; it's delivered automatically at the agent's next natural breakpoint (already implemented — see §2); **but if it's still queued and the user presses Escape, the CLI delivers it right then, telling the agent "stop what you're doing and consider this now."**

The investigation below turned up something better than expected: **the "deliver right now" half of this is already fully supported by existing, already-proven backend plumbing.** `persistent.rs`'s `send_message()` — the exact call the frontend's normal send path already uses — writes directly to the CLI's live stdin **unconditionally**, whether the agent is busy or idle (§3.1). The only reason a queued message doesn't already arrive immediately today is that the **frontend chooses to hold it** in `heldQueue` and wait for a tool-boundary/idle signal before calling that same function (§2). So "Escape delivers the queued message now" is a **small, backend-risk-free frontend change**: skip the wait, call the existing delivery path immediately.

The other half — **does it also silence the agent's current output right away**, the way it visually appears to in the real interactive CLI — is genuinely unverified in this codebase. The one empirical probe that exists (`SPEC_INJECT_AT_TOOL_BOUNDARY`'s `steer-probe.py`) proved steering happens at the **next inference boundary** (right after an in-flight tool's result lands), not instantaneously mid-generation. Whether AgentMux's stream-json protocol usage exposes anything stronger than that is an open question this spec proposes to answer with a new probe, mirroring the existing one, **before** committing to any backend work for it.

## 2. Where we are today

**The "queues, then auto-delivers at the next breakpoint" half already exists and already works correctly** — this was not part of the investigation, it's confirmed-working prior art:
- `sendMessage()` (`frontend/app/view/agent/hooks/useAgentCommands.ts`, ~264-388): while a turn is in flight, the message is dispatched as `PendingMessageQueued{enqueuedWhileBusy:true}` and pushed onto `heldQueue` (~184) instead of being delivered yet.
- An auto-flush `createEffect` in `agent-view.tsx` (~624-640) watches `currentToolAtom` and `turnPhaseAtom.kind`, calling `commands.flushHeldMessages()` as soon as a new tool call starts **or** the turn becomes idle/done.
- `flushHeldMessages()` (~462-480) drains `heldQueue` FIFO via `deliverToBackend()` (~397-454), which calls the `agentinput` RPC.

**The `agentinput` RPC, for a persistent controller, already writes to live stdin regardless of busy state — no idle-gate exists.** Traced precisely:
- `agentmux-srv/src/server/agent_handlers/input.rs` (~236-269): the `agentinput` handler, when the block's controller downcasts to `PersistentSubprocessController`, unconditionally calls `persistent_ctrl.send_message(cmd.message, config)`.
- `persistent.rs:292-306` (`send_message`): spawns the process only `if !self.is_running()`; otherwise it **just writes the message to the live `stdin_tx` immediately** — there is no check anywhere for "is a turn currently in progress." This is the **exact mechanism** `SPEC_INJECT_AT_TOOL_BOUNDARY`'s probe proved causes Claude to steer.

**So today, the only thing preventing a queued message from landing mid-turn is that the frontend hasn't called `deliverToBackend` yet** — it's sitting in `heldQueue`, waiting for the auto-flush effect's watched conditions (new tool start / turn idle). The backend has been ready for this the whole time.

**Escape today** (`AgentFooter.tsx:682-704`): on an empty composer, calls `props.onStopAgent` → `commands.stopAgent()` (`useAgentCommands.ts:502-531`) → `RequestStop` + `ControllerInputCommand({signame:"SIGINT"})`. It never inspects `heldQueue` (already exported and available via `commands.hasHeldMessages()`, `useAgentCommands.ts:494`, but unused by the Escape path).

**Why today's Escape is destructive for exactly the case this spec targets:** for a persistent controller, `send_input` with `SIGINT`/`SIGTERM` (`persistent.rs:988-1010`) calls `stop_process(true)`, which kills the subprocess outright (`persistent.rs:942-954`, via `kill_tx`) — only `session_id` survives, for the *next spawned process* to `--resume`. If a message were sitting in `heldQueue` when Escape fires today, this is what happens: kill the whole CLI process → wait for `Done` → the auto-flush effect fires → `flushHeldMessages` delivers the queued text as a **brand-new turn on a freshly `--resume`'d process** — not a steer of the still-live one. Slower and more destructive than necessary, and unrelated to the interrupt itself (no "here's why I stopped you" framing at all).

## 3. Proposed design — two tiers

### Tier 1 (this PR): deliver the queued message immediately, no interrupt signal at all

When Escape is pressed on an empty composer:
- **If `commands.hasHeldMessages()` is true:** call `commands.flushHeldMessages()` (or a small wrapper) immediately — **do not** call `stopAgent()`/send any signal. This delivers the queued text via the exact same `send_message()` write-to-live-stdin path the auto-flush effect already uses, just without waiting for the tool-boundary/idle trigger. For persistent Claude, this steers the live process per the proven `SPEC_INJECT_AT_TOOL_BOUNDARY` mechanism — consumed at the CLI's next inference step (after the in-flight tool's result, if one is running).
- **If `hasHeldMessages()` is false:** unchanged — `stopAgent()` (today's SIGINT/kill behavior). Nothing to steer, so "stop the agent" is the only reasonable meaning of Escape on an empty box with nothing queued.

This single branch is the entire Tier 1 change. No reducer changes, no new RPC, no backend touch — `flushHeldMessages`/`hasHeldMessages` are already exported from `useAgentCommands`'s return object (`useAgentCommands.ts:111,537`); the only wiring needed is at the `agent-view.tsx:1113` call site that currently passes `onStopAgent={commands.stopAgent}` directly to `AgentFooter`.

This also fully replaces "Send now" (Issue 3 of the motivating analysis) with something strictly better: instead of a separate button that killed the agent as an indirect way to hurry the queue along, the **same Escape key** the user already presses to interrupt now does the right thing contextually — steers if something's queued, interrupts if not. The "Send now" button, its `showSendNow`/`onSendImmediately` wiring, and `isInterruptibleTurn` can be deleted exactly as already scoped in the prior analysis, with no replacement affordance needed — Escape *is* the affordance now, matching the real CLI.

**One-shot providers** (codex/gemini/qwen/kimi — no live stdin, per `SPEC_INJECT_AT_TOOL_BOUNDARY` §5): `flushHeldMessages` already degrades correctly for these today (delivers as the next turn once the current one exits) — Tier 1 doesn't change their behavior, it only changes *when the frontend decides to call* the delivery path, and for one-shot providers that decision already can't take effect any sooner than the current turn's exit regardless. No special-casing needed.

### Tier 2 (needs empirical validation first): also silence the agent's current output right away

The open question: does the real interactive Claude CLI's Escape *only* deliver the next message sooner (same next-inference-boundary timing `SPEC_INJECT_AT_TOOL_BOUNDARY` already proved), or does it *also* immediately truncate whatever the model is currently streaming/whatever tool is in flight — a genuine interrupt, not just an early steer? The existing probe only tested injection during an in-flight **tool call** (four `sleep 4`s); it did not test injection during **plain streaming text** with no tool running, which is the scenario closest to what a literal "stops mid-sentence" experience would require.

This is unverified, not assumed. Before any backend work:
1. **Extend `docs/specs/evidence/steer-probe.py`** (or write a sibling probe) with a case that forces Claude to stream a long plain-text answer (no tool call), inject a second stdin message partway through, and capture whether the CLI (a) keeps streaming the original response to its natural end before addressing the new message, or (b) stops early. This mirrors exactly how `SPEC_INJECT_AT_TOOL_BOUNDARY` §4 resolved its own open questions — captured bytes, not speculation.
2. **If the CLI's stream-json control protocol exposes an actual interrupt/cancel control-request** (distinct from `can_use_tool`, the only control-request type this codebase currently demultiplexes, `persistent.rs:458-489`), capture its exact wire shape from real traffic before writing any Rust to send it — do not guess the JSON shape.
3. **If no such mechanism exists**, the honest answer is that "immediate visual silence mid-stream" isn't achievable for stream-json-driven Claude without killing the process (which is what today's Escape already does, just without the steering benefit) — in which case Tier 1's "steer at the next natural boundary, don't kill" is the correct final behavior, and the user-facing difference from real interactive Claude CLI (which may rely on true terminal-level Ctrl-C handling not exposed over stream-json) would be a documented, accepted gap rather than something to force via a destructive workaround.

**Recommendation: ship Tier 1 alone first.** It's a 5-10 line frontend change reusing fully-proven backend plumbing, it already fixes the reported UX gap for the common case (queued message delivered at the next real breakpoint instead of waiting for full idle or requiring a destructive kill), and it cleanly retires "Send now." Tier 2 is a genuine unknown that deserves its own empirical pass and should not block Tier 1.

## 4. What NOT to do

- **Do not** make Escape-with-queued-message also send `SIGINT`/`stopAgent()` alongside the delivery. Killing the process the delivery just wrote into defeats the entire point (destroys the live session `send_message()` targeted) and would silently regress Tier 1 back to today's kill-then-`--resume` behavior.
- **Do not** invent a wire-level "interrupt" control-request shape for Tier 2 without capturing real bytes first (same discipline `SPEC_INJECT_AT_TOOL_BOUNDARY` used) — guessing a JSON shape the CLI doesn't actually understand is worse than not shipping Tier 2 at all.
- **Do not** thread `pendingContent`/`Interrupting.reason` changes through the `TurnPhase` reducer for this — Tier 1 needs no new reducer state; the queued text already lives in `heldQueue`/`PendingMessage`, which is exactly what needs to be delivered, unchanged.

## 5. Tests

- `useAgentCommands.test.ts` (or a new focused test): Escape-equivalent call with `heldQueue` non-empty calls `flushHeldMessages` and does NOT call `stopAgent`/dispatch `RequestStop`.
- Escape-equivalent call with `heldQueue` empty: unchanged existing behavior (`stopAgent()` fires).
- Regression: `flushHeldMessages` still drains FIFO in order when called via this new immediate path, same as via the existing auto-flush effect.
- Tier 2 (only once/if pursued): a probe-script-backed integration test asserting the observed truncation behavior, gated on whatever the probe actually finds — not written speculatively ahead of the probe.

## 6. Why this is worth doing (and why it's a natural continuation, not a new thread)

This is the direct completion of the "Send now" removal that the unified-failure-reducer PR deliberately left out of scope (`ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md` Issue 3) — same investigation thread, same composer/queue subsystem, same session. It also closes a gap the *other* recent spec (`SPEC_INJECT_AT_TOOL_BOUNDARY`) left explicitly unaddressed: that spec proved mid-turn steering works and proposed wiring it into the passive auto-flush path, but never considered a user-initiated "delivered right now" gesture riding the same already-proven mechanism. Tier 1 here is small precisely because it's standing on already-verified work from both threads rather than starting fresh.
