# Report: the ambient (Haiku) framework, reassessed top down

**Status:** analysis
**Date:** 2026-10-08 · **Author:** agent2
**Trigger:** the repo owner saw this as the agent pane's next-prompt ghost text:

> Output nothing at all - the assistant's message ends by asking for a decision and waiting for user input.

and asked for a top-down reassessment of the ambient system that feeds the Swarm and the next-message suggestion, looking for duplication and bad architecture.

**Builds on:**
- `REPORT_NEXT_PROMPT_SUGGESTION_NONSENSE_AND_AMBIENT_DRY_2026_10_01.md`: the previous round. Section 4.1 shows how its fix produced today's bug.
- `SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`
- `SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md`

---

## 1. Verdict

The plumbing is sound. The *contract with the model* is not, and that is what keeps producing visible nonsense.

- **Plumbing.** One gateway admits, supersedes and cancels per `(entity, purpose)`, `call::Slot` records one outcome per call, and the counters are visible.
- **The contract.** Each purpose asks Haiku for free text, and gives it a different way to say "nothing":
  - an empty reply;
  - the word `KEEP`;
  - "output nothing at all", which a language model can't reliably do.

  What comes back is then patched with a growing set of phrase blocklists, a sanitizer that rewrites replies, and bracket heuristics. Every round so far has added a sentence to a prompt and a phrase to a list. The model then finds a new wording, and this one ("Output nothing at all - …", no brackets, no blocklisted phrase) went straight through.

Three other problems are structural, not cosmetic:

1. **A third of next-prompt calls time out.** Every call spawns a full `claude` CLI with a 15 s budget. Subagent naming has the same problem.
2. **Decisions that code can make are delegated to the model.** "Did the assistant just ask the user something?" is decided by Haiku from a digest. The digest already contains the assistant's last message.
3. **Each purpose's settings are spread over six or more files in two languages.** One caller (the continuity state) bypasses the framework's call path entirely.

The fix is a strict reply format for every purpose, deterministic checks before calling, one purpose registry, and a shared execution budget. Section 6 has the design, and section 7 a PR sequence. The first PR fixes the reported bug on its own.

## 2. What the system is

Eight purposes, all on `claude-haiku-4-5` through `claude -p`:

| Purpose | Trigger | Input | Output goes to | Timeout | Cap |
|---|---|---|---|---|---|
| `activity_summary` (pane/session title) | frontend, on `Submitting`, on turns 1, 2, 5, 8… (`title-schedule.ts`) | the user's message, or the digest | frontend writes `term:ambient_summary` | 15 s | pull (2) |
| `activity_summary_pushed` (title recovery) | backend sweep every 20 s, untitled running agents | digest | backend writes `term:ambient_summary` | 15 s | sweep (2) |
| `next_prompt_suggestion` | frontend, on the backend's turn-end edge | digest | frontend writes `term:next_prompt_suggestion` | 15 s | pull (2) |
| `subagent_name` | Swarm or dock row first expanded; backlog pass | the subagent's whole first message | `SubAgent.display_name` | 15 s | pull (2) / backlog (1) |
| `dispatch_name` | first workflow member seen live; backlog | the first member's whole first message | dispatch name | 15 s | pull / backlog |
| `definition_summary` | `listrecentsessions`, a row with no snapshot | digest | DB, then `agents:changed` | 15 s | 1 |
| `ambient_narration` | a background-launched tool call | the command | an event the pane renders | 15 s | narration |
| `continuity_state` | every successful turn of a Claude agent | up to 30k chars of new turns | a versioned state file | 90 s | **none** (one per agent) |

**Where one purpose's settings live:**

| Setting | Where |
|---|---|
| Purpose tag | `ambient/purpose.rs`, except `continuity_state`, which is in `backend/continuity_state.rs` |
| Prompt | `ambient/prompt.rs`, except continuity's, in `continuity_state.rs` |
| Size limits | `ambient/validate.rs` |
| Validator | `validate::accept_line` / `accept_next_prompt`, or continuity's `accept_state` |
| Timeout | `ambient/cli.rs`, or continuity's own constant |
| Concurrency | `ambient/limits.rs`, `reactive/activity_watcher.rs`, `continuity_state.rs` |
| Caller | `ambient/tasks.rs`, `server/app_api/session.rs`, `continuity_state.rs` |
| Title validation (TS port) | `frontend/app/store/ambient-title.ts` |
| Display names | `frontend/app/store/ambient-outcomes.ts` |

