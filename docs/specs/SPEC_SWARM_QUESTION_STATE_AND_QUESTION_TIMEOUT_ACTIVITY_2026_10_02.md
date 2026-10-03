# SPEC: Swarm "question" state and countdown, and activity that resets a question's timeout

**Status:** implemented (korp/swarm-question-timer). The two open questions are decided (§8).
**Date:** 2026-10-02
**Author:** korp
**Trigger:** Repo owner, 2026-10-02: *"we simply want the working/idle extended with "question" (use the theme primary color) when an agent is currently asking a question. Also, piggyback a fix on that, question panel in the agent pane should reset their timeout on both mouse movement AND typing (currently it appears to be only for mouse movement)"*, and *"also, in the swarm, put a countdown that mirrors the one on the question panel"*; *"ideally its a single DRY path, one timer surfaced multiple places"*; *"also, take any other opportunities to DRY the code in the area you are working in"* (§5).

**Builds on:**
- `SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md` §5.4: the `term:awaiting_user` block-meta flag and the "Waiting for you" row line (#4234).
- `SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`: the auto-timeout and its "merge, never disarm" rule (§5.1), and the countdown's colour bands (§2.4).
- `SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md` and `SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md`: the bounded hover/keyboard pause, which §4 changes.

**Coordination:** AgentX owns ongoing Swarm work: the multi-host rows (#4239, merged) and LAN hosts (#4241, open, backend only). This spec touches only the agent row's status chip (`swarm-view.tsx` `AgentStatusChip`/`chipStatus`, `swarm-view.scss` `.swarm-status-chip`), the question panel (and the decision panel's shared key plumbing, §5), and one new module (`frontend/app/store/question-timer.ts`). In `swarm-line.ts` (AgentX's #4234) it changes one line: the "waiting" condition moves into a helper both readers use (§5.3). It doesn't change what the row line says, or anything multi-host. Remote rows (another channel or host) don't get the countdown: their block meta isn't this window's (§2.2).

## 1. Summary

| # | What | Where |
|---|---|---|
| 1 | **One question timer** per pending question, in one module: its state, its tick, its activity rule, its auto-fire, and how it reads ("23s", the colour band, "paused"). Everything that shows a countdown reads it from there | `store/question-timer.ts` (new), §2 |
| 2 | The Swarm agent chip gets a third state, **question**, in the theme's primary colour, and shows that timer's countdown | `swarm-view.tsx`, `swarm-view.scss`, §3 |
| 3 | The timer **resets on any real activity**: mouse movement over the panel and typing anywhere in that agent's pane | the module's activity rule; the panel reports activity, §4 |
| 4 | **Other duplication in this area** folded into one place each: the editable-target check, the panels' pane-scoped key listener, and the "waiting on the user" condition | §5 |

## 2. One question timer

Today the countdown is a signal local to `AgentQuestionPanel` (`remainingMs`, a 1s `setInterval`, the `hidden()` pause, the auto-fire). Mirroring it into Swarm by publishing a copy would make two countdowns that can disagree. Instead the timer moves out of the panel into one module, and the panel becomes one of its readers.

### 2.1 `frontend/app/store/question-timer.ts`
Per agent block, at most one timer: the pending question's.

```ts
type QuestionTimerState =
    | { kind: "counting"; endsAt: number }      // epoch ms when it auto-selects
    | { kind: "paused"; reason: "activity" | "dormant" }
    | null;                                     // no question pending here
```

**Owner side**, called only by the agent pane that holds the question:
- `startQuestionTimer(blockId, { durationMs, onExpire })` when a question becomes the panel's head (and again for each next question in the queue). It replaces `remainingMs` and the timer effect.
- `noteQuestionActivity(blockId)` on activity (§4.2). The state becomes `paused/activity`, and the quiet timer (re)starts. When the quiet timer fires, the state becomes `counting` with a fresh `endsAt = now + durationMs`.
- `setQuestionTimerDormant(blockId, dormant)` for the agent pane tab going to the background and back (`isDormant`, today's unbounded pause). Waking re-arms at the full duration, as today.
- `endQuestionTimer(blockId)` on answer, cancel, or the panel unmounting.

The module owns the single real timer: one `setTimeout` to `endsAt`, which calls `onExpire` (the panel's `submit(applyRecommendedDefaults())`). Only the owner's renderer schedules it, so a question can't be auto-answered twice.

**Reader side**, used by the panel and by Swarm alike:
- `questionTimer(blockId)`: the state, reactive. In the owner's renderer it's the local entry; in any other renderer (another window's Swarm) it's the published copy (§2.2).
- `questionCountdown(blockId)`: `{ seconds, band: "default" | "warning" | "critical", paused } | null`. `seconds = Math.ceil((endsAt - now) / 1000)`, and the bands are the panel's today (≤10 warning, ≤5 critical, auto-timeout spec §2.4). It ticks from one shared 1s clock signal per renderer, running only while some timer is counting.

The panel's "Auto-selects recommended in 23s" and the Swarm chip's "question 23s" both render `questionCountdown()`. That's the one timer surfaced in two places; a third reader (the OS notification, the tab badge) would read the same.

### 2.2 Publishing across windows
Swarm can run in another window, which is another renderer, so the owner also writes the state to its block's meta under `term:question_timer` (same shape, `null` removes the key). Readers outside the owner's renderer read it from there.
- Written on edges only: start, pause, resume, end. Never per tick. Activity while already paused changes nothing and writes nothing.
- `null` on end and on unmount. Unlike `term:awaiting_user`, the question is still pending after an unmount, but nothing is counting any more.
- Same machine, same clock: every window and the owner share `Date.now()`, so `endsAt` reads the same everywhere.
- Remote rows (another channel or host) never have this key in this window's store; they show plain `question` (§3.3).
- **Stale values.** A renderer crash or a closed window skips the unmount, so the key can outlive its owner. Three rules keep that from showing:
  1. `questionCountdown` returns no state for a published `counting` whose `endsAt` has passed, so the chip shows plain `question`, never `0s` or a negative.
  2. Readers use the key only while §3.2's question condition holds (turn in flight and `term:awaiting_user`). Once the turn ends, a leftover `paused` or `counting` is ignored, the same guard §5.3 keeps for `term:awaiting_user`.
  3. The owner always writes the key when its panel mounts: the live state if a question is pending, `null` otherwise. A reload or a restored window therefore overwrites whatever the crash left behind. A closed window takes its blocks with it, so no Swarm row is left reading the key.

## 3. Swarm: the "question" chip and its countdown

### 3.1 Today
`AgentStatusChip` collapses the finer `AgentDisplayStatus` to two states (repo owner, 2026-09-27): **working** (red, pulsing dot) while a turn is in flight, **idle** (green) otherwise. An agent waiting on a question has a turn in flight, so it reads **working**: the one state someone has to act on looks like the agent is busy.

### 3.2 The third state
- `ChipStatus` becomes `"working" | "question" | "idle"`.
- **question** when the row's agent has a turn in flight (`node.agentStatus === "running"`, or the mounted pane's phase maps to working/tools) **and** its block meta has `term:awaiting_user === true`. This is the condition `resolveSwarmLine` already uses for "Waiting for you", including its guard: the flag counts only while a turn is in flight, so one left behind by a crash can't outrank idle.
- Precedence: question over working over idle.
- Label `question`. Colour `var(--accent-color)` (the theme's primary colour; every theme defines it). The dot pulses like working's (repo owner, 2026-10-02: "just like working and idle, a strobing dot, but the color is the theme primary"); the colour is what tells it apart.
- Subagent rows are unchanged: a subagent never asks the user; its parent does.
- `chipStatus(status)` becomes `chipStatus(status, awaitingUser)`; the row computes `awaitingUser` with the same helper the row line uses (§5.3), from the `blockMeta()` it already reads.

### 3.3 The countdown
The chip reads `questionCountdown(node.blockId)` (§2.1); it has no timer of its own.
- Counting: `question 23s`, with the band's colour on the seconds (`--warning-color` at 10s and below, `--error-color` at 5s and below). The word keeps the primary colour.
- Paused (the user is active in the panel, or the pane tab is in the background): `question · paused`.
- No timer state (the pane isn't mounted anywhere, so nothing is counting, or a remote row): plain `question`.

## 4. The question panel: activity resets the timeout

### 4.1 Today (`AgentQuestionPanel.tsx`)
Two triggers pause the countdown, sharing one mechanism (`onPanelPointerEnter`):
- **`mouseenter` on the panel**: crossing into it, not moving within it.
- **A `keydown` whose target is inside the panel** (an option, the "Other" box), plus `focusin` into it.

Either one hides the countdown for a **flat 15s from the trigger** (`HOVER_HIDE_GRACE_MS`) and then resumes at a fresh full timeout, whatever the user does during those 15s. Activity inside the window is ignored (`maybePauseFor` gates on `!hidden()`, reagentx P1 on #2787). Typing anywhere else in the pane, such as the composer where a user often writes their answer or a follow-up, doesn't count at all.

So what the repo owner sees: moving the mouse into the panel visibly hides the timer, but typing doesn't hold it. In the composer it does nothing. In the panel the countdown comes back 15s later mid-sentence, because activity inside the window doesn't extend it.

### 4.2 New rule
**Any real activity restarts the quiet window.** The countdown stays hidden while the user is active, and resumes at a fresh full timeout 15s after the *last* activity.

Activity is:
- **`pointermove` over the panel** whose coordinates differ from the last one seen. A real move, not the synthetic `mouseenter` a click produces, and not a parked cursor.
- **A `keydown` anywhere in this agent's pane**: the panel and the composer (`eventBelongsToPaneOf`, already used for Enter/Escape), **except auto-repeat** (`e.repeat`). Holding a key is not activity.
- `focusin` into the panel, as today.

### 4.3 Why this doesn't reopen the "work never stops" failure
The 15s cap (hover-pause §9, keyboard-pause §8) was there because the triggers it bounded could fire with nobody there:
- a click's own `mouseenter` left a cursor parked over the panel forever;
- a held key's auto-repeat fired keydown on every repeat.

The new triggers exclude both: a parked cursor makes no `pointermove`, and auto-repeat is ignored. What's left needs a person moving the mouse or pressing keys. While they do, the question isn't abandoned; once they stop, the full countdown runs and auto-selects as before. The "merge, never disarm" rule (auto-timeout §5.1) is unchanged. A question can still be answered by the timeout after the user answered part of it, because activity only defers the timeout, it never cancels it.

### 4.4 Mechanism
The rule lives in the module (§2.1), not the panel:
- `noteQuestionActivity()` replaces the flat `HOVER_HIDE_GRACE_MS` window with a **quiet timer**. Each call clears and restarts a 15s timer, and the state is `paused/activity` until it fires; then the countdown re-arms at the full duration.
- The panel only detects activity and reports it: `pointermove` with changed coordinates over its root, the pane-scoped `keydown` listener it already has (minus `e.repeat`), and `focusin`.
- `mouseenter` stops being a trigger on its own. The first `pointermove` that follows it is one, so hovering still pauses as it does today.
- The panel's `remainingMs`, its interval, `hidden()` and `hideTimeoutId` go away. It renders `questionCountdown()` (hidden while paused, as the countdown is hidden while paused today).

## 5. Other duplication in this area

### 5.1 One editable-target check
`isEditableTarget` is written out twice: once in `AgentQuestionPanel.tsx` and once in `AgentDecisionPanel.tsx`, with identical bodies (`INPUT`/`TEXTAREA`/contentEditable). It moves to `util/focusutil.ts` next to `eventBelongsToPaneOf`, and both panels import it. Checks elsewhere that differ on purpose stay as they are: `composer-focus.ts` also counts `SELECT`, and `userCaretInBlock` excludes non-text inputs.

### 5.2 One pane-scoped key listener for the panels
Both panels install the same plumbing while a request is pending:
- a capture-phase `window` `keydown` listener (the panel is `tabindex=-1` and never focused, codex P1 on #556);
- an `eventBelongsToPaneOf` scope check;
- the `inPanel` / `isEditableTarget` reads the Enter and Escape rules depend on.

It becomes one hook, `usePanelKeys(rootRef, active, onKey)` in `view/agent/components/use-panel-keys.ts`. The hook calls `onKey(e, { inPanel, editable })` only for keys from this pane, and the question panel's `focusin` rides along as an option. Each panel keeps only its own key rules: Enter submits, Escape cancels or minimizes, the decision panel's feedback box. §4.2's pane-wide typing activity is one more rule in the question panel's `onKey` (`!e.repeat` → `noteQuestionActivity`).

### 5.3 One "waiting on the user" condition
`resolveSwarmLine` decides "Waiting for you" with `status === "running" && meta[term:awaiting_user] === true`, and the chip's question state (§3.2) needs exactly that. It becomes `isAwaitingUser(meta, status)`, exported from `swarm-line.ts` next to `META_AWAITING_USER`, and both call it. A one-line change in AgentX's file, made in its own place rather than copied; AgentX agreed on 2026-10-02. The helper keeps the "only while `status === "running"`" guard. That guard is what stops a flag left by a crash from showing on an idle agent (ReAgent P1 on #4234), and `swarm-line.test.ts`'s "ignores a waiting flag once the agent has no turn in flight" pins it; the chip gets the same guarantee by calling the helper.

### 5.4 What the timer module absorbs
Besides the countdown itself (§2), the panel's `countdownSeverity()` thresholds and `countdownSeconds()` rounding move into `questionCountdown()`. Its hide/resume timers (`hideTimeoutId`, `clearHideTimer`, `onPanelPointerEnter`) are replaced by the module's quiet timer. Nothing about the countdown's timing stays in the view.

## 6. Tests
- **Timer module** (`question-timer.test.ts`, fake timers):
  - start → counting with `endsAt`;
  - expiry calls `onExpire` exactly once;
  - activity → paused, then counting at the full duration 15s after the last activity;
  - activity while paused restarts the quiet timer and writes nothing;
  - dormant pauses and waking re-arms;
  - end and unmount clear the state and the published key;
  - edges-only meta writes;
  - a non-owner renderer reads the published copy and never schedules an expiry;
  - `questionCountdown` seconds and bands;
  - stale values (§2.2): a published `counting` past its `endsAt` reads as no state; a leftover key is ignored once the turn ends; mounting the panel overwrites a leftover key.
- **Chip**: `chipStatus` for working + flag → question; idle + stale flag → idle; subagent rows unchanged. A render test for the label, `--accent-color` class, pulsing dot, countdown text, the warning and critical bands, `paused`, and no countdown without timer state.
- **Panel**:
  - typing in the composer pauses the countdown;
  - continuous typing (keydowns 2s apart for 60s) never lets it fire, and it fires a full timeout after the last key;
  - auto-repeat keydowns don't extend the pause;
  - `pointermove` with a changed position extends it, and the same position doesn't;
  - a click (`mouseenter` without movement) followed by stillness lets it resume after 15s (the #2787/§9 regression stays covered);
  - the panel's countdown and the chip's show the same seconds from the one timer;
  - the existing auto-timeout, merge and dormant tests still pass against the module.
- **DRY pieces**: `isEditableTarget` unit tests in `focusutil`; `usePanelKeys` (pane scoping, `inPanel`/`editable`, cleanup when the request ends); `isAwaitingUser` (running + flag, idle + stale flag). The existing decision-panel and swarm-line tests pass unchanged.
- **Live check** on a dev build: an agent asks a question; the Swarm chip shows `question` counting down in step with the panel; typing in the composer shows `question · paused`; stopping resumes both at the full timeout.

## 7. Rollout
One PR: the timer module, the panel moved onto it with the activity rule, the Swarm chip reading it, and §5's consolidations (they touch the same files). They share one timer, so they land together. On merge, the hover-pause spec's §9, the keyboard-pause spec's §8 and the auto-timeout spec get a pointer here (the countdown moved to the module, and the flat window is replaced for the reasons in §4.3).

## 8. Decisions
Both decided by the repo owner on 2026-10-02.
1. **Composer typing counts.** §4.2 counts keystrokes anywhere in the agent's pane, the composer included, since typing a reply is attending to the question. The keyboard-pause spec had excluded the composer ("not engagement with this question"); this replaces that.
2. **The dot pulses**, like working's, in the theme's primary colour (§3.2).
