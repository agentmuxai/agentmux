# Spec: Hidden memory reinjection after compaction

**Status:** proposed
**Date:** 2026-09-22
**Author:** AgentA (agent, `~/.agentmux/agents/agenta-07017`), per direct request from the human operator.
**Related:** `SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md` (the prior
recommendation against this feature, overridden — see §1.1),
`SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` (the trigger this
spec hooks into), `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md`
(the one existing design for `agentmux-cloud` sharing — see §3.5 for how
this spec stays compatible with it without building any part of it).

**Motivating ask (repo owner, live session, verbatim):**

> we want to be able to insert text to the agent without showing it to the
> user. Specially, the global/personal memory needs to be reinjected after
> every compression. when injecting, put just a label, but essentially the
> agent has to read it all. this can be testing via the agent compact button
> (for the user) we also want the token count of each.

This is three features, each built on top of the last:

1. **A general-purpose hidden-injection primitive** — send real text into a
   running agent's conversation, delivered to the model in full, shown to the
   human operator as a label only.
2. **One concrete use of it** — reinject Global Memory + Personal (native)
   memory content after every compaction, so both survive as something the
   agent actually re-reads, not just something structurally still present.
3. **Large-memory handling** — added 2026-09-22, same session: report the
   total memory size at every injection, and when it's grown large — the ask
   specifically calls out Personal memory as the usual case — surface a
   smooth, graduated warning with a real, actionable suggestion: compress it,
   or hand off the next segment of work to another agent rather than let this
   agent's memory keep growing unbounded. See §3.4.

## 1. Relationship to existing specs — read this before building

This is not a green-field ask. Three existing documents bear directly on it,
and one of them reached the **opposite** conclusion for reason #2. Both need
to be reconciled explicitly, not silently overridden.

### 1.1 `SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md` recommended NOT building this

That spec's §4.5 evaluated exactly "re-inject a memory digest after
compaction" and recommended **dropping it** (option 1 of 3), on empirical
grounds: `CLAUDE.md`/the composed memory file occurs **zero times** in a real
Claude Code transcript, before or after a compaction boundary — because it is
delivered in the **system prompt**, which is rebuilt fresh on every API
request, not in the message list compaction rewrites. Its own words: *"a
digest re-stating the memory index would re-deliver something the model
already has in its system prompt — paying tokens and a visible turn per
compaction for no gain."*

**Why this ask is different, not a re-litigation of the same question:**
that spec's objection is to redundancy — sending content the model already
structurally has. This ask's premise is different: *availability in the
system prompt is not the same as the model actually attending to it*,
especially in the disoriented moment right after a large context cut. A
system-prompt line the model can look up if it chooses to is a weaker
guarantee than a message it must process as part of producing its next
turn. This spec is about forcing the read, not about restoring lost bytes.

**Decided, 2026-09-22, after this exact tradeoff was put to the repo
owner directly:** proceed regardless of the token cost the prior spec
weighed against. Verbatim: *"we have no choice, the memory is critical."*
Recorded here because the prior spec's recommendation is a real, written,
reasoned document — overriding it silently would be worse than overriding
it on the record. This resolves what was Open Question 5 (§7).

That distinction should be stated explicitly to whoever reviews this — the
prior spec's empirical findings (§2.1–§2.2 there) are not being contested,
its conclusion about whether a digest is *worth building* is.

**Also directly relevant, and in tension:** §5 of that spec lists as an
explicit **non-goal**: *"Not injecting memory file bodies anywhere. Index
only."* This spec's design (§4 below) does the opposite — full bodies, not
just the index — because "put just a label, but essentially the agent has to
read it all" is a direct requirement for full content, not an index. This is
a deliberate revision of that non-goal for the reinjection case specifically,
not an oversight. It does not have to apply to `buildStartupPayload`'s
separate, still-open §4.3 (Memory section at spawn) — that can stay
index-only independently.

### 1.2 `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` supplies the trigger, already built

The mechanism this spec's own §4.5 conclusion (before recommending against
building it) already settled: fire on the real `compact_boundary` frame —
`AgentEvent::CompactionBoundary` — via the same `handleSendMessage(...)` path
`buildStartupPayload` already uses (`agent-view.tsx:2075-2087`). **Not**
`compaction_started` (too early — races reducer reconciliation, per
`SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md`), and **not**
the ≥50%-drop heuristic (non-Claude only, not a real signal). Trigger and
detection plumbing for this already ships; nothing new needed there.

