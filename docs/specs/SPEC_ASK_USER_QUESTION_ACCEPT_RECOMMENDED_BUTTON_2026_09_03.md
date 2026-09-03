# SPEC: "Accept Recommended" button for AskUserQuestion

**Date:** 2026-09-03
**Status:** implemented — §5's questions were confirmed live via an actual
AskUserQuestion demo, and while confirming them the scope grew to include a
real Cancel (replacing the non-functional "Answer later"). See §7 for what
that changed relative to §§2–4 below, which describe Accept Recommended as
originally scoped and are otherwise accurate.
**Owner:** AgentY
**Trigger:** User request (below) — add a button that submits every
question's recommended option(s) in one click, instead of requiring the user
to click through each option individually.

---

## 0. Ask

> we want to add another button to AskQuestion tool prompt, we want a button
> "Accept Recommended" that simply submits the answer with the recommended
> selections. write a spec to file

## 1. Current behavior (confirmed by reading the code)

`AskUserQuestion` renders via `AgentQuestionPanel.tsx`
(`frontend/app/view/agent/components/AgentQuestionPanel.tsx`), reached from a
`ToolNode` in `status: "awaiting_answer"`. Two actions exist today, in
`.agent-question-panel-actions` (lines 640–656):

- **"Answer later"** (`defer()`, line 373) — leaves the node in
  `awaiting_answer`, no submission.
- **"Submit answer"** (`submit()`, line 284) — `disabled={!allAnswered()}`;
  submits whatever the user has manually selected/typed.

There is no explicit `recommended` field anywhere in the wire protocol or the
`AskUserQuestionOption` type (`types.ts:700–736`). "Recommended" is a
**frontend-only convention**, already built for
[`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`](./SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md)
and reused for the dashed-border highlight on options. It lives entirely in
one exported function (`AgentQuestionPanel.tsx:53–70`):

```ts
const RECOMMENDED_RE = /\(recommended\)\s*$/i;

export function recommendedOptions(options: AskUserQuestionOption[]): AskUserQuestionOption[] {
    const flagged = options.filter((o) => RECOMMENDED_RE.test(o.label));
    if (flagged.length > 0) return flagged;
    return options.length > 0 ? [options[0]] : [];
}
```

Rule: any option whose label ends in `(Recommended)` (Claude Code's own
convention for this tool) is "recommended"; if none are flagged, the first
option in the list is used. Can return more than one option when
`multiSelect` is set.

This function already has two callers:

- **The render loop** (line 597) — computes it per-option purely to add the
  `agent-question-panel-option--recommended` CSS class (dashed border).
  Display-neutral: it never changes the label text.
- **`applyRecommendedDefaults()`** (line 307) — called only from the 30s
  auto-timeout (`createEffect` at line 353), and only fills questions the
  user hasn't touched (`questionAnswered(i)` guard). Anything the user
  already answered, fully or partially, survives untouched — the timeout
  *merges*, it does not overwrite. Guarantees every question ends up
  answered even with a zero-`options` question, via a free-text placeholder
  fallback (`"No option was available to auto-select"`).

Submission goes through `submit(autoFilledCount = 0)` → `buildOutcome()` →
`props.onAnswer(outcome)` → `useAgentQuestions.ts`'s `handleAnswer()` →
`RpcApi.AgentAnswerCommand` (`answers_map` keyed by question text) → the Agent
SDK control-protocol response
([`SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`](./SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md)).
`autoFilledCount` is the one piece of provenance carried through: `0` renders
as a plain `❓ Answered` in history; a nonzero count renders a `⏱️
(Partly/Fully) auto-answered` note (`useAgentQuestions.ts:130–137`,
`AnsweredQuestionMessage.tsx`).

**No existing spec, code, or test mentions "Accept Recommended"** — confirmed
by a full-repo search (`Accept Recommended` / `AcceptRecommended` /
`acceptRecommended`: zero hits). This is a new button, not a rename or
extension of something that half-exists.

## 2. What "Accept Recommended" does — the one behavioral decision that matters

**It answers every question with its recommended option(s), regardless of
what the user has already selected, then submits.**

This is deliberately **not** `applyRecommendedDefaults()`'s merge-only-gaps
behavior. That distinction needs to be explicit, because copying the wrong
function would silently produce the wrong button:

| | `applyRecommendedDefaults` (timeout) | this button |
|---|---|---|
| Already-answered questions | left untouched | **overwritten with the recommendation** |
| Trigger | 30s of inactivity | explicit click |
| Intent | safety net so work never stalls | fast path for "I trust the defaults" |

