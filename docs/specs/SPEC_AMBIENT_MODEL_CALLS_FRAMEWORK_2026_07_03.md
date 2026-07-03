# SPEC: A Unified Framework for Ambient (Non-User-Driven) Model Calls

**Date:** 2026-07-03
**Status:** Draft — proposal, not yet implemented
**Related:** `agentmux-srv/src/server/app_api/session.rs`, `frontend/app/view/agent/hooks/useAgentActivitySummary.ts`,
`frontend/app/view/agent/hooks/useBlockActivity.ts`, `frontend/app/view/swarm/`, `frontend/app/store/token-usage.ts`,
`docs/specs/SPEC_AGENT_OSC_TITLE_ACTIVITY_2026_06_18.md`, `specs/SPEC_AGENT_PANE_HEADER_NAME_PRECEDENCE_2026_06_29.md`,
`docs/retro/retro-haiku-activity-pane-header-2026-06-24.md`

---

## 0. TL;DR

Today there is exactly one "ambient" LLM call in the codebase — a Haiku call that
summarizes what an agent is doing, once per completed turn, feeding both the pane
header text and the Swarm tree (`invoke_cli_for_activity`, `session.rs:198-267`).
It has no debounce, no in-flight guard, no cancellation, and its tokens are
silently discarded — they never reach the app's total-tokens display. It shares
a single meta key (`term:activity`) with a second, unrelated, LLM-free writer
(OSC terminal-title sequences), with no ownership or precedence protocol between
them. The combination produces the two symptoms reported: calls firing too often
and overlapping, and panes/Swarm nodes reading as idle/blank when the agent is
actually active.

This spec proposes an **Ambient Model Call (AMC) framework**: a single mandatory
gateway that every present and future non-user-driven model call must go through,
built on three well-established patterns from outside this codebase — single-flight
request coalescing, generation/epoch-based cancellation and staleness rejection,
and mandatory-gateway cost accounting. It is a point of leverage: because Haiku is
"almost always" the model for this class of call today (per the user), centralizing
now is cheap and prevents the next five ad-hoc call sites from repeating the same
mistakes.

---

## 1. Current state (mapped 2026-07-03)

### 1.1 The one real LLM call site

- Trigger: `frontend/app/view/agent/hooks/useAgentActivitySummary.ts:42-71` — on
  `TurnPhase.kind === "Done"`, calls `RpcApi.AgentActivitySummaryCommand` (20s
  frontend timeout, no retry).
- Backend: `agentmux-srv/src/server/app_api/session.rs:109`
  (`register_session_activity_summary`) tail-reads the last 32KB of the block's
  FileStore output, builds a summarize prompt, and `invoke_cli_for_activity`
  (`session.rs:198-267`) spawns the CLI with `--model claude-haiku-4-5-20251001`,
  15s timeout.
- Result is written to block meta key `"term:activity"`.
- Consumers: `frontend/app/view/agent/agent-model.ts:106-122` (`viewText()`, the
  pane header text) and `frontend/app/view/swarm/swarm-model.ts:221-222`
  (`AgentTreeNode.activitySummary`, the Swarm tree).

This is a **singleton pattern** — grepping for `haiku` and for
`invoke_cli_for_activity`-shaped code across `agentmux-srv/src/agents/`,
`.../server/`, `.../backend/`, and frontend hooks turns up no other ambient LLM
call anywhere in the repo. `agents/failure.rs`'s `classify()` (CLI exit-code/stderr
taxonomy) looks superficially similar but is pure Rust, no subprocess, no model.
A prior "session digest" feature was removed; only its text-extraction helper
(`extract_digest_text`, `session.rs:272`) survives, reused by the activity-summary
handler itself.

**Implication:** there is no legacy fan-out to migrate yet. This is the ideal moment
to build the framework — one call site to move, and every future one built on top
of it instead of copied from it.

### 1.2 Bug 1 — tokens silently discarded

