# Token Accounting & Compaction Control — Current State, Gaps, and a Path Forward

**Date:** 2026-08-18
**Scope:** How AgentMux tracks token usage today, why the numbers look the way they do, whether compaction can be user-triggered, and what a more reliable accounting layer should look like.
**Builds on:** `docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md`, `docs/analysis/TOKEN_TAX_FOLLOWUP_2026_07_04.md` (prior empirical work on the same subsystem — this report doesn't repeat their findings, it extends them toward the accounting/compaction-control questions those docs flagged but didn't resolve).

---

## 0. TL;DR

- **Yes, Claude's API always separates input and output tokens** (plus two more fields — see §1). AgentMux's frontend *receives* all four fields off the wire but **collapses three of them into one number in two separate places** before anything is stored. That's the single biggest fixable gap.
- **"600 down but 100k up" is not a bug — it's the Messages API's stateless design working as intended.** Every turn resends the *entire* conversation as input; only the new reply is output. See §1 for the mechanics and §1.1 for why the *bigger* question is whether that 100k was billed at full price or served from cache.
- **Compaction today is 100% passive.** AgentMux detects and displays a compaction Claude Code's CLI decided to run on its own — it has no way to request one. The existing architecture doc that ruled this out (`SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md`) predates the persistent-controller work that may make it feasible now. See §3.
- **No historical token telemetry exists anywhere.** `docs/analysis/TOKEN_TAX_FOLLOWUP_2026_07_04.md` already flagged this in July; it's still true in August. Everything described below is live-only, in-memory, gone on reload.
- There are currently **two parallel, drifting implementations** of token accounting in this codebase — a collapsed 2-field one driving the UI, and a correct 4-field one (`agentmux-srv/src/agents/types.rs`) that isn't wired to the interactive pane. Reconciling these is the highest-leverage structural fix (§5.1).

---

## 1. How the Claude Messages API actually accounts for tokens

Every response from `POST /v1/messages` carries a `usage` object with **four distinct fields**, not two:

| Field | Meaning | Relative cost |
|---|---|---|
| `input_tokens` | Fresh, uncached prompt tokens processed this request | 1x (full price) |
| `cache_creation_input_tokens` | Tokens newly written into the prompt cache this request | ~1.25x (5min TTL) or ~2x (1hr TTL) |
| `cache_read_input_tokens` | Tokens served from a previously-written cache entry | ~0.1x |
| `output_tokens` | Tokens the model generated this turn | 1x (usually a higher per-token rate than input) |

`input_tokens + cache_creation_input_tokens + cache_read_input_tokens` is the *real* size of everything sent to the model this turn — call that "prompt size." `output_tokens` is unrelated to prompt size; it's just how much the model chose to write back.

### 1.1 Why "600 down but 100,000 up" is completely normal — and what to actually check

The Messages API is **stateless**. There is no server-side conversation object — every single request resends the full transcript (system prompt, tool definitions, every prior user/assistant/tool_result turn) as the prompt, then the model appends one more turn. That means:

- **The "up" number grows monotonically with conversation length**, almost independent of what happened this turn. By turn 50 of a long agentic session, the prompt legitimately *is* ~100k tokens of accumulated history — that's not a leak or a bug, that's the transcript.
- **The "down" number reflects only the newest reply** — often a short tool call, a one-line status update, or a terse answer. 600 output tokens for a single turn is completely ordinary.

So the asymmetry itself is expected and correct. **The question worth asking isn't "why is up so much bigger than down" — it's "was that 100k of up tokens actually expensive, or was it a cache read?"** A 100k-token prompt that's 95k `cache_read_input_tokens` and 5k fresh `input_tokens` costs roughly what a 10k-token prompt would cost at full price. The same 100k number with `cache_read_input_tokens: 0` is genuinely expensive, and if it *should* have been cached, that's a real problem (usually a silent cache invalidator — see the invalidator list in `TOKEN_TAX_FOLLOWUP §4b`: soul/agentmd/memory edits, a skill added/removed, a fresh (non-`--resume`) session, or a changed working directory).