## 3. Evidence: four days of outcome logs

Every ambient call logs one `ambient outcome` line (`ambient/outcome.rs`). Aggregated over this machine's srv logs, 2026-10-04 to 2026-10-08, across every local build that ran:

| Purpose | Calls | Accepted | Kept (`KEEP`) | Rejected | Timeout | Other |
|---|---:|---:|---:|---:|---:|---:|
| `next_prompt_suggestion` | 1,089 | 33.3% | 0 | 30.9% | **33.5%** | 2.3% |
| `activity_summary` | 235 | 15.7% | 77.4% | 0 | 5.1% | 1.7% |
| `activity_summary_pushed` | 113 | 23.0% | 5.3% | 0 | 0.9% | 70.8% (69% `empty_digest`) |
| `subagent_name` | 166 | 57.2% | 0 | 6.6% | **29.5%** | 6.6% |
| `ambient_narration` | 242 | 96.3% | 0 | 0 | 3.3% | 0.4% |
| `definition_summary` | 4 | 3 | 0 | 0 | 0 | 1 |
| `continuity_state` | not recorded (section 5.4) | | | | | |

### Next-prompt rejections

- 237 empty replies: the model correctly said nothing, but counted as *rejected*.
- 83 caught by the absence check, nearly all the model *describing* its abstention in brackets:
  - `(no output)`
  - `(nothing)`
  - `(No output - the assistant's last message asks the user a question and waits for a decision)`
  - `[Output nothing - the assistant's last message explicitly waits for user decisions on three matters.]`
  - …about fifteen more wordings.
- 5 refusals in the "I cannot predict a next instruction because…" form.

### What reached the user

- The reported line is the same abstention without brackets. Nothing in `accept_next_prompt` matches it, so it was counted as *accepted*.
- Accepted texts aren't logged, so how many of the 363 "accepted" were abstentions can't be measured today. The rejected ones put a floor on the habit: at least 25% of the calls that returned text tried to abstain in prose or with an empty reply.

### Subagent-name rejections

These are the model *doing the subagent's task* instead of naming it: "I cannot access the repository files…", "I don't have access to the … repository at the specified path…". The task prompt is passed in whole, so instructions inside it win over "give a ~5-word name".

## 4. The reported bug

### 4.1 How it gets through

1. `build_next_prompt_prompt` says:
   > Output nothing at all if any of these hold: … the assistant's last message asks the user a question or waits for a decision.

   The shared system prompt says "If the instruction cannot be met from the material, output nothing at all." Both were added by the 2026-10-01 round to stop refusals.
2. An empty reply is not something the model produces reliably. It often writes a sentence *about* producing nothing instead, and here it quoted the instruction it was following.
3. `accept_next_prompt` lets it through:
   - one line, about 18 words and 105 characters, within 28 words and 160 characters;
   - it doesn't end in `?`;
   - none of the 19 refusal phrases, 12 meta phrases or 16 risky phrases occur in it ("the assistant's" isn't listed, "user input" isn't);
   - it isn't wrapped in brackets, so `is_wrapped_note` doesn't fire.
4. The frontend doesn't validate (`useNextPromptSuggestion.ts:178`) and writes it to `term:next_prompt_suggestion`. `AgentFooter` shows it as the composer placeholder.

### 4.2 Why another phrase isn't the fix

Adding "output nothing" to `NEXT_PROMPT_META_PHRASES` would catch this wording and not the next one. The 10-01 report added the "output nothing" instruction to stop refusals. It succeeded, and produced this instead. Section 6.1's format makes *any* text that isn't an answer fail to parse, whatever its wording.

### 4.3 Asking the model a question code can answer

