# RCA: the question auto-timeout never fires for an agent in a pane that isn't visible

**Status:** analysis. Written 2026-10-17; the code reading is checked against `main` at 8525dbdb0. Nothing is changed yet.
**Author:** Camper@narko, at the operator's request
**Trigger (operator, 2026-10-17):** "I notice that the auto does not select in cases where the agent asks the question from a non-visible pane. The timeout should trigger regardless of if the agent is visible or not."
**Related:** [`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`](../specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md) (the feature and its goal), [`SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md`](../specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md) §2 and §3 (where the pause was added), [`SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md`](../specs/SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md) (the shared timer), [`SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`](../specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md) (the visibility signal)

## 1. Summary

An agent that asks a question (`AskUserQuestion`) while the user can't see its pane waits indefinitely: the 30-second auto-select never starts. This is deliberate code, not a race: when keep-alive was added to agent panes (#3391), the timer was told to pause for as long as the pane is hidden, so that a question the user was never shown couldn't be answered for them. That rule was widened in Phase 3 of the pane tab contract to cover a window tab that isn't displayed. The two rules together contradict the feature's own goal ("a pending question can never stall work forever", auto-timeout spec §1), and the operator now wants the timeout to run whether or not the pane is visible.

The timer also lives entirely in the renderer. srv parks the question and has no timer of its own, so a hidden pane is only the most common way to reach the gap; section 4 lists the others.

## 2. Observed

- A visible agent's question auto-selects after 30 s (checked in a dev build on 2026-10-16: the recommended option was submitted with no one touching the pane).
- An agent in a pane the user can't see keeps its question open. The Swarm row's chip shows **"question · paused"**, not a countdown, which is the dormant state below.

## 3. Root cause

### 3.1 The chain from "hidden" to "no timer"

1. `usePaneTabVisibility(blockId)` (`frontend/app/block/pane-tab-visibility.ts`) returns `"windowHidden"` when the pane's window tab isn't displayed, else `"dormant"` when it is a kept-alive pane-stack member that isn't its pane's active tab, else `"active"`.
2. `AgentView` turns that into `hidden = () => visibility() !== "active"` (`agent-view.tsx:155`) and passes it down as `isDormant` (`agent-view.tsx:1046`, then `AgentBottomPanels.tsx:132`).
3. `AgentQuestionPanel` uses it in two places (`AgentQuestionPanel.tsx:369-378`):
   - when a question becomes the head of the queue, `startQuestionTimer(..., { dormant: <current value> })`;
   - a standing effect calls `setQuestionTimerDormant(timerKey, isDormant())` on every change.
4. In `question-timer.ts`, `arm()` checks `owner.dormant` first (`:169-173`). When it is set it records `{ kind: "paused", reason: "dormant" }` and returns **without scheduling the expiry `setTimeout`**. `setQuestionTimerDormant` re-runs `arm()` on every flip (`:218-223`): hidden stops the timer, visible starts a **fresh full 30 s**.

So the timer isn't running and late; there is no timer at all until the pane is shown. The comment on `AgentQuestionPanel` says so: "unbounded, since there's no way for a user to 'interact' with a tab they can't see" (`:33-36`).

### 3.2 Why it was written that way