The manual Compact button (`AgentComposerStrip.tsx`'s `onCompact`) sends the
literal text `/compact` through the normal `handleSendMessage` path, which
Claude Code's persistent stream-json stdin protocol recognizes as its real
`/compact` command — verified empirically against a live process
(`docs/reports/REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md`
§6). This is **exactly** the "agent compact button" the ask names as the test
vector, and it emits the same real `compact_boundary` frame an automatic
compaction does — so testing via the button exercises the identical code
path this spec hooks into, not a separate manual-only branch.

### 1.3 `REPORT_AGENT_PANE_SYNTHESIZED_TEXT_AUDIT_2026_08_06.md` — the precedent this spec must not violate, and why it doesn't

Commit `6191a1928` deliberately removed a different synthesized-text
mechanism, stating: *"no artificial messages mixed into transcript content,
even well-intentioned explanatory ones."* Read narrowly this looks like a
blocker. It is not, because that precedent's objection was to *visible*
synthetic text presented as if it were real conversation. This spec's design
(§3) keeps the real, full content **out of the visible transcript** — only a
label renders. The underlying message genuinely sent to the CLI is a new
category the audit's taxonomy doesn't yet have a name for (its §7 covers only
things AgentMux shows, not things AgentMux sends-but-hides) — flagged here so
whoever reviews this treats it as a deliberate, novel category rather than
assuming either precedent (removed-for-being-visible, or already-covered)
applies directly.

## 2. What's confirmed vs. assumed, from direct code reading (2026-09-22, updated same day after a second verification pass)

**Confirmed:**
- No existing AgentMux mechanism sends text to a running agent's conversation
  without it becoming a normal, visible `user_message` transcript node via
  today's *default* path. Checked `handleSendMessage` →
  `useAgentCommands.ts`'s `sendMessage` → dispatches `TurnStart` with the
  message as `content` — the standard, fully-visible send path.
  `postSystemNotification`/`StreamFlush` (the other injection path the
  synthesized-text audit catalogs) is equally visible. **A hiding mechanism
  has to be built new; nothing to reuse as-is.** (Narrowed 2026-09-22, second
  pass: this does NOT mean a backend change is required — see the
  now-resolved "backend RPC" item below. The gap is a frontend dispatch
  choice, not a missing server capability.)
- The correct function name is `estimateTokenCount`, not `estimateTokens`
  (this doc's own earlier draft got the name wrong) —
  `frontend/util/format-count.ts:63` — `estimateTokenCount(text) =
  Math.ceil(text.length / 4)` — the repo's existing chars÷4 estimate
  convention, labeled `(est.)` wherever it's shown
  (`SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md`,
  `SPEC_PER_NODE_TOKEN_ACCOUNTING_2026_08_03.md` §6.2). This spec reuses the
  same function and the same labeling convention rather than introducing a
  second estimate formula — real Claude-only per-round token accounting
  (`SPEC_PER_NODE_TOKEN_ACCOUNTING_2026_08_03.md`) doesn't apply here: that
  derivation needs two consecutive `message_start` frames straddling nothing
  else, and a reinjection turn happening immediately after a compaction is
  exactly the "compaction interference" case that spec's §3.2 says must be
  treated as unavailable.
- Global Memory: `GlobalMemoryWrite`/`Read`/`List`/`Remove`, shared,
  workspace-wide, composed into a provider's native startup file at agent
  launch (`agentmux-srv/src/server/app_api/memory.rs`; composition path per
  `SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md` §1, "System A").
  Personal/native memory: `MemoryWrite`/`Read`/`List`, per-agent, Claude
  Code's own memory directory (`db_agent_native_memory`; "System B" in the
  same spec). Both are readable server-side independent of what any running
  CLI process currently has loaded — this spec's design does not depend on
  a provider's system prompt being fresh (see §4.6 of the carry-over spec,
  "mid-session propagation... unresolved" — this design sidesteps that
  problem entirely by pushing current content as a new real turn rather than
  relying on the system prompt picking up a file change).
- Compaction detection, and the `CompactionBoundary`/`compact_boundary`
  event's exact shape (`trigger`, `preTokens`, `postTokens`, `durationMs`),
  is Claude-only by construction (`translator/claude.rs` is the only
  translator handling `system`/`compact_boundary` frames). **This spec is
  Claude-only**, same scope as everything it builds on.
- **NEW, resolved 2026-09-22: the backend RPC needs zero changes.** Traced
  `run_agent_turn` (`agentmux-srv/src/server/agent_handlers/input.rs:263`) —
  it takes an arbitrary `message: String` and delivers it to the CLI's
  stdin; it has no opinion about what UI treatment the frontend gives the
  resulting turn. The visible `user_message` transcript node is created
  **client-side**: `useAgentCommands.ts`'s `sendMessage` appends to a
  pending zone, and the backend's `agent-message-accepted` echo is what
  promotes that pending entry into a real document node (see the existing
  comment on `messageId` there: *"the acceptance event promotes it"*).
  **Message delivery and transcript-node creation are already decoupled in
  this architecture.** Hiding is therefore a purely frontend concern: a
  reinjection call still calls the same underlying send RPC with the full
  `<system-reminder>`-wrapped content, but dispatches a new `MemoryReinjected`
  reducer command (§3.3) instead of the normal `TurnStart` + pending-zone
  path, so a `MemoryReinjectionNode` (§3.2) is pushed directly instead of a
  `UserMessageNode` ever being created. This closes what was previously an
  open "not traced into the Rust RPC handler" gap.
- **NEW, resolved 2026-09-22: the idle-vs-queued timing question needs no
  new logic at all** — reusing `handleSendMessage` as-is (§3.3) already
  handles both cases correctly, for a reason grounded in existing, already-
  shipped code rather than assumed:
  - **Manual `/compact`** never produces a `result` frame — confirmed
    empirically against a live process in
    `REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md` §6 (the
    CLI emits `system/status` → `compact_boundary` → a fresh `system/init` →
    a synthetic continuation message; never a `result`). Since
    `AgentEvent::Done`/`TurnEnd` only ever comes from a `result` frame
    (`translator/claude.rs`), a naive implementation would leave the pane
    stuck "Working" forever after every manual compact. **AgentMux already
    has a fix for exactly this** (codex P1, PR #2659):
    `reducer.ts`'s `CompactionBoundary` case (`:1467-1478`) checks
    `state.pendingCompactTurn` (set on `TurnStart` when the literal message
    content is `/compact`, `reducer.ts:637`) and, if the turn is still
    "working," synchronously sets `turnPhase = { kind: "Done", ... }` as
    part of processing the very same `CompactionBoundary` command. By the
    time a reinjection call reads pane state afterward, `workingFromPhase`
    (`types.ts:472-475`, true only for `Submitting`/`Streaming`/
    `Interrupting`) already correctly reads `false` for `Done`.
  - **Auto-triggered compaction** happens mid a real, ongoing turn that
    continues afterward with its own eventual `result`/`Done` —
    `pendingCompactTurn` is never set for it (only literal `/compact`
    content sets it), so `turnPhase` correctly stays `Streaming` at the
    moment `CompactionBoundary` lands, and `handleSendMessage`'s existing
    `wasAlreadyWorking` queued-while-busy path (already built, used by every
    ordinary message sent while a turn is running) handles it with no new
    code.

  Net: reuse `handleSendMessage` unmodified for both cases — no special
  timing logic needed, because this problem was already solved generically
  by the existing send path for a structurally identical case.

**Still genuinely open after this pass (real unknowns, not just untraced code):**
- ~~The hidden-response suppression gap~~ — **built 2026-09-22, fourth
  pass.** See §3.3's resolution write-up for what actually shipped (a queue
  wrapper, not the two-phase reducer-command sketch originally proposed
  here) and exactly what remains unverified (no live-instance check; the
  interrupt-mid-hidden-turn path is untested).