Whether the assistant's last message asks the user something is visible in the digest's newest `[assistant]` entry. The pane also knows it from the turn itself (an `AskUserQuestion` or permission request, or text ending in a question). Asking Haiku to judge it costs a call, and the answer comes back unreliably and in prose. The 237 empty and 83 prose abstentions above are mostly this case.

## 5. Findings

Severity: **P1** user-visible wrong output or a large waste; **P2** structural; **P3** cleanup.

### 5.1 P1 — no reply format; three ways to say nothing

- **Next-prompt and the system prompt:** "output nothing at all".
- **Both title prompts:** reply `KEEP`. `KEEP` is then special-cased in four places:
  - `validate::ABSENCE_EXACT` and `leads_with_abstain_token`;
  - `outcome::classify_reply`, as `Kept`;
  - its own prompt-builder tests.
- **Names, previews, narration:** no way to abstain at all.

On top of this sit three filters:
- **Blocklists** (`REFUSAL_PHRASES`, `NEXT_PROMPT_META_PHRASES`, `RISKY_PHRASES`, `ABSENCE_EXACT`, `ABSENCE_PREFIXES`), with exceptions added after false rejects ("No title bar on Windows", "Keep alive pings").
- **`is_wrapped_note`**, a bracket heuristic.
- **`sanitize_ambient_text`**, which *rewrites* replies: fences, quotes, and openers such as "let's go ahead and".

The outcome classifier then labels a correct empty abstain `rejected:empty`. So the "Rejected" figures mix healthy abstentions with real failures, and can't be read as a quality signal.

### 5.2 P1 — the CLI per call: a third of calls time out

- **What runs:** every call is a fresh `claude -p --model claude-haiku-4-5-20251001 …` process (`ambient/cli.rs`), a full Claude Code start for one line of text, under a 15 s limit.
- **Timeout rates:** next-prompt 33.5%, subagent naming 29.5%, titles 5.1%.
  - Subagent naming sends uncapped inputs (5.7), and a next-prompt call waits behind at most one other pull call.
  - So the time is most likely going on process start and long prompts, not on Haiku. That's an inference: outcome lines carry no duration, so it can't be measured today (5.9).
- **A timeout probably still costs money.** The child is killed before its `result` line, so its tokens are never counted, while the API call may already have been billed.
- **The model id is a literal** in one place, and every error string says "activity CLI" whatever the purpose.

### 5.3 P1 — subagent names: a free answer is ignored, and the input is unbounded

Claude Code's `Agent` tool makes the *parent* model write a 3–5 word `description` for every subagent. It is in the parent's `tool_use`, which the pane already links to the subagent (`frontend/app/view/agent/activity/dispatch-correlation.ts`). `subagent_watcher` never reads it, and asks Haiku to name the task from the subagent's whole first message (`read_task_prompt`, no size cap) instead. That one change would remove most naming calls, their 29.5% timeouts and the injection failures.

### 5.4 P2 — one purpose bypasses the framework

`continuity_state.rs` admits through `gateway()` itself and calls `cli::invoke_haiku_with_timeout` directly, not through `call::Slot`. So:
- no outcome is recorded, and it is invisible in the Instance panel's stats;
- `PURPOSE_NAMES` in `ambient-outcomes.ts` has no entry for it;
- its purpose constant lives outside `purpose.rs`;
- it has no global concurrency cap, only one call per agent;
- it has its own validator (`accept_state`) and its own chatter trimmer (`without_trailing_chatter`).

### 5.5 P2 — six independent concurrency caps, no global bound

| Cap | Limit | Covers |
|---|---|---|
| `pull_call_semaphore` | 2 | titles, suggestions, on-click names |
| `definition_summary_semaphore` | 1 | definition summaries |
| `backlog_naming_semaphore` | 1 | backlog naming |
| `narration_semaphore` | its own | narration |
| `activity_watcher` sweep | 2 | title recovery |
| continuity | none | one per agent |

Each choice is justified locally ("a background burst must not block a live request"). But the number of `claude` processes alive at once is bounded by none of them, and a busy machine with many agents gets them all at once.

### 5.6 P2 — the digest only reads Claude's stream format