This repo already has direct empirical proof this mechanism works as described: `TOKEN_TAX_FOLLOWUP_2026_07_04.md §2` measured a *first-ever* session showing `cache_read_input_tokens: 44661` against `cache_creation_input_tokens: 6212` — tens of thousands of tokens served from cache before the session had even done anything, because Anthropic's own system-prompt-plus-tools baseline (~27–32K tokens) is cached by content hash and shared account-wide, not per-session.

**Practical takeaway for anyone staring at a "100k up" number in the UI today: there is currently no way to tell, after the fact, whether that was mostly cache reads or mostly full-price fresh tokens.** That distinction is parsed off the wire and then thrown away before it's stored (§2.2). This is the single most actionable finding in this report.

---

## 2. How AgentMux currently tracks token usage

### 2.1 Data flow

AgentMux makes **zero direct calls to the Anthropic API**. It spawns the real `claude` binary as a subprocess (`-p --output-format stream-json`, `agentmux-srv/src/backend/blockcontroller/subprocess/argv.rs:148-154`) and reads its NDJSON stdout — through a persistent, long-lived process for the interactive pane (`agentmux-srv/src/backend/blockcontroller/persistent.rs`) or a one-shot invocation for background/subagent task runs (`agentmux-srv/src/agents/runner.rs:176-211`). This bounds what AgentMux can ever directly control: it can change what it *writes into* the prompt (CLAUDE.md content), CLI launch flags, and session lifecycle (fresh vs. `--resume`) — it can never place its own `cache_control` breakpoints, because it never builds the request.

For the interactive pane, the raw JSON reaches the frontend close to verbatim. Two places independently parse the same usage fields off the wire:

- `frontend/app/view/agent/providers/claude-translator.ts:106-123`
- `frontend/app/view/agent/useAgentStream.ts:483-515`