- ~~Whether the `MemoryReinjected` reducer command should be dispatched
  synchronously~~ — moot: no `MemoryReinjected` reducer command was built.
  `trigger()`/`onSessionEnd()` are called directly from
  `useAgentStream.ts`'s existing `CompactionBoundary`/`session_end`
  handling — see §3.3.
- Everything in §3.4.4 and §7 — the size-band threshold's exact fraction, the
  auto-fire-vs-confirm question for `WorkEnqueue`, and the cloud-sharing
  scope question — remain real product/design decisions, not things further
  code reading resolves.
- **New, added with the implementation**: no live-instance verification.
  Every claim about correctness in §3.3 rests on unit tests against
  injected fakes plus a clean full-suite run, not an observed real
  compaction. The interrupt-mid-hidden-turn case specifically is untested.

## 3. Design

### 3.1 Content composition

On a real `CompactionBoundary` event (§1.2), fire a server-side (or
frontend-orchestrated — an implementation choice, not fixed by this spec)
fetch of:

- Every Global Memory entry's **full body**, not the index-only shape
  `buildStartupPayload`'s (still unimplemented) §4.3 Memory section uses —
  per §1.1's explicit revision of that non-goal for this case.
- Every Personal (native) memory file's **full body**, same treatment.

Composed into one message, structured so a human who somehow does see the
raw text (dev tools, a raw transcript dump) can still make sense of it:

```
<system-reminder>
Your memory was reinjected after a context compaction. Below is your
complete Global Memory and Personal Memory content — read all of it now,
not just the index, since your recent working context was just summarized.

# Global Memory (N entries)
<full body of entry 1>
---
<full body of entry 2>
...

# Personal Memory (M entries)
<full body of entry 1>
---
...
</system-reminder>
```

The `<system-reminder>` wrapper matches Claude's own documented, trained
convention for background-context-not-a-user-instruction content — the same
framing this very class of message already uses for things like memory
recall elsewhere in the product. Using the same convention rather than
inventing a new phrasing gives the model a framing it already reliably
treats as "read, don't reply to as if a person said it."

**Suppression:** if both sources are empty, do not send anything — matches
the carry-over spec's own stated suppression rule for its Tier 4.5 sketch,
and avoids a pointless turn (and its token cost) when there is nothing to
reinject.

**Dedup:** exactly one reinjection per real `compact_boundary`, keyed the
same way `compact-boundary.ts`'s `contextCompactedNodeId` already dedups —
by the frame's own `timestamp` field, not a client-generated one, so a live
delivery and a later history-replay overlap of the same boundary can't fire
this twice. `compaction_started` (the earlier, live-only, `persist:0` ping)
must **not** be the trigger — confirmed unsuitable in §1.2.

### 3.2 Hiding mechanism — new document-node category

A new node type, e.g. `memory_reinjection`, alongside `compaction_started`/
`context_compacted` in `types.ts`. Unlike those two (which render real
backend data as prose), this node's payload is deliberately minimal — no
content field carrying the injected text, so there is nothing to
accidentally render:

```ts
interface MemoryReinjectionNode {
    type: "memory_reinjection";
    id: string;                    // `memory-reinjected-${frameTimestamp}`
    globalMemoryCount: number;
    personalMemoryCount: number;
    estimatedTokens: number;       // sum of estimateTokenCount() over every entry sent
    perEntryTokens: Array<{ label: string; source: "global" | "personal"; tokens: number; sizeBytes: number }>;
    totalSizeBytes: { global: number; personal: number };  // real on-disk size, §3.4 — not an estimate, unlike estimatedTokens
    // §3.4.2 (revised): computed at build time from Personal's own token
    // total against `contextWindow * fraction` — stored precomputed
    // (not recomputed at render time) so a replayed node shows the band
    // that applied when it happened, not one recomputed against today's
    // pane config.
    sizeBand: "low" | "mid" | "high" | "critical";
    at: number;                    // epoch ms, same fallback rules as contextCompactedLiveTimestamp
}
```

`DocumentRow.tsx` renders this as a single compact label row (matching the
"just a label" requirement) — something like:

> 🧠 Memory reinjected — 4 global, 6 personal (≈2,340 tokens)

with the `perEntryTokens` breakdown available on hover/click, mirroring the
existing tooltip pattern `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md`
already established for tool-call token figures — reusing a UI pattern the
user already knows rather than inventing a second one. **Never** renders the
actual memory body text anywhere in this node — the full content only ever
exists in the real message sent to the CLI, never in `documentAtom`'s node
data for this node type. This is the actual hiding mechanism: not a CSS
`display: none` on real content (which would still ship the content to the
client and risk a future rendering bug exposing it), but the content simply
never being present in the node the frontend stores or replays.

**History replay — implemented 2026-09-22, fifth pass, and it turned out to
be a real, separate leak from the one §3.3 already fixed:** the live queue
wrapper only covers the LIVE session. `parseHistoryLines.ts` re-derives
nodes from the raw on-disk provider transcript independently — and that
transcript genuinely contains the hidden turn's real content (delivery is a
real message, per §3.3), so reopening a pane after this session ended would
have rendered the full reinjection text as an ordinary visible message, with
no live-only mechanism protecting it.

