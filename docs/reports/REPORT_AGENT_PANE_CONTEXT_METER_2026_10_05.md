# REPORT: The agent pane's context meter is wrong in two independent ways

**Date:** 2026-10-05
**Author:** agent2
**Trigger:** Repo owner, 2026-10-05: a freshly started agent pane's composer strip read `17m / 200k`. 17m is impossible, and Sonnet 5.5 has a 1M window, not 200k. "We may need a deep rethink."
**Status:** implemented — analysed at `origin/main` 6943d5fd5 (v0.59.10 era); P0–P3 implemented together and reviewed, see §9.
**Related:** `SPEC_CONTEXT_VISIBILITY_2026_06_17.md` (the meter's design), `REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md`, `PLAN_PANE_REOPEN_SESSION_RESUME_AND_STATS_BAR_2026_07_10.md` (introduced the hydration path), `REPORT_AGENT_RUNTIME_STATE_RECONCILIATION_2026_09_30.md` (§3.6 on `modelUsage`).

---

## 1. Verdict

Both numbers are wrong for separate reasons, and **both come from the same place: the state the pane restores when it opens, before its first live turn.** A pane that has not yet run a turn in this process shows neither of the two values the live path would compute.

| | What the strip showed | Why | Right value |
|---|---|---|---|
| **Used** | `17m` | At mount the pane seeds "context used" from the last `result` event in history. A `result`'s `usage` is the **sum over every API call in the turn**, not the size of the context. A long autonomous turn makes 50+ calls at ~300k each. | The size of the **last API call's** prompt (a few hundred k at most) |
| **Window** | `200k` | At mount no window has been learned yet, so the strip falls back to a static per-provider number in the catalog (`contextWindow: 200_000` for every Claude model). The per-model resolver only runs on a live `message_start`. | `1,000,000` for Sonnet 5.5 (and the CLI says so; §3) |

Neither is a one-line typo. Together they say the meter has **no single definition of "context used" and no authoritative source for "window"**: three code paths compute the first (live, replay, compaction boundary), the second is a model-name table plus a learn-up heuristic, and nothing checks that the two are consistent (`tokens > window` renders happily). §6 proposes a design that fixes the class, and §7 a small first step that fixes this symptom.

**What was verified and what was inferred.** Verified here: the CLI's `result.usage` semantics and `modelUsage.contextWindow` (live probes against the pinned CLI, §2.2 and §3), and the code paths (cited by file and line). Inferred, not observed: that the owner's `17m` came from exactly this path. It fits (17M ≈ 57 calls at 300k; "just started up"; both numbers wrong at once), and I found no other path that can produce an input figure that large. To close the loop, read the last `result` line of that pane's output and compare its summed `usage` to 17m.

---

## 2. Defect 1: "context used" is seeded from a turn-total

### 2.1 The path

1. Opening a pane restores history, then parses it (`parseHistoryLines.ts` ~380-392). For every `session_end` that carries usage it keeps `lastSessionStats`.
2. `session_end` stats come from the Claude translator's handling of the CLI's `result` event: `input = usage.input_tokens + cache_creation + cache_read` (`claude-translator.ts` ~285-297).
3. `useHistoryPagination.ts` (~392 and ~532) dispatches `ReconcileContextFromHistory { tokens: lastSessionStats.input_tokens }`.
4. The reducer (`reducer.ts` ~548) writes it to `lastContextTokens`, only if that is still `null`.
5. `agent-view.tsx` ~1405 passes `lastContextTokens` to the strip, which prints `formatCompactNumber(t) / formatCompactNumber(w)`.

The hydration was added by #2059 (2026-07-10) on the assumption that a `result`'s input is the context. It is not.

### 2.2 The CLI's behaviour (probe, CLI 2.1.288, model `claude-sonnet-5-5`)

A turn that makes three API calls (read a file, read another, answer):

| | tokens |
|---|---|
| call 1 prompt | 43,376 |
| call 2 prompt | 43,500 |
| call 3 prompt (the real context at the end) | **43,623** |
| `result.usage` input total (fresh + cache creation + cache read) | **130,499** |
| sum of the three calls | 130,499 |

`result.usage` is exactly the sum of the calls' prompts. The real context is about a third of it, and the ratio is the number of calls. `num_turns` on the result (3 here) is that count. Two of the repo's own docs already say the same: `SPEC_CONTEXT_VISIBILITY` §1.2 ("cumulative for the turn") and `turn-contribution.ts` ("the whole-turn input the result event reports … sums every call's"). The footer was fixed for it on 2026-10-02; the hydration path never was.

So a turn of N calls at context C reads as roughly N×C. An agent that works for an hour on one instruction (tool call after tool call, which is the normal swarm case) reaches tens of millions. `17m` is unremarkable.

### 2.3 Why it is worse than a wrong label

- **Severity colouring and the countdown.** The strip bands against `compactionThreshold(window)`. 17m against a 167k threshold is the "critical" band, so the reading is red, the countdown reads `~0 to auto-compact`, and Compact is invited, on a pane that is nowhere near full. (`AgentComposerStrip.tsx` ctxBand / ctxCountdownText; the `~0 to auto-compact` text is exercised by its own test.)
- **A false "context compacted" card on the first live turn.** `TokensIn` fires a heuristic compaction when the new input is below 50% of the previous value and the previous is above 10k (`reducer.ts` ~875-886). With `prev = 17m` seeded from history, the first real call (say 300k) trips it, and `useAgentStream.ts` `pushContextCompactedNodes` inserts a "context compacted, 17m → 300k" card into the transcript. This is from reading the code, not observed. It needs a seeded value above 2× the first live value, which any multi-call turn produces.
- **The wrong number is exported.** `agent-view.tsx` ~306 mirrors `lastContextTokens` into block meta `term:ctx-tokens`, and the Swarm view reads that meta (`swarm-model.ts` ~2116, shown by `swarm-view.tsx` ~529). The bogus value is therefore also persisted and shown on the Swarm card.
- **It does not self-heal for an idle pane.** The next live `message_start` replaces it, so a pane that is only being read, or is waiting for a jekt, keeps the wrong number indefinitely.

---

## 3. Defect 2: the window is guessed, though the CLI reports it

### 3.1 The path

`agent-view.tsx` ~1406: `contextWindow={lastContextWindow ?? provider()?.contextWindow}`. `lastContextWindow` is only set by `learnContextWindow()` inside the reducer's `TokensIn`, i.e. on a live `message_start` (`reducer.ts` ~840). Before that it is `null`, and the strip uses the provider catalog's static value. `providers/catalog.ts` ~159 gives the Claude provider `contextWindow: 200_000` regardless of model, and its default model is `sonnet`, labelled "Sonnet 5.5" (~178).

`contextWindowForModel()` (`context-window.ts`) already knows Sonnet 5+ is 1M, but nothing at mount calls it. It also returns 200k for the bare alias `sonnet` (the catalog's own model `value`), so even resolving from block metadata would give the wrong answer today; only a resolved id such as `claude-sonnet-5-5` works.

### 3.2 The code's premise is out of date

The header of `context-window.ts` says "The CLI never reports the effective window (verified — `system/init` and `result` carry `model` but no window field)". That is true of `system/init` and **false of `result`**. The CLI's `result` message carries `modelUsage`, keyed by model, and each entry has `contextWindow` and `maxOutputTokens`. Confirmed two ways against CLI 2.1.288: the binary's result schema lists `contextWindow` as a required integer, and a live probe returned:

```json
"modelUsage": { "claude-sonnet-5-5": {
  "inputTokens": 2, "outputTokens": 4,
  "cacheReadInputTokens": 11854, "cacheCreationInputTokens": 31490,
  "contextWindow": 1000000, "maxOutputTokens": 128000,
  "canonicalModel": "claude-sonnet-5-5", "provider": "firstParty" } }
```

(`system/init` in the same run carries the resolved `model: "claude-sonnet-5-5"` and no window.)

Consequences:
- The table, and the learn-up heuristic that exists to compensate for it, are a workaround for data that is available. They cannot be right in general: the window depends on the model **and on session configuration** (the 1M beta on Sonnet 4.x, a `[1m]` suffix, API provider). A table keyed on a model name cannot know that.
- Caveats on using it (from `REPORT_AGENT_RUNTIME_STATE_RECONCILIATION_2026_09_30.md` §3.6 and the CLI's own field docs): `modelUsage` is **cumulative over the session** and **includes subagents' models**, so the entry for the pane's own model must be picked by id, never "the first" or "the largest". It also only arrives with the first `result`, so a table is still needed for the interval before it.

### 3.3 Latent hazards in the learn-up logic

- Learn-up is one-way. `nextTierAbove(n)` returns `n` itself when `n` exceeds every known tier (`context-window.ts` ~59-62), so one bad observed value above 1M sets the window to that value permanently ("17m / 17m"). It is not reachable today only because the live path feeds it real per-call numbers.
- The memory-reinjection sizing uses `contextWindowForModel(lastSeenModelId) ?? 200_000` (`useAgentStream.ts` ~285/328/746, `memory-reinjection.ts` `FALLBACK_CONTEXT_WINDOW`). Before any live model is seen it assumes 200k. That errs in the safe direction by design, but it is a third place with its own idea of the window.

---

## 4. Why this is a design problem, not two bugs

1. **Three definitions of "context used".** Live: last call's `message_start` prompt (correct). Replay: the turn-sum (wrong). Compaction boundary: `post_tokens` (correct). A fourth consumer, the Swarm, reads a bare number from block meta. None of them carry **which source produced the number, which model, which session, or when**, so nothing can reason about whether a seeded value is trustworthy (the heuristic compaction detector treats a replay seed exactly like a live reading).
2. **No invariant at the display.** `tokens > window` is impossible, yet it renders. A one-line guard would have turned this report into a tooltip.
3. **The window is the weakest link and is derived from the least reliable input.** A static provider constant (200k for all of Claude) is wrong for most current Claude models. A fallback should say "unknown", not guess a smaller number and then colour the meter red on it.
4. **Persisted state has no validity story.** `term:ctx-tokens` survives restarts, model switches, `/clear`, a fresh session and compaction without anything marking it stale. The fresh-session case was patched once (codex P2 on #2507) by resetting `lastSessionStats` at a `fresh` outcome; that is a per-case patch of the same missing concept.
5. **Tests assert the plumbing, not the meaning.** `reducer.test.ts` and `parseHistoryLines.test.ts` cover that stats reach `ReconcileContextFromHistory`; none feed a multi-call `result` and ask whether the seeded number equals the last call's prompt. `context-window.test.ts` pins the table (including bare `sonnet` → 200k) but nothing exercises the mount-time window.

---

## 5. What else uses the same turn-sum

Not wrong by itself, but worth an audit pass because the same field means two things:
- `sessionTotals.input_tokens` and the cache-share percentage in the session-stats popover (`AgentSessionStats.tsx` ~110, ~230) sum turn-sums. As "tokens processed, billing-shaped" that is right; the label `in` reads as context. It should say so.
- `AgentFooter.tsx` ~293 prefers `added_input_tokens ?? input_tokens`, and falls back to the turn-sum when there is no baseline. The fallback re-creates the `↑180k for a one-line question` problem `turn-contribution.ts` documents.
- The status bar's token indicator (`recordTurn`) is fed the same sums and is correct only if it is read as spend.

---

## 6. Proposed design: one `ContextReading`, with a provenance

### 6.1 The model

Replace the loose pair `lastContextTokens` / `lastContextWindow` / `lastContextModel` (plus `term:ctx-tokens`) with one value:

```ts
interface ContextReading {
  tokens: number;            // the last MAIN-agent API call's whole prompt
  window?: number;           // undefined = unknown (never guessed from a provider constant)
  windowSource: "cli" | "table" | "learned";
  model: string;             // resolved id, e.g. claude-sonnet-5-5
  source: "live" | "replay" | "boundary";
  at: number;                // ms
}
```

Rules, each testable on its own:

1. **Tokens come from one function.** `mainAgentUsage()` already extracts the main agent's per-call prompt from a `message_start` and ignores subagents. Extend it to the persisted `assistant` events (which carry the same `message.usage`) and use it for **both** live and replay. The `result`'s `usage` is never used as context. After a compaction boundary the reading is `post_tokens` until the next call reports.
2. **Replay scans for the last such line,** from the end of the loaded window, resetting at a `fresh` session outcome as today. If the restored window holds none, show **no reading** ("session"), not a guess.
3. **Window from the CLI first.** On each `result`, take `modelUsage[<the pane's main model>].contextWindow` (match by the resolved id from `init`/`message.model`; subagent models are other keys). Fall back to `contextWindowForModel` for the interval before the first result and for providers that do not report one. If neither knows, `window` is `undefined` and the strip shows `340k ctx` (that branch already exists). **Remove `provider().contextWindow` as a fallback for Claude** (and audit the other providers' constants, which are equally unverified). Keep learn-up only as a last resort, and never promote to a value above the largest known tier: treat it as an invalid reading and log once.
4. **Display invariant.** If `tokens > window` (or `tokens` exceeds any tier with `window` unknown), render no ratio and no severity, and emit one diagnostic with the inputs. A wrong number should degrade to a missing number.
5. **The heuristic compaction detector only fires when the previous reading's `source` is `live` or `boundary`.** A replay seed can never create a card.
6. **Persist the whole reading** as one meta object (keep `term:ctx-tokens` as a derived field for compatibility), and have the Swarm apply the same validity rules. Invalidate on model switch, `/clear`, fresh session, and when `at` is older than the pane's last session change.

### 6.2 Phasing

| Phase | Change | Size | Closes |
|---|---|---|---|
| **P0** (this symptom) | Replay seeds from the last main-agent call's usage (one helper shared with live); mount-time window from the model seen in the same history (`init`/`assistant`), else unknown; Claude no longer falls back to 200k; display guard for `tokens > window`; heuristic detector ignores replay seeds | small, frontend only | the `17m / 200k` report, the false compaction card, the red `~0 to auto-compact` |
| **P1** | Read `modelUsage[main].contextWindow` from each `result`; table becomes the pre-first-turn seed; learn-up capped | small | the model-name table being the authority; 1M beta / `[1m]` / provider variance |
| **P2** | `ContextReading` as the single state + persisted meta; Swarm and strip share one validity function | medium | the three definitions; stale persisted values |
| **P3** | Audit the turn-sum uses in §5 and relabel (`processed` vs `context`) | small | label confusion |

P0 should not wait for P2. P1 and P2 are what stop the next variant of this bug.

### 6.3 Tests that would have caught it

- A history fixture whose last `result` is a multi-call turn: the seeded context must equal the last `assistant` call's prompt, not `result.usage`. The two probe runs from this report are suitable real fixtures (a small `claude-sonnet-5-5` NDJSON: 1 call and 3 calls, 130,499 vs 43,623).
- A pane restored with a Sonnet 5.5 history reads window 1,000,000 before any live turn; with no model in history it reads *unknown*, not 200,000.
- Renders: `tokens > window` shows no ratio; an unknown window shows `<tokens> ctx`.
- The first live `message_start` after a replay seed does not emit `context-compacted`.
- `modelUsage` with two entries (main model + subagent model) picks the main model's window.
- Property-style: for any sequence of {live call, replay, boundary, `/clear`, model switch} the displayed reading never exceeds its window.

---

## 7. Open questions for the owner

1. **Scope.** P0 alone, or P0 + P1 in one change? They touch the same two files; splitting costs an extra review cycle for little isolation. I would ship them together if the owner wants the window to be authoritative now, and P0 alone if the priority is stopping the false numbers fast.
2. **Other providers' windows.** Codex, Gemini, Kimi and the rest carry hard-coded catalog constants (128k/200k/1M). They were not verified here. Same principle (unknown over wrong), but that is a separate pass per provider.
3. **Compaction threshold.** `compactionThreshold = window − 33k` is documented as the CLI's own auto-compact point. It was not re-verified against CLI 2.1.288 here, and with a 1M window it is the number the severity bands depend on. Worth a probe before P1 relies on it.
4. **Existing panes.** `term:ctx-tokens` already holds bad values in some panes' meta. They are overwritten on the next live turn; P2's invalidation would clear them sooner. No migration is needed for P0.

---

## 8. Method and evidence

- Code read at `origin/main` 6943d5fd5: `context-window.ts`, `reducer.ts` (`ReconcileContextFromHistory`, `TokensIn`, `TurnReset`, `mergeStats`), `useHistoryPagination.ts`, `parseHistoryLines.ts`, `claude-translator.ts`, `main-agent-usage.ts`, `useAgentStream.ts`, `agent-view.tsx`, `AgentComposerStrip.tsx`, `AgentSessionStats.tsx`, `providers/catalog.ts`, `swarm-model.ts`, `turn-contribution.ts`, `util/format-count.ts`. No backend (`crates/`) code computes a window.
- Two throwaway probes of the pinned CLI (2.1.288, `--model sonnet`, `--output-format stream-json`, read-only tool, a scratch directory outside any repo), together about 20 cents of list-price usage. Raw output is kept in the author's scratch area, not in the repo.
- The CLI binary's embedded result schema was searched for `contextWindow`, `modelUsage` and the usage field descriptions.
- Not done: reproducing the owner's exact pane; running the app. Nothing was changed in the repo other than this file.

---

## 9. Implementation

All four phases of §6.2 shipped together, frontend only. After a first pass, three independent reviews (the CLI protocol re-probed live, the reducer under a property test, the view and persistence layer) found further defects; the fixes are folded in below and marked *(review)*.

### 9.1 What the CLI actually does (second probe, CLI 2.1.288)

- **Per-call usage.** Each API call is reported by its `message_start` and again by every `assistant` frame of the same message id, with identical input counts. Without partial messages, a call that writes text and then calls a tool emits two `assistant` frames under one id. Subagent frames carry `parent_tool_use_id` and never emit `stream_event`s.
- **`result.usage`** sums the main agent's calls in the turn (subagents excluded). `modelUsage` is cumulative for the whole session and carries over on `--resume`; each entry has `contextWindow`. Its key is the raw model string, with `canonicalModel` beside it (`claude-haiku-4-5-20251001` → `claude-haiku-4-5`).
- **Compaction.** `compact_boundary.post_tokens` counts only the summary messages, not the system prompt and tools. A `/compact` reported `pre_tokens` 40,697 and `post_tokens` 1,417; the next call's prompt was 39,490. So `post_tokens` is not the context size, and the first implementation (and the code before it) showed a post-compaction meter about 28× too low.
- **Resume** replays no old `assistant` frames, so no old usage reaches the live path.
- **The auto-compact threshold** is window − min(maxOutput, 20K) − 13K, so window − 33K by default. The window used is the *auto-compact* window, which `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, the `autoCompactWindow` setting, or `CLAUDE_CODE_DISABLE_1M_CONTEXT` can make smaller than `contextWindow`. The exact value is available from a local `get_context_usage` control request (no inference call). Not used yet: see §9.6.

### 9.2 The reading

`store/agent-pane-state/context-reading.ts` defines `ContextReading`:
- `tokens`, `model` and `at`;
- `window` and `windowSource` (reported / learned / model);
- `source` (live / history);
- `switchedTo`, set after a model change.

The pane state holds:
- one `context: ContextReading | null`, replacing `lastContextTokens` and `lastContextWindow`;
- two per-model maps, `reportedContextWindows` and `learnedContextWindows`, both outliving `TurnReset`;
- `contextSeedable`.

Every display reads the reading through `plausibleReading`, which refuses a reading larger than its window, or larger than any known window when the window is unknown.

### 9.3 Sources of tokens

`mainAgentUsage` is the only reader. It takes a call's prompt from its `message_start` or its `assistant` frame, and it ignores:
- subagents' lines;
- zero-usage and `<synthetic>` frames;
- *(review)* `assistant` frames with no message id.

*(review)* Usage is read only from Claude Code's stream format (`readsMainAgentUsage`), in both the live and the replay path. The live stream counts each call once, by message id. History replay (`HistoryParser.lastContext`) uses the same function, and resets at a `fresh` session and *(review)* at any compaction boundary, parsed or not. `session_end`/`result` stats are never a context size.

### 9.4 The window

Resolution, per model (`resolveContextWindow`):
1. the window Claude Code reported in `result.modelUsage`;
2. unless a larger one was *learned*: an accepted prompt larger than the window we had proves the next known tier;
3. else the model-name table.

Rules for learning and lookup:
- *(review)* Only models the table recognises learn, because the tiers are Claude's. Any other model has the window reported for it, or none.
- A prompt above every tier proves nothing.
- *(review)* `[1m]` keys are recorded under their bare id and their `canonicalModel` too, the larger window winning on a shared id, since `message.model` carries no suffix.
- *(review)* Learned windows survive `TurnReset`.

A report that an accepted prompt contradicts emits `context-window-refuted`, logged once per model and tier. The per-provider `contextWindow` constants are gone from the catalog. With no window, the strip shows `<tokens> ctx` and no fill band.

### 9.5 Lifecycle and guards

- **Compaction** *(review)*: the reading is cleared until the next call, unless it already postdates the boundary or the boundary belongs to an older compaction.
- **Seeding** *(review)*: `ReconcileContextFromHistory` seeds only while `contextSeedable`. The first live reading, an invalidation, a compaction and a reset all end it. Before this, a `fresh` session drained from the live stream while history loaded could be overwritten by the dead session's seed. History windows merge *under* live ones on every path.
- **Invalidation**: a `fresh` session outcome dispatches `ContextInvalidated`. *(review)* So do Archive and Restore (the `delete`/`replace` transcript file ops), which send no `fresh` outcome.
- **Model switch** *(review)*: a change of the model setting (`/model`, the model menu) dispatches `ContextModelSwitched`. The tokens carry over and the window is blank until the new model's first reply; switching back restores it.
- **Compaction heuristic**: the `TokensIn` heuristic compares only two live readings.
- **Logging**: a refused reading emits `context-reading-rejected` when it is news (the meter had something to show, or nothing). The store logs it once per source and reason, *(review)* re-armed by the next showable reading.
- **Commands that never start a turn**: slash and bang commands handled locally, and *(review)* a failed memory-reinjection send, revert with `TurnStartFailed` rather than `TurnReset`. `TurnReset` wiped the meter and the session totals.

### 9.6 Display and persistence

- `hooks/useContextReading.ts` feeds the strip, its popover and the meta mirror. The tooltip and popover say where the numbers come from (`contextReadingNote`). The popover's turn-sum totals are labelled "Processed".
- The Swarm reads `agent:context` through the same validation. It ignores the legacy `term:ctx-tokens`, which the pane clears.
- *(review)* The mirror writes:
  - nothing at mount until history restore completes, and nothing after a failed restore until the pane has a reading of its own;
  - nothing that matches the persisted value;
  - one write at a time, latest wins, because srv runs each RPC on its own task.
- *(review)* Providers with no reading still get the session popover once a turn reports totals. The Swarm's "has history" check also counts `session:line_count`.

### 9.7 Tests

- **Unit**: `context-reading.test.ts` and `context-window.test.ts`.
- **Reducer**: the "Context reading" block of `reducer.test.ts`, with one test per review finding.
- **Property test** *(review)*: `context-reading.property.test.ts`, 200 seeds × 150 random commands. Two deliberate mutations, removing the seedable guard and allowing a history seed as the heuristic's baseline, are both caught.
- **Hook**: `useContextReading.test.ts` covers mount and settle, the failed restore, ordering, write failure and model switch.
- **History replay**: `parseHistoryLines.test.ts` (the probe's compaction numbers, non-Claude formats) and the NDJSON seed in `useHistoryPagination.test.ts` (a 60-call turn whose result sums to about 18M).
- **Display and wiring**: strip, popover, Swarm line, store logging, and source pins in `main-agent-usage.test.ts`.
- **Gates**: the full frontend suite, `tsc`, ESLint on the new files and the CI lint gates pass.

### 9.8 Not done

- **Live check in a dev build.** A throwaway Claude pane with no agent definition (so nothing is written to the host-wide agent registry) could not start. The pane's sign-in gate held every send, because no identity is bound to an ad-hoc agent id.
- **Exact auto-compact threshold.** `get_context_usage` (§9.1) would replace the 33K constant and honour the overrides. srv already has the pattern (`get_settings`, re-sent at each `result`), so it is a contained follow-up across srv and the frontend.
- **The "context compacted" card** still prints `pre → post_tokens` (for example 40.7k → 1.4k). That is honest about the messages but not the context size; it could take the next call's prompt instead.
- **Other providers' windows** (§7 question 2) are unchanged, except that no provider constant is shown any more.
