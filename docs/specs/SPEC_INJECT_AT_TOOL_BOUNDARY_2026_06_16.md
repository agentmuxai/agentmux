# SPEC: Deliver a queued message mid-turn (at the next tool-call boundary) instead of waiting for idle

**Date:** 2026-06-16
**Status:** **Implemented** (Phase 3 of `SPEC_AGENT_CONTROL_PROTOCOL` — controller-aware
delivery) on branch `agento/mid-turn-message-delivery`. Feasibility resolved empirically;
cross-provider design below.

> **Update (post-pull):** main now ships Claude as a **persistent** controller (the Agent
> Control Protocol Phase 1 landed: `--permission-prompt-tool stdio --permission-mode default`,
> `providers/index.ts:170`, `providers.rs:115`). So Claude agents already have a live stdin —
> the previous draft's "claude defaults to subprocess" no longer holds. The only missing
> piece was **controller-aware muxbus/reactive delivery** (Phase 3 of
> `SPEC_AGENT_CONTROL_PROTOCOL §6`), which this branch implements (see §10).
**Author:** AgentO
**Scope:** Agent runtime (`agentmux-srv/src/agents/**`, `agentmux-srv/src/backend/blockcontroller/**`),
reactive messaging (`agentmux-srv/src/backend/reactive/**`), MCP SendMessage tool
(`agentmux-mcp/**`), provider registry (`frontend/app/view/agent/providers/**`),
agent-pane turn state (`frontend/app/store/agent-pane-state/**`)
**Evidence:** `docs/specs/evidence/steer-probe.py`,
`docs/specs/evidence/steer-probe-claude-run1.txt`,
`docs/specs/evidence/steer-probe-claude-run2.txt`
**Related:**
- `SPEC_ASK_USER_QUESTION_2026_06_15.md` (mid-turn `tool_result` injection on live persistent
  stdin — the existing precedent that already does mid-turn delivery)
