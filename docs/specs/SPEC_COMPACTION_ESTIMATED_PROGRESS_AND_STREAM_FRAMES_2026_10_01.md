# SPEC: compaction progress — what the CLI really emits, an estimated progress bar (Tier 4), and a stream-frame bug found on the way

**Date:** 2026-10-01
**Status:** active — Tier 4 (§5) shipped in PR #4220; the failed-compaction notice (§10) shipped in PR #4232; the stream-frame fix (§3) is built (§8 D1); the rest of status-frame handling (§6) is built (§8 D2); D3 is open.
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

The unit tests use camelCase fixtures, copied from the transcript (the old spec's "live evidence" was a `.jsonl` file). AgentMux's own stored agent output (the `output` blockfiles in `filestore.db`, 0.59.1) contains the stdout form: 3 snake_case boundary lines. (The one camelCase hit in that store is prose in an agent's message quoting this spec, not a frame.) [verified by grep; not watched live in a pane]

**Consequence [inferred from the code, not observed in a running pane]:** the live path in `useAgentStream.ts` that handles `compact_boundary` gets `null` from the parser and dispatches nothing. That path also drives:
- the reducer's `CompactionBoundary` (clearing `compacting`, `lastCompactionBoundaryAt`, the live `context_compacted` node);
- `CompactionSummaryTracker.noteBoundary` (the compaction context-delivery card);
- **`memoryReinjectionController.trigger(...)`**, the hidden Global/Personal Memory reinjection after compaction (`SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md`).

**Correction (2026-10-02):** an earlier revision of this section said history replay reads the camelCase transcript and so hides the problem. It does not. `parseHistoryLines.ts` parses the lines of the `output` blockfile ("raw NDJSON lines as stored in the 'output' blockfile", its own doc comment), which are these same stdout lines, with the same camelCase-only `parseCompactBoundaryFrame`. So a reload misses the real boundary too. [verified from the code and the stored data] The `context_compacted` row from a real boundary therefore likely appears neither live nor after a reload; the only compaction row left is the live one from the `TokensIn` >=50%-drop heuristic (`source: "heuristic"`, not replayable). [inferred]

**Why this spec does not just fix it:** fixing the casing activates code that, if the above holds, has never run live. In particular, with no `timestamp` on the stdout frame, `memoryReinjectionNodeId(null)` gives one fixed id (`memory-reinjected-notime`) for every compaction in a session, so the second reinjection's node would be deduplicated away. (The live and replay `context_compacted` ids would agree, since both fall back to the same content-derived key when the frame has no timestamp, though a `uuid` key would be steadier.) That is a separate change with its own tests and a visible behavior change (a hidden message to the model after every compaction). It needs the owner's decision, §8 D1.

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

## 6. Status frames (decision D2) — built

The `status` frames in §2 give (a) a **hook-independent start** (the `PreCompact` hook arrives over a live-only WPS event that can be missed), (b) a **visible failure**, built in §10, and (c) a **liveness heartbeat**. (a) and (c) are built on D1, frontend only:
- **Reading the frame.** `compactionStatusCommand` (`compact-boundary.ts`) maps `status:"compacting"` to a `CompactionStatusFrame` command with `status: "compacting"`, and `status:null` with a `compact_result` (success or failed) to `status: "ended"`. `useAgentStream` calls it on every `system` frame, at receipt time: the frame has no `timestamp`. A failure still gets its row from `cli-notice.ts`.
- **Live only.** `parseHistoryLines.ts` never calls it. Lines the transcript cursor reads to fill a gap (`gap: true` on `deliver`'s `from`, `transcript-cursor.ts`) may be old or another writer's, so a start or heartbeat from one is ignored; an end frame from one still clears. The cursor already drops lines it has delivered.
- **Start.** When the pane isn't compacting and no hook ping is buffered, the reducer runs the hook's own `CompactionStarted` with all its guards: a no-op when unsubscribed, rejected at or before `lastCompactionBoundaryAt`, dropped on a terminal `Done`, and buffered in `pendingCompactionPing` on `Idle`/`Disconnected`/`Done.completed`, where the existing paths promote or discard it (`ReconcileTurnActive`, `TurnStart`, `TurnEnd`, unsubscribe). So an old frame can't set `compacting` on a pane that isn't working, and everything that cleared a hook-started compaction clears this one.
- **Trigger.** `CompactionState.trigger` stays `"manual" | "auto"`: the transcript node's label and the hook path need one. A status-started compaction is `manual` in the pane's own `/compact` turn (`pendingCompactTurn`), otherwise `auto`, the only way the CLI compacts by itself. A hook ping that lands later for the same compaction keeps the start time (the transcript node's key) and takes the hook's trigger. Left mislabeled: a typed `/compact <instructions>` reads as auto in the "Compacting conversation…" row.
- **Heartbeat.** While compacting, each frame sets `compacting.lastHeartbeatAt` (receipt time) and each repeat after the first counts in `compacting.heartbeats`. The working row (`AgentFooter`) appends "· no update from Claude for 1m 20s" once the last frame is older than 75 s (`HEARTBEAT_QUIET_MS`, 2.5× the ~30 s cadence) and at least one heartbeat arrived (`compactionQuietMs`, `compaction-estimate.ts`). A CLI that sends none, or a hook-only compaction, never shows it.
- **End.** `status:null` with a `compact_result` clears `compacting` and `pendingCompactionPing` without waiting for the boundary: a failure has none, and a success's boundary follows within milliseconds and still records `lastCompactionBoundaryAt` and its node. Before, a compaction that failed mid-turn stayed "Compacting…" until the turn ended.
- **Not done:** a hook ping arriving after a failed compaction's end frame isn't rejected (only a boundary sets the stale-start guard); the watchdog stays suspended for the whole compaction, with the hint as the visible signal. Not live-verified in a pane.
- **Tests:** `reducer.test.ts` (start without the hook, manual in a `/compact` turn, heartbeat only when already compacting, repeats counted, a later hook ping merged, the hook's guards, an old frame on an idle pane only buffered and then discarded, end clears with no boundary, end then boundary); `compact-boundary.test.ts` (the captured frames, gap reads, other frames, replay builds nothing); `compaction-estimate.test.ts` and `AgentFooter.test.tsx` (the hint only past 75 s and only after a heartbeat); `transcript-cursor.test.ts` (gap lines are marked).

Seeding the sample store from transcript history (so a first compaction already has an estimate) is left out; the store fills as compactions happen.

## 7. Tests

- `compaction-estimate.test.ts` (28 tests): both casings parse, with the real captured frame as the fixture; malformed, negative and non-finite input is rejected; the median is robust to an outlier; the scale clamps at 0.5× and 2×; the result clamps to 5 s / 600 s; no samples → `null`; the fill never exceeds 95% and `over` is reported; the store round-trips, de-duplicates by uuid, keeps the newest 10, reads corrupt or throwing storage as empty, and drops bad entries.
- `AgentFooter.test.tsx` (6 tests): no history → plain elapsed counter and no bar; history → `12s / ~30s`, a `progressbar` with `aria-valuenow`, and the "Estimate…" tooltip; overshoot → "longer than usual", indeterminate, fill held at 95%; the estimate does not move when a sample lands or the context size changes mid-run (R4); the next compaction estimates afresh; no bar while reconnecting. Checked to fail: removing the untracked read of the context size breaks the R4 test.
- **Not unit-tested:** the one-line `recordCompactionSample(parseCompactionSample(rawEvent))` call in `useAgentStream.ts` (that hook has no test harness). It is covered by the parser and store tests above and by the live check.
- **Live check, still to do on a dev build:** `/compact` a real conversation twice and watch the second one's bar and `12s / ~30s` text.

## 8. Decisions

- **D1 — fix the boundary casing (and re-key node ids on `uuid`) so the live compaction path actually runs?** **Done (2026-10-03).** The check first found a duplicate send: the controller deferred a mid-turn compaction and only claimed at turn end, by which time srv's 60 s claim window had dropped the `SessionStart` hook's delivery, so both sent the memory. Built:
  - both parsers (`claude.rs` `handle_system_message`, `compact-boundary.ts` `parseCompactBoundaryFrame`) read `compact_metadata`/`compactMetadata` and each field in either casing; malformed frames are still dropped;
  - boundary-derived ids are keyed on the frame's `uuid`: `contextCompactedNodeId` (and through it the summary card's fallback id) and `memoryReinjectionNodeId`. `timestamp` is only a time value, with the existing receipt-time fallbacks;
  - a compaction is claimed when its boundary arrives (`memory-reinjection-controller.ts`), with the boundary's `uuid` (`boundary_uuid` on `memorydelivery:claim_fallback`). "Skip" sends nothing later; "deliver" sends at turn end as before, without a second claim;
  - once per boundary, from the block that compacted: the controller ignores a `uuid` it has seen (a gap re-read, a double trigger). srv records which block's stdout carried each boundary `uuid` (last 256) and answers "skip" to a claim from any other block: another block of the same agent reads the boundary from the shared transcript zone, and its CLI did not compact. A boundary no local stdout carried (another srv instance's) is skipped the same way. A repeat claim by the origin block takes the usual path, so a pane that failed to send is not locked out. A claim without a `uuid` behaves as before. A fresh session still claims when it fires.
- **D2 — the rest of status-frame handling (§6: a hook-independent start, a heartbeat)** after D1, or not at all? **Done (2026-10-03), after D1; see §6.** A live `status:"compacting"` frame starts the compaction through the hook's `CompactionStarted` guards (trigger `manual` in the pane's `/compact` turn, else `auto`), never from history or a gap read; its repeats are heartbeats, and 75 s without one shows "no update from Claude" in the working row; the `status:null` end frame clears `compacting` when no boundary follows. The failure notice is built (§10).
- **D3 — sample store scope:** global (as built) or per account/model.

## 9. Delivery

PR 1 (#4220): this spec + Tier 4. PR 2 (#4232): the failed-compaction notice (§10), plus the §3 correction. D1 and the rest of D2 followed (§8).

## 10. Failed-compaction notice as built (PR #4232)

**Captured live (CLI 2.1.287):** with the fake API answering the summarizing call with HTTP 400, the CLI retried it 4 times, then wrote
`{"type":"system","subtype":"status","status":null,"compact_result":"failed","compact_error":"Error during compaction: API Error: 400 …","session_id":…,"uuid":…}` and ended the turn with `result` `is_error:false`, `num_turns:0`, with no `compact_boundary`. Nothing but that one frame says the compaction failed, so before this change the user saw the spinner stop and nothing else.

**Built (frontend only):** `parseCliNoticeFrame` (`cli-notice.ts`) turns that frame into a `cli_notice` node of a third kind, `compaction_failed`:
- label "Claude Code couldn't compact the conversation", detail = the CLI's reason with its "Error during compaction: " prefix removed, error tone;
- id `compaction-failed-<uuid>`: the same stdout line gives the same id live and on replay, so seeing it twice shows one row;
- no change at the call sites: `useAgentStream` and `parseHistoryLines` already run `parseCliNoticeFrame` on every `system` frame, and `cli_notice` is already handled everywhere a row must be (fixed height, never collapsible, no prompt text, rendering);
- other `status` frames (`compacting`, its heartbeat, `compact_result:"success"`) are ignored, as is a failed frame with no `uuid`.

**Not done:** it doesn't touch the reducer (the turn's end already clears `compacting`), the boundary path (§3), or the start/heartbeat uses of the status frame. Not live-verified in a pane: a real failed compaction is hard to provoke, so the evidence is the captured frame as the test fixture.
