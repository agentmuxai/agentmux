# SPEC: Auto-timeout for AskUserQuestion — 30s countdown, auto-select the recommended option

**Date:** 2026-08-06
**Status:** Proposed — all open questions resolved (§5); ready for implementation.
**Severity:** Low-Medium — no data-loss risk, but an unanswered question
blocks the agent's turn indefinitely today, which defeats unattended/overnight
runs and any workflow where the human isn't watching the pane.
**Trigger:** User request (below) — give `AskUserQuestion` a 30s auto-timeout
with a visible countdown so a pending question can never stall work forever.

---

## 0. Ask

> get the latest agentmuxai/agentmux .. for the AskQuestion tool, we want to
> add a 30 second timer (also add a UI countdown) if the user does not respond
> by zero, the recommended is auto selected. this is so work does not stop.
> write a spec to file

---

## 1. Current behavior (confirmed by reading the code)

- `AskUserQuestion` tool-use is delivered to AgentMux via the Agent SDK
  **control protocol** (`--permission-prompt-tool stdio`), not the stream —
  Claude Code auto-rejects the tool in plain headless mode. See
  `docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`. The CLI sends a
  `control_request` (subtype `can_use_tool`), and AgentMux's backend parks it
  in `pending_questions: HashMap<String,(String, serde_json::Value)>`
  (`agentmux-srv/src/backend/blockcontroller/persistent.rs:250-255`) until a
  `control_response` is sent back via `answer_question()` (`persistent.rs:1695`).
- The frontend surfaces this as a `ToolNode` with `status: "awaiting_answer"`
  and a populated `question` field
  (`frontend/app/view/agent/types.ts:591-618` for the wire shapes). It is
  rendered by `AgentQuestionPanel.tsx`, collected via
  `pendingQuestions()`/`handleAnswer()` in
  `frontend/app/view/agent/hooks/useAgentQuestions.ts`, mounted from
  `agent-view.tsx:1667-1676`.
- **There is no timeout anywhere in this path today.** The panel sits open —
  full or minimized — until a human answers or the pane is closed. This is the
  gap this spec closes.
- `AskUserQuestionOption` (`types.ts:591-594`) is `{ label, description? }` —
  **no explicit "recommended" flag exists in the wire schema.** The only
  signal available is Claude Code's own convention for the `AskUserQuestion`
  tool: *"If you recommend a specific option, make that the first option in
  the list and add '(Recommended)' at the end of the label."* So detection has
  to be heuristic: look for a `(Recommended)` suffix on `option.label`, and
  fall back to the first option in the list when nothing is marked.
- Answer delivery already has two independent paths from the same
  `AnswerOutcome` object (built by `AgentQuestionPanel.submit()`):
  1. **Control-protocol path** — `RpcApi.AgentAnswerCommand` →
     `answer_question()` in `persistent.rs`, which resumes the CLI's parked
     turn with a `control_response`.
  2. **Follow-up-message fallback** — when the control-protocol path fails
     with one of a known-safe set of errors (`useAgentQuestions.ts:60-66`,
     `SAFE_TO_RETRY_VIA_FOLLOWUP`), the answer is instead sent as a normal
     chat turn (`opts.sendMessage(outcome.answer_text)`).

  An auto-timeout only needs to trigger the *same* `submit()` call a human
  click already triggers — no new wire format, no backend change.
- **Existing countdown precedent to reuse**, not invent from scratch:
  `frontend/app/modals/userinputmodal.tsx` — `countdown` signal seeded from a
  timeout, `setInterval` ticking every 1000ms, firing an action at 0
  (lines 17, 80-95), rendered in the modal title as `` `${title} (${countdown()}s)` ``
  (line 157), cleaned up via `onCleanup` clearing both the interval and a
  pending timeout (lines 92-95). This spec's timer follows the identical
  shape, just living inside `AgentQuestionPanel.tsx` instead of a modal.
- Severity color tokens already exist in the app's SCSS and should be reused
  rather than introduced fresh: `--warning-color` (`app/block/block.scss:215,405`)
  and `--error-color` (`app/errors/ErrorBanner.scss`, `app/block/block.scss:592`).

---

## 2. Design

### 2.1 Scope