- `specs/jekt-inject-timing.md` (PTY keystroke injection — the legacy reactive path)
- commit `7b8fe2ae` feat(mcp): add SendMessage tool for agent-to-agent messaging (#1458)

---

## 1. The ask

When you message a **busy** agent, today it isn't acted on until the agent goes **idle**.
We want to deliver it sooner — at the next tool-call boundary — so the agent gets steered
mid-task. And this must work **across all providers**, not just Claude Code.

This revision **resolves the open questions** from the previous draft with live experiments,
and corrects the architecture picture, which changes the conclusion materially.

---

## 2. TL;DR — what we learned

The previous draft assumed agent panes run a **persistent** stream-json process and the only
open question was whether the CLI steers mid-turn. Both assumptions needed correcting:

1. **The CLI *does* steer mid-turn — confirmed (Claude Code).** Injecting a user message on a
   live stream-json stdin mid-turn causes the agent to abandon its in-flight plan and follow
   the new message at the **very next inference step** (after the currently-running tool's
   result). Reproduced twice, deterministically (§4).

2. **But AgentMux agents do not run persistently by default.** Per the provider registry,
   `claude`, `codex`, `gemini`, `qwen`, `kimi`, `muxcode` all use
   **`controllerType: "subprocess"`** — a **fresh one-shot process per turn** (`-p` / `exec`,
   reads its prompt, runs to completion, **exits**)
   (`frontend/app/view/agent/providers/index.ts:160,199,240,268,306,391`). A one-shot
   subprocess has **no mid-turn input channel at all** — there is nothing to inject into until
   it exits. **This is the real reason messages "wait for idle."**

3. **Therefore the headline blocker is execution mode, not detection.** Tool-boundary
   detection already exists in the translators, and the CLI already steers. The missing piece
   is: the agent must be running in a **persistent / streaming-input mode** for a mid-turn
   message to have anywhere to go.

4. **The reactive / MCP `SendMessage` path is the wrong pipe for this.** It delivers via
   `send_input()` (PTY keystrokes), which the persistent controller **rejects outright**
   (`persistent.rs:625`: *"persistent controller does not accept raw input; use
   send_message()"*). Mid-turn delivery must go through `send_message()` (NDJSON user message
   on live stdin), exactly like the AskUserQuestion `send_tool_result` precedent
   (`persistent.rs:170-192`).

**Net:** the feature is achievable and the hard part (CLI steering) is proven — **but only for
providers that can run in a persistent streaming-input mode.** That is **Claude Code today**,
**muxcode** (Claude-compatible, pending confirmation), and **possibly the ACP providers**
(protocol-dependent). The one-shot providers (codex/gemini/qwen/kimi) **structurally cannot**
be steered mid-turn with their current CLIs (§5).

---

## 3. Architecture reality (corrected)

### 3.1 Three execution modes — feasibility differs per mode

Controller is selected from block meta `controller` (`mod.rs:301,349-407`); the per-provider
default lives in the registry (`providers/index.ts`).

| Mode | Controller | How a turn runs | Mid-turn input channel? |
|---|---|---|---|
| **One-shot subprocess** (DEFAULT for claude, codex, gemini, qwen, kimi, muxcode) | `SubprocessController` (`subprocess.rs`) | Fresh process per turn (`-p`/`exec` + `--resume`), reads prompt, runs all tools, emits `result`, **exits** | **None.** No live stdin between turns. |
| **Persistent** (code-complete; used on some host-agent paths incl. AskUserQuestion; opt-in via `persistentLaunchArgs`) | `PersistentSubprocessController` (`persistent.rs`) | Long-lived `--input-format stream-json` process; user messages written as NDJSON lines on a live stdin (`persistent.rs:148-168,340-358`) | **Yes — live stdin.** Raw keystrokes rejected (`persistent.rs:625`). |
| **ACP** (openclaw, pi, copilot) | `AcpController` (`acp.rs`) | Long-lived JSON-RPC session; `session/prompt` per message (`acp.rs:569-611`) | **Session is live**, but a 2nd `session/prompt` mid-prompt is **agent-dependent** (likely serialized). |

### 3.2 Why messages "wait for idle" today

For the default one-shot subprocess agents, a message sent while busy can only become the
**next turn's prompt**, which cannot start until the current subprocess **exits**. So "deliver
while busy" is impossible by construction — it always lands after the turn ends. There is **no
idle-gate in code** (the `InjectionRequest.wait_for_idle` field at `reactive/types.rs:21` is
dead scaffolding, never read); the wait is **structural**, imposed by one-shot execution.

### 3.3 Delivery paths today (and which actually reach a stream-json pane)

- **UI → agent pane:** `useAgentCommands.sendMessage()` → `agentinput` RPC
  (`websocket.rs:884-1048`) → persistent `send_message()` *or* subprocess `spawn_turn()`. For
  one-shot, `spawn_turn` starts the next turn (so a busy agent's message queues client-side
  until the current turn finishes).
- **MCP `SendMessage` / inter-agent reactive:** `agentmux-mcp` → `POST /agentmux/reactive/inject`
  → `Handler::inject_message` → `send_input()` (PTY keystrokes)
  (`reactive/handler.rs:179-335`). **This path is rejected by persistent controllers and is
  not meaningful for one-shot subprocess agents** — it only does something for PTY/TUI shell
  blocks. So the new `SendMessage` tool, as wired, does not cleanly steer a stream-json pane.
- **AskUserQuestion (the working mid-turn precedent):** `agent.answer` →
  `persistent_ctrl.send_tool_result()` writes a `tool_result` to live stdin
  (`websocket.rs:779-809`, `persistent.rs:170-192`) — explicitly **requires a persistent
  controller** ("requires a persistent (host) agent; container/one-shot agents are [not
  supported]", `websocket.rs:809`).

---

## 4. Empirical resolution of the open questions (Claude Code)

Harness `docs/specs/evidence/steer-probe.py`: start `claude --input-format stream-json
--output-format stream-json --verbose --include-partial-messages
--dangerously-skip-permissions`; send a task that forces **four sequential `sleep 4` Bash tool
calls** then prints `FINISHED-ORIGINAL`; the **instant** the first tool call is observed,
inject a second user message on stdin: *"abandon the sleeps, output STEERED-MIDTURN, stop."*

**Both runs, identical and deterministic** (`steer-probe-claude-run1.txt`,
`steer-probe-claude-run2.txt`):

```
 0.0s  MSG1 sent (4x sleep task)
 ~3-6s TOOL_USE #1 (Bash) starts  ->  INJECT interrupt on stdin immediately
~8-11s tool_result done-1 returns (the in-flight sleep finishes)
~10-12s assistant text: STEERED-MIDTURN
~10-12s RESULT subtype=success, result=STEERED-MIDTURN
        => only 1 of 4 sleeps ran; FINISHED-ORIGINAL never emitted
```

**Resolved open questions:**

- **Q1 (gating): does Claude steer on a mid-turn stdin message?** → **YES.** It dropped the
  remaining 3 sleeps and followed the interrupt. Definitive.
- **Q2 (seam precision): where is the message consumed?** → At the **next inference boundary**,
  i.e. **right after the in-flight tool's `tool_result`**. The message *cannot* affect the tool
  already running (consistent with: by the time we see `content_block_start: tool_use`, the
  model already chose it). Practically, **the implementation does not need to precisely time
  the write to the boundary** — Claude buffers the stdin message and consumes it at its next
  step. AgentMux just needs to write it to live stdin during the turn.
- **Q4 (idle fast-path):** trivial — if already idle, `send_message()` starts a normal turn.

**Caveat:** this proves CLI behavior in *persistent* mode. It does **not** change the fact that
AgentMux's default one-shot subprocess agents have no live stdin to write to (§3.2).

---

## 5. Cross-provider matrix ("extend to all providers")

Steerability is determined by **whether the provider can run a persistent streaming-input
session**, then by whether that session honors a mid-turn message.

| Provider | Default controller | Persistent streaming-input mode? | Mid-turn steering feasible? | Notes |
|---|---|---|---|---|
| **claude** | subprocess | **Yes** (`persistentLaunchArgs --input-format stream-json`, `index.ts:161`) | **YES — proven (§4)** | Needs AgentMux to run it persistent + write mid-turn via `send_message()`. |
| **muxcode** | subprocess | Likely (emits `claude-stream-json`; uses `ClaudeTranslator`) | **Likely** — verify it accepts `--input-format stream-json` | First-party CLI; confirm persistent input support. |
| **codex** | subprocess | **No** — `exec` reads stdin **once** then runs to completion/exits; resume is a separate `exec resume` subprocess (verified via `codex exec --help`: *"instructions are read from stdin… non-interactive"*) | **No** (with current CLI) | Best achievable: deliver as next turn after exit. Needs a Codex persistent/streaming-input mode upstream. |
| **gemini** | subprocess | **No** — `-p` one-shot; `--output-format stream-json` is **output-only** (`index.ts:268`) | **No** (with current CLI) | Same as codex. |
| **qwen** | subprocess | **No** — Gemini-CLI fork, same `-p` one-shot (`index.ts:306`) | **No** | Same as gemini. |
| **kimi** | subprocess | **No** — `--print -p ""` one-shot (`index.ts:391`) | **No** | Same. |
| **openclaw** | acp | **Yes** (live JSON-RPC session) | **Maybe** — depends on whether the agent accepts a 2nd `session/prompt` mid-prompt vs serializing/queuing it; ACP also has `session/cancel` | Needs a per-agent probe (§7.4). |
| **pi** | acp | **Yes** | **Maybe** (same as openclaw) | Per-agent probe. |
| **copilot** | acp | **Yes** | **Maybe** (same) | Per-agent probe. |

**Two structural buckets:**
- **Steerable in principle** (persistent stdin or live session): claude ✅, muxcode (likely),
  ACP trio (maybe).
- **Not steerable with current CLI** (one-shot): codex, gemini, qwen, kimi. For these the only
  honest behavior is *deliver-as-next-turn* (which, since the subprocess must exit first, is
  effectively "after idle"). The UI should communicate this rather than pretend mid-turn works.

---

## 6. Tool-boundary detection is already done (all stream providers)

For completeness — detection was never the blocker, and it already exists per-provider, so the
"flush at tool boundary" gate (if we want explicit control over *when* within a turn to
surface a message) is cheap:

| Provider | Translator | Tool-call start | Tool result (next-inference seam) |
|---|---|---|---|
| claude / muxcode | `translator/claude.rs` | `content_block_start: tool_use` (`claude.rs:111-125`) | `user…tool_result` → `ToolResult` (`claude.rs:178+`) |
| codex | `codex-translator.ts` | `item.completed type=function_call` | `function_call_output` |
| gemini / qwen | `gemini-translator.ts` | `type=tool_use` | `type=tool_result` |
| kimi | `kimi-translator.ts` | `assistant.tool_calls[]` | `role=tool` |
| ACP (all) | `acp-translator.ts` | `session/update type=tool_call` | `type=tool_result` |

(For the steerable providers the CLI consumes the queued message at its own next-inference
boundary anyway — see §4 Q2 — so the runtime gate is an *optional* refinement, mainly useful
for batching/ordering or for an explicit "first tool_result only" policy.)

---

## 7. Proposed design

### 7.1 The core change: run steerable providers persistently + write mid-turn

1. **Use the persistent controller** for steerable providers (claude now; muxcode/ACP once
   verified) when an agent is created — or transparently upgrade an agent to persistent the
   first time a mid-turn delivery is requested.
2. **Deliver via the structured channel, not keystrokes.** Mid-turn delivery writes a normal
   NDJSON user message to live stdin through `send_message()` (`persistent.rs:148-168`) — never
   `send_input()`. (For ACP: a `session/prompt`, subject to §7.4.)
3. **Let the CLI do the steering.** Per §4, once written to live stdin during a turn, the CLI
   surfaces the message at its next inference step. No precise boundary timing required for the
   minimum viable version.

### 7.2 Fix the reactive / MCP `SendMessage` path to be controller-aware

`Handler::inject_message` currently hard-codes the PTY keystroke `input_sender`
(`main.rs:818-824`, `handler.rs:272-315`). Replace the single sender with a **controller-aware
deliver step**:

- persistent → `send_message()` (steers mid-turn);
- shell/PTY → keystrokes (today's behavior);
- one-shot subprocess → queue as next turn (and report "will deliver after current turn");
- acp → `session/prompt` (per §7.4).

This makes agent-to-agent `SendMessage` actually work for stream-json panes — which it does not
today (§3.3).

### 7.3 Optional delivery-policy + queue (refinement)

Replace the dead `wait_for_idle: bool` with an explicit policy, plumbed through
`InjectionRequest` (`reactive/types.rs`), the MCP tool, and the `agentinput` RPC:

```
enum DeliverPolicy { Immediate, NextToolBoundary, NextIdle }
```

- `Immediate` is the only valid policy for one-shot/PTY providers.
- `NextToolBoundary` / `NextIdle` matter only when an explicit gate is wanted on top of the
  CLI's own steering — drive the flush from the translator's `ToolResult` / `Done` events
  (the one new wire: translator event → per-block queue flush; the reactive `Handler` has no
  view of turn state today).
- Bound/order per agent (reuse messagebus bounds, `messagebus.rs`); idempotent on `request_id`.

### 7.4 ACP probe + handling

Before claiming ACP steering: test whether openclaw/pi/copilot accept a `session/prompt` while
a prior prompt is in flight. Options the agent may exhibit: (a) interleave/steer; (b) queue
until current completes (= turn-end delivery); (c) error. If (b)/(c), either accept turn-end
delivery for ACP or pair delivery with `session/cancel` (changes turn semantics — get product
sign-off, akin to interrupt).

---

## 8. Open questions — status

| # | Question | Status |
|---|---|---|
| 1 | Does Claude steer on a mid-turn stdin message? | **RESOLVED — yes** (§4, 2 runs) |
| 2 | Where is the message consumed? | **RESOLVED — next inference, after the in-flight tool_result** (§4) |
| 3 | Long-turn behavior (first vs every tool_result)? | **Mostly moot** — CLI consumes at its next step; if we add an explicit gate, recommend "first tool_result only" |
| 4 | Idle fast-path | **RESOLVED — trivial** |
| 5 | Does muxcode support `--input-format stream-json` persistent? | **RESOLVED — no, today.** muxcode is `controllerType: "subprocess"` with `persistent_launch_args: None` (`providers.rs`, `providers/index.ts`). It emits `claude-stream-json` so it *could* gain a persistent mode (Claude-compatible), but until it does it is in the one-shot, not-steerable bucket. Tracked as a provider follow-up. |
| 6 | Do ACP agents honor a mid-prompt `session/prompt`? | **PARTIALLY RESOLVED.** Delivery is now wired: `deliver_agent_message` routes ACP agents through `AcpController::send_input` → a `session/prompt` on the live JSON-RPC session (§10). Whether the *agent* acts on a 2nd prompt mid-turn vs. queues it is **agent-side and unprobed** (no ACP CLI installed locally: `pi/openclaw/copilot` absent). `send_input` does not gate on an in-flight turn, so the prompt is always sent; honor is the agent's choice. Re-probe when an ACP CLI is available. |
| 7 | Do codex/gemini/qwen/kimi have any persistent streaming-input mode? | **RESOLVED — no** (one-shot CLIs; codex confirmed via `--help`). Not steerable until upstream adds one. |
| 8 | Should one-shot providers expose interrupt-and-resend instead? | **OPEN — product decision.** It changes turn semantics; out of scope here. |

---

## 9. Recommendation

1. **Reframe the feature:** the win is *running steerable providers persistently and writing
   the message to live stdin mid-turn* — not "detecting the tool boundary" (already done) and
   not the CLI (already steers, §4).
2. **Ship it for Claude Code first** (proven): persistent controller + mid-turn `send_message()`
   + make the reactive/MCP `SendMessage` path controller-aware (§7.2). This also fixes
   agent-to-agent messaging to stream-json panes, which is currently broken (§3.3).
3. **Extend to muxcode and the ACP trio** after the §7.4 / Q5 probes.
4. **Be honest for one-shot providers** (codex/gemini/qwen/kimi): mid-turn steering is not
   possible with current CLIs; deliver as next turn and label it as such. Revisit if/when those
   CLIs gain a persistent streaming-input mode.
5. **Do not use `send_input()` keystrokes for stream-json agents** — it is rejected by design;
   use `send_message()`.

---

## 10. Implementation (this branch — Phase 3 of `SPEC_AGENT_CONTROL_PROTOCOL §6`)

Controller-aware delivery for muxbus Tier-1 / MCP `SendMessage` / inter-agent reactive
injection. Before this, `ReactiveHandler::inject_message` always sent PTY keystrokes via
`send_input`, which a persistent (stream-json) controller **rejects** — so messages silently
missed Claude panes. Now the reactive path delivers on each controller's *structured* channel,
which also lands the message **mid-turn (steering)** rather than waiting for idle.

**What landed:**
- `blockcontroller/persistent.rs`: `send_user_message(message)` — writes a `{type:"user",…}`
  NDJSON line to the **already-running** persistent stdin (no spawn config; errors if not
  running). Mirrors the AskUserQuestion `answer_question` precedent.
- `blockcontroller/mod.rs`: `AgentDelivery { Structured, Pty }` + `deliver_agent_message(block_id,
  message)` — persistent → `send_user_message` (Structured); ACP → `send_input` →
  `session/prompt` (Structured); everything else (shell/term PTY, one-shot subprocess) → `Pty`.
- `reactive/types.rs`: `MessageSender` closure type (`(block_id, message) -> Result<bool>`;
  `true`=structured delivered, `false`=use PTY, `Err`=structured controller failed).
- `reactive/handler.rs`: optional `message_sender` on `Handler` (+ `set_message_sender` on
  `Handler` and `ReactiveHandler`); `inject_message` tries it first — on `Ok(true)` it returns
  success **without** PTY keystrokes; on `Ok(false)` it falls through to the existing keystroke
  path; on `Err` it surfaces the failure and does **not** fall back (persistent rejects raw
  input).
- `main.rs`: wires `set_message_sender` → `deliver_agent_message`.
- Tests: `reactive/tests.rs` — structured delivery skips PTY; PTY fallback still keystrokes;
  structured failure does not fall back. (`cargo test -p agentmux-srv reactive` → 52 passed.)
- Incidental: fixed a **pre-existing** broken test build — `LanDiscoveryController::new` gained a
  6th `auth_key` param but 3 test call sites (`server/tests.rs`, `agent_handlers.rs` ×2) weren't
  updated, so the crate's test target didn't compile on main. Added `String::new()` at each.

**Why no explicit tool-boundary gate / `DeliverPolicy` queue (spec §7.3):** the empirical result
(§4 Q2) shows the CLI itself consumes a queued stdin message at its **next inference boundary**
(after the in-flight tool's result). So writing to the live stdin *is* the steer — no
runtime-side boundary detection or queue is required for the MVP. A `DeliverPolicy` enum / queue
remains a possible refinement (batching/ordering, explicit "first tool_result only") and is left
as a follow-up.

**Scope / non-goals:** one-shot subprocess providers (codex/gemini/qwen/kimi/muxcode) keep PTY
fallback (no live channel to steer); ACP delivery is wired but mid-turn *honor* is agent-side
(Q6). No frontend changes — this is the inter-agent/muxbus delivery path; the agent-pane UI
already delivers via `agentinput → send_message`.

---

## Appendix A — key file references

| Concern | File:line |
|---|---|
| Per-provider controller type (subprocess default) | `frontend/app/view/agent/providers/index.ts:160,199,240,268,306,391` |
| Claude persistent launch args | `frontend/app/view/agent/providers/index.ts:161` |
| ACP controller types | `frontend/app/view/agent/providers/index.ts:359,424,452` |
| Controller selection (meta → controller) | `agentmux-srv/src/backend/blockcontroller/mod.rs:301,349-407` |
| Persistent stdin write (`send_message`) | `agentmux-srv/src/backend/blockcontroller/persistent.rs:148-168` |
| Mid-turn `tool_result` precedent (`send_tool_result`) | `agentmux-srv/src/backend/blockcontroller/persistent.rs:170-192` |
| Persistent rejects raw input | `agentmux-srv/src/backend/blockcontroller/persistent.rs:625-627` |
| stdin writer drain loop | `agentmux-srv/src/backend/blockcontroller/persistent.rs:340-358` |
| AskUserQuestion requires persistent | `agentmux-srv/src/server/websocket.rs:779-809` |
| `agentinput` RPC (persistent vs subprocess) | `agentmux-srv/src/server/websocket.rs:884-1048` |
| Reactive inject (PTY keystrokes) | `agentmux-srv/src/backend/reactive/handler.rs:179-335` |
| Reactive inject HTTP + forwarding | `agentmux-srv/src/server/reactive.rs:18-143` |
| `InjectionRequest.wait_for_idle` (dead scaffold) | `agentmux-srv/src/backend/reactive/types.rs:21` |
| Input-sender wiring (hard-coded keystrokes) | `agentmux-srv/src/main.rs:818-824` |
| ACP prompt delivery (`session/prompt`) | `agentmux-srv/src/backend/blockcontroller/acp.rs:569-611` |
| Tool-boundary parsing (claude) | `agentmux-srv/src/agents/translator/claude.rs:103-176` |
| Translators (codex/gemini/kimi/acp) | `frontend/app/view/agent/providers/{codex,gemini,kimi,acp}-translator.ts` |
| Frontend turn phases / `toolsActive` | `frontend/app/store/agent-pane-state/types.ts:118-152`, `reducer.ts:452-467` |

## Appendix B — reproducing the steering probe

```bash
python3 docs/specs/evidence/steer-probe.py \
  /path/to/claude --input-format stream-json --output-format stream-json \
  --verbose --include-partial-messages --dangerously-skip-permissions
```

Outputs a timestamped event trace; `STEERED-MIDTURN` appearing before `RESULT` (with only the
first of four sleeps run) demonstrates mid-turn steering. See the two captured run logs.
