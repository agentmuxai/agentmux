# SPEC: compaction progress — what the CLI really emits, an estimated progress bar (Tier 4), and a stream-frame bug found on the way

**Date:** 2026-10-01
**Status:** active — Tier 4 (§5) shipped in PR #4220; the stream-frame fix (§3) and status-frame handling (§6) are not built and need the decisions in §8.
**Author:** Agent3 (UID `fb3e692d-caf9-48e3-b20a-e659361aa057`)
**Trigger:** Repo owner, 2026-10-01: *"search online, latest claude CLI system, can agentmux get the progress of the compression?"*, then *"write spec to file on implements. sure, lets try the tier 4"*.
**Researched against:** `agentmuxai/agentmux` `main` @ `807c749ce`; Claude Code CLI **2.1.287** (the version AgentMux has installed under `~/.agentmux/shared/cli/claude/`).
**Extends:** `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` — Tiers 1–3 shipped; this spec builds its Tier 4 and corrects two of its assumptions (§3).

Evidence labels: **[verified]** I ran the check or read the code/data myself · **[reported]** from a third-party source, not re-measured · **[inferred]** reasoning, not observed. Times are UTC.

---

## 1. The question, answered

**Can AgentMux get the real progress of a compaction? No.** Compaction is one summarizing model call. The CLI reports when it starts, that it is still running (about every 30 s), and when it ends with before/after token counts and the duration. It never reports a percentage. Everything finer-grained stays inside the CLI's own spinner (§2).

So the honest progress display is an **estimate**: how long the previous compactions took, applied to the current one, and labeled as an estimate.

## 2. What the CLI emits on stream-json [verified live]