From there: `reducer.ts` (`frontend/app/store/agent-pane-state/reducer.ts:688-757`) updates a live `turnTokens` value; `useTurnLifecycle.ts`'s `finalizeTurn` (`:73-127`) merges the terminal `result` event's totals (preferred, since it's authoritative) or the live value (fallback) into `SessionStats` on turn end, dispatches a `TurnEnd` that the reducer sums into a lifetime `sessionTotals` (`reducer.ts:1317-1357`), and separately calls `recordTurn()` into `frontend/app/store/token-usage.ts` — a third, app-session-wide, per-provider running total that only the status-bar `TokenUsageIndicator`/`TokenBreakdownPopover` read from.

A second, more structured implementation also exists: `agentmux-srv/src/agents/{runner.rs, translator/claude.rs, types.rs}` parses the same raw usage into a proper 4-field `TokenCounts` struct (`types.rs:138`) and emits `AgentEvent::Cost{cost_usd, tokens}`. Per its own module doc (`agents/mod.rs:4-12`), **this is explicitly not yet wired to the interactive agent pane** — it currently only drives the headless drone Agent block executor. So the more-correct implementation exists in the codebase today; it's just not the one users see.

### 2.2 The collapsing problem

Both frontend parse sites take all three prompt-related fields off the wire and immediately sum them into one "input" number:

```
// claude-translator.ts:110-111 and useAgentStream.ts:489-496 — identical comment in both:
// "input_tokens is only the uncached prompt; cache_creation/cache_read carry the rest of the real prompt size"
```

The code is *aware* the distinction matters — it says so in a comment — and then discards it anyway. Every stored value downstream (`turnTokens`, `SessionStats`, `sessionTotals`, the `token-usage.ts` store, and the `↑` number in the status bar) is this same undifferentiated sum. There is no code path today, live or historical, that can answer "what fraction of this session's input tokens were cache reads?" — despite the raw data briefly existing in memory to answer exactly that.

### 2.3 What the UI shows today

- **`TokenUsageIndicator.tsx`** — a status-bar button: `↑{compact(input)} ↓{compact(output)}`, tooltip "Total tokens this session." Cumulative, app-session-wide, all providers combined by default.
- **`TokenBreakdownPopover.tsx`** — opens on click: per-provider `↑input ↓output` rows, a total row, a "Reset counter" button (explicitly resets only this running display, not the actual conversation).
- **`AgentFooter.tsx` / `AgentComposerStrip.tsx`** — per-pane "Worked" line (tokens + elapsed time for the last/live turn), and a context-fill meter: `"Context window: X / Y tokens (Z%) ... Auto-compacts around N tokens."`

None of these three surfaces distinguish cache-read from fresh tokens. None persist history — everything resets on app reload.

### 2.4 Known accuracy gaps (already documented in code)

- **Multi-call-turn undercounting**: the live `turnTokens` value is *overwritten*, not accumulated, on each `message_start`/`message_delta` — `reducer.ts:1317-1320` explicitly notes this undercounts a turn that made multiple API calls. Mitigated (not fixed) by preferring the terminal `result` event's total when one is available.
- **Empty boundary markers**: Claude's persistent-mode controller emits an empty `session_end` (`stats: {}`) as a mere turn-boundary marker after every plain-text turn, while the real usage-bearing `result` only fires at process teardown — `parseHistoryLines.ts:18-34` has to specifically track "the last `session_end` that actually carried usage" so a later empty marker doesn't clobber real historical stats on history replay.
- **No historical persistence at all**: confirmed independently by this investigation and by `TOKEN_TAX_FOLLOWUP §4`/`§5` — `agentmux-srv/src/backend/storage/` has no table or column for `cache_read_input_tokens` / `cache_creation_input_tokens`, or for any per-turn usage record. "Is caching actually working for this session" is answerable only live, in the moment, by reading the UI before it resets.

---

## 3. How compaction currently works

**Fully passive.** AgentMux detects and renders a compaction that Claude Code's own CLI already decided to run — it never requests, triggers, or influences one. This is Claude Code CLI's own internal auto-compact mechanism, unrelated to the Anthropic API's separate server-side "compaction" beta feature (AgentMux doesn't use the API directly, so that beta is irrelevant here).