`digest::extract_digest_parts` reads Claude's `assistant`/`user` stream-json frames. For Codex, Gemini, Kimi, Qwen, Copilot and the ACP panes it returns nothing, so:
- those panes never get a title, a suggestion or a definition preview;
- the recovery sweep re-admits them every time their output grows, which is the 69% `empty_digest`, and never succeeds.

srv already has a provider-neutral layer for this (`agents::translator`: Claude, Codex and Gemini to a shared `AgentEvent`), and the digest doesn't use it.

### 5.7 P2 — material read as instructions

`with_material` puts the input in a tagged block, which helps for digests, but a subagent's task prompt is itself an instruction to an agent ("Investigate X and report file:line…"). With no size cap and no format the reply must follow (5.1), Haiku does the task. The format in 6.1 and an input cap fix both.

### 5.8 P2 — frontend duplication and two real defects

**Defects**
- **A hidden suggestion can still be inserted** (`AgentFooter.tsx:1117-1122`, verified). Tab and → read `term:next_prompt_suggestion` directly. At send time the placeholder hides the old suggestion (`suggestionGenMaskedAtSend`), but the meta still holds it until the async clear lands, or for the whole turn if that write fails. Pressing Tab in that window inserts the stale text.
- **A call is spent while you type** (`useNextPromptSuggestion.ts:166-183`, verified). The RPC is sent at turn end whatever is in the composer; the composer is only checked when the reply arrives.
- **Two mount-time gaps.**
  - The hook mounts for every pane with no provider check (`agent-view.tsx:779`), so non-Claude panes call an RPC that can only return empty (5.6).
  - `lastTurnWasHidden` resets on remount.

**Duplication**
- **Two copies of the pull pattern.** The suggestion and title hooks each have their own turn counter, `generation: Date.now()`, 20 s timeout, silent catch, `recordTurn`, then a meta write. The counters advance on different events, and neither checks for unmount.
- **`GenerateName` and `recordTurn`** are copied three times: `swarm-view.tsx:1217`, `ActivityDock.tsx:220` and `:233`. All three ignore the reply and type it as `any`.
- **`subagent:named`** is handled in three places, two of them identical (`swarm-model.ts:1325`, `subagent-source.ts:70`).
- **Five fallback chains for a subagent or dispatch name.** One ignores `dispatch_name`, and one keeps the shared-slug collision the Swarm fixed.
- **The title validator is ported by hand** (`ambient-title.ts`). It's kept in step by a shared JSON corpus that both test suites load. That stops drift, but every rule change is still two changes. The `KEEP` handling and absence lists exist only because of 5.1.
- **Meta key names are literal strings.** `term:ambient_summary`, `term:next_prompt_suggestion` and `…_gen` are repeated in about 10 places, while the newer keys have constants.

**Writers and triggers**
- **Several writers per key.**
  - `term:ambient_summary`: the title hook, the session-end clear (`useBlockActivity.ts`), and the backend sweep (`activity_watcher.rs`).
  - `term:next_prompt_suggestion`: the hook, and the session-end clear, which skips the generation bump.
- **Two triggers for one moment.** Titles fire on the frontend `Submitting` phase, suggestions on the backend turn-end edge, and hidden turns are detected differently in each.

**Token accounting**
- **Five purposes' tokens never reach the status-bar totals**: `dispatch_name`, `definition_summary`, `activity_summary_pushed`, `ambient_narration` and `continuity_state`. The backend logs them and drops them.
- `token-usage.ts` says "four ambient call sites"; there are five.

### 5.9 P2 — what the logs can't tell you

- **Accepted replies aren't logged.** That is why the reported line couldn't be found in the logs, and why the share of accepted abstentions can't be measured.
- **No duration on the outcome line.** The 33% timeout can't be split into process start, queue wait and model time.
- **`classify_reply` and `rejection_reason` are written for titles.** They judge "too long" by `STORED_TITLE`, and treat `KEEP` as a title concept.
- **An empty abstain counts as `rejected:empty`.**

### 5.10 P3 — smaller defects