The whole value of a one-click "accept recommended" action is that it does
what it says regardless of stray clicks made before pressing it — a user who
half-answered three questions and then decides "just go with the
recommendations" should get exactly that, not a mix of their partial picks
and the recommended fill for the rest. If overwrite-on-click turns out to be
surprising in practice, the fix is copy (e.g. a confirm/tooltip), not silently
changing this to a merge — a merge makes the button's outcome depend on click
order, which is a worse UX than either "always overwrite" or "disabled once
anything is answered."

New function, `acceptRecommended()`, sibling to `applyRecommendedDefaults()`:

```ts
const acceptRecommended = () => {
    const r = request();
    if (!r) return;
    r.questions.forEach((q, i) => {
        const recommended = recommendedOptions(q.options);
        if (recommended.length > 0) {
            setQ(i, { selected: recommended.map((o) => o.label), other: "" });
        } else {
            // Same zero-options fallback as applyRecommendedDefaults, for the
            // same reason: allAnswered() must pass afterward or submit() no-ops.
            setQ(i, { selected: [], other: "No option was available to auto-select" });
        }
    });
    submit(0);
};
```

`autoFilledCount: 0` — this is a deliberate user action (one click, but a
click), not the agent's turn stalling unattended. It renders identically to a
manual `❓ Answered` in history. **Alternative considered and rejected:** a
distinct history marker (e.g. `✅ Accepted recommended`) so a later reader can
tell "the user actually read each option" from "the user clicked the
shortcut." Rejected for v1 — it would require a new `AnswerOutcome` field
threaded through `useAgentQuestions.ts` and `AnsweredQuestionMessage.tsx`
for a distinction the transcript's plain answer text already mostly conveys
(the selected labels are the recommended ones, visible either way). Worth
revisiting only if this turns out to matter for audit trails in practice —
tracked as an open question in §5, not built here.

## 3. UI placement and behavior

**Button:** `.agent-question-panel-btn` (same base as the other two — border,
transparent background), **not** the `--submit` accent-filled variant.
"Submit answer" stays the only visually primary action; "Accept Recommended"
sits alongside "Answer later" as a secondary action. Rationale: this is a
shortcut for the *default* path, not a replacement primary CTA — a filled
accent button here would compete with "Submit answer" for visual priority and
push the user toward the shortcut over actually reading the options, which
under-serves a question the agent thought worth surfacing at all.

**Order in `.agent-question-panel-actions`:** Answer later → **Accept
Recommended** → Submit answer. Placed in the middle so reading order matches
commitment order: defer (no commitment) → accept the defaults (low
commitment) → submit my own choices (full commitment).

**Always enabled**, unlike "Submit answer" (`disabled={!allAnswered()}`).
`recommendedOptions()` always returns something to select (or the
placeholder text), by the same guarantee `applyRecommendedDefaults()` already
relies on — there is no state where clicking it can't produce a valid
outcome. If `request()` is null the handler simply no-ops (mirrors every
other action in this component).

**No countdown/timer interaction needed.** Clicking submits immediately,
which is a `props.onAnswer` call same as "Submit answer" — the panel is
expected to unmount (queue advances) the same way, so there is no new
interaction with the 30s auto-timeout, the hover-pause, or the keyboard-pause
mechanisms to design around.

## 4. What does NOT change

- **No wire-protocol or backend change.** `recommendedOptions()` is already
  entirely client-side; `AnswerOutcome`, `answers_map`, the RPC
  (`AgentAnswerCommand`), and the Rust backend
  (`CommandAgentAnswerData`) are untouched.
- **No change to `applyRecommendedDefaults()`** or the auto-timeout behavior
  — they remain the merge-only-gaps safety net described in
  [`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`](./SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md).
  `acceptRecommended()` is new, not a rename.
- **No change to the option-highlight styling** (`--recommended` dashed
  border) — it already shows the user what this button would pick, before
  they click it.

## 5. Open questions — resolved

Confirmed live via an actual `AskUserQuestion` demo (using this very spec's
§5 as the question set), not assumed:

1. **Overwrite vs. merge?** → **Overwrite everything**, as §2 already argued.
2. **History marker for "accepted via shortcut"?** → **No distinction** —
   `autoFilledCount: 0`, renders as a plain `❓ Answered`, exactly as §2
   proposed.
3. **Copy?** → **"Accept Recommended"**, as originally written.