- **Trigger**: purely the CLI's own logic. The CLI reports which of its own two triggers fired — `Auto` (its context-fill heuristic) or `Manual` (someone typed `/compact` *inside the CLI session itself*, not through AgentMux) — via `CompactionTrigger` (`agentmux-srv/src/agents/types.rs:117-134`). AgentMux only *reads* the threshold for display (`compactionThreshold(window) = window - 33,000`, `context-window.ts:26`) — it never sets it.
- **`agentmux-bashwrap/src/precompact.rs`** is a Claude Code `PreCompact` hook, auto-registered into the CLI's `settings.json`, invoked the instant compaction *begins* (before it's known to have finished). It publishes a live-only `compaction_started` event so the UI can show "Compacting…" — it has no power to cause or block compaction, and always exits 0 with no stdout.
- **No RPC, button, or stub exists to trigger one manually.** `frontend/app/store/rpc-api/agent.ts` has nothing compaction-related. The relevant slash-command architecture spec (`SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md`, appendix) explicitly marked `/compact` **"✗ — Requires live CLI session; offer 'start new pane' instead"** and left open the question "does `/compact` have any meaningful representation in stream-json mode?" — **that spec predates the persistent pane controller** (`blockcontroller/persistent.rs`), which *does* keep one long-running `claude` process alive with piped stdin for the entire conversation. The premise the spec rejected on ("no live session to send it to") may no longer hold. See §6.
- **What the user sees**: the actual compaction UI lives in `DocumentRow.tsx:266-311` — a divider reading `"context compacted — you ran /compact"` or `"— auto-compacted"`, with `"Earlier history summarized · {before} → {after} tokens · took {N}s"` when real CLI-reported numbers are available (falls back to a heuristic-detected boundary, no duration/trigger, for non-Claude providers). (Note: `components/CompactResult.tsx` is a same-named-but-unrelated component that collapses tool-call *results* like Grep/Glob output — not conversation compaction. Don't confuse the two when reading the code.)
- **Effect on tracked totals**: compaction *corrects* the running context estimate rather than resetting or double-counting — `reducer.ts`'s `CompactionBoundary` case (~lines 1147-1215) reconciles `lastContextTokens` to the CLI-reported `postTokens`, with a race guard against a stale/delayed boundary event clobbering a newer one. A parallel heuristic (≥50% token-count drop from a >10k baseline) remains the only detection path for Codex/Gemini/Copilot, which have no structured compaction signal, and is suppressed for a window after a real boundary fires so the two don't double-trigger.

---

## 4. Should we track "up" and "down" tokens?

**Yes — but as four buckets, not two.** "Up"/"down" (input/output) is already the coarsest possible view, and the API gives strictly more information for free. The recommendation is specifically:

```
input_tokens              — fresh prompt tokens, full price
cache_creation_input_tokens — fresh tokens written to cache, elevated price
cache_read_input_tokens   — served from cache, ~0.1x price
output_tokens              — model's reply, full price
```

This isn't new plumbing — **the raw data already flows through both frontend parse sites today** (§2.2) and is discarded immediately after. Preserving it through `TurnTokens` → `SessionStats` → `sessionTotals` → `token-usage.ts` is a matter of not collapsing three numbers into one at the point of parsing, not building a new data pipeline. The properly-typed 4-field struct already exists server-side (`agents/types.rs`); it just isn't the one feeding the UI (§5.1).

---

## 5. Getting a more reliable picture of token usage — recommendations

### 5.1 Consolidate the two parallel implementations (highest leverage)

Right now a correct, 4-field `TokenCounts` implementation exists in the Rust unified agent runner and an incorrect, 2-field, twice-duplicated implementation exists in the frontend and drives everything users actually see. Two implementations of the same concept in one codebase, one of which is already known-better, is a standing invitation for the two to drift further apart. The recommendation is to finish wiring the interactive pane onto the unified runner's event stream (the module doc already anticipates this as "PR 1") rather than fixing the frontend's collapsing bug in place — fixing it in two files that will need to be deleted anyway is wasted work.

### 5.2 Preserve the 4-field breakdown end-to-end

If full consolidation isn't scheduled soon, the smaller fix is: stop summing `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` at the point of parsing in `claude-translator.ts` and `useAgentStream.ts`. Carry all four fields through every downstream type (`TurnTokens`, `SessionStats`, `sessionTotals`, the `token-usage.ts` store). Collapse to a single number only at display time, in views that genuinely need brevity (e.g. `AgentFooter`'s compact "Worked" line) — never earlier.

### 5.3 Surface cache-hit rate as a first-class metric

Once §5.2 lands, expose `cache_read / (input + cache_creation + cache_read)` per turn and per session. This is the single most actionable number for the "why is this session expensive" question (§1.1) and is currently invisible anywhere in the product. A sudden drop in cache-hit rate mid-session is also a good automatic signal that something silently invalidated the cache (per the trigger list in `TOKEN_TAX_FOLLOWUP §4b`) — worth a subtle UI nudge rather than requiring a manual investigation each time.

### 5.4 Fix per-turn accumulation properly

Replace the "overwrite live value, prefer terminal `result` event, fall back to live value" pattern (§2.4) with a live running accumulator that sums every `message_start`/`message_delta` usage delta within a turn, still reconciled against the terminal event's authoritative total when one arrives (rather than only used as a fallback). This closes the multi-call-turn undercounting gap without waiting on the terminal event to correct it.

### 5.5 Compute real dollar cost, not a flat multiplier