- **A mislabelled fallback** (`session.rs:352-358`). When the frontend sends no user message, `activity_summary` substitutes the multi-turn *digest*. The prompt then presents it as *"The user just said: …"*.
- **"Nothing to send" recorded two ways.** `activity_summary` calls `abandon(EmptyDigest)`; `next_prompt_suggestion` just returns, which records `not_run`.
- **`word_target` is computed from the pane width** for a title the pane header no longer shows (`agent-model.ts` `viewText = () => []`).
- **The outcome stats are polled every 12 s** whether or not the Stats panel is open, and summarised twice.
- **About a dozen stale or contradictory comments.**
  - Several files say `turnJustEndedAtom` drives the title.
  - `ActivitySummaryResult.ts` says the backend writes the title; `session.rs` says the frontend does.
  - The title is still described as a "per-turn paraphrase" or as shown in the header.
- **No AI marker on definition previews.** `RecentSessionRow.preview` looks the same as a real user prompt. Narration, by contrast, is tagged.

## 6. Target design

### 6.1 One reply format for every purpose

Every one-line purpose asks for exactly one of two replies:

```
ANSWER: <the text>
SKIP
```

`parse_reply(raw, purpose) -> Reply` is the only reader:

| The model's raw reply | Parsed as | Outcome |
|---|---|---|
| exactly `SKIP` (case and trailing punctuation ignored) | `Skipped` | `skipped`, a healthy result |
| `ANSWER: x`, where `x` is one line within the purpose's limits | `Answer(x)`, then the purpose's own semantic check (risky words for suggestions, absence words for titles) | `accepted`, or `rejected:unsafe` / `rejected:absence` |
| anything else: an empty reply, prose, a bracketed note, a refusal, several lines | `Malformed` | `rejected:format`, raw text logged |

Why this ends the cycle:

- **The default flips.** A reply is used only if it has the right form, not shown unless it matches a blocklist. "Output nothing at all - …", `(no output)`, "I cannot predict…" and an empty reply all lack the `ANSWER:` prefix, so all are discarded, without anyone predicting the wording.
- **`SKIP` replaces every abstain route.**
  - `KEEP` in the title prompts, "output nothing" in the next-prompt prompt and the system prompt, and the missing route for names.
  - The `KEEP` special cases, `is_wrapped_note`, `leads_with_abstain_token`, `REFUSAL_PHRASES`, `NEXT_PROMPT_META_PHRASES`, most of the absence lists, and the opener-stripping in `sanitize_ambient_text` all go, in Rust and in the TS port.
  - `RISKY_PHRASES` stays: that is a decision about the content, not the format.
- **Stored titles must still be judged** (`is_usable_title`), for values written by older builds. That shrinks to a small, frozen legacy list instead of a list that grows every week.
- **Material can't hijack the reply as easily** (5.7). A model that starts doing the subagent's task doesn't produce `ANSWER: <5 words>`.

The continuity state keeps its multi-line format (required heading) under the same rule: a block that doesn't start with `## Last user request` is `rejected:format`. That's what `accept_state` already does; it moves behind the same call path.

Before rolling out, the format needs a measurement run: the live smoke test in `ambient/cli.rs`, extended to a fixed set of about 30 recorded digests, including the abstention cases above. It should show Haiku 4.5 keeps the `ANSWER:`/`SKIP` form above 95%.

### 6.2 Decide in code what code can decide

Before calling the model, each purpose checks its preconditions:

- **next_prompt_suggestion**
  - **Skip when the turn ended waiting for the user:**
    - the newest digest entry is `[assistant]` and ends in `?`;
    - or the turn's last tool was `AskUserQuestion` or a permission request.
  - **Skip when the turn didn't end normally:** errored, stopped or interrupted.
  - **Skip when the newest entry is the user's** (the assistant hasn't replied).
  - **Skip when the composer isn't empty at turn end.** The frontend sends `composer_empty`, or doesn't call at all.
  - This turns most of today's abstentions into no call at all.
- **activity_summary:** no change; it already skips when nothing is new.
- **Title recovery sweep:** don't admit a block whose provider has no digest. With 6.5 that becomes "a provider without a translator".

### 6.3 One purpose registry

