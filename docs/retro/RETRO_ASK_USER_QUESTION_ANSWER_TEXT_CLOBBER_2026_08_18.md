# Retro: Answered `AskUserQuestion` silently reverted to the old collapsed row (PR #2630 follow-up)

**Date:** 2026-08-18
**Owner:** AgentA
**Area:** `frontend/app/store/agent-document/reducer.ts` (`mergeReplacement`), `frontend/app/view/agent/hooks/useAgentQuestions.ts`, `frontend/app/view/agent/components/AnsweredQuestionMessage.tsx`

---

## 1. Symptom

PR #2630 shipped "answered questions render as user input" — an answered
`AskUserQuestion` should show the same `.agent-user-message` treatment as
typed input once it scrolls into history, instead of the generic collapsed
tool row. Asked to verify it live: `task dev`, answer a fresh question —
it rendered exactly like it always had, the plain collapsed row. Not a
stale-build or wrong-window issue (checked both); reproduced on a fresh
`task dev` launch built directly from latest `main`.

## 2. Investigation false start

The initial ask was framed as "we had work that was supposed to do X, find
out why it was skipped, write a retro." That framing was wrong and cost the
first round of investigation: `git log`/`gh pr list` showed PR #2630
merged hours earlier, fully implemented, 33/33 + 27/27 tests passing, spec
marked `Status: Implemented`. Nothing was skipped — the feature had
shipped and was live. The real defect only surfaced once the actual running
app was exercised end-to-end rather than trusting the merged PR's own
description and test-plan checkboxes.

**Lesson:** "it merged, tests pass, spec says Implemented" is not the same
claim as "it works when you actually use it." The PR's own test plan had an
honest unchecked box — `[ ] Visual check in task dev across a few themes` —
that turned out to be exactly the gap that mattered.

## 3. Root cause

`AskUserQuestion` is a **real tool call** from the CLI's perspective, not a
pane-only construct. The flow:

1. User answers → `useAgentQuestions.ts`'s `handleAnswer` optimistically
   sets `status: "success"`, `answerText`, `timeoutNote` on the node and
   dispatches it via `StreamFlush`. This is a frontend-only annotation —
   nothing else in the codebase writes these fields.
2. The answer is delivered to the CLI (control protocol or follow-up
   message) and the agent's turn resumes.
3. Because `AskUserQuestion` really is a tool call, the underlying CLI
   stream **still emits its own `tool_result` event** for that same
   `tool_use_id` once the turn resumes — that's how the answer re-enters
   the model's own context. `claude-translator.ts`'s `buildToolResults`
   has no special-casing for `AskUserQuestion`; it treats it exactly like
   `Read`/`Bash`/anything else.
4. That event flows through `stream-parser.ts`'s `toolResultToNode()`,
   which builds a **fresh, generic** `ToolNode` — no concept of
   `answerText`/`timeoutNote` at all, since those aren't part of the raw
   event shape.
5. Since the node id already exists, `useAgentStream.ts` routes it as an
   *update*, not a new node. The reducer's `mergeReplacement()`
   (`reducer.ts`) had exactly one carry-over case — a streaming `log`
   buffer — and `AskUserQuestion` never streams one. With nothing to
   carry over, it fell through to a **wholesale replace**: the fresh,
   generic node from step 4 completely overwrote the optimistically
   answered one, silently dropping `answerText` back to `undefined`.
6. `ToolBlock.tsx`'s render gate is `answerText != null` — once that field
   is gone, the node falls straight back to the pre-#2630 collapsed-row
   rendering. By the time a human looks at the pane, this has already
   happened; the new styling was visible for at most one animation frame.

This happens on essentially **every** live answer to a persistent-agent
question — not a rare race, a guaranteed sequence — which is exactly why it
looked identical to the pre-PR behavior every time it was tested.

## 4. Why this wasn't caught earlier

PR #2630's own tests construct fixed `ToolNode` fixtures directly in the
"already answered" state and assert on the render output — they never
simulate the follow-on `tool_result` echo that the live CLI always sends.
The optimistic-update path and the event-stream path were tested
independently and each looked correct in isolation; the bug only exists at
the seam between them, which no test exercised. The PR's own test plan
called this out honestly (`[ ] Visual check in task dev` left unchecked)
but that step was never actually run before merge.

## 5. Fix

`mergeReplacement()` now special-cases an already-answered `AskUserQuestion`
(`toolName === "AskUserQuestion" && answerText != null`): instead of the
wholesale replace, it keeps the fresh replacement's other fields but
preserves `answerText`/`timeoutNote`/`questionText`/`summary` from the
existing node. The CLI's own `tool_result` echo still lands and updates
whatever else it legitimately owns (status, duration, params) — it just
can't clobber the frontend-only annotation fields it never carried in the
first place.

Added a reducer regression test that reproduces the exact sequence (answer
→ generic echo for the same id) and confirmed it fails without the fix
(`answerText` → `undefined`) before confirming it passes with it.

## 6. Follow-up UX changes (same PR, direct feedback after live-verifying the fix)

Once the clobbering bug was fixed and the feature was actually visible for
the first time, two follow-up requests came from watching it live rather
than reading the spec:

- **Size reverted.** #2630 deliberately enlarged the answer text
  (`--answered-question` SCSS modifier, `font-size: 1.25em`) as its
  original ask. Seeing it live, the request was to match ordinary input
  size instead — removed the modifier entirely.
- **The question now prints too.** #2630 only ever rendered the answer
  half — `question` is cleared to `undefined` on answer (needed so the
  question panel stops treating the node as pending) and nothing captured
  what it had said. Added a `questionText` field, flattened from
  `question.questions[].question` in `handleAnswer` *before* `question` is
  cleared, and `AnsweredQuestionMessage` now renders it as plain agent text
  (the same `.agent-markdown-block`/`Markdown` treatment a normal assistant
  message gets) directly above the answer. The resolved node now reads as
  an ordinary question-then-answer exchange instead of only ever showing
  half of it. `questionText` needed the same clobber-protection as
  `answerText` in `mergeReplacement()` for the same reason.

## 7. What went well

- Didn't stop at "PR merged, tests green" — insisted on driving the actual
  running app before accepting the feature as done, which is what
  surfaced the real bug.
- Traced the full event path (optimistic update → control-protocol
  delivery → CLI's own tool_result echo → parser → reducer → render gate)
  from code alone before touching anything, and confirmed the hypothesis
  with a regression test that fails on the un-fixed code, not just green
  tests on the fixed code.
- Caught mid-flight that "use a long-running-process method" meant the
  established Bash-heartbeat pattern already documented in agent memory
  for this repo, not the newer `mcp__agentmux__Shell` tool — the latter's
  Windows launch path hits a separate, already-documented PATH gap (Gap
  B in this repo's own CLAUDE.md) that looks identical to a real failure
  (`exit_code: 200`, ~53 lines) if you don't know to check for it.

## 8. Follow-up

None identified — `mergeReplacement`'s guard is scoped narrowly to
`AskUserQuestion`'s specific annotation fields and doesn't change behavior
for any other tool. If a future feature adds another frontend-only
optimistic field to `ToolNode`, the same clobbering risk applies whenever
that tool also has a real terminal event of its own arriving after the
optimistic update — worth a shared pattern (e.g. an explicit "preserved
fields" allowlist per tool) if a third case shows up, but not worth
generalizing for one instance today.
