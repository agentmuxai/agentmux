# Agent Recurring-Task / Polling Primitives — Design Hardening
**Date:** 2026-08-04
**Status:** Phase 0 implemented (same-day follow-up PR) — Phases 1-2 still proposed, not started.
**Scope:** `agentmux-mcp` (`Loop` tool), `agentmux-srv` (`Cron`, `/agentmux/reactive/inject`, `wrap_jekt_message`)
**Trigger:** Live use of `mcp__agentmux__Loop` this session to babysit a GitHub PR's review status (poll every 1m, react when approved+mergeable). Concretely: ~20+ fires, most reporting "no change," each one arriving as a full agent-to-agent JEKT message with the complete envelope overhead; no way to express "wake me only when the review state changes" declaratively; no automatic detection of the ~26-consecutive-unchanged-checks stretch that made the loop unproductive; the human directly observed and flagged this as "poorly designed" in the moment.

**Correction, and the actual point of this doc (2026-08-04, same day):** the first draft of this doc concluded `ScheduleWakeup` doesn't exist and proposed AgentMux build a `PollCondition`/backoff/circuit-breaker system from scratch to fix the pain points above. **Both are wrong in the same way** — the research only looked inside the `agentmux` repo. `ScheduleWakeup` is real: it's a **harness-native, top-level Claude Code tool** (not `mcp__agentmux__`-prefixed), and so are native `CronCreate`/`CronList`/`CronDelete`. Pulled their actual schemas directly: **native `ScheduleWakeup` already has essentially everything Part 3 of the first draft proposed building** — cache-window-aware backoff guidance, zero delivery overhead (it's the harness re-invoking its own session, not a message to anyone), explicit guidance against polling harness-tracked work, and a `reason` field for transparency. Native `CronCreate` already has jitter guidance, a `durable` flag (session vs. persisted — the exact Loop-vs-Cron split, unified into one tool), and 7-day auto-expiry (a form of stuck-loop bound). **The right fix isn't reimplementing this in AgentMux — it's using what already exists and stopping AgentMux's own `Loop`/`Cron` from shadowing it under near-identical names.** This is also, in retrospect, the direct answer to why this session's `Loop`-based PR-babysitting was the wrong tool choice: it was a same-session, self-scheduling, externally-tracked-state-polling task — exactly `ScheduleWakeup`'s stated use case — routed instead through AgentMux's cross-agent messaging pipeline. The rest of this doc is rewritten around that correction. See §1.9 for the concrete comparison and Part 2 for the resulting direction: **align with, and delegate to, native harness primitives (1:1) rather than build parallel ones.**

---

## TL;DR

- `Loop`, `Cron`, and `SendMessage` are not three delivery mechanisms — they're three schedulers sitting on **one shared delivery primitive** (`POST /agentmux/reactive/inject` → `wrap_jekt_message`), and every single fire of any of them pays the same fixed ~600-650 byte / ~150 token JEKT envelope cost, **even for a self-loop with nothing to report.**
- **Claude Code already ships native `ScheduleWakeup` and `CronCreate`/`CronList`/`CronDelete` tools that solve the self-scheduling case AgentMux's `Loop`/`Cron` also try to solve — with better-designed backoff, jitter, durability, and expiry semantics than AgentMux built.** AgentMux's versions exist under the *same names* (`CronCreate` vs. `mcp__agentmux__CronCreate`), which is exactly the confusion a prior draft spec already flagged and never resolved.
- There is **no server-side condition evaluation** in AgentMux's own `Loop`/`Cron` — every "check and react" cycle costs a full LLM turn. Native `ScheduleWakeup` sidesteps this differently (and arguably better): it doesn't poll at all when the harness can already track completion, and hands the model explicit guidance for picking a sane delay when it must poll externally-tracked state.
- **No backoff, no stuck-loop detection in AgentMux's own primitives.** Both were explicitly proposed in a prior draft spec (`SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md`, Phase P2/P3) and never implemented — a documented plan stalling after partial adoption, same pattern found twice elsewhere this session (migration markers, docs staleness). Native `ScheduleWakeup`/`CronCreate` already have both, unprompted.
- `wait_for_idle`, the one field in AgentMux's own delivery path that could have carried "wait for a quiet moment" semantics, is dead — three separate prior specs already independently confirm srv never reads it.

---

## Part 1 — Current state (verified against source, not the tool descriptions)

### 1.1 One delivery primitive, three schedulers

- `Loop` lives entirely client-side in `agentmux-mcp/src/main.rs` — a per-`Loop()` call `tokio::spawn` task in an in-process `HashMap<loop_id, LoopEntry>` that dies with the agent pane (`main.rs:42-55, 1348-1372`). Deliberately srv-free by design (`docs/specs/SPEC_MCP_LOOP_TOOL_2026_06_16.md`: *"a loop has no state beyond 'fire the inject on a timer'... would mean new endpoints + an AppState registry + scheduler tasks for a feature that is purely a timer"*).
- `Cron` lives server-side (`agentmux-srv/src/backend/cron/mod.rs`, `backend/storage/cron.rs`) — persisted SQLite rows, one `tokio` task per enabled job, computed via the `cron` crate, survives srv restarts (unlike `Loop`, which dies with the pane).
- Both — and `SendMessage` — construct the identical `InjectRequest` and `POST` it to the identical `/agentmux/reactive/inject` endpoint (`agentmux-mcp/src/main.rs:1338` for Loop, `backend/cron/mod.rs:193-216` for Cron, `main.rs:985-993` for SendMessage — the request shape is byte-for-byte the same). Server-side this becomes one call: `Handler::inject_message` (`agentmux-srv/src/backend/reactive/handler.rs:192`).
- **There is no branch anywhere for `source_agent == target_agent`.** A self-loop is, to the delivery code, indistinguishable from one agent messaging a different agent.

### 1.2 Every fire pays the full JEKT envelope, unconditionally

`wrap_jekt_message` (`agentmux-srv/src/backend/reactive/sanitize.rs:197-230`) is called exactly once, unconditionally, on every inject (`handler.rs:276` — the only call site in the codebase):

```rust
let structured_tag = format!(
    "[JEKT:FROM={from} TO={target_agent} TIER={effective_tier} DELIVERY={delivery_tier} \
     TRUST={trust} MSGID={msg_id} PRIORITY={priority} TS={ts_secs}]"
);
format!(
    "{structured_tag}\n{sep}\nFrom: {from} | To: {target_agent} | ts={ts_secs}{warn}\n{msg}\n{sep}\n{hint}\n[/JEKT]"
)
```

Measured directly: the two box-drawing separator lines are 180 bytes each in UTF-8 (multi-byte `─`, not ASCII), the structured tag runs ~150-160 bytes — **~600-650 bytes / ~150 tokens of fixed overhead per fire**, before the actual prompt content. For a 1-minute babysitting loop, that's ~150 tokens spent on envelope alone every 60 seconds, whether or not anything changed.

The human-facing UI *does* mitigate this — `JektBubble.tsx` collapses jekt messages to a one-line summary by default. **But this is a frontend rendering trick applied after the fact**; it doesn't reduce what the agent's own context actually receives. The expensive resource (LLM context) pays the full cost every time; only the cheap resource (a human's scrollback) is protected.

### 1.3 No declarative "poll until X" — condition logic lives in re-derived prose

Every fire re-injects the *same static prompt text*, and the condition-check + branching logic ("if approved and mergeable, merge and stop; if changed, investigate; if unchanged, one-line status") has to be written into that prompt as instructions and re-parsed by the model from scratch on every single fire. There is no way to say, declaratively, "poll this until condition X holds" — the tool has no concept of a condition at all, only "re-inject this text on a timer."

### 1.4 No backoff

Both `Loop` (fixed `interval: String` parsed once) and `Cron` (fixed cron expression) fire on a strictly regular schedule for their entire lifetime. No linear/exponential backoff, no jitter, no adaptive slow-down when nothing has changed for a while.

### 1.5 `wait_for_idle` is dead scaffolding

`InjectRequest.wait_for_idle: bool` (`reactive/types.rs:47`) is hardcoded `false` at every call site across the codebase and never read by `Handler::inject_message`. Confirmed dead independently by three prior specs' own words (`SPEC_MCP_LOOP_TOOL_2026_06_16.md`, `SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md`, `SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16.md`) — the one field that could plausibly have carried "don't wake the agent mid-turn" or "wait for a quiet window" semantics has been inert since it was added.

### 1.6 Stuck-loop detection: proposed, never built

`SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md` §3.1.4 (Phase P3) proposed hashing the last N injected prompts and circuit-breaking on repeats. No hash tracking, no circuit breaker, exists anywhere in `agentmux-mcp` or `agentmux-srv` today. This session hit exactly the gap that proposal was meant to close: ~26 consecutive unchanged checks with no automatic signal, requiring a human to notice and ask whether to pause, and then me stopping the loop manually.

### 1.7 `ScheduleWakeup` is real — it's a harness-native tool, not an AgentMux one (correction from first draft)

`Loop`'s own shipped description says *"Do NOT use for one-off tasks — use ScheduleWakeup for that"* — and this is **correct advice, not a dangling reference.** `ScheduleWakeup` is a top-level Claude Code tool (not routed through `mcp__agentmux__` MCP calls at all), confirmed live by fetching its actual schema. It genuinely does not exist inside the `agentmux` *repo* — grepping the repo for it finds nothing, and the AgentMux MCP tool-list test (`agentmux-mcp/src/main.rs:1841`, 27 tools enumerated) correctly doesn't include it, **because it isn't AgentMux's to implement.** The first draft of this doc searched only the repo and concluded it was missing; that was the wrong scope for the question. `Loop`'s description was already pointing agents at the right tool for the one-off case — it just doesn't say so clearly enough to stop an agent (this one, this session) from reaching for AgentMux's `Loop` for a same-session recurring self-check instead, where native `ScheduleWakeup`'s re-arm-by-resubmitting-the-prompt pattern (see §1.9) covers that case too.

### 1.8 This has partial prior art, and it already stalled once

`SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md` is a real, already-written analysis of Loop/Cron gaps — some of its recommendations shipped (`LoopList`, `max_iterations`), several explicitly didn't (idle-aware firing "deferred to follow-up," stuck-loop detection never built). This is the same pattern independently found twice already this session in unrelated subsystems (migration markers, docs staleness): **a documented plan, partially executed, with no mechanism forcing the rest to land.** Any fix proposed here should account for that pattern rather than just add a fourth spec to the pile.

**That same doc already named the exact problem this rewrite is about, in one sentence, and it was never acted on:** *"The confusion: Claude Code CLI exposes its own `CronCreate`/`CronList`/`CronDelete` tools... but these are ephemeral... and are not persisted by AgentMux's server."* That's the name collision in §1.9 below, flagged in this repo's own docs over a month before this session re-discovered it the hard way.

### 1.9 Native harness primitives already solve most of what §1.1-1.6 are missing

Direct schema comparison, native (top-level, Claude Code) vs. AgentMux (`mcp__agentmux__`-prefixed):

| Capability | AgentMux `Loop`/`Cron` | Native `ScheduleWakeup`/`CronCreate` |
|---|---|---|
| Backoff / adaptive delay | None (§1.4) | Explicit cache-window-aware guidance baked into the tool description itself (60-270s stays in the 5-min prompt-cache TTL; 300s+ commits to a longer wait; "don't pick 300s, it's worst-of-both") |
| Delivery cost for a self-wakeup | Full JEKT envelope via muxbus, every fire (§1.2) — ~150 tokens fixed overhead | **Zero** — it's the harness re-invoking its own session; there is no message, no envelope, nothing to parse |
| Guidance against wasteful polling | None — a `Loop` will happily poll something the platform already tracks | Explicit: *"Do NOT schedule a short-interval wakeup to poll for background work you started — when harness-tracked work finishes, you are re-invoked automatically"* |
| Durable vs. session-only | Two separate tool families (`Loop` = session-only, `Cron` = persisted) | One tool, one `durable: bool` flag |
| One-shot vs. recurring | `Loop` = recurring only, `CronCreate` (AgentMux) = both via `max_fires` | One tool, one `recurring: bool` flag |
| Stuck-loop bound | None (§1.6) | 7-day auto-expiry on recurring native cron jobs; `ScheduleWakeup`-chained loops end whenever the agent stops re-arming them (no infinite-fire failure mode by construction) |
| Stampede avoidance | None | Explicit jitter guidance ("avoid :00 and :30 minute marks... every user who asks for 9am gets `0 9`... which means requests from across the planet land on the API at the same instant") + scheduler-level deterministic jitter |
| Transparency to the human | Collapsed JEKT bubble, opaque reasoning | `reason` field, shown directly to the user: *"watching CI run" beats "waiting"* |

**This is not a close call.** Native `ScheduleWakeup`/`CronCreate` are more thoughtfully designed than what AgentMux built, for the specific case of an agent scheduling its own future turn. AgentMux's `Loop`/`Cron` add exactly one thing native tools can't do: **target a different agent, or persist/deliver across whatever CLI/provider that other agent runs** (native `CronCreate` schedules *this* Claude Code session's own future prompt; it has no concept of "some other agent, possibly running Codex or Gemini, elsewhere"). That cross-agent/cross-provider case is real and AgentMux is right to have a mechanism for it — but it's a narrower case than "any recurring check," and it's not what this session's self-loop needed.

---

## Part 2 — Goals / non-goals

**Primary goal (per explicit direction, 2026-08-04): keep AgentMux's scheduling primitives 1:1 with what Claude Code has already built in, rather than re-deriving parallel design decisions AgentMux is worse-positioned to get right.** Concretely, that means:

- **For same-agent, same-session self-scheduling (the case this session actually needed): stop routing it through `mcp__agentmux__Loop`/`Cron` at all.** Use native `ScheduleWakeup` (one-off or self-re-arming recurring) or native `CronCreate` (durable, cron-expression-based) directly — they already have the backoff, jitter, durability, and expiry semantics AgentMux would otherwise have to reinvent to match (§1.9).
- **Resolve the name collision.** `mcp__agentmux__CronCreate` and native `CronCreate` coexisting under the same bare name (disambiguated only by MCP prefix, which is easy to miss — this session almost certainly would have grabbed the wrong one without careful checking) was flagged in `SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md` over a month ago and never fixed. Either rename AgentMux's tools to be unambiguous at a glance (e.g. `AgentMuxCron*`/`CrossAgentLoop`) or make each tool's description explicitly cross-reference the other so an agent picks correctly without needing to already know both exist.
- **Scope AgentMux's own `Loop`/`Cron` down to what native tools genuinely can't do**: targeting a *different* agent, and working across whatever CLI/provider that agent runs (native scheduling is inherently single-session/single-provider). Stop treating them as a general-purpose "recurring task" primitive for every case, including the self-loop case they were never the best fit for.
- Where AgentMux's own primitives remain the right tool (the cross-agent case), still borrow the *design*, not just the delivery mechanism: backoff guidance, jitter, a durability flag instead of two separate tool families, and a stuck/expiry bound — because these are good ideas regardless of which tool implements them, and native `ScheduleWakeup`/`CronCreate` already prove them out.
- Fix `Loop`'s description to state the self-loop-vs-cross-agent distinction explicitly, rather than relying on an agent to infer it (this session didn't).

**Non-goals:**
- Not building a bespoke `PollCondition`/server-side-condition-evaluation system in AgentMux (this was the first draft's Part 3 — retired by this rewrite). If a genuinely AgentMux-specific cross-agent polling need turns out to require it later, revisit then; don't build it speculatively to match a native capability that already exists for the case actually observed this session.
- Not building a general workflow/DAG engine — `agentmux-srv/src/drone/executor/blocks/condition.rs` already exists for that in the separate Drone subsystem, orthogonal to this doc.
- Not redesigning JEKT envelope semantics for genuine cross-agent messaging — that delivery path is correct for what it's actually for (a different agent, possibly a different provider); the issue was only ever using it for same-agent wakeups.

---

## Part 3 — Proposed direction

### Self-loop case: delegate, don't build

For an agent scheduling its *own* future turn (recurring self-checks, one-off reminders, babysitting external state like a CI run or a PR) — the exact shape of this session's actual need — the answer is: **use native `ScheduleWakeup`/`CronCreate` directly.** No AgentMux engineering work is required for this case; it already works today, better than what a from-scratch AgentMux implementation would likely produce on a first pass. The work here is *documentation and habit*, not code:
- Update `mcp__agentmux__Loop`'s own description to say explicitly: *"For a same-session self-check (no other agent involved), prefer native `ScheduleWakeup` (one-off/adaptive) or `CronCreate` (durable/cron-expression) — they have no delivery overhead and built-in backoff guidance. Use this tool only when the target is a different agent."*
- Same clarifying cross-reference the other direction isn't needed (native tools don't know AgentMux exists, appropriately — they're host-level primitives).

### Cross-agent case: keep AgentMux's own primitives, but align their design with native ones

`mcp__agentmux__Loop`/`Cron` remain necessary for their actual differentiator — targeting another agent, potentially on a different CLI/provider, via the muxbus delivery path (`wrap_jekt_message` → PTY/structured-stdin). That's legitimate and native tools can't do it. But their *design* should borrow from §1.9 rather than staying as-is:
- **Durability as one flag, not two tool families.** Fold `Loop` and `Cron` into one primitive with a `durable: bool` (matching native `CronCreate`'s shape exactly), rather than maintaining two separately-evolving implementations that have already started drifting (`LoopList`/`max_iterations` exist for `Loop`; `Cron` has no equivalent).
- **Backoff and jitter guidance**, adapted for the cross-agent case (the cache-window math is Claude-session-specific and won't directly translate, but "don't let every stampede land on the same instant" and "slow down once nothing's changing" both do).
- **An expiry/stuck bound**, matching native `CronCreate`'s 7-day auto-expiry — AgentMux's own primitives currently have no equivalent (§1.6).
- Rename to resolve the collision from §1.9/§1.8 — an agent should not need to already know two tools of the same name exist and pick the MCP-prefixed one correctly by instinct, which is what this session actually had to do.

### What's explicitly *not* being built

The first draft's `PollCondition` (server-side declarative condition evaluation with zero-LLM-cost polling) is retired as a goal for now. It would meaningfully improve the cross-agent case too, but building it isn't justified by anything observed this session — the actual pain (self-loop overhead, no backoff, no stuck detection) is fully addressed by delegating to native tools for the case that needs it. Revisit only if a genuine cross-agent polling need surfaces that native tools structurally can't cover and the muxbus-delivery overhead is shown to matter at that point.

---

## Part 4 — Phased plan

### Phase 0 — cheap, immediate (S)
- Update `Loop`'s tool description to explicitly recommend native `ScheduleWakeup`/`CronCreate` for same-session self-checks, and state the cross-agent-only scope clearly (see Part 3). This directly prevents the mistake made this session — no code change, just fixing what the tool tells the agent to do.
- Resolve the `CronCreate`/`CronList`/`CronDelete` name collision with the native tools of the same name — at minimum, add an explicit disambiguating line to each AgentMux tool's description ("this is AgentMux's cross-agent cron, distinct from the native per-session `CronCreate`"); a rename is the more durable fix but needs a compat/migration decision (see below).
- Add an expiry/stuck bound to AgentMux's own `Cron` (e.g. auto-disable a job after N consecutive fires with no observable state change, or a hard max-age default) — closes §1.6 without needing full condition evaluation.

### Phase 1 — Loop/Cron consolidation onto a `durable` flag (M)
- Merge `Loop` and `Cron` into one tool family differentiated by `durable: bool`, matching native `CronCreate`'s shape. Requires a compat decision for existing `loop_id`/cron-job-id callers and any in-flight jobs at rollout time — scope this properly rather than assuming it's a clean swap.
- Add backoff/jitter as an opt-in policy on the unified primitive.

### Phase 2 — Rename to fully resolve the collision (S, depends on Phase 1's shape being settled)
- Once the unified primitive's shape is stable, rename away from the bare `Cron*`/`Loop` names so the distinction from native tools is visible without reading descriptions (e.g. an `AgentMux`-prefixed or `CrossAgent`-prefixed family).

---

## Priority order

`Phase 0` (this week — the description fix is the single highest-value, lowest-risk change in this doc: it would have prevented this session's actual mistake, and costs nothing but editing a string) → `Phase 1` (real but bounded engineering, needs its own compat scoping) → `Phase 2` (depends on 1, cosmetic/naming once the shape is settled).