Once the 4-field breakdown is preserved, cost can be computed correctly per the current per-model pricing table (input/output/cache-write/cache-read all have different per-token rates) instead of the flat guess a 2-bucket view would force. This is a natural pairing with §5.3 — "N tokens, $X, Y% cache hit rate" is a materially more useful status-bar tooltip than the current raw arrow-counts.

### 5.6 Persist history, not just live counters

`TOKEN_TAX_FOLLOWUP §5` flagged this in July and it's still true: nothing survives a reload. Persisting per-turn usage records (even just the four raw numbers + timestamp + provider + session id) turns "is caching actually working" from a one-off manual experiment into a query, and is the prerequisite for any trend view, cost dashboard, or regression alert. This doesn't need to be a new table from scratch — the drone Agent block executor already computes `AgentEvent::Cost` records that could be the seed of a shared storage schema.

### 5.7 Note the context-window-size limitation honestly

`context-window.ts` guesses the model's context window from a model-id string pattern match and learns upward from observed usage, because — per its own header comment — the CLI never reports the effective window directly. This is a real accuracy ceiling on the "% of context used" meter specifically, but it's a CLI limitation AgentMux is working around reasonably, not a bug to "fix" outright. Worth documenting as a known limitation rather than silently trusting the percentage as exact.

---

## 6. Giving users a manual "compact now" option

**Feasibility has changed since the architecture was last assessed, and it's worth re-testing rather than treating as settled.** The spec that ruled this out (`SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md`) reasoned from the one-shot `--print` invocation model, where there's no live process to send anything to after the fact. The persistent pane controller built since then (`blockcontroller/persistent.rs`) keeps exactly the thing that was missing: one long-running `claude` process with a piped, still-open stdin for the whole conversation, and an existing control-protocol channel already used for `AskUserQuestion` and permission-prompt round-trips.