Replace the scattered constants with one table, `ambient::purpose::ALL: &[PurposeSpec]`:

```rust
pub struct PurposeSpec {
    pub tag: &'static str,              // "next_prompt_suggestion"
    pub label: &'static str,            // shown in the Stats panel (replaces PURPOSE_NAMES)
    pub reply: ReplyShape,              // Line(Limits) | Block { required_heading }
    pub check: fn(&str) -> Verdict,     // purpose-specific semantics after parsing
    pub timeout: Duration,
    pub class: Priority,                // Interactive | Background
    pub counts_toward_totals: bool,     // token accounting (5.8)
}
```

- **One entry point**, `ambient::run(spec, entity, generation, prompt, target)`, used by every caller, the continuity state included. It does admit, schedule (6.4), invoke, parse (6.1), check, and record the outcome with duration and, at debug level, the accepted text.
- **Prompts stay functions in `prompt.rs`**, each ending with the shared format instruction.
- **The frontend's display names** come from `PurposeSpec::label`, served by the outcomes RPC, not a second list in TS.

### 6.4 One execution budget

- **One scheduler** with a global cap (start at 3) and two priority classes. Interactive work (titles, suggestions, on-click names) goes ahead of background work (sweep, backlog, previews, narration, continuity). It replaces the six semaphores; a background call can still never block an interactive one, which is what each separate semaphore was for.
- **Measure, then decide how calls run.** Add `duration_ms` and `queued_ms` to the outcome line first. If process start dominates, as expected, the choice is between:
  - a longer timeout, the cheapest option, though it doesn't fix the latency;
  - **one long-lived Haiku worker per auth identity** (`claude -p --input-format stream-json`, reused across calls, the same way persistent agent panes work), which removes the start-up cost.

  The second is the real fix, but it should be taken on measured numbers, not assumed. It also makes timed-out spend countable: the worker survives and reports usage.
- **The model id** becomes a `PurposeSpec` field, or one constant.

**Measured 2026-10-08, and what was decided.** The exact ambient command, run from the real `ambient-calls` folder with an agent's own `claude` (2.1.288) and identity environment, takes 1.2–1.6 s, and `claude --version` starts in 0.1 s. Process start does **not** dominate, so no worker. Prompt size doesn't explain it either: a call costs about 430 prompt tokens there. (From a folder with large `CLAUDE.md` files above it, the same call costs 17,500 tokens, which is why ambient calls run in their own folder.) The 15 s timeouts are most likely the API itself: many agents on one login, with the CLI retrying on overload. The fix taken, in PR 4:
- **Time limits per class:** 30 s interactive, 45 s background, 90 s for the continuity summary. The pane's RPC timeout went from 20 s to 45 s to outlast them.
- **Two classes, each with one cap:** interactive 2, background 3, so 5 at once in total. They replace the six semaphores, and the continuity summary, which had no cap, now has one.

`run_ms` and `queued_ms` on the outcome lines (PR 1) will show whether the longer limits recover the timed-out calls.

### 6.5 Provider-neutral digest

Build the digest from `agents::translator` `AgentEvent`s instead of raw Claude frames. Claude, Codex and Gemini translators exist; Kimi, Qwen and the ACP panes get one as each provider is moved to its rich protocol. Titles and suggestions then work for every provider with a translator, and the sweep stops spinning on the rest.

The hidden-reinjection filter stays. It is about AgentMux's own message, not the provider's format.

### 6.6 Subagent and dispatch names

- **Use the parent's `Agent` `description`** as the subagent name, and the first member's description for a workflow dispatch, when present. Correlation by `tool_use_id` already exists.
- **Call Haiku only when there's no description**, with the task prompt capped (about 1,500 characters, head and tail), under 6.1's format.

### 6.7 Frontend consolidation

