# SPEC: Defer jekt / inter-agent message delivery until a safe turn boundary — never truncate an in-progress explanation

**Date:** 2026-09-10
**Status:** proposed — design only, not yet implemented.
**Author:** Loap #2
**Scope:** Reactive/jekt delivery (`agentmux-srv/src/backend/reactive/**`), controller-aware
delivery (`agentmux-srv/src/backend/blockcontroller/mod.rs::deliver_agent_message`,
`persistent.rs::send_user_message`), muxbus/cloud subscriber and messaging-bridge delivery paths
(`cloud_subscriber.rs`, Slack/Discord/Telegram/WhatsApp bridges), MCP `SendMessage` tool.
**Explicitly out of scope:** the direct human-operator UI path (`agentinput` RPC, dispatched via
`COMMAND_AGENT_INPUT` in `agentmux-srv/src/backend/rpc_types/commands.rs:109` to the handler in
`agentmux-srv/src/server/agent_handlers/input.rs:849-864`) — a human choosing to interrupt their own agent's in-progress turn by
typing into its pane is intentional steering, not the problem this spec addresses.
**Related:**
- `docs/specs/SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16.md` — implemented the controller-aware
  mid-turn delivery this spec proposes narrowing. Its own evidence (§4) is the direct cause of
  the bug this spec fixes; see §2 below.
- `docs/specs/jekt-inject-timing.md` — PTY keystroke mechanics (legacy `ject` path, unaffected —
  see §5).