Fixed at the actual right layer once traced: `parseHistoryLines.ts` and
`useAgentStream.ts` both construct their `ClaudeCodeStreamParser` from the
SAME shared class (`stream-parser.ts`, `isReplay: true` vs. default
`false`), and that class already had exactly the right precedent for this
shape of problem — `tryParseJekt`, which recognizes a special marker
occupying a whole user-message payload and returns a different node type
instead of the plain `UserMessageNode` it would otherwise become. Added the
mirror, `tryParseMemoryReinjection`, plus one new stateful field
(`hidingUntilNextUserMessage`) that suppresses every OTHER event type
(text/thinking/tool_call/tool_result/agent_message/error_result) between a
recognized reinjection's own user-message event and the next real one — the
closest a stateless-per-line replay parse can get to mirroring the live
queue wrapper's behavior, covering the assistant's reply on replay too, not
just the outgoing side.

Recognition/reconstruction itself lives in `memory-reinjection.ts`
(`isMemoryReinjectionMessage`, `parseReinjectionMessage`,
`buildMemoryReinjectionNodeFromReplay`) — shared with `stream-parser.ts`
for the same "two consumers can't independently drift" reason
`compact-boundary.ts` was originally extracted for. Two things are
genuinely NOT recoverable from the replayed message text alone, and this
degrades honestly rather than fabricating them: per-entry labels
(`composeReinjectionMessage` never wrote them into the message, only raw
bodies) become synthetic `"Entry N"` placeholders, and `sizeBytes` falls
back to the recovered text chunk's own length rather than the original
real on-disk size. `sizeBand` also falls back to `FALLBACK_CONTEXT_WINDOW`
(shared with the live send path's own fallback) since the pane's actual
context window isn't reliably known at a stateless replay-parse call site —
the replayed band may not exactly match what was shown live. All flagged
in code comments, not silently assumed.

Also confirms the auditability property this section originally
speculated about: the full content genuinely is in the raw transcript file
on disk (`~/.claude/projects/.../*.jsonl`) — someone with direct filesystem
access can still see exactly what was sent, which is real and intentional,
not a leak this fix is trying to close. What this fix closes is AgentMux's
own UI re-surfacing that content as an ordinary visible message on reopen.

### 3.3 Delivery — reusing the real send path, not inventing a raw-stdin bypass

Send through the exact same path `buildStartupPayload` already uses
(`handleSendMessage`, confirmed real and working) rather than a new raw
stdin-write mechanism — deliberately, for two reasons: it is the only send
path already proven to reach a live Claude Code process correctly (`/compact`
itself goes through it, per §1.2), and reusing it means this reinjection
correctly participates in the existing queued-while-busy machinery instead
of needing its own.

The one addition needed on this path — **confirmed frontend-only, §2** — is a
way to suppress the normal `TurnStart`/visible-`user_message` side effect for
this specific call, and push a `memory_reinjection` node (§3.2) instead of
letting the normal document-node creation run. No backend/Rust change is
required: `run_agent_turn` (`input.rs:263`) already just forwards an
arbitrary message string to the CLI, agnostic to how the frontend chooses to
represent the resulting turn. The cleanest shape, matching how
`CompactionBoundary`/`CompactionStarted` already have dedicated reducer
commands rather than overloading `TurnStart`: a new reducer command
(`MemoryReinjected`) that the reinjection call site dispatches instead of the
normal `TurnStart` + implicit node creation `handleSendMessage` triggers for
an ordinary message — still calling the same underlying send RPC underneath
with the full `<system-reminder>`-wrapped content, just skipping the
pending-zone/promotion path that would otherwise create a visible
`UserMessageNode`.

**Newly discovered, 2026-09-22, third pass (implementation attempt surfaced
this — not caught by code reading alone):** suppressing the outgoing message
is only half the problem. This is a genuine CLI turn — the model produces a
real response to it, same as any other message. If only the *outgoing* side
is hidden, that response streams through the normal pipeline unmodified and
renders as an ordinary, visible `AgentMessageNode` — a visible assistant
reply to an invisible user message. Two failure modes, either one bad
enough to block on: it can leak information about what was reinjected (the
model may reference specific memory content in acknowledging it), or at
minimum it reads as a non-sequitur assistant turn with no visible prompt.

**Resolution — implemented 2026-09-22, same day, fourth pass. Simpler than
the two-phase-reducer-command sketch originally proposed here, and worth
recording exactly why:**

Rather than a `MemoryReinjectionStarted`/`MemoryReinjectionBoundary` pair of
NEW reducer commands, the actual implementation reuses the REAL `TurnStart`/
`TurnEnd` completely unmodified for turn-state bookkeeping (§2's own
finding — a hidden reinjection is a genuinely real turn state-machine-wise,
so `workingFromPhase`/queued-while-busy already works with zero new
reducer surface) and solves the rendering-suppression half at a different,
better layer: `stream-flush-queue.ts`'s own module doc comment establishes
that `StreamFlushQueue` is ALREADY the single mandatory choke point every
document-write producer in `useAgentStream.ts` goes through — *"give every
new producer a pushXxx method here instead of scheduling its own flush."*
That existing discipline means hiding can be ONE wrapper at that one
interface (`hiding-stream-flush-queue.ts`'s `createHidingStreamFlushQueue`)
instead of a pane-level flag checked at every individual node-creation
branch — safer by construction (a wrapped interface can't be partially
bypassed the way a scattered `if (hiding) return` could be by a missed
branch), and needed zero changes to `useToolChunkStream`/`useShellNodeStream`/
`useCompactionStream`/`useTurnLifecycle`/`usePendingMessageAcceptance` even
though all five push through the same queue — they simply received the
already-wrapped instance.

Built as three files, TDD throughout (22 total new tests across the three,
plus the wiring itself passing a full `npx tsc --noEmit` and the complete
`frontend/app/view/agent/` + `frontend/app/store/` suite — 182 files, 2835
tests, zero regressions):

- `hiding-stream-flush-queue.ts` — the wrapper itself. Only the six PUSH
  methods are gated; read-only/lifecycle methods always delegate (nothing
  was ever pushed while hiding, so they safely no-op on the real queue's
  empty pending arrays).
- `memory-reinjection-controller.ts` — orchestrates trigger → fetch → send
  → hide → `onSessionEnd()` → label, with ZERO Solid/RPC dependencies of
  its own (every side effect injected), so the actual logic is unit-tested
  without a live app. Delivery bypasses the pending-zone entirely by
  calling the raw send RPC directly (not the full `sendMessage` path) —
  the other half of "hidden," since that path is what creates the visible
  `UserMessageNode` this feature must never produce.
- `memory-reinjection-fetch.ts` — the real `BundleApi`/`NativeMemoryApi`
  glue (thin, verified by direct reading, not unit-tested — it's pass-
  through, not logic).

Wired into `useAgentStream.ts` at exactly three points: the queue is wrapped
once at creation (every downstream producer benefits for free), `trigger()`
is called right after the existing `CompactionBoundary` dispatch, and
`onSessionEnd()` is checked at the existing `session_end` handling —
ordered deliberately BEFORE `queue.pushNewNode`, since clearing the hiding
flag as `onSessionEnd`'s own side effect is what lets that same push then
pass through the (now-unhidden) wrapped queue.

**What's still open, honestly:** this closes the RENDERING leak (§3.3's
main concern) with real, passing test coverage for the controller's own
success/failure/re-entrancy logic. It has NOT been verified against a live
running instance — no part of this session had access to one. The
interrupt-mid-hidden-turn case (user presses Esc while a hidden reinjection
is in flight) is untested; `TurnEnd`'s existing outcome-selection logic
(`Interrupting` → `"stopped"`) should apply identically since nothing here
touches that path, but "should" is not "verified." Recommended before
shipping: the same empirical check this spec's own §1.2 citation used for
`/compact` itself — run it against a real pane and watch what actually
happens.

### 3.4 Large-memory handling — sizing, a graduated warning, and a real offload suggestion

Added 2026-09-22, same session as the rest of this spec, per direct follow-up
from the repo owner: *"we'll need to think about large sizes of memory...
this applies more to personal memory. we want to know the total size of the
memory at the injection and a suggestion about how to compress or move to
other agents."*

**Why Personal memory specifically, not Global:** Global Memory is
human-curated (written via `GlobalMemoryWrite`, by a person or an agent
acting on explicit instruction) and empirically small — the carry-over
spec's §2.5 measured every agent on this machine at **zero** global-brain
rows. Personal/native memory is agent-authored, accumulates automatically
over a long-running agent's life with no natural ceiling, and is exactly
the thing `MemoryWrite`'s own doc comment already anticipates growing
without bound — the "oversized file" handling already built into
`agent_native_memory_upsert` (§3.4.1 below) exists for precisely this
reason. The ask's framing — "applies more to personal memory" — matches
what's already true structurally, not just anecdotally.

#### 3.4.1 Total size — already real for Personal memory, net-new for Global

`db_agent_native_memory`'s `NativeMemoryMirrorRow` already carries
`size_bytes` per file — the file's **real on-disk size**, deliberately not
derived from `content.len()` so an oversized file's mirror row still matches
its live size instead of forcing a full re-read on every list
(`agent_native_memory.rs:136-143`, with a dedicated regression test at
`:235` guarding exactly this). **Personal memory's total size at injection
is `agent_native_memory_list_meta(...).map(|r| r.size_bytes).sum()` — no new
instrumentation needed, only a new call site.**

Global Memory has no equivalent `size_bytes` column today (checked
`agentmux-srv/src/server/app_api/memory.rs` — no `size` field on its
read/list path). Computing its total means summing `content.len()` over
every entry at injection time — cheap (Global Memory is small, per above),
but real new code, unlike Personal memory's free reuse.

Both totals land in `MemoryReinjectionNode.totalSizeBytes` (§3.2) as **real
byte counts**, distinct from `estimatedTokens` (§4's chars÷4 estimate) —
the ask asks for both ("total size" and "token count of each"), and they
answer different questions: bytes is an exact, storage-level fact; tokens is
always an estimate here, labeled as such per §2's existing-convention reuse.

**Real data, measured 2026-09-22 against every `identity-store.db` on this
machine** (same empirical-not-guessed methodology the carry-over spec's §2
used for Global Memory): this agent's own Personal memory is 22 files
totaling **61,725 bytes**; the largest single channel measured on this
machine is **76,164 bytes**. Small in absolute terms today — directly
informs §3.4.2's threshold choice below, and is worth re-measuring
periodically as real usage grows rather than treated as a one-time
constant.

#### 3.4.2 A graduated warning, not a hard cap — resolves §7 open question #2

§6/§7 of the original draft left "is there a size cap" as an open question,
torn between "has to read it all" and an unbounded message. This follow-up
resolves it: **no cap, no truncation** — capping would silently violate "the
agent has to read it all," the same objection that already ruled out
`buildStartupPayload`'s "…and N more" Peer-Agents pattern for this case. In
its place, a **smooth escalating signal to the human**, reusing a pattern
this exact codebase already ships and users already recognize rather than
inventing a new one: `AgentComposerStrip.tsx`'s `ctxBand()` for context-fill
color banding — `low` / `mid` / `high` / `critical`, `critical` getting a
bold weight + `⚠` glyph on top of the existing pulsing-red color, deliberately
*not* a new standalone banner component (that file's own header comment
documents why: its tiered-wrap layout is already hand-tuned/fragile, and a
mounted row would need its own container-query wiring). This spec reuses
that exact banding scheme and that exact "strengthen the inline label,
don't add new chrome" decision, rather than re-deciding a UI question this
codebase already made once:

```ts
// Mirrors ctxBand()'s own thresholds AND its own denominator CHOICE
// (AgentComposerStrip.tsx:550-556) — same fractions, and (revised
// 2026-09-22, second pass) the same "band against a real per-pane
// reference point" philosophy, not an arbitrary absolute byte number.
// ctxBand bands the conversation's real token count against the model's
// OWN compaction threshold (itself a function of contextWindow); this
// bands Personal memory's ESTIMATED TOKEN total against a fraction of that
// SAME pane's contextWindow — so the same absolute memory size reads
// differently on a 32K-context pane than a 200K-context one, which a fixed
// byte threshold could not do. See the surrounding prose for why bytes
// alone were rejected and for the real measured data behind the 0.10
// default.
function memorySizeBand(personalEstimatedTokens: number, contextWindow: number, fraction = 0.10): SizeBand {
    const healthyThreshold = contextWindow * fraction;
    const f = personalEstimatedTokens / healthyThreshold;
    if (f >= 0.9) return "critical";
    if (f >= 0.75) return "high";
    if (f >= 0.5) return "mid";
    return "low";
}
```

**Why tokens-against-context-window, not bytes-against-a-fixed-number
(revised from the original draft, same day, second pass):** the original
draft banded raw Personal-memory bytes against a flat "healthy" byte count,
left as an unresolved open question because — unlike a model's context
window — no natural reference point exists for "how big is too big" in
absolute bytes. Reusing `ctxBand()`'s actual *denominator choice*, not just
its band fractions, removes that problem: there already is a natural
reference point, the pane's own `contextWindow`, and it's already computed
and threaded through `AgentComposerStrip.tsx` today. A `0.10` (10%) default
is proposed, grounded in the real measurement above: this agent's own
61,725-byte Personal memory is roughly 15,400 estimated tokens (`61,725 ÷
4`) — under 5% of a 32K-context model's window already, but over 50% of
a small 8K-context provider config, illustrating exactly why a
window-relative fraction reads correctly across model sizes where a fixed
byte number cannot. `0.10` is a proposed starting default, not empirically
tuned against real "this felt too large" user reports — flagged as
revisable in §3.4.4.

Banded on **Personal memory alone**, not the combined total (§ intro,
above) — a large Global Memory total would need its own separate signal if
it ever became real, but per §2.5's measurement it isn't today, and
conflating the two would blur exactly the distinction the ask draws.

`DocumentRow.tsx`'s `memory_reinjection` label (§3.2) carries the band
visually the same way the composer strip's `ctx` text already does — plain
at `low`/`mid`, a `high`-band color shift, `critical` gaining the bold+⚠
treatment. Below `mid`, no extra text — matches Tier 3's own "silent below
that — not worth calling out yet" rule
(`SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` §7 Tier 3), applied
here instead of invented fresh.

#### 3.4.3 The suggestion — two concrete actions, not a vague nudge

At `high` band and above, the label's expanded state (§7 open question 4 —
this is the case that motivates allowing one) surfaces two real actions,
each backed by an existing AgentMux primitive rather than a new one:

1. **Compress.** A worded nudge, not an automated rewrite — automatically
   summarizing an agent's own memory without review risks silently discarding
   something that mattered, which is a materially worse failure than a
   verbose reminder. The suggestion names concrete existing tools the agent
   already has: `MemoryList` to see what's grown large, `MemoryWrite` to
   consolidate near-duplicate or superseded entries, `MemoryHistory`/
   `MemoryDiff` (already in the agent's own toolset per this session's own
   operator-config notes) to check what a consolidation would discard before
   committing to it. This spec does not design a new compression algorithm —
   it points the agent at the tools it already has, the same "point at real
   primitives" philosophy §3.4.3.2 uses for delegation.
2. **Segment the work to another agent.** Concrete, not hand-wavy: the
   suggestion offers to create a real `WorkItem` via `WorkEnqueue`
   (`agentmux-mcp/src/main.rs:2228` → `work_queue.rs:85-131`) — `payload` set
   to a human/agent-written summary of the next segment of work, `kind` tagged
   so it's filterable, left `target_agent`-empty (any agent may claim it)
   unless the operator names a specific sibling. This reuses `WorkItem`
   exactly as designed — *"the instruction injected into whichever agent
   claims this"* is precisely a work-segment handoff — rather than building a
   parallel, single-purpose delegation mechanism. Whether this action fires
   automatically or only on explicit confirmation is **not decided by this
   spec** (§3.4.4) — given `WorkEnqueue` is itself a real, visible, workspace-
   shared mutation (per its own doc: *"shared, workspace-wide notes/queue,
   not just yours"*), auto-firing it without confirmation is a materially
   bigger step than anything else in this spec and deserves its own explicit
   answer, not a default inherited from the rest of the design.

Both actions are suggestions surfaced to the **human operator** (via the
expanded label, §3.4.2) — not something the agent silently acts on for
itself. An agent unilaterally deciding to enqueue work for another agent, or
to rewrite its own memory, because a size band crossed a threshold, is a
different and much larger design question than this spec's scope (hidden
injection + a label). Keeping the human in the loop for the *action*, while
only the *warning* is automatic, matches the "smooth way to inform the user"
wording in the ask — informing, not autonomously acting.

#### 3.4.4 Open — deliberately not resolved here

- ~~The exact `healthyThreshold`~~ — **narrowed 2026-09-22, second pass**:
  the reference-point problem itself is resolved (§3.4.2 now bands against
  `contextWindow`, same as `ctxBand()`, using real measured data —
  `SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md` §2.5's "measure
  real data, don't guess" methodology, applied here to Personal memory
  instead of Global). What remains open is narrower: whether `0.10` is the
  right fraction, which is a product-feel question ("does this feel like it
  fires too early/late in practice") that only shows up once the feature is
  actually used, not one more code-reading pass would resolve.
- Whether "segment work to another agent" ever fires the `WorkEnqueue` call
  automatically (with undo) versus always requiring an explicit click — see
  §3.4.3 point 2.
- Whether the compress suggestion should eventually become a semi-automated
  flow (agent proposes a consolidated rewrite, human approves before it
  lands) rather than a purely worded nudge — flagged as a natural v2, not
  designed here.

### 3.5 Forward compatibility — `agentmux-cloud` sharing

Added 2026-09-22, per a direct follow-up: *"note that this will be extended
to agentmux-cloud too for sharing, if that helps enlighten architecture."*
Grounded against the one existing design for this, rather than guessed at:
`SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md` — proposed, nothing
built yet, but the transport and conflict model are already designed in
real detail (MuxBus WebSocket wake + authenticated REST pull/push,
per-entry `content_hash`/`parent_version_id` versioning, three-way-merge-or-
conflict-copy on divergence).

**Two asymmetries worth carrying into this spec's design now, so nothing
here has to be reshaped later:**

1. **That sync spec's scope is Global Memory only — explicitly, §2.1 there:
   *"agent-writable Global Memory only... system-tier entries out of scope."*
   Personal/native memory has no cross-instance design anywhere, sync or
   otherwise.** This matters directly for §3.4: this spec treats Personal
   memory as the primary large-size case (§3.4 intro — it's the one that
   grows unboundedly). If "sharing" is meant to reach Personal memory too,
   that is new territory beyond what any existing spec covers, not an
   extension of an already-designed path — flagged here rather than
   assumed either way (see the new open question below).
2. **`WorkEnqueue`'s `db_work_queue` — the delegation primitive §3.4.3
   proposes — is already global across every channel on one machine
   (`work_queue.rs`'s own header comment: *"lives in the always-global
   identity store, never the per-channel one"*), but has no cross-instance
   transport today.** "Hand off to another agent" in §3.4.3 currently means
   a same-machine sibling. A future cross-instance version of that
   suggestion — claiming work posted from a different physical instance —
   would need the same kind of MuxBus-based extension the memory-sync spec
   already sketches for `db_bundles`, applied to `db_work_queue` instead.
   Not designed here; noted so whoever builds §3.4.3 doesn't assume
   same-machine is a permanent constraint when naming the mechanism in UI
   copy.

**What this spec does now, on the strength of this note:** keeps
`MemoryReinjectionNode` (§3.2) and the size-accounting fields (§3.4.1)
already source-tagged (`"global" | "personal"`) and already using the same
real-bytes-vs-estimated-tokens split the cloud-sync spec's own
`content_hash`-based versioning would need to key off of — no reshape
required if Global Memory sync ships later and this spec's total-size
figure needs to start reflecting a synced, multi-instance total rather than
a purely local one. **Does not** attempt to build any part of cross-instance
sync itself — that remains entirely `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_
2026_09_20.md`'s scope, itself gated on its own §4 open questions and
`SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md`'s unbuilt history/diff/
revert read side.

New open question, added to §7: does "sharing" mean Personal memory gets a
cross-instance design of its own (net-new, nothing exists today), or does it
mean this feature's *suggestions* (§3.4.3) become cloud-aware once Global
Memory sync ships, with Personal memory staying local? Not decided here.

## 4. Token accounting — "the token count of each"

Interpreted as: **per-memory-entry token count**, not just a session total —
i.e. the `perEntryTokens` breakdown in §3.2's node shape, one line per
Global Memory entry and per Personal Memory file, each computed via the
existing `estimateTokens()` chars÷4 estimate, labeled `(est.)` in the UI
exactly as the hover-peek spec's convention already does elsewhere.

Also worth surfacing, not just per-injection: a running total across a
session of how many tokens this feature has spent reinjecting memory,
displayed in the session-stats popover (`AgentSessionStats.tsx`) alongside
existing cost/token totals — this is the same class of concern
`docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md` already tracks for this
codebase, and a feature that reinjects potentially-large content on every
compaction is exactly the kind of thing that analysis line of work would
want visibility into. Flagged as a should-have, not blocking v1.

## 5. Testing plan

1. **Unit**: `estimateTokens`-based `perEntryTokens` computation — pure
   function, given a fixed set of memory entries, produces the expected
   per-entry and total figures. Matches the existing test style for
   `compact-boundary.ts`'s pure parsing functions.
2. **Unit**: the new shared parsing module (§3.2) reconstructs an identical
   `MemoryReinjectionNode` from a live frame and from a replayed history
   line — same dual-path regression pattern `compact-boundary.test.ts`
   already uses for `compact_boundary` itself.
3. **Manual, via the ask's own named test vector**: open a pane with at
   least one Global Memory entry and one Personal Memory entry present,
   click the Compact button (`onCompact`), let `/compact` run to completion,
   and confirm:
   - Exactly one `memory_reinjection` label row appears, not the full memory
     text.
   - The transcript, scrolled past, shows no way to distinguish this from
     any other label row — no leaked content.
   - Reopening the pane (history replay) still shows the same label, not the
     full content and not a missing node.
   - The underlying `.jsonl` transcript file (direct filesystem read,
     bypassing AgentMux's UI entirely) *does* contain the full injected
     content — confirms delivery actually reached the model, not just that
     the UI correctly hid something that was never sent.
4. **Suppression case**: a pane with zero Global and zero Personal memory
   entries — compact, confirm no `memory_reinjection` node appears at all.
5. **Dedup case**: force the same `compact_boundary` frame to be seen twice
   (live + a subsequent history-range overlap, mirroring how
   `contextCompactedNodeId`'s dedup is tested) — confirm only one
   reinjection fires.
6. **Unit — `memorySizeBand()` (§3.4.2)**: `frontend/app/view/agent/
   memory-reinjection.ts`/`.test.ts` carries a first pass of this today
   (22 tests, TDD), banded on raw Personal-memory bytes — pending an update
   to the token/contextWindow-relative signature this section now specifies
   (§3.4.2 revision, same day). Once updated: same four-band boundary-value
   style as `ctxBand()` — 0%, 49%, 50%, 74%, 75%, 89%, 90%, 100%, and past
   100% of the threshold, asserting the correct band at each. Banded on
   Personal-memory estimated tokens only — a test with a large Global total
   and small Personal total must still report a low band, confirming
   §3.4.2's "Personal alone, not combined" rule isn't silently reverted to a
   combined sum later.
7. **Manual — `critical` band**: seed enough Personal memory content to
   cross the threshold, compact, and confirm the label shows the bold+⚠
   treatment and both suggested actions (§3.4.3) render. Confirm neither
   action fires anything on its own — `WorkEnqueue` must show zero new items
   until the human explicitly confirms, per §3.4.3's "suggestion, not
   autonomous action" requirement being an actual guarantee, not just prose.

## 6. Risks / tradeoffs — stated plainly, not resolved by this spec

- **Cost**: every compaction now costs additional tokens — both the
  reinjected content itself and the model's processing of it — partially
  offsetting the very token budget compaction just freed. This is the
  concern `SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md` §4.5 raised
  before recommending against building this — **accepted explicitly, 2026-
  09-22**: *"we have no choice, the memory is critical."* Recorded as a
  deliberate, informed override of a written recommendation, not an
  oversight — see §1.1.
- **A real turn with no human behind it.** Unlike every other item in the
  synthesized-text audit's catalog, this sends actual new content into the
  model's context that no human typed and the human explicitly must not see
  — a genuinely new category of AgentMux behavior, not a variation on
  anything shipped today. Worth a second pair of eyes specifically on
  whether "hidden from the user, but real to the model" needs its own
  disclosure/consent story beyond what this spec covers (e.g., should this
  be an opt-out setting rather than always-on?) — treated here as an open
  question (§7), not assumed either way.
- **Large memory sets — resolved 2026-09-22, see §3.4.** Deliberately no
  cap, no truncation — the same "has to read it all" requirement §3.1
  already honors. Unlike `buildStartupPayload`'s Peer-Agents "…and N more"
  pattern, silently dropping memory content is not an acceptable failure
  mode here. In place of a cap: a graduated size-band warning plus two
  concrete, human-confirmed actions (compress, or hand a work segment to
  another agent via `WorkEnqueue`). The underlying cost concern (a large
  hidden message is still a large real API payload, on top of the token cost
  already flagged above) is *surfaced*, not *prevented* — the human decides
  what to do about it, per §3.4.3's "suggestion, not autonomous action"
  design.

## 7. Open questions

1. Should this be an opt-out setting, or always-on for every Claude pane
   with any memory content? Not decided here.
2. ~~Is there a size cap on total reinjected content~~ — **resolved
   2026-09-22: no cap.** See §3.4 for the graduated-warning design that
   replaces it. That section opens its own new unresolved questions (§3.4.4):
   the exact size-band threshold, and whether the delegation suggestion ever
   auto-fires `WorkEnqueue` versus always requiring explicit confirmation.
3. ~~Does the reinjection send need to explicitly wait for the pane to be
   idle~~ — **resolved 2026-09-22, second pass**: no new logic needed.
   Reusing `handleSendMessage` unmodified handles both the manual case
   (`pendingCompactTurn` already forces `turnPhase` to `Done` synchronously
   within the same `CompactionBoundary` reducer case, PR #2659) and the
   auto case (turn stays genuinely `Streaming`, and the existing queued-
   while-busy path already handles that generically). See §2.
4. Should the label be expandable/clickable to reveal the actual content for
   debugging (e.g. for the operator, not hidden from *them* specifically,
   just not shown by default), or strictly label-only with the raw
   `.jsonl` as the only way to inspect it? The ask says "just a label" —
   read here as the default rendering, not necessarily a hard rule against
   ever offering to reveal it.
5. ~~§1.1's cost/redundancy tradeoff — does the repo owner still want this
   built~~ — **resolved 2026-09-22**: yes, explicitly, after the prior
   spec's recommendation was put to them directly. See §1.1.
6. **§3.5** — does eventual `agentmux-cloud` sharing mean Personal memory
   gets its own cross-instance sync design (net-new — nothing like it exists
   for any spec today), or that this feature's *suggestions* become
   cloud-aware once Global Memory sync ships while Personal memory stays
   local? Not decided here.

## 8. Out of scope

- Non-Claude providers (Codex, Gemini, etc.) — no equivalent
  `compact_boundary` signal exists for them (§1.2); this spec doesn't attempt
  a heuristic-triggered version for them.
- `buildStartupPayload`'s own §4.3 Memory section (index-only, at spawn) —
  a separate, still-open item in the carry-over spec, unaffected by this one.
- Mid-session propagation of a memory *edit* into a running agent
  (`SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md` §4.6) — this spec's
  mechanism happens to sidestep that problem for the compaction case
  specifically (§2), but does not solve it generally (e.g. editing memory
  mid-session with no compaction in between still doesn't reach a running
  process).
- A general hidden-injection API usable for purposes other than memory
  reinjection, even though §3 builds one primitive along the way — this spec
  scopes the primitive to what memory reinjection needs (a suppressible
  `TurnStart` + a label-only node type), not a fully general "send hidden
  text for any reason" surface. Worth revisiting as its own spec if a second
  use case appears.