`invoke_cli_for_activity` parses the Haiku subprocess's stream-json stdout only for
`type == "assistant"` text blocks; it never reads the `result`/usage event, and
`ActivitySummaryResult` has no token-count field to carry it back
(`session.rs:242-266`). The app's total-tokens counter
(`frontend/app/store/token-usage.ts`'s `recordTurn()`) is fed exclusively by
`frontend/app/view/agent/useAgentStream.ts:426-444` from the *main* agent turn's
`session_end` event. The Haiku subprocess is a real, billed API call that never
touches this counter — a structural under-count, not a display lag.

### 1.3 Bug 2 — two uncoordinated writers to one meta key

`term:activity` has two independent writers with no merge/precedence protocol
(last-write-wins via plain `UpdateObjectMeta`):

1. `frontend/app/view/agent/hooks/useBlockActivity.ts:39-83` — subscribes to a
   `block:activity` WPS event sourced from **OSC 0 terminal-title escape
   sequences** the CLI itself emits (parsed by
   `agentmux-srv/src/backend/osc_extractor.rs`, a pure state machine, no LLM, no
   subprocess — see `docs/specs/SPEC_AGENT_OSC_TITLE_ACTIVITY_2026_06_18.md`).
   300ms debounced. Cleared only on `ControllerStatus === "done"`.
2. `useAgentActivitySummary.ts:54-66` — the Haiku writer (§1.1). No debounce, no
   in-flight guard (two 15-20s Haiku calls from quick successive turns can run
   concurrently), and — per its own comment at line 15 — `term:activity` is
   **never cleared on a new turn**, so a stale prior-turn summary can sit
   indefinitely if the newer RPC errors silently (its `.catch()` swallows all
   failures, lines 67-69).

`specs/SPEC_AGENT_PANE_HEADER_NAME_PRECEDENCE_2026_06_29.md:108` explicitly treats
`term:activity`'s generation/ownership as out of scope — this gap has been
noted, not fixed, before.

### 1.4 Bug 3 — Swarm renders "idle"/blank for actually-active agents

`frontend/app/view/swarm/swarm-view.tsx:163-167` (`phaseToDisplayStatus`):
`if (!phaseAccessor) return "idle";` — if a block's `TurnPhase` was never
registered in the *current renderer's* registry
(`frontend/app/store/agentActivity.ts:55-57`, populated only by `registerActivity()`
from a *mounted* agent-pane component), the status defaults to `"idle"`
regardless of actual backend state. A subagent running in an unmounted pane,
a background tab, or another workspace has no registry entry — it reads as idle.
Combined with a null `activitySummary` (§1.3, `swarm-view.tsx:231`
`<Show when={node.activitySummary}>` renders nothing), this is the concrete
mechanism behind "clearly something is in progress but the UI shows nothing."

---

## 2. Prior art

Four groups of patterns, all directly applicable and already load-bearing in
mainstream systems facing the same shape of problem (frequent, cheap,
non-user-initiated background calls updating live UI state):

**Request coalescing / single-flight.** Go's `singleflight.Do(key, fn)`: concurrent
callers with the same key share one in-flight execution rather than issuing
duplicate work; no persistent cache, pure concurrent dedup.
([nickyt.co](https://www.nickyt.co/blog/gos-singleflight-package-and-why-its-awesome-for-concurrent-requests-4122/))
RxJS `switchMap`: unsubscribes (truly cancels) the previous inner call the
instant a new trigger arrives — the standard pattern for "only the latest
matters," explicitly contrasted with `mergeMap` for calls with side effects.
([learnrxjs.io](https://www.learnrxjs.io/learn-rxjs/operators/transformation/switchmap))
Cloudflare calls the server-side version "request collapsing," framed around
avoiding thundering-herd duplicate work.
([stanza.dev](https://www.stanza.dev/courses/redis-caching/advanced-caching/redis-caching-request-coalescing))

**Cancellation-on-stale-context.** `AbortController` paired with a monotonic
generation/epoch counter — `abort()` only *signals* cancellation, it doesn't
guarantee the work stops, so the receiver independently compares the counter
value captured at request time against the current counter at resolution time
and discards on mismatch even if the abort signal didn't land.
([dev.to](https://dev.to/bdestrempes/a-practical-guide-to-the-abortcontroller-api-5420))
Fencing tokens generalize this: the *receiver*, not the caller, is the
authority that rejects stale writes.
([levelup.gitconnected.com](https://levelup.gitconnected.com/beyond-the-lock-why-fencing-tokens-are-essential-5be0857d5a6a))
TanStack Query's own bug history shows *ignoring* a stale response is not
enough — a documented failure mode where a stale result still satisfied a
newer query; the fix was active discard (`removeQueries`), not passive
staleness. ([github.com/TanStack/query#6953](https://github.com/TanStack/query/discussions/6953))

**Mandatory-gateway cost accounting.** OpenTelemetry's GenAI semantic
conventions define vendor-neutral `gen_ai.*` span attributes
(`gen_ai.usage.input_tokens`/`output_tokens`, `gen_ai.operation.name`) so any
call site — user-facing or background — emits a comparable, taggable record.
([github.com/open-telemetry/semantic-conventions-genai](https://github.com/open-telemetry/semantic-conventions-genai))
LLM gateways (LiteLLM/Langfuse/Helicone) tag calls with free-form
`tags`/`metadata` (e.g. `purpose=background-summary`) filterable in a
dashboard. ([docs.litellm.ai](https://docs.litellm.ai/docs/observability/helicone_integration))
The structural fix industry converges on once a codebase has more than one LLM
call site: route **all** calls through one internal client that owns
credentials, so no new call site can physically bypass logging/accounting —
this is the direct answer to "how do we make sure Haiku tokens are never
silently discarded again."
([red-gate.com](https://www.red-gate.com/simple-talk/ai/the-llm-layer-youre-probably-missing-llm-gateway-pattern-explained/))

**Binding async results to UI entity lifecycle.** The reusable synthesis across
the above: an entity-scoped generation counter, used simultaneously as the
single-flight coalescing key, the cancellation scope, and the write-time
staleness check — "if current entity generation ≠ response generation, discard."

---

## 3. Proposed design: the Ambient Model Call (AMC) framework

### 3.1 Scope and one hard rule

**Every current and future non-user-driven model call in the backend must be
issued through the AMC gateway. No other code path may spawn a model
subprocess for augmentation/summarization purposes.** This is the structural
guarantee that makes under-counted tokens (§1.2) and uncoordinated concurrent
calls (§1.3) impossible by construction rather than by convention.

This does **not** apply to the main user-driven agent turn pipeline
(`useAgentStream.ts` / the primary CLI invocation) — that path already has its
own token accounting and is not "ambient."

### 3.2 Core primitives

**Entity + purpose key.** Every AMC request is keyed by
`(entity_id, purpose)` — e.g. `(block_id, "activity_summary")`. `entity_id` is
whatever the caller is augmenting (today: a block; the key shape generalizes to
panes, workspaces, or non-block entities later). `purpose` is a short stable
string used for both single-flight coalescing and cost-dashboard tagging
(mirroring the LiteLLM/Langfuse `purpose=` tag pattern, §2).

**Generation counter per entity.** Each entity carries a monotonically
increasing generation, bumped whenever its underlying state advances to a new
"round" (for blocks: a new turn starting). A request captures
`(entity_id, generation)` at issue time. On resolution, the AMC gateway itself
— not the caller — checks the entity's *current* generation against the
request's captured generation; a mismatch means **discard**, never write.  This
is the fencing-token pattern (§2): the authority to reject stale results lives
at the write boundary, not scattered across every call site's own ad hoc check
(unlike today's `useAgentActivitySummary.ts` turn-ID check, which is a
caller-side filter only).

**Single-flight per `(entity_id, purpose)`.** Before issuing a subprocess, the
gateway checks for an in-flight request with the same key. If found: either
attach to its result (dedup) or drop the new trigger (latest-wins), per
purpose-level policy — activity summaries want latest-wins (a fresher turn
supersedes a summary of an older one), so a new trigger for a key with an
in-flight request **cancels** the old one rather than queuing behind it
(`switchMap` semantics, §2), rather than the current "let two Haiku calls run
concurrently" behavior.

**Real cancellation, not just discard-on-arrival.** When a new trigger
supersedes an in-flight request for the same key, the gateway actively kills
the subprocess (Rust: hold a `tokio::process::Child` handle + generation guard,
`child.kill()` on supersession) rather than letting it run to completion and
discarding the result — this also stops burning tokens on a call whose answer
is already known to be moot, which a pure "check on arrival" design does not.

**Debounce / minimum interval.** Per-purpose configurable minimum spacing
between calls for the same entity (default: no more than one activity-summary
call per N seconds per block, independent of how many turns complete in that
window) — directly answers "these calls are too often." `useBlockActivity.ts`'s
existing 300ms debounce is the right shape; the gateway makes it a first-class,
uniformly-applied primitive rather than a pattern reimplemented ad hoc per call
site.

**Mandatory accounting.** Every AMC call, success or failure, records
`(purpose, model, input_tokens, output_tokens)` through the same counter the
main-turn pipeline uses (`token-usage.ts`'s `recordTurn`-equivalent), tagged
distinctly (e.g. a new `"ambient"` service bucket, or `purpose`-qualified
buckets like `"ambient:activity_summary"`) so the total-tokens view can show
ambient vs. user-driven cost as a breakdown, not just a merged number. Because
issuing an ambient model call and recording its usage happen inside the same
gateway function, no call site can add a ninth ambient feature next year that
silently repeats §1.2.

### 3.3 Sketch (illustrative, not final API)

```rust
// agentmux-srv/src/ambient/mod.rs (new)
pub struct AmbientCallKey {
    entity_id: String,   // e.g. block id
    purpose: &'static str, // e.g. "activity_summary"
}

pub struct AmbientCallRequest {
    key: AmbientCallKey,
    generation: u64,      // caller's snapshot of entity generation
    model: &'static str,  // defaults to the configured ambient model (Haiku today)
    prompt: String,
    policy: SupersedePolicy, // Cancel | Dedup | Queue(n)
}

// Gateway owns: in-flight table keyed by AmbientCallKey, per-entity generation
// store, debounce timers, and the ONLY code path allowed to spawn an ambient
// model subprocess. Returns None if superseded/cancelled before completion.
pub async fn call(req: AmbientCallRequest) -> Option<AmbientCallResult>;
```

```ts
// frontend/app/hooks/useAmbientModelCall.ts (new, generalizes useAgentActivitySummary.ts)
// Callers pass (entityId, purpose, generation) instead of hand-rolling their
// own turn-id staleness check; the hook itself no longer decides whether to
// fire — it just declares "this entity, this generation, wants this purpose
// refreshed," and the backend gateway owns coalescing/cancellation/debounce.
```

### 3.4 Fixing the shared-key problem (§1.3)

Split `term:activity` into two explicitly-owned keys instead of one
contested one:

- `term:osc_title` — the free, LLM-less CLI-emitted title (current
  `useBlockActivity.ts` writer). Infrequent, essentially free; always safe to
  show if present.
- `term:ambient_summary` — the AMC-gateway-owned Haiku summary, generation-
  stamped, cleared/superseded automatically by the gateway's generation check.

Precedence at render time (`agent-model.ts:viewText()`, `swarm-model.ts`):
prefer `term:ambient_summary` if present and its generation matches the
entity's current generation; else fall back to `term:osc_title`; else show
nothing. This replaces last-write-wins with an explicit, generation-aware
precedence rule — directly closing §1.3.

### 3.5 Fixing the Swarm idle-vs-unknown gap (§1.4)

Adjacent to AMC proper but part of the same "don't lose the binding" mandate
the user named explicitly: `phaseToDisplayStatus`'s
`if (!phaseAccessor) return "idle"` should distinguish "confirmed idle" from
"no registry entry for this block in this renderer" (return e.g.
`"unknown"`/`"untracked"` and render it distinctly, not as a false idle). The
deeper fix is widening `agentActivity.ts`'s registry so a tracked block reports
real phase regardless of which window/workspace renders it, rather than only
those with a currently-mounted pane component — but that is a larger, separate
change; the minimal fix (stop conflating "unknown" with "idle") should ship
regardless.

---

## 4. Migration plan

1. Build the AMC gateway (`agentmux-srv/src/ambient/`) with the primitives in
   §3.2, initially supporting exactly one purpose: `activity_summary`.
2. Port `register_session_activity_summary` / `invoke_cli_for_activity`
   (`session.rs:109-267`) to call through the gateway instead of spawning the
   CLI directly. Delete the direct-spawn code once ported.
3. Add per-entity generation to blocks (bumped on new-turn-start — likely
   already adjacent to wherever `activeTurnId` is set today, per
   `useAgentActivitySummary.ts:59`).
4. Split `term:activity` → `term:osc_title` / `term:ambient_summary` (§3.4);
   update `agent-model.ts:viewText()` and `swarm-model.ts` precedence.
5. Wire ambient token usage into `token-usage.ts` as a distinct bucket;
   surface an ambient-vs-user breakdown in `TokenUsageIndicator.tsx`.
6. Fix `phaseToDisplayStatus`'s idle/unknown conflation (§3.5), independently
   shippable.
7. Any future ambient call (pane-title generation for a *new* purpose,
   speculative next-step suggestions, etc.) is required to go through the
   gateway from day one — no direct subprocess spawns for augmentation
   purposes anywhere else in the codebase.

## 5. Non-goals

- Not proposing a general-purpose LLM gateway for **user-driven** turns — that
  pipeline (`useAgentStream.ts`) already has its own accounting and lifecycle;
  AMC is scoped to non-user-initiated calls only.
- Not solving cross-window/cross-workspace activity registry visibility in
  this spec (§3.5's deeper fix) — flagged as a follow-up, not blocking.
- Not picking a different model than Haiku for ambient calls — out of scope;
  the framework is model-agnostic (the `model` field in §3.3's sketch), Haiku
  simply remains the default today.

## 6. Open questions

- Should `SupersedePolicy` (cancel vs. dedup vs. queue) be a per-purpose
  constant, or configurable at the call site? Leaning per-purpose constant to
  keep the gateway's behavior predictable and auditable.
- Where should the per-entity generation counter live — colocated with
  existing block/turn state (`activeTurnId`) or a new dedicated store? Leaning
  colocated, to avoid a second source of truth for "which turn is this."
- Rate limiting / global ambient-call budget (e.g. cap ambient calls per
  minute across the whole app) is mentioned as desirable by the user's framing
  ("calls fire too often") but not designed in detail here — likely a simple
  token-bucket in front of the gateway, deferred to implementation.