All three landed on the option this spec had already recommended. What
changed the scope of the work wasn't any of these three — it was a follow-up
question asked immediately after: "we don't need the Answer later, get rid of
that... in fact we want a way to exit out." See §7.

## 6. Implementation surface

Landed in `frontend/app/view/agent/components/AgentQuestionPanel.tsx` and its
`.scss` sibling, exactly as scoped here:

- `acceptRecommended()` (§2) inside `AgentQuestionPanel`.
- The button in `.agent-question-panel-actions` (§3; final position is
  Cancel → Accept Recommended → Submit answer — see §7 for why the left slot
  is Cancel rather than the "Answer later" this section originally assumed).
- `.agent-question-panel-btn--recommended` in `AgentQuestionPanel.scss` — a
  bordered/text secondary-accent style, distinct from both the neutral Cancel
  and the filled-accent Submit.
- `AgentQuestionPanel.test.tsx`: overwrite-on-click semantics (including over
  an already-answered question), the zero-options fallback, `autoFilledCount`
  staying `0`.

No backend, RPC, or type changes were needed **for this half of the work**
(§4 is accurate for Accept Recommended specifically) — but §7 needed all
three, for Cancel.

## 7. Addendum — Cancel: a real protocol-level decline, not a UI dismiss

**What changed the scope.** While confirming §5 live, the user pointed out
that the panel's other existing action — **"Answer later"** (`defer()`,
§1) — doesn't work: it only sets a `minimized` signal and logs a message. It
never tells the agent anything, so the agent stays blocked on the question
forever. The request: remove it entirely, replace it with a **Cancel** that
actually tells the agent the user declined to answer, with Escape mapped to
the same action — landing on a **3-button row: Cancel, Accept Recommended,
Submit answer** (left-to-right = increasing commitment: exit → accept
defaults → submit your own choices).

**The mechanism, verified against the official Agent SDK docs
(code.claude.com/docs/en/agent-sdk/{permissions,user-input}) rather than
assumed:** `AskUserQuestion` goes through the exact same `canUseTool`
control-protocol callback as ordinary tool permission requests. That callback
supports `{behavior: "deny", message}` as a fully general, documented
response — never previously used in this codebase for AskUserQuestion, which
only ever sent `{behavior: "allow", updatedInput: {answers}}`.

**Backend** (`agentmux-srv/src/backend/blockcontroller/persistent.rs`): new
`deny_question(tool_use_id, message) -> Result<(), String>`, structurally a
mirror of `answer_question` — same `pending_questions` lookup/removal, same
dead-air safety net (a pending tool_use can be abandoned by the CLI whether
the response was an allow or a deny) — sending `behavior: "deny"` with a
fixed, server-owned message (`ASK_USER_QUESTION_DENY_MESSAGE`) instead of
`allow` + `updatedInput`. New RPC command `agentcancel` /
`CommandAgentCancelData { blockid, tool_use_id }`, registered in
`server/websocket.rs` mirroring `agent.answer`'s handler — including its
exact error wording, since the frontend's `SAFE_TO_RETRY_VIA_FOLLOWUP`
allowlist (`useAgentQuestions.ts`) matches on those substrings for both
commands.

**Frontend:** `handleCancel()` in `useAgentQuestions.ts` mirrors
`handleAnswer()`'s optimistic-transition + allowlisted-fallback shape. The
resolved node lands on `status: "denied"` — an existing, fully-wired
`ToolNode` status (icon `⊘`, fail-terminal auto-collapse, `STATUS_LABEL`)
reused rather than adding a new one, and the correct semantic fit: a rejected
permission request, distinct from `"canceled"`, which is reserved for
orphan-scrub forcibly terminating a stale in-flight call — a different
scenario from a live, successful decline. `AgentQuestionPanel.tsx` lost the
entire `minimized`/`setMinimized` signal and the minimized-chip UI along with
`defer()` — there is no more "leave it pending, minimized" state, since
Cancel is a terminal, delivered action. `AnsweredQuestionMessage.tsx` renders
a declined question with a compact "🚫 Cancelled — no answer provided" note
instead of the answer bubble, since there is no answer to show.

**Corrections to §§3–4 above, now that both features shipped together:**
§3's "Answer later → Accept Recommended → Submit answer" ordering is
Cancel → Accept Recommended → Submit answer. §4's "no backend, RPC, or type
changes" is true only for Accept Recommended; Cancel required all three,
described above.

Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