I ran the real CLI 2.1.287 in the mode AgentMux uses (`-p --input-format stream-json --output-format stream-json --verbose`) against a local fake Anthropic API (`scripts/cli-probe/fake-anthropic.mjs` from #4189, extended with a delay on the summarizing call), in a throwaway HOME with a dummy key: no account, no cost. I sent `hello`, then `/compact`.

| t (ms) | Frame on stdout |
|---|---|
| 357 | `{"type":"system","subtype":"status","status":"compacting","session_id":…,"uuid":…}` |
| 30 358 | the same frame again — a **~30 s heartbeat** while the call runs |
| 45 379 | `{"type":"system","subtype":"status","status":null,"compact_result":"success",…}` (`"failed"` plus `compact_error` on failure — read from the binary, not triggered) |
| 45 391 | `{"type":"system","subtype":"compact_boundary","uuid":…,"compact_metadata":{"trigger":"manual","pre_tokens":25040,"post_tokens":733,"cumulative_dropped_tokens":24307,"duration_ms":1513},"logical_parent_uuid":…}` |

The CLI binary also defines finer stages (`compact_progress`: `compact_start`, `hooks_start` for the pre-compact, post-compact and session-start hooks, `compact_end`). They are handled only by its own spinner and are **not** written to stdout. [verified from the binary: they sit in the "ignored by the output translator" case list.]

The `PreCompact` hook (already wired by AgentMux, `agent_config.rs`) gives the same start signal, and the official docs add that a `PreCompact` hook can block compaction [reported: code.claude.com/docs/en/hooks].

## 3. Finding: AgentMux reads the boundary frame with the wrong key casing [verified]

The **stdout** `compact_boundary` frame uses snake_case, `compact_metadata` with `pre_tokens`, `post_tokens`, `cumulative_dropped_tokens`, `duration_ms`, and has **no `timestamp`**. The **transcript** (`~/.claude/projects/**.jsonl`) entry for the same boundary uses camelCase, `compactMetadata` with `preTokens`, `durationMs`, **has** a `timestamp`, and carries the same `uuid`.

Both live parsers read only the camelCase form:
- `crates/srv/src/agents/translator/claude.rs` `handle_system_message`: `frame.get("compactMetadata")`.
- `frontend/app/view/agent/compact-boundary.ts` `parseCompactBoundaryFrame`: `e.compactMetadata`.

The unit tests use camelCase fixtures, copied from the transcript (the old spec's "live evidence" was a `.jsonl` file). AgentMux's own stored agent output (the `output` blockfiles in `filestore.db`, 0.59.1) contains the stdout form: 3 snake_case boundary lines, 1 camelCase. [verified by grep; not watched live in a pane]

**Consequence [inferred from the code, not observed in a running pane]:** the live path in `useAgentStream.ts` that handles `compact_boundary` gets `null` from the parser and dispatches nothing. That path also drives:
- the reducer's `CompactionBoundary` (clearing `compacting`, `lastCompactionBoundaryAt`, the live `context_compacted` node);
- `CompactionSummaryTracker.noteBoundary` (the compaction context-delivery card);
- **`memoryReinjectionController.trigger(...)`**, the hidden Global/Personal Memory reinjection after compaction (`SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md`).

The history replay (`parseHistoryLines.ts`) reads the camelCase transcript, which is why compactions still show up after a reload and the problem stayed hidden.

**Why this spec does not just fix it:** fixing the casing activates code that, if the above holds, has never run live. In particular, with no `timestamp` on the stdout frame, `memoryReinjectionNodeId(null)` gives one fixed id (`memory-reinjected-notime`) for every compaction in a session, so the second reinjection's node would be deduplicated away; and the live `context_compacted` node id (content- or uuid-derived) would differ from the history node id (timestamp-derived) unless both are re-keyed on `uuid`. That is a separate change with its own tests and a visible behavior change (a hidden message to the model after every compaction). It needs the owner's decision, §8 D1.

## 4. Requirements

- **R1:** while a compaction runs, the working row shows an *estimated* progress bar and the typical duration ("usually ~30 s"), only when there is data for an estimate.
- **R2:** it never presents the estimate as real progress: it is labeled as an estimate, never reaches 100% on its own, and switches to a plain "longer than usual" state when the run overshoots.
- **R3:** with no samples, nothing changes from today (the "Compacting… Ns" elapsed counter).
- **R4:** the estimate is stable for the whole compaction (it does not move as new samples arrive mid-run).
- **R5:** it does not change what the existing compaction consumers do (§3): no reducer, translator or node-id change.

## 5. Design — Tier 4

**Samples.** One sample per finished compaction: `{ uuid, preTokens, durationMs }`, from the stdout `compact_boundary` frame. A new small parser (`frontend/app/view/agent/compaction-estimate.ts`, `parseCompactionSample`) accepts both casings (`compact_metadata`/`compactMetadata`), so it also works if the CLI changes, and **does not** share code with `parseCompactBoundaryFrame` (R5). `useAgentStream` records the sample at the existing `compact_boundary` branch, before the unchanged parse.
- Kept in `localStorage` (`agentmux:compaction-samples:v1`), newest 10, global across panes and agents: compaction time depends on the summarizing model call and the context size, not on the agent.
- De-duplicated by `uuid`, so a frame seen twice (a stream re-read) counts once.
- Validated on read (JSON, shape, finite positive numbers); anything else is dropped. Storage that is unavailable or throws behaves as "no samples".

**Estimate.** `estimateCompactionMs(samples, contextTokens)`:
1. No samples → `null` (R3).
2. `base` = median of the sample durations (robust to one slow outlier).
3. With a known `contextTokens` and sample `preTokens`, scale by the context size, clamped so a single odd size can't swing it: `base × clamp(contextTokens / median(preTokens), 0.5, 2)`.
4. Clamp the result to `[5 s, 600 s]`.

The caller computes it once when `compacting` starts (R4), from `state.lastContextTokens`.

**Display** (`AgentWorkingRow`, `AgentFooter.tsx`). When `compacting` is set and an estimate exists:
- right side: `12s / ~30s`;
- a thin bar under the row, fill `min(elapsed / estimate, 0.95)`, `role="progressbar"` with `aria-valuetext` saying "estimated, about N%";
- tooltip (on the right-hand text): "Estimate from your earlier compactions. Claude Code doesn't report compaction progress.";
- once elapsed exceeds the estimate: the fill stops at 95% and turns indeterminate, right side `42s · longer than usual` (R2).
With no estimate, exactly today's row.

## 6. Not built here — status frames (decision D2)

The `status` frames in §2 could give (a) a **hook-independent start** (the `PreCompact` hook arrives over a live-only WPS event that can be missed), (b) a **visible failure** (`compact_result:"failed"`, `compact_error`, which AgentMux cannot see today), (c) a liveness heartbeat. They are not wired because:
- they go through `useAgentStream`'s stdout path and the reducer's `CompactionStarted` race guards (`pendingCompactionPing`, `lastCompactionBoundaryAt`), the same machinery §3 says is unproven live;
- a status frame has no `trigger`, and a re-read of an old frame could set a stale `compacting` state;
- showing a failure needs a new node type (`AgentErrorNode` has an HTTP `code`, `CliNoticeNode` is install/version only).
It should be designed together with the §3 fix. Seeding the sample store from transcript history (so a first compaction already has an estimate) is also left out; the store fills as compactions happen.

## 7. Tests

- `compaction-estimate.test.ts` (28 tests): both casings parse, with the real captured frame as the fixture; malformed, negative and non-finite input is rejected; the median is robust to an outlier; the scale clamps at 0.5× and 2×; the result clamps to 5 s / 600 s; no samples → `null`; the fill never exceeds 95% and `over` is reported; the store round-trips, de-duplicates by uuid, keeps the newest 10, reads corrupt or throwing storage as empty, and drops bad entries.
- `AgentFooter.test.tsx` (6 tests): no history → plain elapsed counter and no bar; history → `12s / ~30s`, a `progressbar` with `aria-valuenow`, and the "Estimate…" tooltip; overshoot → "longer than usual", indeterminate, fill held at 95%; the estimate does not move when a sample lands or the context size changes mid-run (R4); the next compaction estimates afresh; no bar while reconnecting. Checked to fail: removing the untracked read of the context size breaks the R4 test.
- **Not unit-tested:** the one-line `recordCompactionSample(parseCompactionSample(rawEvent))` call in `useAgentStream.ts` (that hook has no test harness). It is covered by the parser and store tests above and by the live check.
- **Live check, still to do on a dev build:** `/compact` a real conversation twice and watch the second one's bar and `12s / ~30s` text.

## 8. Decisions

- **D1 — fix the boundary casing (and re-key node ids on `uuid`) so the live compaction path actually runs?** Recommended: yes, as its own PR, with memory reinjection's id fixed first and a check of what hidden reinjection does after every compaction. Alternative: leave the live path as it is and rely on history replay.
- **D2 — status-frame handling (§6)** after D1, or not at all?
- **D3 — sample store scope:** global (as built) or per account/model.

## 9. Delivery

One PR: this spec + Tier 4 (frontend only, no srv change). D1 and D2 are follow-ups.