Before keep-alive, switching away from an agent tab **unmounted** its panel, which implicitly stopped the timer. Keep-alive (#3391, `SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md` §2) removed that implicit stop, and the spec named the consequence it was avoiding: the timeout "would keep counting down invisibly and auto-answer a question the user was never shown." The fix preserved the old behaviour by pausing explicitly. The same reasoning is applied to `useAgentFailure`'s auto-retry (`useAgentFailure.ts:257`, "a hidden tab never auto-resends a turn nobody can see").

Phase 3 of the pane tab contract (`pane-tab-visibility.ts`) then folded "window tab not displayed" into the same signal, so a pane in a window tab you aren't looking at (all window tabs stay mounted, `workspace.tsx:189`) became dormant too. The comment in `agent-view.tsx:148-154` says this was intended: neither timer "fires invisibly while backgrounded".

### 3.3 Why this is a defect, not only a design choice

- The feature exists so that "work does not stop" (`AgentQuestionPanel.tsx:347-356`, auto-timeout spec §5.1), and the people it protects most are the ones running several agents who are looking at one of them. An agent in the background is exactly the unattended case.
- The earlier design decisions in that spec refuse to let user interaction cancel the timeout, for the same reason ("a user who answers one question ... and then steps away would otherwise cancel the safety net entirely"). Hiding the pane is the same kind of absence and is treated the opposite way.
- The pause is unbounded and silent. The only sign is "question · paused" on the Swarm chip, which reads as a pause for the user's benefit, not as "this will never be answered".

## 4. Related gaps in the same design

These share the cause (the timer is a renderer-side, visibility-gated mechanism) and should be decided together with the fix.

| Case | What happens today | Evidence |
|---|---|---|
| Pane in a hidden pane-stack tab | Paused, no expiry scheduled | `question-timer.ts:169-173` |
| Pane in a window tab that isn't displayed | Paused, same | `pane-tab-visibility.ts`, `agent-view.tsx:155` |
| Becoming visible again | Starts a **fresh** 30 s, however long the question waited | `question-timer.ts:218-223` |
| No renderer holds the pane | No timer exists: the owner is the renderer that mounts the panel | `question-timer.ts:10-13` ("Only the owner schedules the expiry"); a crashed or closed owner leaves only a stale published copy (`:278-282`) |
| srv | Parks the question (`"AskUserQuestion parked; awaiting user answer"`, `persistent/input.rs`) and has no timeout | `git grep` of `crates/` finds none for questions |
| Same rule for auto-retry after a failure | Pauses while hidden, so a failed turn in a hidden pane isn't retried either | `useAgentFailure.ts:257` |

The no-renderer row is not verified end to end here; it follows from where the timer is owned.

## 5. Fix options

**A. Stop gating the timer on visibility (smallest).** Remove the `isDormant` input from the question timer: `arm()` always schedules the expiry, and `setQuestionTimerDormant` goes away. A hidden pane's countdown keeps running and the Swarm chip shows it (`question 23s`) instead of "paused". Activity still defers it for the visible pane only. Fixes the reported case; leaves the no-renderer gap.

**B. Move the timer to srv.** srv already knows when a question is parked and when it is answered. It starts the timer at park, and on expiry sends the same recommended-defaults answer through the existing answer path. The renderer then only displays the countdown (from the published state it already reads) and reports activity to pause it. Fixes every row of section 4 and makes "once per question" true by construction, but needs srv to know the recommended option, which today is chosen in the panel (`applyRecommendedDefaults`).

**C. Keep a bounded pause.** Pause while hidden for a limit (say 5 to 10 minutes), then run the timer. Smaller change in intent than A, but it keeps the surprise the operator described, and the number is arbitrary.

Recommendation: **A now, B as the follow-up.** A is a few lines plus the test that asserts the pause, and it matches what the operator asked for. B is the version that cannot be bypassed by where a pane happens to be, and it also covers the auto-retry's sibling gap if that is wanted.

Decisions for the operator:
1. **Fresh versus continuing countdown on reveal.** Under A there is nothing to restart. If the timer has already expired while hidden, the panel simply isn't there when the user looks.
2. **The auto-retry after a failure** has the same gate. Change it too, or leave it?
3. **What the user sees afterwards.** A hidden pane's question that was auto-answered shows the existing "auto-selected" note in the transcript. Consider also a Swarm or notification hint, since the user wasn't looking.

## 6. Tests the fix needs

- Unit (`question-timer.test.ts`): a timer started while dormant still expires; a flip to dormant no longer pauses it; activity still defers it.
- Panel (`AgentQuestionPanel`): `isDormant = true` at mount, `onExpire` fires after the duration with fake timers.
- Swarm chip: a hidden pane shows a counting state, not "paused".
- Live, in a dev build: ask a question from a pane in a background pane tab and in a background window tab; the recommended option is submitted after the timeout and the agent continues.