- Applies only to `AgentQuestionPanel` (the `AskUserQuestion` tool). The
  sibling `AgentDecisionPanel` (tool-permission "always ask" prompts) is
  **explicitly out of scope** — different tool, different risk profile
  (approving a destructive action unattended is not the same as picking a
  clarifying-question default); a separate spec should cover that if wanted.
- **One timer per panel instance** — i.e., per `tool_use_id` of the head
  question-set, not per individual question. A request can carry multiple
  `questions[]` (see `AskUserQuestionRequest`, `types.ts:605-610`); all of
  them share one 30s countdown and are auto-filled + submitted together at 0.
- **One question-set visible at a time.** The queue (`queueDepth()` in
  `AgentQuestionPanel.tsx:59`) already only renders the head; when the head
  auto-submits, the next queued question-set becomes head with a fresh
  `tool_use_id`, and gets its own fresh 30s window for free (see §2.3).

### 2.2 Recommended-option detection

New pure helper, colocated with `AgentQuestionPanel.tsx`:

```ts
const RECOMMENDED_RE = /\(recommended\)\s*$/i;

function recommendedOptions(options: AskUserQuestionOption[]): AskUserQuestionOption[] {
    const flagged = options.filter((o) => RECOMMENDED_RE.test(o.label));
    if (flagged.length > 0) return flagged;
    // Per the AskUserQuestion tool's own instructions to the model: when a
    // recommendation isn't explicitly marked, it's conventionally the first
    // option in the list. Falling back to it is always safe — worst case
    // it's just "the first option," the same outcome as a human clicking
    // through without reading closely.
    return options.length > 0 ? [options[0]] : [];
}
```

- **Single-select** (`multiSelect: false`): auto-select
  `recommendedOptions(q.options)[0]`.
- **Multi-select** (`multiSelect: true`): auto-select **all** flagged
  options; if none are flagged, select just `options[0]` — never leave a
  multi-select question with zero selections, since `allAnswered()`
  (`AgentQuestionPanel.tsx:105-109`) requires at least one selected option or
  free text per question.
- Detection is **display-neutral**: the rendered label text is unchanged
  (the `"(Recommended)"` suffix, if present, stays visible — it's meaningful
  context for a human deciding whether to intervene). Additionally apply an
  `agent-question-panel-option--recommended` class (computed from the same
  `recommendedOptions()` call) to visually highlight which option(s) the
  countdown will pick, so a watching user can predict the outcome before it
  happens.

### 2.3 Timer lifecycle

**Resolved (§5.1): there is no disarm mechanism.** The timer always fires at
30s regardless of interaction; at zero it *merges* rather than blindly
overwrites — see the merge rule below. This replaces an earlier draft of this
section that disarmed the whole panel on first interaction (rejected: it
directly contradicted the "work does not stop" goal — a human who answers
question 1 of 2 and then gets pulled away would otherwise cancel the safety
net entirely, leaving question 2 blocked forever, the exact failure mode this
spec exists to fix).

- `const AUTO_TIMEOUT_MS = 30_000;` — module-level constant in
  `AgentQuestionPanel.tsx`.
- `const [remainingMs, setRemainingMs] = createSignal(AUTO_TIMEOUT_MS);`
- Extend the existing `createEffect` that resets `state()` whenever the head
  question changes (`AgentQuestionPanel.tsx:68-73`, keyed on `tool_use_id`) to
  also (re)arm the timer in the same effect body:
  - Clear any previous interval.
  - Reset `remainingMs` to `AUTO_TIMEOUT_MS`.
  - Start a new `setInterval(1000ms)` that decrements `remainingMs`; on
    reaching 0, for each question `i` where `!questionAnswered(i)`
    (`AgentQuestionPanel.tsx:100-103` — no selection and no non-empty "Other"
    text), apply the recommended default from §2.2 into `state()[i]`.
    Questions the user already answered — fully or partially — are left
    exactly as-is. Then call `submit()`; the resulting `outcome.autoFilledCount`
    reflects how many questions were auto-filled (see §2.5).
  - `onCleanup` clears the interval — mirrors `userinputmodal.tsx:92-95` —
    so unmount or a `tool_use_id` change (new head question) never leaves a
    stale interval running against the wrong question.
