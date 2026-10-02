# SPEC: The swarm always shows a useful line for every agent — hardening the ambient summary

**Date:** 2026-10-02
**Status:** active — PR 1 shipped in #4185, PR 2 in #4234, PR 3 in #4238, PR 4 in #4243; enforcing grounding (the last step of PR 4) is not decided. The section 9 decisions were answered on 2026-10-02 (the recommendations, see 9.1).
**Author:** AgentX (narko), at the owner's request
**Affects:** `crates/srv/src/ambient/` (`prompt.rs`, `validate.rs`, `tasks.rs`), `crates/srv/src/server/app_api/session.rs`, `crates/srv/src/backend/reactive/activity_watcher.rs`, `frontend/app/store/activitySummary.ts`, `frontend/app/view/agent/hooks/useAgentActivitySummary.ts` and `useBlockActivity.ts`, `frontend/app/view/swarm/swarm-model.ts` and `swarm-view.tsx`.
**Builds on:** `docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md` (the title prompt this spec corrects), `docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md` (the gateway), `docs/specs/SPEC_AMBIENT_SUMMARY_SANITIZATION_AND_TERSENESS_2026_07_08.md`.
**Related:** `docs/reports/REPORT_NEXT_PROMPT_SUGGESTION_NONSENSE_AND_AMBIENT_DRY_2026_10_01.md` (the same family of failure in the ghost text; #4143 and #4154 fixed refusals and consolidated the call path, but not the title), `docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md`.

## 1. What the owner saw

In the Swarm view the row for AgentX read **`(none yet)`**, although AgentX had been in conversation with the owner for hours. The owner's expectation, which this spec adopts as the requirement: **every agent row always shows a meaningful, honest line**, and when nothing better exists the line says what the agent is doing in plain words (for example "Summarizing work completed"), never a placeholder.

## 2. What happened, and how sure we are

Investigation was read-only. The live object store was copied (database and write-ahead log) and the copy was queried; nothing on the running instance was touched.

| Claim | Evidence | Confidence |
|---|---|---|
| AgentX's stored `term:ambient_summary` was literally `(none yet)` | 15 copies of AgentX's block record in the write-ahead log carry that exact value, all before a copy carrying `Develop hardening spec for swarm ambient summary quality`, which is what the block holds now | **Proven** |
| That string is the prompt's own placeholder | `build_session_title_prompt` writes `Current title: (none yet)` into the model's input whenever the stored title is empty (`ambient/prompt.rs:37`) | **Proven** (code) |
| The validator cannot reject it | `is_placeholder` matches only the bare words `none`, `n/a`, `na`, `nothing`, `empty`, `null` after trimming punctuation; `(none yet)` becomes `none yet`, which is not in the list (`ambient/validate.rs`). It is 2 words, alphabetic, and has no refusal phrase | **Proven** (code, and the stored value is the proof it passed) |
| Once stored it is fed back and re-echoed | The next call reads the stored title as `current_title` (`app_api/session.rs:361`), which is now non-empty, so the prompt shows `Current title: (none yet)` as if it were a real title, and says "if the current title still accurately describes the overall goal, repeat it back EXACTLY" | **Proven** (code); that the model *did* repeat it on later calls is consistent with the 15 copies but the copies alone do not prove 15 model calls, because the block record is rewritten for any meta change |
| It started on the first call after the restart | This block (`ee19694d`) is new since the move to 0.59.4, and `useBlockActivity.ts` clears `term:ambient_summary` on a session boundary, so the title starts empty. The first user message sent to the new block was a two-word nudge | **Inferred**: the exact message that produced the first echo is not recorded |
| A second agent shows the same class of failure | AgentY's stored value was `no goal established yet` before `Review available work items and tasks`. It is also placeholder-shaped, 4 words, and passed | **Proven** (stored value) |

Why the row showed text at all: `swarm-view.tsx` hides the line when `activitySummary` is empty (`<Show when={node.activitySummary}>`). A blank row is therefore the *good* outcome of an empty title; the bad outcome is a stored string that is not a title. That is what the validator let through.

### 2.1 Defects found on the way

These are not what put `(none yet)` on screen, but each lets the swarm show nothing or something wrong, so they belong in the same hardening.

1. **A placeholder is fed to the model as data.** The same pattern exists in `continuity_state.rs:129` (`(none yet: this is the first state for this conversation)`) and in the title prompt's `(no new message — re-evaluate from the title alone)`. Anything in band can be echoed.
2. **The title only changes when a human submits a message.** `useAgentActivitySummary.ts` fires on `TurnPhase` entering `Submitting`, skips hidden turns, and is deferred at mount. An agent driven mostly by jekts or tool work, or one reattached mid-turn, can sit on an empty or stale title indefinitely.
3. **The title is cleared on every session boundary** (`useBlockActivity.ts`), so every restart begins blank, and a restart is exactly when the owner looks at the swarm.
4. **There is no fallback and no last-known-good.** When the call returns nothing, the title is left as it is (empty).
5. **A second pipeline produces summaries nobody sees.** The backend sweep in `activity_watcher.rs` runs a Haiku call every 20 s per running agent and publishes `agent:summary`; **no frontend code subscribes to that event**. It spends model calls and its output goes nowhere. It could be the fix for defect 2, or it should be removed.
6. **The module is silent.** Nothing in `crate::ambient` logged at info or warn during the whole run of this instance, and its failure paths log at debug. A rejected, empty, or failed summary leaves no trace, which is why this took a database copy to diagnose.
7. **The digest can have no conversation in it.** The 2026-10-01 report shows the digest can be only `[summary] N turns, $X total cost`; the sweep and the title fallback use the same digest.

## 3. What the research says

Sources are listed in section 10. They are design notes and merged PRs from open-source projects, not standards, and each claim below was checked against the page itself. One claim from a search summary, that Claude Code exposes a `generate_session_title` control request, was **not** supported by the cited issue and is not used.

1. **Keep the placeholder out of the model's input.** Store *null* for "no title" and let the presentation layer show a fallback; never hand the generator a stored placeholder as if it were content. (OpenProgram's session-naming design shows a preview of the first message when the title is empty; the `lys` project moved to `title: null` and a first-wins write `WHERE title IS NULL`.) Echoing a placeholder is the failure mode this prevents; the sources do not discuss the echo itself, so that part is inference from the evidence in section 2.
2. **Structured, validated output with bounded retry.** `lys` (PR #46) requires a schema-constrained reply, rejects an empty, blank, oversized or malformed title, retries *only* output errors up to a fixed count (3), never reports a title failure as a chat failure, and logs every outcome with structured fields.
3. **Re-evaluate on a schedule and replace conservatively.** `muxterm` (PR #288) checks after completed human turns 2, 5, 8 and every third after, feeds the four most recent human messages with the newest first and a 1,200-character cap, replaces a generated title only if it still matches the previous generated title, and stops entirely once a title is set by hand. An OpenClaw proposal adds a minimum-distance check so near-identical titles do not churn.
4. **Resumed sessions keep their names.** The Claude Code feature request (issue #47176) limits auto-titling to *new* sessions and lets a manual rename outrank a generated one. For AgentMux the equivalent is: restoring the last title on resume beats regenerating a blank.
5. **Degrade through a deterministic layer that never depends on the model.** Write-ups on LLM fallbacks agree on a tiered ladder ending in a deterministic, non-LLM rendering, and on telling the UI which tier produced the text (a `fallback_type`-style flag) so a degraded result is never presented as a model's judgement. "Raw information is better than no information."
6. **A nullable field gives the model a legitimate way to say "unknown".** With no allowed way to abstain, a required string pushes a model toward an invented placeholder. Structured-output guidance for 2026 recommends null for unknown values. Here the equivalent is an explicit abstain token the validator treats as "no title", instead of free text.
7. **Constrained decoding guarantees shape, not truth.** A reply can be valid and still wrong ("valid but wrong"), so the code-side check must judge meaning, not only form.

## 4. Principles

- **P1 Never blank, never a placeholder, never a refusal.** Whatever is stored or shown is a real title, a restored one, or a clearly labelled deterministic fallback.
- **P2 The model never sees our placeholders.** Absence is represented by absence, not by a string.
- **P3 A stored value is validated again before it is trusted.** The write path is not the only gate: values already in a database from before this change, or written by an older build, are judged on read and on feed-back.
- **P4 The fallback never depends on the model**, so the swarm is useful when Haiku is down, slow, rate-limited or unauthenticated.
- **P5 Say which source produced the text** (`generated`, `restored`, `heuristic`, `status`), so the UI can show a fallback as a fallback and a later change can tell them apart.
- **P6 Every outcome is observable.** A rejected or failed call is a counted, logged event, not silence.
- **P7 A user's or agent's explicit title always wins** and is never overwritten by a generated one.

## 5. Design

### 5.1 One predicate for "is this a title?" (`ambient::validate`)

Replace the five-word list in `is_placeholder` with a single predicate, `is_usable_title(text, source_material)`, used by **every** place that accepts, feeds back or displays a title:

1. **Shape** (unchanged): one line, within the word and character limits, at least one letter.
2. **Absence patterns.** Reject text that is *about* the absence of a title rather than a title. Normalise first (lower-case, strip surrounding brackets, quotes and punctuation, collapse spaces), then reject when the whole text is, or begins with, any of: `none`, `none yet`, `no title`, `no goal`, `no goal established`, `no goal yet`, `untitled`, `unknown`, `not set`, `not yet`, `tbd`, `to be determined`, `placeholder`, `nothing yet`, `no activity`, `no activity yet`, `n/a`, `na`, `null`, `empty`. A text that is entirely wrapped in `(...)` or `[...]` is rejected whatever it says: a title is not a parenthetical note. The match is on whole words, as `has_phrase` already does, so "Fix the untitled-tab bug" is not rejected.
3. **Grounding (advisory first).** A title should share at least one content word (after stop-word removal) with the material it was made from: the user message and the digest. Failing this is **logged and counted but not enforced** in the first release, because titles legitimately abstract. Enforce only after the measurement in 5.8 shows a rejection rate and a false-reject rate that justify it.
4. **Refusal phrases** (unchanged) and, for a title, the existing length limits.

The corpus of known-bad values starts with the two real ones, `(none yet)` and `no goal established yet`, and grows from the counters in 5.8.

### 5.2 A prompt with no placeholders, and a way to abstain (`ambient::prompt`)

Two prompts, chosen by whether a *valid* current title exists:

- **Create** (no usable current title): no "Current title" line at all, and no "repeat it back" instruction. "Write a short title (N words or fewer) for what this session is about, from the material below. If the material does not show what the work is, reply with exactly `KEEP`."
- **Maintain** (a usable current title): the current title is shown as data, with the existing stability rule, and `KEEP` is the explicit way to say "unchanged". "Repeat it back exactly" is replaced by "reply `KEEP`", so the model never has to reproduce a string, and a title cannot be echoed back as new text.

`KEEP` is not a title: the validator rejects it (5.1), and the caller treats it as "no change". The two other in-band placeholders (`continuity_state.rs:129`, `(no new message ...)`) are replaced the same way, by omitting the line or by an instruction, never by a string the model could return.

### 5.3 Validate on read and break the loop

- `readActivitySummary` (`frontend/app/store/activitySummary.ts`) runs the same predicate, ported to TypeScript from one shared specification and kept in step by a shared test corpus (5.9), and returns `undefined` for a stored value that is not a usable title. Existing bad values in users' databases therefore stop displaying immediately, with no migration.
- `session:activity_summary` treats a stored title that fails the predicate as **empty** when it builds the prompt (`app_api/session.rs:361`), so a bad value is never fed back and the loop cannot sustain itself.
- The frontend write (`useAgentActivitySummary.ts`) writes only a value that passes, and writes nothing on `KEEP`.

### 5.4 The fallback ladder (never blank)

`resolveSwarmLine(meta, context)` returns `{ text, source }` using the first rung that applies:

1. **`generated`**: a usable `term:ambient_summary`.
2. **`restored`**: the last good title for this agent, kept across a restart (5.5).
3. **`heuristic`**: a deterministic one-liner from the latest *human* message of the session, cut to the word limit with the leading filler stripped (greetings, "please", "can you"). No model call. If the latest message is a nudge with no content ("u there", "continue"), use the latest message that has content.
4. **`status`**: a plain phrase from the agent's status, so a row is never empty. Proposed wording (the owner's call, section 9): running with tool activity, "Working"; running without, "Thinking"; waiting on the user, "Waiting for you"; idle with conversation history but no usable title, **"Summarizing work completed"** (the phrase the owner suggested; it fits an agent that has done work and whose summary is not ready yet); idle with no history at all, "No activity yet".

**Built in #4234, with one change to this order:** "Waiting for you" (a pending AskUserQuestion, kept in `term:awaiting_user`, honoured only while the agent has a turn in flight) outranks `restored` and `heuristic`, though not `generated`. It is the one state someone has to act on, and an old goal would bury it. Approval prompts are not wired to it yet.

Rungs 3 and 4 render in the muted style the swarm already uses for secondary text, with a tooltip naming the source ("Shown until a summary is ready"), so a fallback is never mistaken for the agent's own account of its work (P5). The wording is a product decision (section 9).

### 5.5 Restore across restart (stop clearing to blank)

`useBlockActivity.ts` clears `term:ambient_summary` on a session boundary, which is correct for the *live* title but throws away the last good one. Keep the last good title per agent definition (the `agent_activity_summaries` store that the picker already uses, keyed by definition), write it when a generated title is accepted, and on a new session surface it as `restored` until a fresh title replaces it. The owner's restart therefore opens onto the previous goal, not a blank, which is the situation that produced this report.

**Built in #4234, differently:** the title is kept in block meta (`term:restored_summary`), not in `db_agent_activity_summaries`. That table is the picker's once-per-definition cache ("never regenerated once non-empty"), a different contract, and reading it from the swarm needs a new RPC. Block meta already survives a restart, so `useBlockActivity` moves the ended session's title there instead of discarding it, which covers the reported case. What it does not do is carry a title to a **new pane of the same agent**; that still needs the per-definition store and is a follow-up.

### 5.6 When a title is (re)computed

Today: only when a human submission enters `Submitting`, not for hidden turns, deferred at mount. Add, **without** removing that:

- **Empty-title recovery.** If an agent has conversation content and no usable title, compute one from the digest on mount and on the first turn end, using the same create prompt. Retry on later turns while it is still empty, up to a bounded count, as the `lys` design does (research point 2).
- **Schedule, not every turn.** Once a title exists, re-evaluate on the `muxterm` shape (after completed human turns 2, 5, 8, then every third), replacing only if the new title passes the predicate, is not `KEEP`, and differs enough from the old one to be news (a minimum-distance check), so the title is stable.
- **Fast first title.** The first title for a fresh session is computed right after the first completed turn, from the user message and the first reply, not left to wait for a later submission.

**Built in #4238.** The schedule and the news check are in `store/title-schedule.ts` (turn count in `term:human_turns`, cleared at session end; a new title replaces the old only when topic-word overlap is under 0.5, so an expanded goal that keeps most of the old words keeps the old title). The fast first title is covered by the recovery of 5.7, which runs within one 20 s sweep of the first output when no title exists, rather than by a separate turn-end trigger.

### 5.7 The backend sweep: wire it or remove it

The sweep (`activity_watcher.rs`) already produces a digest-based summary every 20 s for running agents and publishes `agent:summary`, with no subscriber. Decide one of:

- **Wire it as the empty-title recovery of 5.6**: the frontend subscribes and accepts a sweep summary *only while the title is empty*, through the same predicate. The cost already exists; it would start producing value.
- **Remove it.** Its calls are pure cost today.

The recommendation is the first (section 9). Whichever is chosen, its digest must carry conversation, not only the cost line (the 2026-10-01 report, defect 7): skip the call when the digest has no human or assistant text.

**Built in #4238, with one change:** the backend writes the recovered title to `term:ambient_summary` itself (one transaction that keeps any title that appeared meanwhile, then `waveobj:update`), instead of a frontend subscriber accepting it. The Swarm shows agents whose panes are not mounted, which a frontend subscriber could not reach. The sweep now attempts only running agents with no usable title and new output, at most 3 completed attempts while the title stays empty, with a title prompt (`build_session_title_from_activity_prompt`) rather than the old "what is being worked on" summary; `agent:summary` and its prompt are removed.

### 5.8 Observability

Every ambient call records one outcome, in a single structured log line at info (not debug) and a counter: `accepted`, `kept` (the abstain token), `rejected` with the reason (`empty`, `shape`, `absence_pattern`, `refusal`, `ungrounded`), `empty_digest`, `superseded`, `spawn_failed`, `timeout`. Per purpose, the swarm summary and the ghost text separately. Surface the counts in the muxlog and the instance panel, so "the swarm has been blank for an hour" is visible without copying a database. Log the rejected text, truncated, because the rejected values are the corpus for 5.1.

**Built in #4243.** `ambient/outcome.rs` records one outcome per call from the gateway (admission, `Slot::run`) and the two places a title call has nothing to send, as an info line (`ambient outcome`, with the rejected text truncated to 120 characters) and a per-purpose counter. `ungrounded` is not a label yet: grounding is still advisory and not checked. An admitted call given up before running records `not_run` (the slot's drop), so every admitted call ends in exactly one outcome. The counts are read over `ambient.outcomes`; the Instance panel shows a **Titles** row (accepted candidates, kept, refused, failed; "accepted" is a candidate the validator took, not necessarily a title change). Counters reset when srv restarts.

### 5.9 Tests

- **Replay of the real failures.** `(none yet)` and `no goal established yet` are rejected by the predicate, by the read path, and by the feed-back path. A stored `(none yet)` renders as the fallback, not as text.
- **The loop.** With a fake CLI that echoes any `Current title:` line it is given, run the title call twice from an empty title. With the old prompt the second call returns the placeholder; with the new prompts neither call ever sees a placeholder, and the stored title never becomes one.
- **Prompt hygiene.** No prompt built by `crate::ambient::prompt` contains a parenthesised placeholder, and none contains the substring `none yet`; `KEEP` appears only as an instruction.
- **Predicate corpus shared by Rust and TypeScript**: one file of accepted and rejected strings, loaded by both test suites, so the two implementations cannot drift (the lesson of the indicator work, where each side was fixed alone).
- **Whole-word safety.** "Fix the untitled-tab bug", "Handle null values in the parser" and "No-op rename threshold" are accepted.
- **Ladder.** Each rung is chosen in order, a muted style and a source tooltip are attached to rungs 3 and 4, and no input produces an empty `text`.
- **Restore.** A new session shows the last good title as `restored`, then the generated one replaces it.
- **Triggers.** A resumed agent with content and no title gets one without any user submission; a hidden turn still never reaches the model (the existing P0 from #3502 stays).

## 6. What this does not change

The Ambient Model Call gateway's admission and cancellation, the title length limits, the sanitiser, the hidden-turn rule, and the ghost-text pipeline (it already has the refusal hardening from #4143; it gains the shared predicate and the observability, nothing else).

## 7. Delivery plan

| PR | Scope | Why this order |
|---|---|---|
| **1: stop the bleeding** | The shared predicate (5.1); read-side validation and the feed-back guard (5.3); the create/maintain prompts with `KEEP` (5.2); the replay and loop tests (5.9). No UI change beyond a bad value no longer displaying | Fixes the owner's symptom and the self-sustaining loop; small and verifiable against the two real values |
| **2: never blank** | The fallback ladder and its rendering (5.4); restore across restart (5.5) | Needs the wording decision (section 9) |
| **3: recovery and triggers** | Empty-title recovery, the re-evaluation schedule (5.6); wire or remove the sweep (5.7) | Behavioural change; benefits from PR 1's predicate |
| **4: observability** | Outcome logging and counters, the muxlog and instance-panel surface (5.8); then decide on enforcing grounding | Gives the data to tune 5.1 |

## 8. Risks

- **A stricter predicate can reject a good title.** Mitigated by whole-word matching, a corpus of accepted titles in the tests, advisory-first grounding, and logging every rejection so a false reject is visible.
- **`KEEP` could itself be echoed or over-used.** It is rejected as a title, so the worst case is "no change", which is the safe outcome; the counters show an over-used `KEEP`.
- **A heuristic title can be wrong.** It is shown muted and labelled as a fallback, and is replaced as soon as a generated title exists.
- **Restoring an old title can mislead after the work has moved on.** It is labelled `restored` and replaced by the first fresh title; the schedule in 5.6 makes that quick.
- **The shared corpus couples two test suites.** That is the point; it is one file.

## 9. Decisions for the owner

1. **The fallback wording.** Recommendation: "Working" / "Thinking" while running, "Waiting for you" when it needs you, and for an idle agent with history the **last real title if there is one**, otherwise "Summarizing work completed", otherwise "No activity yet". Is that the voice you want, and should a fallback line be visibly muted?
2. **The backend sweep.** Wire it as the empty-title recovery (recommended), or remove it?
3. **Persist across restart.** Keep the last good title per agent definition and show it as `restored` (recommended), or start each session blank until a real title exists?
4. **How strict.** Ship the absence-pattern list now and grounding as advisory only (recommended), or also enforce grounding from the start?
5. **Re-evaluation cadence.** The `muxterm` schedule (turns 2, 5, 8, then every third) or the present per-submission refresh with only the predicate and the abstain token added?

### 9.1 Answers (2026-10-02)

The owner said to use the recommendations: the wording of 1 with fallback lines muted, the sweep wired as the empty-title recovery (2), persistence with `restored` (3), the absence-pattern list now and grounding advisory only (4), and the `muxterm` re-check schedule (5). Items 1 and 3 are built in #4234; 2, the grounding half of 4, and 5 belong to PRs 3 and 4.

## 10. Sources

- OpenProgram, *LLM Title Generation* (placeholder shown as a first-message preview when the title is empty): [openprogram.io](https://openprogram.io/docs/reference/design/runtime/session/name.html).
- `w3lt/lys` PR #46, structured title output, bounded retry of output errors only, `title: null` and a first-wins write: [github.com/w3lt/lys/pull/46](https://github.com/w3lt/lys/pull/46).
- `kenotron-ms/muxterm` PR #288, re-check schedule, newest-first inputs with a cap, replace only when unchanged, manual title stops checks: [github.com/kenotron-ms/muxterm/pull/288](https://github.com/kenotron-ms/muxterm/pull/288).
- `openclaw/openclaw` issue #99583, lazy generation, cheap models, a minimum distance to skip no-op renames: [github.com/openclaw/openclaw/issues/99583](https://github.com/openclaw/openclaw/issues/99583) (proposal, read from a search summary, not opened in full).
- `anthropics/claude-code` issue #47176, auto-titling for new sessions only, manual rename outranks a generated title: [github.com/anthropics/claude-code/issues/47176](https://github.com/anthropics/claude-code/issues/47176) (a feature request, not documentation of shipped behaviour).
- *Deterministic Fallbacks for Every AI Path*, a validated fallback ladder with a deterministic last tier and a flag telling the UI which tier produced the output: [dev.to](https://dev.to/techamit95ch/deterministic-fallbacks-for-every-ai-path-1e6d).
- Structured-output guidance on null for unknown values and on validating meaning, not only shape: [LLM Structured Output in 2026](https://dev.to/pockit_tools/llm-structured-output-in-2026-stop-parsing-json-with-regex-and-do-it-right-34pk) and [How Structured Outputs and Constrained Decoding Work](https://letsdatascience.com/blog/structured-outputs-making-llms-return-reliable-json) (read from search summaries, not opened in full).
