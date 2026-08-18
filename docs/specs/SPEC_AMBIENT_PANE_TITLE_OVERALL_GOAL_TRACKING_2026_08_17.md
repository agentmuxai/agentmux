# SPEC: Pane title tracks the session's overall goal, not the latest micro-step

**Date:** 2026-08-17
**Author:** Lark
**Status:** Implemented

---

## TL;DR

The agent pane header's Haiku-generated summary (`term:ambient_summary`) currently
answers "what is currently being worked on" — a fresh, stateless, context-blind
paraphrase of the last ~30 lines of raw CLI output, regenerated from scratch on
every completed turn. In a real session this means the title thrashes between
micro-steps ("reviewing the diff", "Feature merged to main branch", "checking
review status") instead of settling on something like a PR title ("invert user
input, answer-question style") that describes the session's actual, still-mostly-
stable overall goal.

The fix is not "call it less often" — it's that **every call is a blank slate**.
Give the model two cheap, cheaply-available anchors it doesn't have today — the
*current* title and the user's *newest* message — and an explicit instruction to
prefer keeping the current title unless the goal has genuinely changed, and the
same per-turn cadence stops mattering: a stable goal just keeps getting echoed
back unchanged.

---

## Why now

Direct user feedback (this session): the pane title for the AskUserQuestion/
inverted-color-surface work update almost every turn, tracking whatever the most
recent tool call or review comment was, rather than staying on something like
"invert user input, answer-question style" for the session's whole duration —
the way a PR title would.

---

## Current state of the code

### What generates it

- Frontend trigger: `frontend/app/view/agent/hooks/useAgentActivitySummary.ts:88-113`
  — fires once per genuine, backend-confirmed turn completion (the
  `turn_active: true→false` edge, not `TurnPhase.kind === "Done"` — see
  `docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md` for why that
  distinction already exists). In an interactive session with many user
  messages, that's once per response — frequent, but not literally "every
  micro-step" by itself; see "The actual bug" below for why it still *reads*
  that way.
- Backend RPC handler: `register_session_activity_summary` in
  `agentmux-srv/src/server/app_api/session.rs:374-461`.
- Context extraction: `read_recent_activity_digest`
  (`session.rs:571-605`) — reads the **last 32 KB** of the block's persisted
  output, takes the **last 30 non-empty lines** of that, and
  `extract_digest_text` (`session.rs:876-`) turns those into
  `[assistant]`/`[tool]`/`[user]`/`[error]` snippet lines.
- Model call: `invoke_ambient_haiku_call` (`session.rs:622-`) — spawns a
  **fresh, stateless** `claude -p` process per call (`--model
  claude-haiku-4-5-20251001`, no `--resume`, no session continuation). It
  receives *only* the prompt string below; nothing else survives between
  calls.
- The literal prompt (`session.rs:434-439`, `word_target` is 5–12 based on
  pane width):

  ```
  Summarize in {word_target} words or fewer what is currently being worked on.
  Plain text only — no markdown, no code fences, no backticks, no quotes,
  no punctuation, no preamble.

  Recent activity:

  {extracted}
  ```

- Consumption: the result is written to block meta key `term:ambient_summary`;
  `frontend/app/store/activitySummary.ts:19-25` (`readActivitySummary`) prefers
  it over the free CLI-emitted `term:osc_title` OSC signal, and both
  `agent-model.ts` (pane title) and `swarm-model.ts` read through that same
  function. This is the **only** consumer found — there is no separate
  "live activity" surface today; the pane title *is* this value.

### The actual bug

Two compounding problems, not one:

1. **The prompt asks for the wrong thing.** "What is currently being worked on"
   is a request for the micro-step, by construction — there's no instruction
   anywhere asking for the *overall* goal.
2. **Every call is a blank slate.** `invoke_ambient_haiku_call` gets no memory
   of the previous title, no access to the first user message, no PR title, no
   git commits — nothing that would let it recognize "this is still the same
   task." Its only input is a 30-line tail of raw stream-json, which is
   dominated by whatever tool call/assistant text happened most recently.
   Structurally, once the original ask scrolls past 30 lines of output (which
   happens almost immediately once tool calls start), the model **cannot see
   it anymore** — it isn't being asked to ignore the original goal, it
   literally has no way to know what it was.

No existing spec proposes goal-level tracking — confirmed by reading
`docs/retro/retro-haiku-activity-pane-header-2026-06-24.md`,
`docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md` §0/1.1 ("a Haiku
call that summarizes what an agent is doing, once per completed turn"),
`docs/specs/SPEC_AMBIENT_SUMMARY_SANITIZATION_AND_TERSENESS_2026_07_08.md`
(terseness/formatting only, §2.3 explicitly out-of-scopes trigger/behavior
changes), and `specs/SPEC_AGENT_PANE_HEADER_NAME_PRECEDENCE_2026_06_29.md:108`
(explicitly treats "Haiku-per-turn" generation as out of scope, layout only).
This has always been a micro-activity summarizer; the "overall goal" framing is
new.

---

## Target state

### 1. Anchor the call on the current title, not a blank slate

Pass two things the pipeline doesn't pass today:

- **The current title**, if one exists — already sitting in `block.meta`
  (`term:ambient_summary`) at the point the backend handler runs; no new
  plumbing needed to obtain it.
- **The user's newest message verbatim** — not a stream-json tail digest of
  it. The frontend already has this text at the moment it's submitted (wherever
  `AgentInputCommand`/`handleSendMessage` fires); thread it through as a new
  field on the RPC payload (`CommandActivitySummaryData.user_message: Option<String>`)
  instead of re-deriving it from `read_recent_activity_digest`. This sidesteps
  digest-extraction entirely for this purpose — no 32 KB tail read, no
  stream-json parsing, no risk of the message having already scrolled out of
  the window.

### 2. Rewrite the prompt to ask for a stable, PR-title-style summary

```
You maintain a short running TITLE for this work session, similar to a git
pull-request title — it describes the OVERALL GOAL of the session, not the
current micro-step or the most recent tool call.

Current title: {current_title_or_literal_"(none yet)"}

The user just said:
{user_message}

Decide: does this message represent a genuinely NEW or EXPANDED top-level
goal, or is it a continuation, follow-up, clarification, correction, or a
step within the SAME goal the current title already describes?

- If the current title still accurately describes the overall goal, repeat
  it back EXACTLY, unchanged.
- Otherwise, output an updated title covering the (possibly still-in-progress)
  overall goal, in {word_target} words or fewer.

Plain text only — no markdown, no code fences, no backticks, no quotes,
no punctuation, no preamble.
```

The "repeat it back exactly if unchanged" instruction is what actually buys
stability — not a lower call frequency. A stable goal now costs the same one
Haiku call per turn it always did, but produces the same output every time,
which reads as "the title basically never changes," matching what a PR title
feels like even though (unlike a PR) nothing here is cached/skipped.

### 3. Move the trigger from turn-end to submit-time

Currently the call fires on `turnJustEndedAtom` — after the agent's full
response (including all tool calls) completes. The goal can only change at the
point the *user* says something new; the agent's own tool calls never change
it. Firing on the `Submitting` transition instead (`useAgentActivitySummary.ts`
already tracks this via `activeTurnId`) means:

- The title updates immediately when the user asks something, instead of
  lagging behind a potentially long tool-heavy turn.
- The call no longer needs `read_recent_activity_digest` / the FileStore tail
  read at all for this purpose — the trigger site already has the literal text
  that just got submitted.

This is a secondary improvement (perceived responsiveness + simpler data path),
not what fixes the core complaint — §1/§2 do that regardless of which edge the
trigger fires on.

---

## Edge cases

- **First message of a session.** `current_title` is empty/`"(none yet)"` —
  the model has nothing to anchor to, so it synthesizes fresh from the first
  message alone. This is the bootstrap case and needs no special handling
  beyond the literal `"(none yet)"` placeholder in the prompt.
- **A genuine mid-session pivot** (user abandons the original task for an
  unrelated one). The prompt explicitly allows for this ("does this message
  represent a genuinely NEW... top-level goal") — the model is expected to
  replace the title, not preserve it unconditionally. This is a real,
  intentional behavior change from pure "never update," which the user's own
  example (a session-long stable title) doesn't rule out — an actual new ask
  should still get a new title.
- **Model instability / prompt non-compliance.** Haiku-tier models don't always
  follow "repeat back exactly" instructions perfectly — output could drift by
  punctuation or minor rewording even when the intent is "keep it." Consider a
  cheap post-processing normalization (trim/casefold compare against the
  current title before writing) so near-identical output doesn't count as a
  "change" for whatever UI affordance (if any) might want to distinguish
  "title changed" from "title reaffirmed" later. Not required for the core fix.
- **Very long user messages.** The prompt now embeds the user's message
  verbatim rather than a truncated digest — for a very long paste, consider
  capping it (e.g. first ~500 chars) before insertion so the ambient call's
  own prompt doesn't balloon in size/cost. Existing `word_target` bounds only
  the *output*, not this new input.

---

## Tests

- Backend: a fixed `current_title` + a `user_message` that's clearly a
  continuation ("also fix the lint warning in that file") should keep the
  existing title in the constructed prompt (behavioral prompt-construction
  test, not an LLM-output test — verify the prompt string embeds both fields
  correctly, not that Haiku "chose right").
- Frontend: `useAgentActivitySummary`-equivalent trigger test — firing on the
  `Submitting` transition rather than `turnJustEndedAtom`, using the literal
  submitted text rather than triggering a digest read.
- Regression: confirm `term:ambient_summary` write path (`ObjectService.
  UpdateObjectMeta`) and the `activeTurnId`-based staleness guard are
  unaffected by the trigger-edge change — a fast follow-up submission before
  the previous call returns must still discard the stale result.

---

## Implementation notes (added post-implementation)

**How the frontend actually gets the just-submitted text to the trigger site.**
The spec didn't originally nail this down. Turns out `TurnPhase.Submitting`
already had a `pendingContent: string` field — added in an earlier PR but never
wired up (`reducer.ts`'s own comment: *"The TurnStart payload doesn't carry
pendingContent; a later PR can thread that through"*). This is that later PR:
the `TurnStart` action gained an optional `content?: string` field (optional so
the ~90 existing reducer/hook test call sites that construct it without one
keep compiling unchanged), threaded from both call sites that dispatch
`TurnStart` with real text in hand (`agent-view.tsx`'s `handleSendMessage`, and
`usePendingMessageAcceptance.ts`'s queue-drain path). `useAgentActivitySummary`
then reads `phase.pendingContent` directly off the `Submitting` phase object —
no new plumbing needed beyond that one optional field.

**Backend prompt-building was extracted into `build_session_title_prompt()`**
(pure function, `session.rs`) so the new prompt shape has direct unit test
coverage instead of only being exercisable through the full RPC handler.

---

## Order of delivery

1. **Prompt + context change** (§1/§2) — the actual fix. Ships independent of
   §3; even firing on the old `turnJustEndedAtom` edge, an anchored, stability-
   biased prompt already stops the thrashing.
2. **Trigger-edge change** (§3) — optional follow-up for responsiveness and to
   drop the now-unnecessary FileStore tail read for this call site.

---

## Out of scope

- No change to `next_prompt_suggestion`'s sibling ambient call (`session.rs:530-539`)
  or `subagent_name` (`session.rs:264-267`) — different purposes, not part of
  this complaint.
- No new "live per-step activity" surface. `term:ambient_summary` is the only
  consumer of this value found (pane title + swarm view); if per-step visibility
  is wanted again later, it would need its own separate meta key/UI surface —
  not proposed here since nothing today reads such a thing.
- No change to the 20s timer-driven push sweep in
  `agentmux-srv/src/backend/reactive/activity_watcher.rs` — per
  `docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md` §5 its
  `agent:summary` event is already unconsumed by the frontend (dead code), so
  it isn't part of the visible bug and isn't touched here.

---

## Related

- `docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md` — the gateway
  (admission/cancellation/generation-fencing) this call is routed through;
  unaffected by this spec.
- `docs/specs/SPEC_AMBIENT_SUMMARY_SANITIZATION_AND_TERSENESS_2026_07_08.md` —
  prior formatting-only pass over the same prompt.
- `docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md` — prior
  frequency fix (turn-completion detection); this spec doesn't revisit that
  detection logic, only which edge (`Submitting` vs `turnJustEndedAtom`) it's
  attached to.
- `specs/SPEC_AGENT_PANE_HEADER_NAME_PRECEDENCE_2026_06_29.md` — layout
  precedence between title sources; explicitly out-of-scopes the generation
  logic this spec changes.