- **Keeps running while minimized.** The whole point is "work does not
  stop" even if the human dismissed the panel without answering, so the
  interval must **not** be gated on `!minimized()`. Only the *rendering* of
  the countdown differs between the full panel and the minimized chip
  (§2.4) — the timer itself is identical either way.
- **No interaction listeners needed.** Because there's no disarm, `toggleOption`/
  `setOther` (`AgentQuestionPanel.tsx:79-98`) need no changes at all — the
  merge-at-timeout logic reads whatever is in `state()` at the moment the
  interval fires, which is already kept live by those existing setters.
- **Manual submit still wins whenever it happens first.** Clicking "Submit
  answer" transitions the node before the interval next ticks; the effect's
  `onCleanup` (triggered by the resulting `tool_use_id` change) tears down the
  interval, so there is no race with the timer.
- **Queue advance is automatic.** When the head auto-submits, it transitions
  out of `awaiting_answer` (same as a manual submit), the queue's `head()`
  memo recomputes to the next pending question-set (a different
  `tool_use_id`), and the reset effect above re-arms a fresh 30s timer for
  it. No queue-specific auto-timeout code is needed.

### 2.4 UI countdown

**Resolved (§5.3): concrete token values below** (superseding "left to
implementation-time judgment").

- **Full panel** (`.agent-question-panel-header`,
  `AgentQuestionPanel.tsx:230-236`): add a countdown chip, e.g.
  `Auto-selects recommended in {Math.ceil(remainingMs() / 1000)}s`.
  `transition: color 120ms ease;` (matches the panel's existing
  `80ms ease`-family transitions closely enough to feel consistent without
  being a literal copy of the option-hover timing). Color escalates by
  remaining time:
  - `> 10s`: `var(--secondary-text-color)` — same muted tone already used for
    the queue-depth chip (`.agent-question-panel-queue`) and option
    descriptions, so it doesn't compete with the question text.
  - `≤ 10s, > 5s`: `var(--warning-color)`, `font-weight: 600`.
  - `≤ 5s`: `var(--error-color)`, `font-weight: 600`, plus a subtle pulse —
    `animation: amux-question-countdown-pulse 1s ease-in-out infinite;` with
    `@keyframes amux-question-countdown-pulse { 50% { opacity: 0.6; } }`.
    Kept to an opacity pulse only (no color/size change in the keyframe) so
    it reads as "urgent" without being distracting.
- **Minimized chip** (`.agent-question-panel-minimized`,
  `AgentQuestionPanel.tsx:208-218`): append the same countdown, same color
  thresholds, next to "Question waiting" so a user who minimized the panel
  still sees the time pressure without having to reopen it.
- The countdown is always visible from panel-open to submit (manual or
  automatic) — there is no "disarmed" state to hide it early, per §2.3.

### 2.5 Delivered answer / audit trail

- The value delivered to the agent — both `answers_map` (control-protocol
  path) and `answer_text` (follow-up-message fallback) — is **identical in
  shape** to a normal human answer: the recommended option's exact label(s).
  **No wire-format change** on `AnswerOutcome`, `CommandAgentAnswerData`
  (`agentmux-srv/src/backend/rpc_types/block.rs:160-170`), or the
  `control_response` payload. This keeps the entire backend untouched.
- For human-facing traceability only, extend `AnswerOutcome`
  (`AgentQuestionPanel.tsx:19-30`) with `autoFilledCount: number` — how many
  of the request's questions were filled by the timeout merge (§2.3), `0` for
  a fully manual submit. When building the optimistic transcript summary in
  `useAgentQuestions.ts:123` (currently
  `` `❓ Answered — ${outcome.answer_text...}` ``), prefix based on the count
  relative to `outcome.answers.length`:
  - `0` → unchanged, `` `❓ Answered — ...` ``.
  - `< answers.length` (partial) →
    `` `⏱️ Partly auto-answered (${autoFilledCount}/${answers.length} — no response in 30s) — ...` ``.
  - `=== answers.length` (fully timed out, nothing was touched) →
    `` `⏱️ Auto-answered (no response in 30s) — ...` ``.

  A boolean would have collapsed the "user answered Q1, timeout filled Q2"
  case into the same label as "user never touched anything" — the count
  distinguishes them, which matters for anyone auditing the transcript later.
  This is the only change outside `AgentQuestionPanel.tsx`/its styles/tests.

---

## 3. Files touched

- `frontend/app/view/agent/components/AgentQuestionPanel.tsx` — timer state
  and lifecycle, `recommendedOptions()` helper, merge-at-timeout logic,
  countdown rendering (full + minimized), `autoFilledCount` on `AnswerOutcome`.
- `frontend/app/view/agent/components/AgentQuestionPanel.scss` — countdown
  chip styling (default/`--warning-color`/`--error-color` states + pulse
  keyframe), `--recommended` option highlight class.
- `frontend/app/view/agent/hooks/useAgentQuestions.ts` — consume
  `outcome.autoFilledCount` when building the transcript summary text
  (~line 123).
- `frontend/app/view/agent/components/AgentQuestionPanel.test.tsx` — new
  test cases per §7 (file already exists — see current Enter/Escape
  keyboard-handling tests for the mount/mock conventions to follow).
- **No `agentmux-srv` (Rust) changes.** No `agentmux-common` changes. No new
  RPC command, no new field on any wire struct that crosses the process
  boundary.

---

## 4. Edge cases & decisions

| Case | Behavior |
|---|---|
| No option anywhere marked `(Recommended)` | Falls back to `options[0]` per question (documented model convention) |
| Multi-select, no flagged options | Auto-select `options[0]` only — never zero |
| Multi-select, several flagged options | Auto-select all of them |
| User answers question 1 of a 2-question set, never touches question 2, timer hits 0 | Question 1 keeps the user's answer untouched; question 2 gets the recommended default; both submit together as one outcome with `autoFilledCount: 1` (see §5.1) |
| Panel minimized, never touched | Timer keeps running in the background; auto-submits at 0; panel disappears (queue advances, or closes if empty) |
| Queue has 3 pending question-sets | Only the head is shown/timed; each becomes head in turn and gets its own fresh 30s window |
| User submits manually with 1s left on the clock | Manual `submit()` simply runs first — the resulting `tool_use_id` change tears down the interval via `onCleanup`, so there's no race |
| Pane closed while a question is pending | Existing `onCleanup` (unrelated to this change) already handles teardown; the timer's own `onCleanup` clears alongside it |
| Timer fires while user is mid-keystroke in an "Other" field | Whatever text is currently in the reactive `state()` signal at that instant counts as "answered" (per `questionAnswered()`) and is kept as-is — no special-casing needed, since `setOther` already updates `state()` on every keystroke |

---

## 5. Resolved design decisions

Originally left open for review; resolved on 2026-08-06 before implementation.

1. **Per-panel disarm vs. per-question merge — resolved: merge, no disarm.**
   The first draft of this spec disarmed the *entire* panel's timer on the
   first interaction with *any* question in a multi-question set. Rejected:
   it directly contradicts the spec's own stated goal. If a human answers
   question 1 of 2 and then gets pulled away — the exact "human isn't
   watching anymore" scenario this feature exists to handle — a full disarm
   would cancel the safety net entirely and question 2 could block forever,
   reproducing the original bug for anyone who partially engages before
   leaving.

   **Resolution:** the timer is never disarmed. It always fires at 30s. At
   zero, questions the user already answered (fully or even partially — any
   selection or non-empty "Other" text) are left exactly as they are;
   only genuinely untouched questions get the recommended default; the
   merged result is what submits. This is also the simpler implementation —
   it removes the need for a `disarmed` signal and any interaction
   listeners, since the merge logic only ever reads `state()` at the moment
   the interval fires, and that signal is already kept live by the existing
   `toggleOption`/`setOther` setters. See §2.3, §4.

   One accepted tradeoff: a human who is actively deciding between options
   (clicking around, still undecided) with less than 30s left gets cut off
   at exactly 30s same as someone who never showed up. This is a narrow edge
   case and is the literal reading of the ask ("if the user does not respond
   by zero, the recommended is auto selected") — the countdown UI (§2.4)
   exists specifically to make that deadline visible in advance rather than
   a surprise.

2. **Timeout duration configurability — resolved: non-goal, hardcoded.**
   `AUTO_TIMEOUT_MS = 30_000` as a plain constant, no settings toggle, per
   the ask. Confirmed as out of scope for this pass (§6); a per-user or
   per-agent override is a reasonable follow-up if requested later, but
   nothing here blocks adding one afterward — it's a single constant with
   one obvious injection point (a settings-store read where it's currently a
   literal).

3. **SCSS token values for the urgency escalation — resolved: specified.**
   No longer "left to implementation-time judgment." §2.4 now specifies
   exact thresholds and tokens: `--secondary-text-color` above 10s,
   `--warning-color` at ≤10s, `--error-color` with a 1s opacity-pulse
   keyframe at ≤5s, `transition: color 120ms ease`. Chosen to reuse existing
   app-wide severity tokens (confirmed present via `--warning-color` in
   `app/block/block.scss` and `--error-color` in `app/errors/ErrorBanner.scss`)
   rather than introduce new ones, and to keep the pulse subtle (opacity
   only, no color or size change in the keyframe) so it signals urgency
   without being distracting.

---

## 6. Non-goals

- `AgentDecisionPanel` (tool-permission "always ask" prompts) — no
  auto-timeout added here; separate risk profile, separate spec if wanted.
- Any backend/Rust change. The answer is delivered through the exact same
  control-protocol / follow-up-message paths that already exist today.
- A user-configurable timeout duration or a settings toggle to disable the
  feature entirely.
- Signaling "this was auto-selected" to the model itself over the wire — only
  the AgentMux-local transcript is annotated (§2.5); the CLI/model sees a
  normal answer, indistinguishable from a human one.

---

## 7. Test plan

**Unit** (`AgentQuestionPanel.test.tsx`, extending the existing
`@solidjs/testing-library` + `vitest` setup):

- `recommendedOptions()`: returns the flagged option(s) when one or more
  labels end in `(Recommended)` (case-insensitive); falls back to
  `[options[0]]` when none are flagged; returns `[]` for an empty options
  array.
- Using `vi.useFakeTimers()`: a fully-idle panel auto-fires `onAnswer` at
  exactly 30s with the expected `answers_map` (single-select → the
  recommended label; multi-select with flags → all flagged labels;
  multi-select with no flags → `[options[0]]`); `autoFilledCount` equals the
  full question count.
- **Merge case:** a 2-question set where the user answers question 1 only
  (via `toggleOption`), then fake-advance past 30s — question 1's chosen
  label survives unchanged in the submitted outcome, question 2 gets the
  recommended default, `autoFilledCount === 1`.
- **Manual-submit-wins case:** answer every question and call `submit()`
  manually before 30s — advancing fake time past 30s afterward fires no
  second submit (the interval was torn down by the `tool_use_id` change).
- The countdown value decrements once per simulated 1000ms tick and reaches
  exactly 0 at the 30s mark (no off-by-one).
- `useAgentQuestions.ts`'s summary-building picks the right prefix for each
  of the three `autoFilledCount` bands (`0`, partial, full) — small addition
  to that hook's own existing tests, if any (confirm exact file at
  implementation time).

**Manual / integration:**

- `task dev`, trigger a live `AskUserQuestion` call, leave the panel fully
  untouched. Confirm: (a) the countdown renders and ticks down correctly in
  both the full panel and the minimized chip; (b) color escalates at the
  10s/5s thresholds; (c) at 0 the recommended option(s) submit automatically
  and the agent's turn resumes without further input; (d) the transcript
  line shows the `⏱️ Auto-answered` marker, distinguishable from a normal
  answer.
- Repeat with a multi-question panel: manually answer one question shortly
  after it opens, leave the other(s) untouched. Confirm the countdown keeps
  running (it is never dismissed by the interaction), and at 0 the manually
  answered question keeps its answer while the rest fill in with the
  recommended default, submitted together as one outcome — transcript line
  shows the partial marker (§2.5), not the full-auto one.
- Repeat once more, answering *every* question manually before 0 — confirm
  submission happens immediately on click with no further wait, and the
  transcript line shows the normal (non-auto) marker.