**Update 2026-08-18 — empirically verified, feasible with zero new backend surface.** Ran a controlled probe (`claude.exe -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages`, same shape as the persistent controller's launch args, in an isolated scratch directory, real CLI v2.1.218) sending `{"type":"user","message":{"role":"user","content":"/compact"}}` over piped stdin — the identical wire shape `persistent.rs`'s existing `send_message()` already builds for any ordinary chat turn. Findings:

1. **`/compact` sent this way is recognized as the real command, not literal text.** The CLI immediately emits a progress frame — `{"type":"system","subtype":"status","status":"compacting"}` — followed by a terminal one: `{"type":"system","subtype":"status","status":null,"compact_result":"success"|"failed","compact_error"?:"..."}`. (First attempt returned `compact_result:"failed"`, `compact_error:"Not enough messages to compact."` — correct behavior for a nearly-empty test session, not a probe failure; padding the session with a few throwaway turns before retrying produced `compact_result:"success"`.)
2. **On success, the CLI then emits the exact same `subtype: "compact_boundary"` event AgentMux's backend already parses for auto-compaction** — `{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual","pre_tokens":27875,"post_tokens":1677,"cumulative_dropped_tokens":26198,"duration_ms":19447,"preserved_segment":{...},"preserved_messages":{...}}}`. `trigger:"manual"` maps directly onto the `CompactionTrigger::Manual` variant that already exists in `agentmux-srv/src/agents/types.rs:126-134` — the backend was already written to expect this value, just never had a code path that could produce it before now.
3. Immediately after, the CLI re-emits a fresh `system/init` frame (same `session_id`) and injects a synthetic continuation-summary `user`-role message plus a `<local-command-stdout>Compacted </local-command-stdout>` marker (`isReplay: true`) — the same shape auto-compaction already produces, since it's the same underlying CLI mechanism regardless of trigger.

**Net conclusion: no new backend RPC, no new control-protocol frame, and (most likely) no change to the existing `compact_boundary` parser are needed.** A "Compact now" feature can call the *existing* send-message path with the literal string `/compact` — see §5.2-adjacent implementation below, delivered alongside this report's other recommendation.

The "start a new pane instead" advice in the 2026-04-14 spec was a reasonable conclusion for the constraints that existed *then* (before the persistent controller existed) — this finding supersedes it for the persistent-pane case specifically; the one-shot `--print` path the original spec was reasoning about still has no live session to send anything to.

---

## 7. Best-practice infrastructure for accounting — summary design

Pulling the above into one target architecture:

1. **One source of truth.** Retire the frontend's duplicate parsing once the unified Rust runner drives the interactive pane (§5.1) — two independently-maintained parsers of the same wire format is the root cause of the current collapsing bug and a standing risk of future drift.
2. **Never collapse at ingestion.** Store all four raw `usage` fields as far downstream as possible; collapse to a display-friendly number only in the specific view that needs it, and even then prefer showing the breakdown when space allows.
3. **Track three time scales, not one.** Per-turn (diagnosing a single expensive exchange), per-session (the current running total), and lifetime/historical (trend and regression detection) are different questions and should be different rollups, not one counter overwritten in place.
4. **Prefer authoritative terminal totals over accumulated live deltas, but don't drop the live signal.** Keep reconciling against the CLI's own `result`/`session_end` totals when they arrive (already the right instinct in the current code) — but accumulate properly in between so a mid-turn or dropped-terminal-event case doesn't silently undercount.
5. **Make cache health a visible, first-class number**, not something recoverable only by re-deriving it from raw fields nobody currently sees together (§5.3).
6. **Persist, don't just display.** A live-only counter answers "what's happening right now"; a persisted per-turn log is what turns "is our caching strategy working" from an occasional manual experiment (as in `TOKEN_TAX_FOLLOWUP`) into an ongoing, queryable fact.
7. **Compute real cost from the real per-field pricing table**, not a flat rate — this is only possible once #2 is in place, and it's the number users actually care about more than raw token counts.
8. **Be honest about the ceilings AgentMux can't move.** The subprocess-only integration model (§2.1) means no direct `cache_control` placement and no server-reported context-window size — these are architectural facts, not bugs, and the accounting layer should present numbers with that caveat rather than implying more precision than the underlying CLI actually provides.

---

## 8. Suggested priority order

1. §5.2 (stop collapsing the 4 fields) — small, self-contained, unblocks everything else.
2. §5.3 (surface cache-hit rate) — immediate user value, depends only on #1.
3. §6 step 1 (empirically test `/compact` over the persistent controller's stdin) — cheap to test, answers the "can we even do this" question before any UI work is committed.
4. §5.1 (consolidate onto the unified Rust runner) — larger, but prevents #1–3 from being built twice.
5. §5.6 (persist per-turn usage history) — the prerequisite for any longer-term cost/trend tooling, and directly closes the gap `TOKEN_TAX_FOLLOWUP` already flagged twice.

---

## Sources

- `docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md`, `docs/analysis/TOKEN_TAX_FOLLOWUP_2026_07_04.md` — prior empirical investigation of this same subsystem; this report extends rather than repeats their findings.
- Code investigation (this report): `frontend/app/statusbar/{TokenUsageIndicator,TokenBreakdownPopover}.tsx`, `frontend/app/store/agent-pane-state/{reducer,context-window,types}.ts`, `frontend/app/store/token-usage.ts`, `frontend/app/view/agent/hooks/useTurnLifecycle.ts`, `frontend/app/view/agent/parseHistoryLines.ts`, `frontend/app/view/agent/providers/claude-translator.ts`, `frontend/app/view/agent/useAgentStream.ts`, `frontend/app/view/agent/compact-boundary.ts`, `frontend/app/view/agent/virtualization/DocumentRow.tsx`, `agentmux-bashwrap/src/precompact.rs`, `agentmux-srv/src/agents/{runner.rs,types.rs,translator/claude.rs}`, `agentmux-srv/src/backend/blockcontroller/persistent.rs`, `docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md`.
- Anthropic Messages API `usage` schema and prompt-caching pricing/mechanics (input/output/cache-creation/cache-read token separation, cache TTL economics, stateless-request behavior).