- `docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md` — the mid-turn `tool_result` precedent this
  spec does **not** touch (it answers a question the model itself asked; it is not an
  externally-injected message racing the model's own output).
- `agentmux/CLAUDE.md`'s "Jekt (agent-to-agent message) security rules" section — the marker
  format and delivery this spec's default policy applies to.

---

## 1. The ask

Repo owner, verbatim: *"we want to make sure jekt messages (like from github) never cut a train
of thought midway. In fact, any tool call, or anything should never break a train of
thought/explanation section."*

Concretely: today, an agent working through a multi-step task — running a tool, then explaining
its result, then running the next tool, and so on — can have that explanation cut off mid-way by
an unrelated incoming jekt (a GitHub/ReAgent review notification, a CI result, another agent's
status ping) that has nothing to do with the task at hand. The fix must not be jekt-specific in
the narrow "JEKT marker" sense — it must cover every automated, non-human-initiated path that can
write into a running agent's live input.

---

## 2. Why this happens today (root cause, not a new bug — a known trade-off revisited)

`SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16.md` shipped controller-aware delivery
(`deliver_agent_message`, `persistent.rs::send_user_message`) specifically so that a message sent
to a **busy** persistent-controller agent (Claude Code today) would not have to wait for the
agent to go fully idle — it gets written to the agent's live stdin immediately, and the CLI
"steers": it consumes the message at its **next inference step**, which its own evidence (§4 of
that spec) shows is **right after the in-flight tool's `tool_result`**, abandoning whatever the
model had planned to do next — including any explanatory text it was about to produce about that
very tool result.

This was framed as the desired outcome ("steering"), and for a message the human operator
explicitly wants to interrupt with, it is. But `deliver_agent_message` is the same code path used
for **every** reactive/jekt delivery — GitHub/ReAgent review notifications via muxbus, CI-result
pings, another agent's routine coordination message, the Slack/Discord/Telegram/WhatsApp bridges
— none of which are the human asking to interrupt anything. `InjectionRequest.wait_for_idle`
(`reactive/types.rs:47`) — the field that should gate this — is **dead scaffolding, never read**
(confirmed at every one of its ~15 call sites: `websocket.rs`, `messagebus.rs`,
`cloud_subscriber.rs`, the four bridge modules, `handler.rs`'s nudge path). There is no code
distinguishing "this sender wants to steer the agent" from "this sender just has an FYI for it."

`persistent.rs::send_user_message`'s own doc comment is explicit that this is unconditional:
*"Whether the process was busy or idle, delivering this message (re)starts an active turn"*
(`persistent.rs:2147-2153`). Nothing about turn phase — whether the model is mid-tool-call or
mid-text — changes that.

**What is proven vs. assumed:** the steer-probe evidence (§4 of the 06-16 spec) proves the CLI
abandons an in-flight *tool* plan when injected mid-turn. It does **not** test injection while the
model is actively producing a text content block with no tool call in flight — that is, the exact
"cutting off the explanation itself, not just the plan to write one" case. This spec's design does
not require resolving that open question first (see §4 — the proposed fix defers delivery
regardless of which sub-case is true), but it is worth recording as a real gap in the existing
evidence, not something this spec is assuming away.

---

## 3. Scope of "any tool call, or anything"

Reading the ask literally, every path that can write into a running agent's live input on behalf
of something other than the human operator's own deliberate action is in scope:

| Path | Code | Currently steers mid-turn? |
|---|---|---|
| Reactive HTTP inject (jekt, `mux`/`ject`) | `reactive/handler.rs::inject_message` → `deliver_agent_message` | Yes, for persistent controllers (this spec's primary target) |
| MCP `SendMessage` tool | same `deliver_agent_message` path | Yes, same as above |
| muxbus/cloud subscriber (GitHub/ReAgent notifications, CI results) | `cloud_subscriber.rs` | Yes, same path — **this is the literal "jekt messages (like from github)" case named in the ask** |
| Messaging bridges (Slack/Discord/Telegram/WhatsApp) | four bridge modules | Yes, same path |
| ACP `session/prompt` mid-prompt | `acp.rs::send_input` | Unproven/agent-dependent (§7.4 of the 06-16 spec) — treat as "assume yes, must be gated" until proven otherwise |
| One-shot subprocess providers (codex/gemini/qwen/kimi/default muxcode) | `subprocess.rs` | **No** — structurally cannot (no live stdin between turns); already behaves like "wait for idle" today. No change needed. |
| PTY keystrokes (shell/TUI blocks, legacy `ject`) | `send_input` | N/A — these are real terminal keystrokes into a shell, not an LLM turn; "explanation section" doesn't apply. No change needed (`jekt-inject-timing.md` unaffected). |
| Direct human UI (`agentinput`) | `agent_handlers/input.rs:849-864` | Yes — **intentionally out of scope**, see header. |

So the actual code change is narrowly scoped to **persistent-controller delivery via
`deliver_agent_message`/`send_user_message`**, plus the same gate applied to ACP's
`send_input`/`session/prompt` once it's confirmed to steer. Every other path either already can't
interrupt mid-turn, or is the human's own deliberate action.

---

## 4. Proposed design

### 4.1 Default policy: defer non-human-initiated delivery to the next turn boundary

Replace the dead `wait_for_idle: bool` with the `DeliverPolicy` enum the 06-16 spec sketched but
never wired (§7.3 of that spec), and — this is the actual behavior change — **flip the default**:

```rust
enum DeliverPolicy {
    /// Write to live stdin now; may steer mid-turn (today's unconditional behavior).
    Immediate,
    /// Queue; flush the instant the current turn reaches a completion boundary
    /// (a `result`/turn-done event), never mid-turn.
    NextIdle,
}
```

- **`NextIdle` becomes the default** for every path in §3's "Yes" rows: reactive/jekt HTTP
  inject, MCP `SendMessage`, muxbus/cloud subscriber, and all four messaging bridges.
- **`Immediate` is reserved** for the direct human UI path (`agentinput`), which does not go
  through `deliver_agent_message` at all today and needs no change.
- No sender-settable "urgent" override is proposed here. The ask was "never" — adding an
  escape hatch that any automated sender could flip is exactly the kind of exception that
  erodes the guarantee this spec exists to provide. If a genuine need for automated mid-turn
  steering surfaces later, it should be its own follow-up spec with its own justification, not a
  flag threaded through on day one.

### 4.2 Mechanism: a per-block pending-delivery queue, flushed one message per turn

**Revised after ReAgent review (2026-09-11) — the first draft of this section had four real
soundness gaps, all fixed below: (1) draining the whole queue in one burst on turn-completion
re-triggers a new active turn for the 2nd+ message and writes to live stdin while THAT turn is
running, which is exactly the mid-turn write this spec exists to prevent; (2) the busy-check and
the turn-completion flush were two separately-timed checks, racing (a message enqueued right as
the queue is observed empty and "drained" would be stranded with no future trigger); (3) no
handling was specified for the process exiting/being replaced before a `result`/`Done` event ever
fires, silently losing already-accepted messages; (4) queue overflow was specified to silently
drop a message after already returning success to the caller.**

`persistent.rs` already tracks turn-active state (`health_monitor.mark_turn_active_returning_was_active`,
`persistent.rs:2153`) and already has a controller-level home for per-block mutable state.
Add, all guarded by **one mutex** covering both the turn-active flag and the pending queue
together — this single lock is what closes gap (2):

```rust
struct DeliveryState {
    turn_active: bool,
    pending: VecDeque<PendingMessage>, // bounded — see overflow handling below
}
```

1. **Enqueue-or-send is one atomic operation.** `deliver_agent_message` with
   `DeliverPolicy::NextIdle` takes the lock once: if `!turn_active`, it sets `turn_active = true`
   and calls `send_user_message` **inside the same critical section** (idle fast-path — matches Q4
   of the 06-16 spec). If `turn_active`, it pushes onto `pending` and returns success, still inside
   the same lock. Because both branches share one lock with the flush path below, there is no
   window where a message can be enqueued against a queue that a concurrent flush already observed
   as empty and finished draining.
2. **Flush delivers exactly ONE message per turn-completion event, not the whole queue.** On the
   `result`/`Done` event, take the lock: if `pending` is non-empty, `pop_front()` **one** entry and
   call `send_user_message` for it **inside the same critical section**, leaving `turn_active =
   true` (the send just started a new turn) and the rest of `pending` untouched. Only if `pending`
   was already empty does this event set `turn_active = false`. This closes gap (1): the next
   queued message is only ever sent in response to *that new turn's own* completion event, so no
   two `send_user_message` calls for the same block are ever separated by less than one full turn
   — the no-mid-turn guarantee holds for every queued message, not just the first.
3. **Abnormal termination (process exit/stop/replacement) before a `result`/`Done` fires.** Treat
   controller-level process-exit as an alternate path into the same locked transition as a normal
   completion — but rather than assuming a same-process next turn, branch on why the process ended:
   - If the block is **respawning for the same logical agent** (crash-restart, resume), the
     `pending` queue is carried over to the new process instance and flushes (§4.2.2) against
     *its* first completion event, same as before.
     - If the block is **being stopped/removed/replaced** (not coming back), every entry still in
     `pending` is a delivery that was accepted but will now never be sent. These must not be
     silently dropped: each `PendingMessage` carries the means to report failure back to its
     origin (the same channel `reactive/handler.rs` already uses to report a failed injection —
     see Appendix), and the drain-on-teardown path calls that failure report for every stranded
     entry rather than discarding them.
4. **Queue overflow does not silently drop an already-"succeeded" delivery.** `deliver_agent_message`
   only returns success once a message is either sent (idle fast-path) or actually pushed onto
   `pending`. If the bounded queue is full at enqueue time, it returns an explicit error (e.g.
   `DeliveryError::QueueFull`) instead of a false success — the caller (reactive handler, muxbus
   consumer, bridge) sees this exactly like any other transient delivery failure and can retry,
   the same as it already has to handle a network error to the target instance. Nothing is ever
   reported delivered and then quietly lost.
5. `InjectionRequest.wait_for_idle` is removed (it was unread dead code) rather than repurposed,
   since its name would collide confusingly with the new explicit enum.

This requires no new visibility into content-block-level turn phase (text vs. tool_use) — it
sidesteps the open question flagged in §2 entirely by never writing to stdin until the whole turn
is done, which is the strictly stronger and simpler guarantee the ask calls for.

#### 4.2.1 Why one lock, not two

The original draft implicitly used two separate points of truth (a turn-active flag read by the
enqueue path, a queue drained by the completion path) without specifying they shared a lock. Any
implementation of this spec MUST treat `turn_active` and `pending` as one piece of state behind one
mutex — splitting them reintroduces gap (2) even if each individually looks correct.

#### 4.2.2 Why one-at-a-time, not burst-drain

`send_user_message` starts a new active turn (§2, `persistent.rs:2147-2153`). Sending message #2
immediately after message #1 — before message #1's turn has actually completed — would write to
live stdin while a turn is active, which is precisely what this spec forbids. The queue must
therefore be drained strictly one entry per observed turn-completion event, never more.

### 4.3 ACP

Per the 06-16 spec's own open question (§7.4/§8 Q6), whether an ACP agent honors or queues a
second `session/prompt` mid-prompt is agent-side and unprobed. Until probed: apply the same
queue-and-flush-on-completion wrapper at the `deliver_agent_message` layer (§4.1/4.2) rather than
relying on the ACP agent's own queuing behavior, since that behavior is unverified and this spec's
guarantee should not depend on an unproven assumption about third-party agent behavior.

---

## 5. What does *not* change

- **Subprocess (one-shot) providers** (codex/gemini/qwen/kimi/default muxcode): already
  structurally equivalent to `NextIdle` (no live stdin exists until the process exits). No code
  change; noted here so the reader doesn't wonder why they're absent from the change list.
- **PTY keystroke delivery** (shell/TUI blocks): out of scope. A running shell command isn't an
  LLM turn with an "explanation section" — keystroke-into-terminal semantics are unrelated to the
  problem being solved. `jekt-inject-timing.md` stands as-is.
- **`SPEC_ASK_USER_QUESTION`'s `send_tool_result` precedent**: unaffected. That path answers a
  `tool_use` the model itself emitted and is waiting on — it is not an externally-sourced message
  racing the model's own in-progress output, so it isn't the case this spec is guarding against.
- **The direct human UI path**: unaffected, by design (see header and §4.1).

---

## 6. Trade-off being made explicitly

This spec trades away the "steer sooner" benefit `SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16`
delivered for jekt/inter-agent traffic specifically. A GitHub/ReAgent notification, a teammate's
coordination ping, or a muxbus message will now always wait for the current turn to finish before
surfacing — the same latency profile as before that spec shipped, for these senders only. The
06-16 spec's actual proven contribution (mid-turn steering works, and the reactive/MCP path can
reach a stream-json pane's live stdin at all — it previously silently missed it, §3.3/§3.4 of that
spec) is preserved; only the *default policy* for *non-human* senders changes from `Immediate` to
`NextIdle`. The human operator's own ability to interrupt their agent from its pane is completely
unaffected.

---

## 7. Open questions

| # | Question | Notes |
|---|---|---|
| 1 | Does an agent mid-text-block (no tool in flight) actually get interrupted by a live-stdin write today, or does the CLI already only consume at content-block boundaries? | Unresolved per §2 — this spec's design (§4.2) makes the answer irrelevant to correctness (nothing is written until turn-end regardless), but it's worth a follow-up steer-probe variant (text-only turn, no tool calls) for completeness/documentation. |
| 2 | Should `NextIdle` for a **long-running** turn (many tool calls, minutes) have any escape valve — e.g. a human-visible "N messages waiting" indicator so a stuck-feeling agent doesn't look unresponsive to jekts? | UI/product question, not a correctness question. Worth a follow-up if long turns become common; not a blocker for this spec's core guarantee. |
| 3 | ACP mid-prompt behavior once an ACP CLI is available to probe (pi/openclaw/copilot) | Tracked already in the 06-16 spec (§7.4, Q6); this spec's queue wrapper (§4.3) makes the probe result a non-blocker either way. |

---

## Appendix — key file references

| Concern | File:line |
|---|---|
| Dead `wait_for_idle` field (all ~15 call sites never read it) | `agentmux-srv/src/backend/reactive/types.rs:47` |
| Controller-aware delivery entry point | `agentmux-srv/src/backend/blockcontroller/mod.rs` (`AgentDelivery`, `deliver_agent_message`) |
| Persistent live-stdin write, unconditional turn-restart comment | `agentmux-srv/src/backend/blockcontroller/persistent.rs:2141-2153` |
| Turn-active tracking (reusable for idle-flush trigger) | `agentmux-srv/src/backend/blockcontroller/persistent.rs::mark_turn_active_returning_was_active` |
| Reactive HTTP inject entry point | `agentmux-srv/src/backend/reactive/handler.rs::inject_message` |
| Reactive HTTP + forwarding | `agentmux-srv/src/server/reactive.rs:18-143` |
| Steering evidence (proves mid-turn abandonment, tool-call case only) | `docs/specs/evidence/steer-probe.py`, `steer-probe-claude-run1.txt`, `steer-probe-claude-run2.txt` |
| ACP prompt delivery (`send_input` → `session/prompt`) | `agentmux-srv/src/backend/blockcontroller/acp.rs:695-730` |
| Human UI delivery path (unaffected) | `agentmux-srv/src/server/agent_handlers/input.rs:849-864` (dispatch), `agentmux-srv/src/backend/rpc_types/commands.rs:109` (`COMMAND_AGENT_INPUT`) |