- **`useAmbientPull({ purpose, trigger, request, onResult })`** replaces the bodies of both hooks: turn counter, generation, timeout, unmount guard, `recordTurn`, write.
- **The title is written once, by the backend.** `session:activity_summary` writes `term:ambient_summary` itself, guarded by generation like the recovery sweep's transaction (`store_recovered_title`). Two writers become one code path, and the frontend's `isTitleNews` check moves to Rust next to the validator.
- **Tab/→ accept checks the same mask** the placeholder uses (`suggestionGenMaskedAtSend`), so a hidden suggestion can't be inserted.
- **Three duplicates become helpers:**
  - **Names:** one `requestSubagentName(id)` for all three call sites, typed.
  - **`subagent:named`:** one handler.
  - **Labels:** one `subagentLabel()` / `dispatchLabel()` for every name fallback.
- **One `meta-keys.ts`** for every `term:*` key both sides use.
- **Token accounting:** the background purposes' tokens go into the status-bar totals (`PurposeSpec::counts_toward_totals`), served by the backend.
- **Gate the suggestion hook** on providers that can produce a digest (6.5).

## 7. Plan

Each PR is independently useful and reviewable. Order by value.

| PR | Scope | Fixes |
|---|---|---|
| **1. Next-prompt: format, gates, accept fix** | `ANSWER:`/`SKIP` for `next_prompt_suggestion` only; 6.2's checks (question, waiting, turn outcome, composer); the Tab/→ mask check; log accepted text at debug; `duration_ms` on outcome lines. A recorded-digest live test (6.1). | The reported bug and its whole class; an end to calls that can only abstain. |
| **2. One format and one registry** | `PurposeSpec` and `ambient::run`; `ANSWER:`/`SKIP` for every one-line purpose; continuity through the same path; retire `KEEP`, the phrase lists and opener-stripping in Rust and TS (keep a frozen legacy check for stored titles); outcome labels `skipped` / `rejected:format`. | 5.1, 5.4, 5.9, most of 5.8's validator duplication. |
| **3. Subagent names from `description`** | Read the `Agent` description; Haiku fallback with a capped input. | 5.3, 5.7, most naming calls and their timeouts. |
| **4. Execution** | One prioritized scheduler; measure with PR 1's durations; then a worker or a timeout change, as section 6.4 decides; count spend on timeouts. | 5.2, 5.5. |
| **5. Provider-neutral digest** | Digest from `AgentEvent`s; sweep and suggestion gated by translator support. | 5.6, the sweep's `empty_digest` spin. |
| **6. Frontend consolidation** | `useAmbientPull`, backend-only title writes, the name and label helpers, `meta-keys.ts`, accounting for every purpose, stale comments. | 5.8, 5.10. |

PR 1 is small and stands alone. PRs 2 and 3 can run in parallel. PR 4 should follow PR 1's measurements.

## 8. Not proposed

- **Replacing Haiku or the gateway.** The model is adequate once its replies have a format, and the admit/supersede/cancel design is right.
- **Calling the Anthropic API directly from srv.** The ambient calls deliberately run through the user's own `claude` login and auth environment. A direct API path would need its own credentials and a decision about the user's subscription terms. That's a separate question, not a cleanup.
- **Removing the hidden-reinjection safeguards** (`HIDDEN_REINJECTION_BLOCKS` and the marker scan). They exist for a real leak and are orthogonal to everything here.

## 9. Method

- **Read in full:**
  - `crates/srv/src/ambient/` (`mod.rs`, `purpose.rs`, `call.rs`, `cli.rs`, `prompt.rs`, `validate.rs`, `outcome.rs`, `digest.rs`, `tasks.rs`, the start of `sanitize.rs`, `limits.rs`);
  - the two pull handlers in `server/app_api/session.rs`;
  - `backend/continuity_state.rs` (the call path);
  - `backend/reactive/activity_watcher.rs`;
  - `subagent_watcher::read_task_prompt`;
  - `agents/translator/mod.rs`.
- **Frontend:** mapped with file and line references. The three key claims were re-checked by hand: the Tab/→ accept path, the trigger that ignores the composer, and the hand-ported title lists.
- **Logs:** the outcome figures are every `ambient outcome` line in this machine's srv logs from 2026-10-04 to 2026-10-08. Rejected texts that name local files or accounts are not quoted here.
- **History:** the previous round's report (2026-10-01) and the ambient specs listed at the top.
